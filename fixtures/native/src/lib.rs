use windows_task::{
    handler,
    handler::{HandlerContext, TaskHandler},
};

#[derive(Default)]
struct Fixture;

#[handler(clsid = "08c50c37-3d58-4c7f-b13f-1b319e1b1301")]
impl TaskHandler for Fixture {
    fn run(self, context: HandlerContext) -> windows_task::Result<()> {
        match context.data.as_deref() {
            Some("panic") => panic!("intentional handler fixture panic"),
            Some("panic-retained") => {
                let reporter = context.reporter.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    drop(reporter);
                });
                panic!("intentional panic with retained reporter");
            }
            Some("wait") => {
                while !context.control.is_cancelled() {
                    context.control.wait_if_paused();
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            Some("progress") => {
                for value in [25, 75, 50, 100] {
                    context.reporter.report(value)?;
                }
            }
            Some("concurrent-progress") => {
                let gate = std::sync::Arc::new(std::sync::Barrier::new(16));
                let workers: Vec<_> = (1..=16)
                    .map(|index| {
                        let gate = gate.clone();
                        let reporter = context.reporter.clone();
                        std::thread::spawn(move || {
                            gate.wait();
                            reporter.report(index * 6)
                        })
                    })
                    .collect();
                let results: Vec<_> = workers
                    .into_iter()
                    .map(|worker| worker.join().expect("progress worker"))
                    .collect();
                for result in results {
                    result?;
                }
                context.reporter.report(100)?;
            }
            Some("concurrent-complete" | "concurrent-complete-failure") => {
                let gate = std::sync::Arc::new(std::sync::Barrier::new(16));
                let workers: Vec<_> = (0..16)
                    .map(|_| {
                        let gate = gate.clone();
                        let reporter = context.reporter.clone();
                        std::thread::spawn(move || {
                            gate.wait();
                            reporter.complete()
                        })
                    })
                    .collect();
                let results: Vec<_> = workers
                    .into_iter()
                    .map(|worker| worker.join().expect("completion worker"))
                    .collect();
                assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
                let native_failures = results
                    .iter()
                    .filter(|result| {
                        result
                            .as_ref()
                            .err()
                            .is_some_and(|error| error.kind() != windows_task::ErrorKind::Conflict)
                    })
                    .count();
                assert_eq!(
                    native_failures,
                    usize::from(context.data.as_deref() == Some("concurrent-complete-failure"))
                );
                assert_eq!(
                    context
                        .reporter
                        .report(100)
                        .expect_err("completed reporter")
                        .kind(),
                    windows_task::ErrorKind::Conflict
                );
            }
            Some("complete" | "complete-failure") => context.reporter.complete()?,
            Some("fresh-control") => {
                if context.control.is_cancelled() || context.control.is_paused() {
                    return Err(windows_task::Error::new(
                        windows_task::ErrorKind::Conflict,
                        "new Start inherited old control",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
}
