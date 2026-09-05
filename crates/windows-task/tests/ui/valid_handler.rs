use windows_task::{handler, handler::{HandlerContext, TaskHandler}};
#[derive(Default)]
struct Valid;
#[handler(clsid = "08c50c37-3d58-4c7f-b13f-1b319e1b1301")]
impl TaskHandler for Valid {
    fn run(self, _: HandlerContext) -> windows_task::Result<()> { Ok(()) }
}
fn main() {}
