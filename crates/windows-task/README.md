# windows-task

Owned Rust models and a dedicated COM worker for Windows Task Scheduler 2.0.
Rust 1.85 is supported. XML, validation, manifests and scheduling models also
work on other platforms; native operations report `UnsupportedPlatform`.

`build` applies the native action/trigger limits and the full portable
validation, so a definition that builds is a definition that validates. The
`Principal` constructors pair an identity with the logon type that matches it:

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

Every model and path failure converts into `windows_task::Error`, so one `?`
carries construction, parsing and native calls. A rejected definition keeps its
findings, and findings render for people:

```rust
use windows_task::{ErrorKind, model::{ExecAction, TaskDefinition}};
let error = TaskDefinition::builder(ExecAction::new("")).build().unwrap_err();
assert_eq!(error.kind(), ErrorKind::InvalidDefinition);
assert_eq!(
    error.diagnostics()[0].to_string(),
    "error[empty_value] actions[0].command: an exec command cannot be empty\n  remediation: Supply a non-empty value for the indicated field."
);
```

Connect, register and wait for a result correlated to the specific run instance:

```rust,no_run
use windows_task::{TaskPath, client::{Scheduler, RegistrationOptions, RunOptions, WaitOptions},
    model::{ExecAction, TaskDefinition}};
let scheduler = Scheduler::builder().local().connect_blocking()?;
let task: TaskPath = r"\Acme\Backup".parse()?;
let definition = TaskDefinition::builder(ExecAction::new("agent.exe")).build()?;
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

A manifest field that is omitted takes the default documented on its Rust type,
and those defaults match Task Scheduler's own behaviour for the equivalent
absent Task XML element — an omitted trigger `enabled` schedules, exactly as an
absent `<Enabled>` element does. Writing a manifest still emits every value, so
a generated document keeps pinning what it pinned. `examples/` holds both the
minimal and the fully explicit form.

The default `tracing` feature records allowlisted operation metadata. The calling
application owns subscriber configuration; the library never installs one.
Reconciliation returns a structured journal on both success and failure. Examine
`unresolved`, `rollback_failures` and `irreversible_effects` before assuming the
original state was restored. Task Scheduler does not offer atomic compare/update.

The optional `handler` feature provides COM DLL exports through the `handler`
attribute. Handler panics require unwind builds for containment; an embedding
application that chooses `panic = "abort"` cannot catch those panics.
