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

fn approved_host(actions: Option<&str>, environment: Option<&str>, ack: Option<&str>) -> bool {
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
    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let enabled = blocking.history_enabled()?;
    let folder = FolderPath::root()
        .join(&format!("windows-task-ci-gap-{}", uuid::Uuid::new_v4()))
        .expect("fixture namespace");
    let task = folder.task("bookmark-anchor").expect("fixture path");
    let mut probe = None;
    let outcome = (|| {
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
        ],
    );
    let stopped = probe.map_or_else(
        || scheduler.shutdown(DEADLINE),
        |probe| probe.finish(&scheduler),
    );
    cleanup_result(outcome, [stopped])
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
