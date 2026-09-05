use windows_task::{handler, handler::{HandlerContext, TaskHandler}};

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
                for value in [25, 75, 50, 100] { context.reporter.report(value)?; }
            }
            Some("complete" | "complete-failure") => context.reporter.complete()?,
            Some("fresh-control") => {
                if context.control.is_cancelled() || context.control.is_paused() {
                    return Err(windows_task::Error::new(windows_task::ErrorKind::Conflict, "new Start inherited old control"));
                }
            }
            _ => {}
        }
        Ok(())
    }
}
