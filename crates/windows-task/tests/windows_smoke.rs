#![cfg(windows)]

use windows_task::{
    Error, ErrorKind, FolderPath,
    client::{ListOptions, RegistrationMode, RegistrationOptions, Scheduler},
    model::{Action, ExecAction, LogonType, PrincipalIdentity, ServiceAccount, TaskDefinition},
};

#[test]
fn local_scheduler_can_be_inspected() -> windows_task::Result<()> {
    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let capabilities = blocking.capabilities()?;

    assert!(
        capabilities.highest_version >= 0x0001_0002,
        "Task Scheduler 2.0 or newer is required"
    );
    drop(blocking.list_folders(&FolderPath::root(), false)?);
    drop(blocking.list_tasks(&FolderPath::root(), ListOptions::default())?);
    Ok(())
}

#[test]
fn repeated_sessions_confirm_shutdown_and_reject_new_work() -> windows_task::Result<()> {
    let iterations = std::env::var("WINDOWS_TASK_SESSION_ITERATIONS")
        .ok()
        .map_or(32, |value| {
            value
                .parse::<usize>()
                .expect("positive session iteration count")
        });
    assert!((1..=100_000).contains(&iterations));
    for _ in 0..iterations {
        let scheduler = Scheduler::builder().local().connect_blocking()?;
        let view = scheduler.blocking();
        view.capabilities()?;
        scheduler.shutdown(std::time::Duration::from_secs(5))?;
        assert_eq!(
            view.capabilities().expect_err("session is closed").kind(),
            ErrorKind::WorkerStopped
        );
        scheduler.shutdown(std::time::Duration::ZERO)?;
    }
    Ok(())
}

#[test]
#[ignore = "mutates a disposable Windows host; run cargo xtask test --suite windows"]
fn isolated_task_round_trip() -> windows_task::Result<()> {
    if std::env::var_os("WINDOWS_TASK_MUTATION_TESTS").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        return Err(Error::new(
            ErrorKind::Other,
            "native mutation suite requires WINDOWS_TASK_MUTATION_TESTS=1",
        ));
    }

    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let folder = FolderPath::root()
        .join(&format!("windows-task-ci-{suffix}"))
        .expect("generated folder name is valid");
    let task = folder
        .task("disabled-smoke")
        .expect("static task name is valid");

    let outcome = (|| {
        blocking.create_folder(&folder, None)?;

        let mut definition = TaskDefinition::new(Action::Exec(
            ExecAction::new("cmd.exe").args(["/d", "/c", "exit", "0"]),
        ));
        let identity = scheduler.connection_info();
        let user = identity.user.as_deref().ok_or_else(|| {
            Error::new(ErrorKind::Authentication, "connected user is unavailable")
        })?;
        let user = identity
            .domain
            .as_deref()
            .filter(|domain| !domain.is_empty())
            .map_or_else(|| user.to_owned(), |domain| format!("{domain}\\{user}"));
        definition.principal.identity = PrincipalIdentity::User(user);
        definition.principal.logon_type = LogonType::InteractiveToken;
        definition.settings.enabled = false;

        let registered = blocking.register(
            &task,
            definition,
            RegistrationOptions {
                mode: RegistrationMode::Create,
                disabled: true,
                ..RegistrationOptions::default()
            },
        )?;
        if registered.path != task || registered.enabled {
            return Err(Error::new(
                ErrorKind::Other,
                "registered smoke task did not round-trip as the requested disabled task",
            ));
        }
        let read_back = blocking.get_task(&task)?;
        if read_back.path != task {
            return Err(Error::new(
                ErrorKind::Other,
                "read-back task path differs from the registered path",
            ));
        }
        let information = windows_task::client::SecurityInformation::DACL;
        let task_acl = blocking.task_security(&task, information)?;
        blocking.set_task_security(&task, task_acl.clone(), information)?;
        let actual = blocking.task_security(&task, information)?;
        if actual != task_acl {
            return Err(
                Error::new(ErrorKind::Conflict, "task DACL roundtrip changed access")
                    .with_context("before", task_acl.as_sddl())
                    .with_context("after", actual.as_sddl()),
            );
        }
        let folder_acl = blocking.folder_security(&folder, information)?;
        blocking.set_folder_security(&folder, folder_acl.clone(), information)?;
        if blocking.folder_security(&folder, information)? != folder_acl {
            return Err(Error::new(
                ErrorKind::Conflict,
                "folder DACL roundtrip changed access",
            ));
        }
        // Start from native canonical trustee names (the local administrator
        // may be returned as LA rather than its numeric SID). Change protection
        // while retaining the exact ACE order and access rights.
        let dacl = task_acl
            .as_sddl()
            .strip_prefix("D:")
            .ok_or_else(|| Error::new(ErrorKind::Conflict, "native DACL section unavailable"))?;
        let explicit = windows_task::SecurityDescriptor::from_sddl(format!(
            "D:P{}",
            dacl.trim_start_matches('P')
        ))
        .expect("protected native fixture descriptor");
        blocking.set_task_security(&task, explicit.clone(), information)?;
        let actual = blocking.task_security(&task, information)?;
        if actual != explicit {
            return Err(
                Error::new(ErrorKind::Conflict, "explicit task ACL did not roundtrip")
                    .with_context("actual", actual.as_sddl()),
            );
        }
        blocking.set_folder_security(&folder, explicit.clone(), information)?;
        if blocking.folder_security(&folder, information)? != explicit {
            return Err(Error::new(
                ErrorKind::Conflict,
                "explicit folder ACL did not roundtrip",
            ));
        }
        Ok(())
    })();

    // Cleanup is attempted even when registration or read-back failed. A unique
    // folder keeps the test from touching any pre-existing scheduler state.
    let task_cleanup = blocking.delete_task(&task);
    let folder_cleanup = blocking.delete_folder(&folder);
    cleanup_result(outcome, [task_cleanup, folder_cleanup])
}

fn cleanup_result<const N: usize>(
    outcome: windows_task::Result<()>,
    cleanup: [windows_task::Result<()>; N],
) -> windows_task::Result<()> {
    let failures: Vec<_> = cleanup
        .into_iter()
        .filter_map(Result::err)
        .filter(|error| error.kind() != ErrorKind::NotFound)
        .collect();
    match outcome {
        Err(error) => Err(error.with_context("cleanup_failures", format!("{failures:?}"))),
        Ok(()) if failures.is_empty() => Ok(()),
        Ok(()) => Err(
            Error::new(ErrorKind::Other, "native fixture cleanup failed")
                .with_context("cleanup_failures", format!("{failures:?}")),
        ),
    }
}

#[test]
#[ignore = "applies disabled tasks in a unique native namespace"]
fn isolated_apply_has_no_second_diff() -> windows_task::Result<()> {
    use windows_task::{
        manifest::{ManagedTask, TaskManifest},
        reconcile::{ApplyOptions, apply},
    };
    if std::env::var("WINDOWS_TASK_MUTATION_TESTS").as_deref() != Ok("1") {
        return Err(Error::new(
            ErrorKind::Other,
            "explicit mutation suite required",
        ));
    }
    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let folder = FolderPath::root()
        .join(&format!("windows-task-ci-{}", uuid::Uuid::new_v4()))
        .expect("unique namespace");
    let path = folder.task("apply-disabled").expect("fixture path");
    let mut manifest = TaskManifest::new(uuid::Uuid::new_v4(), "native-fixture", folder.clone());
    let outcome = (|| {
        let identity = scheduler.connection_info();
        let user = identity
            .user
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "connected user unavailable"))?;
        let user = identity
            .domain
            .as_deref()
            .filter(|domain| !domain.is_empty())
            .map_or_else(|| user.to_owned(), |domain| format!("{domain}\\{user}"));
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("fixture.exe")));
        definition.registration.source = Some("fixture-origin".into());
        definition.settings.enabled = false;
        definition.settings.use_unified_scheduling_engine = true;
        definition.principal.identity = PrincipalIdentity::User(user);
        definition.principal.logon_type = LogonType::InteractiveToken;
        manifest.tasks.push(ManagedTask {
            path: path.clone(),
            definition,
            credentials: Default::default(),
        });
        let first = apply(&blocking, &manifest, ApplyOptions::default()).map_err(|failure| {
            failure.cause.with_context(
                "observed_fixture",
                format!(
                    "{:?}",
                    blocking
                        .get_task(&path)
                        .map(|task| task.snapshot.definition().cloned())
                ),
            )
        })?;
        if !first.succeeded() {
            return Err(Error::new(
                ErrorKind::Other,
                "first native apply was not successful",
            ));
        }
        let second = apply(&blocking, &manifest, ApplyOptions::default())
            .map_err(|failure| failure.cause)?;
        if !second.plan.is_empty() {
            return Err(
                Error::new(ErrorKind::Conflict, "second native apply still has a diff")
                    .with_context("plan", format!("{:?}", second.plan)),
            );
        }
        Ok(())
    })();
    cleanup_result(
        outcome,
        [blocking.delete_task(&path), blocking.delete_folder(&folder)],
    )
}

#[test]
#[ignore = "runs a task and enables Operational history on a disposable Windows host"]
fn isolated_execution_returns_exact_exit_code() -> windows_task::Result<()> {
    use windows_task::{
        client::{RunOptions, WaitOptions},
        history::ResultConfidence,
    };
    if std::env::var("WINDOWS_TASK_MUTATION_TESTS").as_deref() != Ok("1") {
        return Err(Error::new(
            ErrorKind::Other,
            "explicit mutation suite required",
        ));
    }
    let executable = std::env::var("WINDOWS_TASK_EXECUTION_FIXTURE")
        .map_err(|_| Error::new(ErrorKind::Other, "execution fixture path required"))?;
    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let history_was_enabled = blocking.history_enabled()?;
    let folder = FolderPath::root()
        .join(&format!("windows-task-ci-{}", uuid::Uuid::new_v4()))
        .expect("unique namespace");
    let task = folder.task("exact-exit").expect("fixture path");
    let outcome = (|| {
        if !history_was_enabled {
            blocking.set_history_enabled(true)?;
        }
        blocking.create_folder(&folder, None)?;
        let mut definition =
            TaskDefinition::new(Action::Exec(ExecAction::new(executable).args(["7", "50"])));
        definition.principal.identity =
            PrincipalIdentity::ServiceAccount(ServiceAccount::LocalSystem);
        definition.principal.logon_type = LogonType::ServiceAccount;
        blocking.register(
            &task,
            definition,
            RegistrationOptions {
                mode: RegistrationMode::Create,
                ..RegistrationOptions::default()
            },
        )?;
        let handle = blocking.run(&task, RunOptions::default())?;
        let outcome = blocking.wait_for_run(
            &handle,
            WaitOptions {
                timeout: std::time::Duration::from_secs(30),
                ..WaitOptions::default()
            },
        )?;
        if outcome.confidence != ResultConfidence::Exact || outcome.result_code != 7 {
            return Err(Error::new(
                ErrorKind::Conflict,
                "execution outcome was not exact exit code 7",
            )
            .with_context("outcome", format!("{outcome:?}")));
        }
        Ok(())
    })();
    cleanup_result(
        outcome,
        [
            blocking.delete_task(&task),
            blocking.delete_folder(&folder),
            if history_was_enabled {
                Ok(())
            } else {
                blocking.set_history_enabled(false)
            },
        ],
    )
}
