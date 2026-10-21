# Windows-task

Safe Rust models, XML, and clients for Windows Task Scheduler.

## Installation

```sh
cargo add windows-task --git https://github.com/P4suta/windows-task
```

## Usage

```rust
use windows_task::model::{
    DailyTrigger, ExecAction, Principal, ServiceAccount, TaskDateTime, TaskDefinition,
};
let definition = TaskDefinition::builder(ExecAction::new("agent.exe").args(["--once"]))
    .description("Nightly agent run")
    .run_as(Principal::service_account(ServiceAccount::LocalSystem))
    .trigger(DailyTrigger::new(TaskDateTime::wall_clock(2026, 9, 5, 3, 30, 0)?))
    .build()?;
let xml = windows_task::xml::to_string(&definition)?;
assert_eq!(windows_task::xml::from_bytes(xml.as_bytes())?, definition);
# Ok::<(), windows_task::Error>(())
```

## Documentation

[Read the documentation](docs/README.md).

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [Massachusetts Institute of Technology (MIT) license](LICENSE-MIT), at your option.
You license each contribution you submit under the same two licenses, without other terms, unless you state otherwise.
