# windows-task

Owned Rust models and a dedicated COM worker for Windows Task Scheduler 2.0.
Rust 1.85 is supported. XML, validation, manifests and scheduling models also
work on other platforms; native operations report `UnsupportedPlatform`.

```rust
use windows_task::model::{Action, ExecAction, TaskDefinition};
let definition = TaskDefinition::new(Action::Exec(ExecAction::new("agent.exe")));
assert!(definition.validate().is_valid());
let xml = windows_task::xml::to_string(&definition)?;
assert_eq!(windows_task::xml::from_bytes(xml.as_bytes())?, definition);
# Ok::<(), windows_task::Error>(())
```

Connect, register and wait for a result correlated to the specific run instance:

```rust,no_run
use windows_task::{TaskPath, client::{Scheduler, RegistrationOptions, RunOptions, WaitOptions},
    model::{Action, ExecAction, TaskDefinition}};
let scheduler = Scheduler::builder().local().connect_blocking()?;
let task: TaskPath = r"\Acme\Backup".parse().expect("absolute task path");
let definition = TaskDefinition::new(Action::Exec(ExecAction::new("agent.exe")));
scheduler.blocking().register(&task, definition, RegistrationOptions::default())?;
let run = scheduler.blocking().run(&task, RunOptions::default())?;
let outcome = scheduler.blocking().wait_for_run(&run, WaitOptions::default())?;
assert_eq!(outcome.confidence, windows_task::history::ResultConfidence::Exact);
scheduler.shutdown(std::time::Duration::from_secs(5))?;
# Ok::<(), windows_task::Error>(())
```

Exact completion requires readable Operational history. Enabling that log is an
explicit administrative operation. Estimates require `allow_polling_fallback`.
`Drop` signals shutdown; use `shutdown` to confirm termination within a deadline.

The default `tracing` feature records allowlisted operation metadata. The calling
application owns subscriber configuration; the library never installs one.
Reconciliation returns a structured journal on both success and failure. Examine
`unresolved`, `rollback_failures` and `irreversible_effects` before assuming the
original state was restored. Task Scheduler does not offer atomic compare/update.

The optional `handler` feature provides COM DLL exports through the `handler`
attribute. Handler panics require unwind builds for containment; an embedding
application that chooses `panic = "abort"` cannot catch those panics.
