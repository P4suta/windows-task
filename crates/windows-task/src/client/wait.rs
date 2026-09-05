//! Executor-neutral run correlation with an injectable clock and observer.

use super::{
    AtomicBool, BlockingScheduler, Duration, Error, ErrorKind, HistoryEvent, HistoryEventKind,
    HistoryQuery, Instant, Ordering, Result, ResultConfidence, RunHandle, RunOutcome, RunningTask,
    SystemTime, TaskPath, WaitOptions, thread,
};
use crate::history::FallbackReason;

pub(super) trait Observer {
    fn history(&self, query: HistoryQuery) -> Result<Vec<HistoryEvent>>;
    fn running(&self) -> Result<Vec<RunningTask>>;
    fn last_result(&self, path: &TaskPath) -> Result<i32>;
    fn elapsed(&self) -> Duration;
    fn now(&self) -> SystemTime;
    fn sleep(&self, duration: Duration);
}

#[derive(Default)]
struct Correlation {
    completed: bool,
    action: Option<(u64, i32)>,
    terminal: Option<i32>,
}

impl Correlation {
    fn ingest(&mut self, handle: &RunHandle, events: Vec<HistoryEvent>) -> Option<i32> {
        for event in events {
            if event.instance_id != Some(handle.instance_id)
                || event.task_path.as_ref() != Some(&handle.path)
            {
                continue;
            }
            match event.kind {
                HistoryEventKind::Completed => {
                    self.completed = true;
                    self.terminal = event.result_code.or(self.terminal);
                }
                HistoryEventKind::Stopped => {
                    self.terminal = Some(event.result_code.unwrap_or(0x0004_1306));
                }
                HistoryEventKind::Failed if matches!(event.event_id, 101 | 103 | 104 | 311) => {
                    self.terminal = event.result_code.or(self.terminal);
                }
                HistoryEventKind::ActionCompleted => {
                    if let Some(code) = event.result_code {
                        if self
                            .action
                            .is_none_or(|(record, _)| event.record_id > record)
                        {
                            self.action = Some((event.record_id, code));
                        }
                    }
                }
                _ => {}
            }
        }
        self.terminal.or_else(|| {
            self.completed
                .then_some(self.action)
                .flatten()
                .map(|(_, code)| code)
        })
    }
}

pub(super) struct Live<'a> {
    pub(super) scheduler: &'a BlockingScheduler,
    pub(super) started: Instant,
}
impl Observer for Live<'_> {
    fn history(&self, query: HistoryQuery) -> Result<Vec<HistoryEvent>> {
        self.scheduler.history(query)
    }
    fn running(&self) -> Result<Vec<RunningTask>> {
        self.scheduler.running_tasks(true)
    }
    fn last_result(&self, path: &TaskPath) -> Result<i32> {
        self.scheduler.get_task(path).map(|task| task.last_result)
    }
    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

pub(super) fn observe_run(
    observer: &impl Observer,
    handle: &RunHandle,
    options: WaitOptions,
    cancelled: Option<&AtomicBool>,
) -> Result<RunOutcome> {
    if options.poll_interval.is_zero() {
        return Err(Error::new(
            ErrorKind::InvalidDefinition,
            "run wait poll interval must be non-zero",
        ));
    }
    let started_at = observer
        .now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let mut history_error = None;
    let mut history_read = false;
    let mut running_read = false;
    let mut observed_running = false;
    let mut last_running = false;
    let mut absent_since = None;
    let mut correlation = Correlation::default();

    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(Error::new(ErrorKind::Cancelled, "run wait was cancelled")
                .with_target(handle.instance_id.to_string()));
        }
        if observer.elapsed() >= options.timeout {
            return Err(Error::new(
                ErrorKind::Timeout,
                "task run wait elapsed; exact completion is unconfirmed",
            )
            .with_target(handle.instance_id.to_string())
            .with_context("wait_started_unix_seconds", started_at.to_string())
            .with_context("observed_running", observed_running.to_string())
            .with_context(
                "last_running",
                if running_read {
                    last_running.to_string()
                } else {
                    "not_probed".into()
                },
            )
            .with_context(
                "history",
                history_error.map_or_else(
                    || {
                        if history_read {
                            "readable_no_completion"
                        } else {
                            "not_probed"
                        }
                        .into()
                    },
                    |kind| format!("{kind:?}"),
                ),
            ));
        }
        if history_error.is_none() {
            match observer.history(HistoryQuery {
                task: Some(handle.path.clone()),
                instance_id: Some(handle.instance_id),
                // A caller may begin waiting well after run() returned. The
                // instance GUID is the boundary; a new wall-clock cutoff would
                // discard an already completed instance.
                since: None,
                limit: Some(256),
                forward: false,
            }) {
                Ok(events) => {
                    history_read = true;
                    if let Some(result_code) = correlation.ingest(handle, events) {
                        return Ok(RunOutcome {
                            instance_id: handle.instance_id,
                            result_code,
                            confidence: ResultConfidence::Exact,
                            fallback_reason: None,
                        });
                    }
                }
                Err(error) if options.allow_polling_fallback => {
                    history_error = Some(error.kind());
                    #[cfg(feature = "tracing")]
                    tracing::warn!(phase = "fallback", reason = "history_unavailable", kind = ?error.kind());
                }
                Err(error) => return Err(error.with_context("completion", "unconfirmed")),
            }
        }
        last_running = observer
            .running()?
            .iter()
            .any(|run| run.instance_id == handle.instance_id);
        running_read = true;
        if last_running {
            observed_running = true;
            absent_since = None;
        } else {
            absent_since.get_or_insert(observer.elapsed());
        }
        if options.allow_polling_fallback
            && absent_since.is_some_and(|since| {
                observer.elapsed().saturating_sub(since) >= options.history_grace
            })
            && (observed_running || observer.elapsed() >= options.history_grace)
        {
            #[cfg(feature = "tracing")]
            tracing::warn!(
                phase = "fallback",
                reason = if history_error.is_some() {
                    "history_unavailable"
                } else {
                    "completion_not_observed"
                }
            );
            return Ok(RunOutcome {
                instance_id: handle.instance_id,
                result_code: observer.last_result(&handle.path)?,
                confidence: ResultConfidence::PollingFallback,
                fallback_reason: Some(if history_error.is_some() {
                    FallbackReason::HistoryUnavailable
                } else {
                    FallbackReason::CompletionNotObserved
                }),
            });
        }
        observer.sleep(
            options
                .poll_interval
                .min(options.timeout.saturating_sub(observer.elapsed()))
                .min(Duration::from_secs(1)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use uuid::Uuid;

    struct Fake {
        elapsed: Cell<Duration>,
        fail_history: bool,
        completion_at: Option<Duration>,
        instance: Uuid,
        last_result_reads: Cell<usize>,
    }
    impl Observer for Fake {
        fn history(&self, query: HistoryQuery) -> Result<Vec<HistoryEvent>> {
            assert!(
                query.since.is_none(),
                "waiting later must not discard earlier completion"
            );
            if self.fail_history {
                return Err(Error::new(ErrorKind::AccessDenied, "history denied"));
            }
            Ok(
                if self
                    .completion_at
                    .is_some_and(|at| self.elapsed.get() >= at)
                {
                    vec![HistoryEvent {
                        record_id: 1,
                        event_id: 102,
                        kind: HistoryEventKind::Completed,
                        timestamp: self.now(),
                        task_path: Some("\\Test\\Run".parse().expect("fixture path")),
                        instance_id: Some(self.instance),
                        result_code: Some(7),
                        fields: Default::default(),
                        message: None,
                    }]
                } else {
                    vec![]
                },
            )
        }
        fn running(&self) -> Result<Vec<RunningTask>> {
            Ok(vec![])
        }
        fn last_result(&self, _: &TaskPath) -> Result<i32> {
            self.last_result_reads.set(self.last_result_reads.get() + 1);
            Ok(99)
        }
        fn elapsed(&self) -> Duration {
            self.elapsed.get()
        }
        fn now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(100) + self.elapsed.get()
        }
        fn sleep(&self, duration: Duration) {
            self.elapsed.set(self.elapsed.get() + duration);
        }
    }
    fn fixture() -> (Fake, RunHandle, WaitOptions) {
        (
            Fake {
                elapsed: Cell::new(Duration::ZERO),
                fail_history: false,
                completion_at: None,
                instance: Uuid::nil(),
                last_result_reads: Cell::new(0),
            },
            RunHandle {
                path: "\\Test\\Run".parse().expect("fixture path"),
                instance_id: Uuid::nil(),
                engine_process_id: None,
            },
            WaitOptions {
                timeout: Duration::from_secs(5),
                ..WaitOptions::default()
            },
        )
    }
    #[test]
    fn delayed_exact_completion_does_not_become_a_last_result_estimate() {
        let (mut observer, handle, options) = fixture();
        observer.completion_at = Some(Duration::from_secs(3));
        let result = observe_run(&observer, &handle, options, None).expect("delayed completion");
        assert_eq!(result.confidence, ResultConfidence::Exact);
        assert_eq!(result.result_code, 7);
        assert_eq!(observer.last_result_reads.get(), 0);
    }
    #[test]
    fn other_instance_completion_is_not_accepted() {
        let (mut observer, handle, options) = fixture();
        observer.completion_at = Some(Duration::ZERO);
        observer.instance = Uuid::from_u128(1);
        let error = observe_run(&observer, &handle, options, None).expect_err("wrong instance");
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert_eq!(
            error.context().get("last_running").map(String::as_str),
            Some("false")
        );
        assert_eq!(observer.last_result_reads.get(), 0);
    }
    #[test]
    fn inaccessible_history_requires_explicit_estimate_permission() {
        let (mut observer, handle, mut options) = fixture();
        observer.fail_history = true;
        assert_eq!(
            observe_run(&observer, &handle, options, None)
                .expect_err("strict history")
                .kind(),
            ErrorKind::AccessDenied
        );
        options.allow_polling_fallback = true;
        let result = observe_run(&observer, &handle, options, None).expect("explicit estimate");
        assert_eq!(result.confidence, ResultConfidence::PollingFallback);
        assert_eq!(
            result.fallback_reason,
            Some(FallbackReason::HistoryUnavailable)
        );
    }
    #[test]
    fn cancellation_and_zero_interval_do_not_probe() {
        let (observer, handle, mut options) = fixture();
        assert_eq!(
            observe_run(&observer, &handle, options, Some(&AtomicBool::new(true)))
                .expect_err("cancelled")
                .kind(),
            ErrorKind::Cancelled
        );
        options.poll_interval = Duration::ZERO;
        assert_eq!(
            observe_run(&observer, &handle, options, None)
                .expect_err("zero interval")
                .kind(),
            ErrorKind::InvalidDefinition
        );
    }

    #[test]
    fn native_completion_without_result_uses_latest_matching_action() {
        let (observer, handle, _) = fixture();
        let event = |record_id, kind, result_code| HistoryEvent {
            record_id,
            event_id: if kind == HistoryEventKind::Completed {
                102
            } else {
                201
            },
            kind,
            timestamp: observer.now(),
            task_path: Some(handle.path.clone()),
            instance_id: Some(handle.instance_id),
            result_code,
            fields: Default::default(),
            message: None,
        };
        let mut correlation = Correlation::default();
        assert_eq!(
            correlation.ingest(
                &handle,
                vec![event(10, HistoryEventKind::ActionCompleted, Some(1))]
            ),
            None
        );
        // Reverse query order, duplicate older action, and the Windows 102
        // template (which does not contain ResultCode).
        assert_eq!(
            correlation.ingest(
                &handle,
                vec![
                    event(13, HistoryEventKind::Completed, None),
                    event(12, HistoryEventKind::ActionCompleted, Some(7)),
                    event(10, HistoryEventKind::ActionCompleted, Some(1)),
                ]
            ),
            Some(7)
        );
    }
}
