//! Password-backed compensation on an explicitly acknowledged disposable host.
use super::*;
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};
use windows_task::{
    Password, TaskPath,
    client::BlockingScheduler,
    manifest::{ManagedTask, TaskManifest},
    reconcile::{
        ApplyOptions, Change, CredentialPurpose, apply, apply_with_credentials, plan_live,
    },
};

#[test]
#[ignore = "creates a temporary local account; acknowledged GitHub-hosted CI only"]
fn password_backed_native_update_is_restored_after_authentication_failure()
-> windows_task::Result<()> {
    if !history_gap::approved_host(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        std::env::var("RUNNER_ENVIRONMENT").ok().as_deref(),
        std::env::var("WINDOWS_TASK_ACCOUNT_TESTS").ok().as_deref(),
    ) {
        return Err(Error::new(
            ErrorKind::Other,
            "account fixture requires acknowledged GitHub-hosted CI",
        ));
    }
    let username = format!("wt{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
    let password = zeroize::Zeroizing::new(format!("Wt!{}7a", uuid::Uuid::new_v4().simple()));
    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let folder = FolderPath::root()
        .join(&format!(
            "windows-task-ci-password-{}",
            uuid::Uuid::new_v4()
        ))
        .expect("fixture folder");
    let paths = [
        folder.task("a-first").expect("first task"),
        folder.task("z-second").expect("second task"),
    ];
    let outcome = (|| {
        account(&username, Some(&password))?;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            exercise_recovery(&blocking, &folder, &paths, &username, &password)
        }))
        .unwrap_or_else(|_| {
            Err(Error::new(
                ErrorKind::Other,
                "password recovery assertion failed; cleanup follows",
            ))
        })
    })();
    let outcome = cleanup_result(
        outcome,
        [
            blocking.delete_task(&paths[0]),
            blocking.delete_task(&paths[1]),
            blocking.delete_folder(&folder),
            account(&username, None),
        ],
    );
    cleanup_result(outcome, [scheduler.shutdown(Duration::from_secs(10))])
}

fn exercise_recovery(
    blocking: &BlockingScheduler,
    folder: &FolderPath,
    paths: &[TaskPath; 2],
    username: &str,
    password: &str,
) -> windows_task::Result<()> {
    let mut original = TaskManifest::new(uuid::Uuid::new_v4(), "password-fixture", folder.clone());
    for path in paths {
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("cmd.exe")));
        definition.settings.enabled = false;
        let machine = std::env::var("COMPUTERNAME")
            .map_err(|_| Error::new(ErrorKind::Other, "fixture computer name unavailable"))?;
        definition.principal.identity = PrincipalIdentity::User(format!("{machine}\\{username}"));
        definition.principal.logon_type = LogonType::Password;
        original.tasks.push(ManagedTask {
            path: path.clone(),
            definition,
            credentials: Default::default(),
        });
    }
    let mut valid =
        |_: &TaskPath, _: Option<&str>, _: CredentialPurpose| Ok(Password::new(password));
    apply_with_credentials(blocking, &original, ApplyOptions::default(), &mut valid).map_err(
        |failure| {
            // This fixture constructs all settings itself. Capture only these
            // typed settings and the logon mode, never credentials or actions.
            let observed = blocking
                .get_task(&paths[0])
                .and_then(|task| {
                    let definition = task.snapshot.definition()?;
                    Ok(format!(
                        "settings={:?}; logon_type={:?}",
                        definition.settings, definition.principal.logon_type
                    ))
                })
                .unwrap_or_else(|error| {
                    format!(
                        "unavailable: kind={:?}; native_code={:?}",
                        error.kind(),
                        error.native_code()
                    )
                });
            let journal: Vec<_> = failure
                .report
                .journal
                .iter()
                .map(|entry| (entry.phase, entry.outcome))
                .collect();
            failure
                .cause
                .with_context("fixture_stage", "initial_registration")
                .with_context("fixture_observed_settings", observed)
                .with_context("fixture_journal", format!("{journal:?}"))
        },
    )?;
    let mut desired = original.clone();
    for task in &mut desired.tasks {
        task.definition.registration.description = Some("requested replacement".into());
    }
    let missing = apply(blocking, &desired, ApplyOptions::default())
        .expect_err("missing registration password");
    assert!(
        !missing.report.succeeded(),
        "preflight failure cannot report success"
    );
    assert!(
        missing.report.applied.is_empty(),
        "credential preflight prevents mutation"
    );
    assert!(
        plan_live(blocking, &original, Default::default())?.is_empty(),
        "preflight must leave definitions unchanged"
    );
    let mut purposes = Vec::new();
    let mut failing = |path: &TaskPath, _: Option<&str>, purpose: CredentialPurpose| {
        purposes.push((path.clone(), purpose));
        Ok(Password::new(
            if path == &paths[1] && purpose == CredentialPurpose::DesiredRegistration {
                "Deliberately-wrong-fixture-password!7"
            } else {
                password
            },
        ))
    };
    let failure = apply_with_credentials(blocking, &desired, ApplyOptions::default(), &mut failing)
        .expect_err("second native registration must reject the wrong password");
    assert_eq!(
        failure.cause.kind(),
        ErrorKind::Authentication,
        "native password rejection must retain its classification"
    );
    assert!(
        failure.cause.native_code().is_some(),
        "native password rejection retains HRESULT"
    );
    assert!(
        failure
            .report
            .applied
            .contains(&Change::UpdateTask(paths[0].clone())),
        "first update must have committed before second registration failed"
    );
    assert!(
        failure
            .report
            .rolled_back
            .contains(&Change::UpdateTask(paths[0].clone())),
        "first update must be compensated with its backup credential"
    );
    assert!(
        failure.report.rollback_failures.is_empty(),
        "credential-backed restoration must succeed"
    );
    assert!(
        failure.report.unresolved.is_empty(),
        "all task states must be confirmed"
    );
    assert!(
        purposes.contains(&(paths[0].clone(), CredentialPurpose::Rollback)),
        "restoration must request a dedicated backup credential"
    );
    assert!(
        plan_live(blocking, &original, Default::default())?.is_empty(),
        "native definition restored"
    );
    let report = serde_json::to_string(&failure.report).expect("structured report");
    assert!(
        !report.contains(password),
        "report must not serialize the secret"
    );
    eprintln!(
        "fixture password recovery kind={:?} native_code={:?} applied={} restored={} unresolved={}",
        failure.cause.kind(),
        failure.cause.native_code(),
        failure.report.applied.len(),
        failure.report.rolled_back.len(),
        failure.report.unresolved.len()
    );
    apply_with_credentials(blocking, &desired, ApplyOptions::default(), &mut valid)
        .map_err(|failure| failure.cause)?;
    let repeated = apply_with_credentials(blocking, &desired, ApplyOptions::default(), &mut valid)
        .map_err(|failure| failure.cause)?;
    assert!(
        repeated.succeeded() && repeated.plan.is_empty(),
        "valid retry is idempotent"
    );
    Ok(())
}

fn account(username: &str, password: Option<&str>) -> windows_task::Result<()> {
    // The username is generated locally from a UUID. The secret only crosses
    // stdin, never argv, environment, an artifact, or captured subprocess output.
    account_process(
        if password.is_some() {
            "create"
        } else {
            "remove"
        },
        username,
        password,
    )
}

fn account_process(mode: &str, username: &str, password: Option<&str>) -> windows_task::Result<()> {
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/windows_smoke/account.ps1"
            ),
            "-Mode",
            mode,
            "-Name",
            username,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::new(ErrorKind::Other, "cannot start account fixture process"))?;
    let input = (|| {
        let mut stdin = child.stdin.take().expect("piped input");
        if let Some(password) = password {
            stdin.write_all(password.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        Ok::<_, std::io::Error>(())
    })();
    let output = child.wait_with_output().map_err(|_| {
        Error::new(
            ErrorKind::Other,
            "account fixture process status unavailable",
        )
    })?;
    if input.is_err() || !output.status.success() {
        let mut error = Error::new(ErrorKind::Other, "account fixture operation failed")
            .with_context("exit_code", format!("{:?}", output.status.code()))
            .with_context("stage", mode);
        // Parse only the controlled catch block's allowlisted metadata. Never
        // attach raw stdout/stderr or PowerShell exception messages.
        if let Ok(evidence) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            for field in ["hresult", "category", "error_type", "native_code", "phase"] {
                if let Some(value) = evidence.get(field) {
                    error = error.with_context(field, value.to_string());
                }
            }
        }
        return Err(error);
    }
    Ok(())
}

#[test]
fn account_process_error_metadata_never_captures_the_secret() {
    let error = account_process("probe", "unused", Some("SENTINEL_STDIN_SECRET"))
        .expect_err("controlled subprocess failure without account mutation");
    assert!(error.context().contains_key("hresult"));
    assert!(error.context().contains_key("error_type"));
    assert_eq!(
        error.context().get("phase").map(String::as_str),
        Some("\"probe\""),
        "native bindings must compile before the controlled failure"
    );
    assert!(
        !format!("{error:?}").contains("SENTINEL"),
        "exception body and stdin secret must be excluded"
    );
}
