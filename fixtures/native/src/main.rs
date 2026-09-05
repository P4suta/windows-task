fn main() {
    let mut args = std::env::args().skip(1);
    let code: i32 = args.next().unwrap_or_else(|| "0".into()).parse().expect("exit code");
    let delay: u64 = args.next().unwrap_or_else(|| "0".into()).parse().expect("delay milliseconds");
    std::thread::sleep(std::time::Duration::from_millis(delay));
    std::process::exit(code);
}
