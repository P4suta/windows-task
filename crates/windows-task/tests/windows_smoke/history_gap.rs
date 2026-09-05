//! Destructive provider acceptance, gated to an acknowledged disposable CI host.
use super::*;
use std::{
    process::Command,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};
use windows_task::{
    client::{BlockingScheduler, HistoryWatcher},
    history::{HistoryEvent, HistoryQuery, OPERATIONAL_CHANNEL},
};

const DEADLINE: Duration = Duration::from_secs(10);

pub(super) fn approved_host(
    actions: Option<&str>,
    environment: Option<&str>,
    ack: Option<&str>,
) -> bool {
    actions == Some("true") && environment == Some("github-hosted") && ack == Some("1")
}

#[test]
fn clearing_requires_both_a_disposable_host_and_explicit_acknowledgement() {
    assert!(approved_host(
        Some("true"),
        Some("github-hosted"),
        Some("1")
    ));
    for (actions, environment, ack) in [
        (None, Some("github-hosted"), Some("1")),
        (Some("false"), Some("github-hosted"), Some("1")),
        (Some("true"), Some("self-hosted"), Some("1")),
        (Some("true"), None, Some("1")),
        (Some("true"), Some("github-hosted"), None),
        (Some("true"), Some("github-hosted"), Some("0")),
    ] {
        assert!(!approved_host(actions, environment, ack));
    }
}

#[test]
#[ignore = "clears the Operational log; explicitly acknowledged GitHub-hosted CI only"]
fn cleared_native_log_reports_a_gap_and_terminates_the_watcher() -> windows_task::Result<()> {
    require_disposable_host()?;
    verify_gap(false)
}

#[test]
#[ignore = "overwrites retained Operational events; explicitly acknowledged GitHub-hosted CI only"]
fn overwritten_native_log_reports_a_gap_and_terminates_the_watcher() -> windows_task::Result<()> {
    require_disposable_host()?;
    verify_gap(true)
}

fn require_disposable_host() -> windows_task::Result<()> {
    if !approved_host(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        std::env::var("RUNNER_ENVIRONMENT").ok().as_deref(),
        std::env::var("WINDOWS_TASK_CLEAR_EVENT_LOG")
            .ok()
            .as_deref(),
    ) {
        return Err(Error::new(
            ErrorKind::Other,
            "Event Log clearing requires GitHub-hosted CI and WINDOWS_TASK_CLEAR_EVENT_LOG=1",
        ));
    }
    Ok(())
}

fn verify_gap(rollover: bool) -> windows_task::Result<()> {
    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let enabled = blocking.history_enabled()?;
    let folder = FolderPath::root()
        .join(&format!("windows-task-ci-gap-{}", uuid::Uuid::new_v4()))
        .expect("fixture namespace");
    let task = folder.task("bookmark-anchor").expect("fixture path");
    let mut probe = None;
    let original = rollover.then(log_configuration).transpose()?;
    let outcome = (|| {
        if rollover {
            configure_log(1_048_576, 0)?;
            clear_log()?;
        }
        blocking.set_history_enabled(true)?;
        blocking.create_folder(&folder, None)?;
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("cmd.exe")));
        definition.settings.enabled = false;
        definition.principal.identity =
            PrincipalIdentity::ServiceAccount(ServiceAccount::LocalSystem);
        definition.principal.logon_type = LogonType::ServiceAccount;
        blocking.register(
            &task,
            definition,
            RegistrationOptions {
                mode: RegistrationMode::Create,
                disabled: true,
                ..RegistrationOptions::default()
            },
        )?;
        probe = Some(Probe::start(&blocking, task.clone())?);
        let probe = probe.as_ref().expect("reader started");
        let first = probe.receive(DEADLINE)?;
        if first.task_path.as_ref() != Some(&task) {
            return Err(Error::new(
                ErrorKind::Conflict,
                "watcher did not establish the fixture anchor",
            ));
        }
        eprintln!("fixture anchor record_id={}", first.record_id);
        if rollover {
            overwrite_anchor(&blocking, &task, first.record_id)?;
        } else {
            clear_log()?;
        }
        let started = Instant::now();
        loop {
            match probe.receive(DEADLINE.saturating_sub(started.elapsed())) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::HistoryGap => {
                    eprintln!(
                        "fixture gap kind={:?} native_code={:?}",
                        error.kind(),
                        error.native_code()
                    );
                    break Ok(());
                }
                Err(error) => break Err(error),
            }
        }
    })();
    let outcome = cleanup_result(
        outcome,
        [
            blocking.delete_task(&task),
            blocking.delete_folder(&folder),
            blocking.set_history_enabled(enabled),
            original.map_or(Ok(()), |(size, mode)| configure_log(size, mode)),
        ],
    );
    let stopped = probe.map_or_else(
        || scheduler.shutdown(DEADLINE),
        |probe| probe.finish(&scheduler),
    );
    cleanup_result(outcome, [stopped])
}

fn clear_log() -> windows_task::Result<()> {
    let status = Command::new("wevtutil.exe")
        .args(["cl", OPERATIONAL_CHANNEL])
        .status()
        .map_err(|error| {
            Error::new(
                ErrorKind::Other,
                format!("cannot start fixture log clear: {error}"),
            )
        })?;
    if !status.success() {
        return Err(Error::new(ErrorKind::Other, "fixture log clear failed")
            .with_context("exit_code", format!("{:?}", status.code())));
    }
    Ok(())
}

fn log_configuration() -> windows_task::Result<(u64, u32)> {
    let output = log_script(
        "Write-Output ($log.MaximumSizeInBytes.ToString() + ' ' + ([int]$log.LogMode).ToString())",
    )?;
    let values: Vec<_> = output.split_whitespace().collect();
    if values.len() != 2 {
        return Err(Error::new(
            ErrorKind::Other,
            "unexpected fixture log configuration",
        ));
    }
    let size = values[0]
        .parse()
        .map_err(|_| Error::new(ErrorKind::Other, "invalid fixture log size"))?;
    let mode = values[1]
        .parse()
        .map_err(|_| Error::new(ErrorKind::Other, "invalid fixture log mode"))?;
    Ok((size, mode))
}

fn configure_log(size: u64, mode: u32) -> windows_task::Result<()> {
    log_script(&format!(
        "$log.MaximumSizeInBytes={size}; $log.LogMode=[System.Diagnostics.Eventing.Reader.EventLogMode]{mode}; $log.SaveChanges()"
    ))?;
    Ok(())
}

fn log_script(body: &str) -> windows_task::Result<String> {
    // Only fixture-controlled code and parsed numeric configuration enter this command.
    let script = format!(
        "$ErrorActionPreference='Stop'; $log=[System.Diagnostics.Eventing.Reader.EventLogConfiguration]::new('{OPERATIONAL_CHANNEL}'); try {{ {body} }} finally {{ $log.Dispose() }}"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|error| {
            Error::new(
                ErrorKind::Other,
                format!("fixture configuration process: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(
            Error::new(ErrorKind::Other, "fixture log configuration failed").with_context(
                "stderr",
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ),
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn overwrite_anchor(
    blocking: &BlockingScheduler,
    task: &windows_task::TaskPath,
    anchor: u64,
) -> windows_task::Result<()> {
    let started = Instant::now();
    // The fixture stops draining its eight-item queue. This fills the public
    // watcher's bounded queue, holding its bookmark while the provider rolls over.
    for index in 0..20_000 {
        blocking.set_enabled(task, index % 2 == 0)?;
        if index % 256 == 255 {
            let oldest = blocking.history(HistoryQuery {
                forward: true,
                limit: Some(1),
                ..HistoryQuery::default()
            })?;
            if let Some(oldest) = oldest.first() {
                if oldest.record_id > anchor.saturating_add(1024) {
                    eprintln!(
                        "fixture retention anchor={anchor} oldest={} mutations={}",
                        oldest.record_id,
                        index + 1
                    );
                    return Ok(());
                }
            }
        }
        if started.elapsed() > Duration::from_secs(180) {
            break;
        }
    }
    Err(Error::new(
        ErrorKind::Timeout,
        "fixture did not overwrite its retained bookmark within the bounded workload",
    ))
}

struct Probe {
    receiver: mpsc::Receiver<windows_task::Result<HistoryEvent>>,
    reader: JoinHandle<windows_task::Result<()>>,
}

impl Probe {
    fn start(
        blocking: &BlockingScheduler,
        task: windows_task::TaskPath,
    ) -> windows_task::Result<Self> {
        let watcher = blocking.watch_history(
            HistoryQuery {
                task: Some(task),
                since: Some(SystemTime::UNIX_EPOCH),
                ..HistoryQuery::default()
            },
            Duration::from_millis(25),
        )?;
        let (sender, receiver) = mpsc::sync_channel(8);
        let reader = thread::Builder::new()
            .name("history-gap-fixture".into())
            .spawn(move || read_until_terminal(watcher, &sender))
            .map_err(|error| Error::new(ErrorKind::WorkerStopped, error.to_string()))?;
        Ok(Self { receiver, reader })
    }

    fn receive(&self, timeout: Duration) -> windows_task::Result<HistoryEvent> {
        self.receiver.recv_timeout(timeout).map_err(|error| {
            Error::new(
                match error {
                    mpsc::RecvTimeoutError::Timeout => ErrorKind::Timeout,
                    mpsc::RecvTimeoutError::Disconnected => ErrorKind::WorkerStopped,
                },
                "fixture event was not delivered before the deadline",
            )
        })?
    }

    fn finish(self, scheduler: &Scheduler) -> windows_task::Result<()> {
        drop(self.receiver);
        let shutdown = scheduler.shutdown(DEADLINE);
        let started = Instant::now();
        while !self.reader.is_finished() {
            if started.elapsed() >= DEADLINE {
                return cleanup_result(
                    shutdown,
                    [Err(Error::new(
                        ErrorKind::Timeout,
                        "fixture reader termination is unconfirmed",
                    ))],
                );
            }
            thread::sleep(Duration::from_millis(5));
        }
        let joined = self
            .reader
            .join()
            .unwrap_or_else(|_| Err(Error::new(ErrorKind::Other, "fixture reader panicked")));
        cleanup_result(shutdown, [joined])
    }
}

fn read_until_terminal(
    mut watcher: HistoryWatcher,
    sender: &mpsc::SyncSender<windows_task::Result<HistoryEvent>>,
) -> windows_task::Result<()> {
    while let Some(event) = watcher.next() {
        let terminal = event.is_err();
        if sender.send(event).is_err() {
            break;
        }
        if terminal {
            if watcher.next().is_some() {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "watcher resumed after a history gap",
                ));
            }
            break;
        }
    }
    watcher.shutdown(DEADLINE)
}
