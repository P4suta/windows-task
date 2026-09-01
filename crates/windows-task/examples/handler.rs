//! Minimal COM handler implementation. Build real handlers as `cdylib` crates.

use windows_task::{
    Result, handler,
    handler::{HandlerContext, TaskHandler},
};

#[derive(Default)]
struct ExampleHandler;

#[handler(clsid = "e4ef9b55-4f33-4dd2-a658-6eb2c58c576b")]
impl TaskHandler for ExampleHandler {
    fn run(self, context: HandlerContext) -> Result<()> {
        context.reporter.report_with_message(50, "working")?;
        if !context.control.is_cancelled() {
            context.reporter.report(100)?;
        }
        Ok(())
    }
}

fn main() {
    std::hint::black_box(ExampleHandler);
    println!("handler CLSID: {WINDOWS_TASK_HANDLER_CLSID}");
}
