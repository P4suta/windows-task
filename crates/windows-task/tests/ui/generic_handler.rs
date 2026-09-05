struct Invalid<T>(T);
#[windows_task::handler(clsid = "08c50c37-3d58-4c7f-b13f-1b319e1b1301")]
impl<T> Default for Invalid<T> {
    fn default() -> Self { panic!() }
}
fn main() {}
