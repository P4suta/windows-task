//! Allowlisted diagnostics: never format caller input or error messages here.

#[derive(Clone, Debug)]
pub(crate) struct Operation {
    #[cfg(feature = "tracing")]
    span: tracing::Span,
    #[cfg(feature = "tracing")]
    dispatch: tracing::Dispatch,
    #[cfg(feature = "tracing")]
    queued: std::time::Instant,
}

impl Operation {
    pub(crate) fn scope<T>(&self, work: impl FnOnce() -> T) -> T {
        #[cfg(feature = "tracing")]
        {
            tracing::dispatcher::with_default(&self.dispatch, || self.span.in_scope(work))
        }
        #[cfg(not(feature = "tracing"))]
        {
            work()
        }
    }
    pub(crate) fn new(name: &'static str) -> Self {
        #[cfg(feature = "tracing")]
        {
            let dispatch = tracing::dispatcher::get_default(Clone::clone);
            if dispatch.is::<tracing::subscriber::NoSubscriber>() {
                return Self {
                    span: tracing::Span::none(),
                    dispatch,
                    queued: std::time::Instant::now(),
                };
            }
            let span = tracing::info_span!("windows_task.operation", operation = name,
                operation_id = %uuid::Uuid::new_v4());
            span.in_scope(|| tracing::debug!(phase = "queued"));
            Self {
                span,
                dispatch,
                queued: std::time::Instant::now(),
            }
        }
        #[cfg(not(feature = "tracing"))]
        {
            let _ = name;
            Self {}
        }
    }

    pub(crate) fn run<T>(&self, work: impl FnOnce() -> crate::Result<T>) -> crate::Result<T> {
        #[cfg(feature = "tracing")]
        {
            if self.dispatch.is::<tracing::subscriber::NoSubscriber>() {
                return work();
            }
            self.scope(|| {
                tracing::debug!(phase = "started", queue_ms = u64::try_from(self.queued.elapsed().as_millis()).unwrap_or(u64::MAX));
                let started = std::time::Instant::now();
                let result = work();
                match &result {
                    Ok(_) => tracing::debug!(phase = "completed", elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
                    Err(error) => tracing::warn!(phase = "failed", kind = ?error.kind(),
                        native_code = error.native_code(), elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
                }
                result
            })
        }
        #[cfg(not(feature = "tracing"))]
        {
            self.scope(work)
        }
    }
}

#[cfg(all(test, feature = "tracing"))]
mod tests {
    use super::Operation;
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("test log").extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn cross_thread_trace_preserves_parent_and_never_formats_secrets() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = Buffer(Arc::clone(&bytes));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let parent = tracing::info_span!("caller");
            let trace = parent.in_scope(|| Operation::new("register"));
            std::thread::spawn(move || {
                trace.run(|| {
                    Err::<(), _>(
                        crate::Error::new(crate::ErrorKind::Authentication, "SENTINEL_PASSWORD")
                            .with_target("SENTINEL_TARGET")
                            .with_context("xml", "SENTINEL_XML")
                            .with_native_code(-1),
                    )
                })
            })
            .join()
            .expect("worker finished")
            .expect_err("injected error");
        });
        let log = String::from_utf8(bytes.lock().expect("log buffer").clone()).expect("UTF8 logs");
        assert!(
            log.contains("caller") && log.contains("register") && log.contains("failed"),
            "captured trace: {log}"
        );
        assert!(
            !log.contains("SENTINEL"),
            "trace must not format error bodies, targets or context"
        );
    }
}
