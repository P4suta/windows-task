//! Bounded, cancellation-aware delivery shared by the native watcher and tests.

#[cfg(any(windows, test))]
use crate::{Error, ErrorKind, history::HistoryQuery};
use crate::{Result, history::HistoryEvent};
#[cfg(any(windows, test))]
use std::time::SystemTime;
use std::{sync::mpsc, time::Duration};

#[derive(Clone, Debug)]
pub(super) struct Cursor {
    #[cfg(any(windows, test))]
    pub(super) bookmark: String,
    #[cfg(any(windows, test))]
    pub(super) record_id: u64,
    #[cfg(any(windows, test))]
    pub(super) timestamp: SystemTime,
}

pub(super) struct Page {
    pub(super) events: Vec<HistoryEvent>,
    pub(super) cursor: Option<Cursor>,
    pub(super) more: bool,
}

/// Native handles remain owned by the returned batch until every parse or
/// bookmark operation has completed, including early error returns.
#[cfg(any(windows, test))]
pub(super) trait PageSource {
    type Handle;
    fn seek(&mut self, cursor: &Cursor) -> Result<()>;
    fn next(&mut self, count: usize) -> Result<Vec<Self::Handle>>;
    fn render(&self, handle: &Self::Handle) -> Result<HistoryEvent>;
    fn bookmark(&self, handle: &Self::Handle) -> Result<String>;
}

#[cfg(any(windows, test))]
pub(super) fn read_page(
    source: &mut impl PageSource,
    query: &HistoryQuery,
    cursor: Option<Cursor>,
) -> Result<Page> {
    if let Some(cursor) = &cursor {
        source.seek(cursor)?;
        let handles = source.next(1)?;
        let anchor = handles.first().ok_or_else(gap)?;
        let anchor = source.render(anchor)?;
        if anchor.record_id != cursor.record_id || anchor.timestamp != cursor.timestamp {
            return Err(gap());
        }
    }
    let handles = source.next(256)?;
    let more = handles.len() == 256;
    let mut next_cursor = cursor;
    let mut events = Vec::new();
    for handle in handles {
        let event = source.render(&handle)?;
        next_cursor = Some(Cursor {
            bookmark: source.bookmark(&handle)?,
            record_id: event.record_id,
            timestamp: event.timestamp,
        });
        if query
            .task
            .as_ref()
            .is_some_and(|path| event.task_path.as_ref() != Some(path))
            || query
                .instance_id
                .is_some_and(|id| event.instance_id != Some(id))
            || query.since.is_some_and(|since| event.timestamp < since)
        {
            continue;
        }
        events.push(event);
    }
    Ok(Page {
        events,
        cursor: next_cursor,
        more,
    })
}

#[cfg(any(windows, test))]
fn gap() -> Error {
    Error::new(
        ErrorKind::HistoryGap,
        "history continuation anchor is missing or changed",
    )
    .with_operation("history.resume")
}

pub(super) fn deliver(
    mut fetch: impl FnMut(Option<Cursor>) -> Result<Page>,
    sender: &mpsc::SyncSender<Result<HistoryEvent>>,
    stop: &mpsc::Receiver<()>,
    interval: Duration,
) {
    let mut cursor = None;
    loop {
        if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            return;
        }
        let page = match fetch(cursor.take()) {
            Ok(page) => page,
            Err(error) => {
                send(sender, stop, Err(error));
                return;
            }
        };
        cursor = page.cursor;
        for event in page.events {
            if !send(sender, stop, Ok(event)) {
                return;
            }
        }
        if !page.more
            && !matches!(
                stop.recv_timeout(interval),
                Err(mpsc::RecvTimeoutError::Timeout)
            )
        {
            return;
        }
    }
}

fn send(
    sender: &mpsc::SyncSender<Result<HistoryEvent>>,
    stop: &mpsc::Receiver<()>,
    mut event: Result<HistoryEvent>,
) -> bool {
    loop {
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            Err(mpsc::TrySendError::Full(value)) => event = value,
        }
        if !matches!(
            stop.recv_timeout(Duration::from_millis(10)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, ErrorKind, history::HistoryEventKind};
    use std::{cell::Cell, rc::Rc};

    struct Handle {
        event: HistoryEvent,
        live: Rc<Cell<usize>>,
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }

    struct Source {
        events: Vec<HistoryEvent>,
        position: usize,
        live: Rc<Cell<usize>>,
        allocated: usize,
        parse_failure: Option<u64>,
        bookmark_failure: Option<u64>,
        seek_failure: Option<ErrorKind>,
    }
    impl Source {
        fn new(count: u64) -> Self {
            Self {
                events: (1..=count).map(event).collect(),
                position: 0,
                live: Rc::new(Cell::new(0)),
                allocated: 0,
                parse_failure: None,
                bookmark_failure: None,
                seek_failure: None,
            }
        }
    }
    impl PageSource for Source {
        type Handle = Handle;
        fn seek(&mut self, cursor: &Cursor) -> Result<()> {
            if let Some(kind) = self.seek_failure {
                return Err(Error::new(kind, "injected seek failure"));
            }
            let id = cursor
                .bookmark
                .parse::<u64>()
                .expect("bookmark supplied by source");
            self.position = self
                .events
                .iter()
                .position(|event| event.record_id == id)
                .ok_or_else(gap)?;
            Ok(())
        }
        fn next(&mut self, count: usize) -> Result<Vec<Handle>> {
            let end = (self.position + count).min(self.events.len());
            let handles = self.events[self.position..end]
                .iter()
                .map(|event| {
                    self.live.set(self.live.get() + 1);
                    Handle {
                        event: event.clone(),
                        live: Rc::clone(&self.live),
                    }
                })
                .collect();
            self.allocated += end - self.position;
            self.position = end;
            Ok(handles)
        }
        fn render(&self, handle: &Handle) -> Result<HistoryEvent> {
            if self.parse_failure == Some(handle.event.record_id) {
                return Err(Error::new(
                    ErrorKind::Serialization,
                    "injected event parse failure",
                ));
            }
            Ok(handle.event.clone())
        }
        fn bookmark(&self, handle: &Handle) -> Result<String> {
            if self.bookmark_failure == Some(handle.event.record_id) {
                return Err(Error::new(
                    ErrorKind::HistoryUnavailable,
                    "injected bookmark failure",
                ));
            }
            Ok(handle.event.record_id.to_string())
        }
    }

    #[test]
    fn actual_page_algorithm_continues_without_duplicates_across_boundaries() {
        let mut source = Source::new(769);
        let query = HistoryQuery {
            limit: Some(1),
            ..HistoryQuery::default()
        };
        let mut cursor = None;
        let mut ids = Vec::new();
        loop {
            let page = read_page(&mut source, &query, cursor).expect("next page");
            cursor = page.cursor;
            ids.extend(page.events.iter().map(|event| event.record_id));
            assert_eq!(source.live.get(), 0);
            if !page.more {
                break;
            }
        }
        assert_eq!(ids, (1..=769).collect::<Vec<_>>());
        let empty = read_page(&mut source, &query, cursor).expect("stable end");
        assert!(empty.events.is_empty());
    }

    #[test]
    fn parse_and_bookmark_failures_release_the_entire_native_style_batch() {
        for bookmark_failure in [false, true] {
            let mut source = Source::new(512);
            if bookmark_failure {
                source.bookmark_failure = Some(125);
            } else {
                source.parse_failure = Some(125);
            }
            let result = read_page(&mut source, &HistoryQuery::default(), None);
            assert!(result.is_err());
            assert_eq!(
                source.allocated, 256,
                "whole batch must be owned before parsing"
            );
            assert_eq!(source.live.get(), 0, "including every unprocessed handle");
        }
    }

    #[test]
    fn clear_retention_and_reused_record_ids_are_reported_as_gaps() {
        for reuse_ids in [false, true] {
            let mut source = Source::new(512);
            let cursor = read_page(&mut source, &HistoryQuery::default(), None)
                .expect("first page")
                .cursor;
            if reuse_ids {
                for event in &mut source.events {
                    event.timestamp += Duration::from_secs(1);
                }
            } else {
                source.events.drain(..256);
            }
            let error = read_page(&mut source, &HistoryQuery::default(), cursor)
                .err()
                .expect("lost anchor");
            assert_eq!(error.kind(), ErrorKind::HistoryGap);
            assert_eq!(source.live.get(), 0);
        }
    }

    #[test]
    fn failed_anchor_parse_and_access_denial_keep_their_classification() {
        for denied in [false, true] {
            let mut source = Source::new(512);
            let cursor = read_page(&mut source, &HistoryQuery::default(), None)
                .expect("first page")
                .cursor;
            if denied {
                source.seek_failure = Some(ErrorKind::AccessDenied);
            } else {
                source.parse_failure = Some(256);
            }
            let error = read_page(&mut source, &HistoryQuery::default(), cursor)
                .err()
                .expect("injected failure");
            assert_eq!(
                error.kind(),
                if denied {
                    ErrorKind::AccessDenied
                } else {
                    ErrorKind::Serialization
                }
            );
            assert_eq!(source.live.get(), 0);
        }
    }

    fn event(record_id: u64) -> HistoryEvent {
        HistoryEvent {
            record_id,
            event_id: 102,
            kind: HistoryEventKind::Completed,
            timestamp: SystemTime::UNIX_EPOCH,
            task_path: None,
            instance_id: None,
            result_code: Some(0),
            fields: Default::default(),
            message: None,
        }
    }

    #[test]
    fn drains_multiple_pages_then_reports_a_gap() {
        let (sender, receiver) = mpsc::sync_channel(8);
        let (stop, stopped) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut page = 0;
            deliver(
                |cursor| {
                    if page == 4 {
                        return Err(Error::new(ErrorKind::HistoryGap, "fixture log cleared"));
                    }
                    if page > 0 {
                        assert_eq!(cursor.expect("continuation").record_id, page * 256);
                    }
                    let start = page * 256 + 1;
                    page += 1;
                    Ok(Page {
                        events: (start..=page * 256).map(event).collect(),
                        cursor: Some(Cursor {
                            record_id: page * 256,
                            bookmark: "fixture".into(),
                            timestamp: SystemTime::UNIX_EPOCH,
                        }),
                        more: true,
                    })
                },
                &sender,
                &stopped,
                Duration::from_secs(60),
            );
        });
        for id in 1..=1024 {
            assert_eq!(
                receiver
                    .recv_timeout(Duration::from_secs(5))
                    .expect("event delivery")
                    .expect("valid event")
                    .record_id,
                id
            );
        }
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("gap delivery")
                .expect_err("gap")
                .kind(),
            ErrorKind::HistoryGap
        );
        worker.join().expect("watcher exited");
        drop(stop);
    }

    #[test]
    fn cancellation_unblocks_a_full_delivery_queue() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            ready.send(()).expect("test synchronization");
            deliver(
                |_| {
                    Ok(Page {
                        events: (0..100).map(event).collect(),
                        cursor: None,
                        more: true,
                    })
                },
                &sender,
                &stopped,
                Duration::from_secs(60),
            );
        });
        started.recv().expect("worker started");
        stop.send(()).expect("stop request");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            worker.is_finished(),
            "full queue must not block cancellation"
        );
        worker.join().expect("watch worker");
        drop(receiver);
    }
}
