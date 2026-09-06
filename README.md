# windows-task

Safe, task-oriented Rust for Windows Task Scheduler 2.0.

`windows-task` turns the Task Scheduler COM and XML APIs into an owned Rust
model, a dedicated-thread client, typed event history, and an ownership-safe
desired-state engine. The model, XML, validation, schedules, and manifests run
on every platform; live scheduler operations return `UnsupportedPlatform`
outside Windows.

The project is pre-1.0, but it is built for real automation rather than as a
thin COM binding. Rust 1.85 is the minimum supported version.

## What it covers

- Task Scheduler schema 1.2 through 1.6: actions, all modern triggers,
  principals/logon types, settings, maintenance, privileges, metadata, and
  SDDL.
- Bounded UTF-8/UTF-16 Task XML parsing, canonical writing, exact raw snapshots,
  and preservation of unknown extension elements.
- Local and remote `ITaskService` sessions isolated on a private COM MTA worker.
- Blocking and executor-neutral async APIs for tasks, folders, running
  instances, validation, security descriptors, and `RunEx`.
- Operational Event Log queries, remote Event Log sessions, watching, and exact
  run-instance correlation with a labeled polling fallback.
- TOML, JSON, and YAML manifests with ownership markers, dry-run plans,
  opt-in adoption/pruning, credential preflight, and reverse-order rollback.
- Five-field POSIX cron compilation and common schedule recipes without hiding
  Windows wall-clock/DST semantics.
- A proc macro and runtime for in-process `ITaskHandler` COM DLLs, including
  class-factory exports, pause/resume/stop control, marshaled status reporting,
  and architecture-aware registry tooling.
- An official `windows-task` CLI for inspection, mutation, history, diagnostics,
  plan, and apply.

## Library quick start

```toml
[dependencies]
windows-task = "0.1"
```

These snippets are the crate documentation verbatim, so `cargo test --doc`
verifies them. `build` applies the native action/trigger limits and the full
portable validation, so a definition that builds is a definition that validates.
The `Principal` constructors pair an identity with the logon type that matches
it:

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

Live Windows use:

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

Remote credentials are consumed from zeroizing UTF-16 storage. The library
does not call process-wide `CoInitializeSecurity`; it configures individual COM
proxies and leaves process security ownership to the application.

## CLI

Install the binary package:

```powershell
cargo install windows-task-cli
```

Representative commands:

```powershell
windows-task doctor
windows-task task list '\' --recursive --hidden
windows-task task get '\Acme\Backup' --raw
windows-task task run '\Acme\Backup' --wait
windows-task history query --task '\Acme\Backup' --limit 50
windows-task validate .\desired-state.toml --native
windows-task plan .\desired-state.toml --prune
windows-task apply .\desired-state.toml --prune --yes
```

Remote access uses `--computer SERVER`. Supply
`--connection-credential TARGET` to read a generic Windows Credential Manager
entry; there is deliberately no plaintext password flag. Destructive commands
require `--yes`. History enable/disable is explicit and never happens as a side
effect of connecting.

See [the CLI guide](docs/cli.md), the
[minimal manifest](crates/windows-task/examples/desired-state-minimal.toml) and
the fully explicit
[manifest example](crates/windows-task/examples/desired-state.toml).

## Desired state and rollback

A manifest field that is omitted takes the default documented on its Rust type,
and those defaults match what Task Scheduler does for the equivalent absent Task
XML element — an omitted trigger `enabled` schedules, just as an absent
`<Enabled>` element does. Writing a manifest always emits every value, so a
generated document keeps pinning what it pinned even if a default later changes.

Each managed task gets a deterministic ownership marker containing the manifest
owner UUID and task path in `RegistrationInfo.Source`. Existing Source text is
preserved after a newline. Windows rewrites `URI` to the task path. By default:

- an unowned collision is an error;
- pruning is off and can delete only tasks carrying the same ownership marker;
- registration triggers are suppressed during apply and rollback;
- all old XML and ACLs are captured before the first mutation;
- password-backed updates/deletes require a separately resolvable rollback
  credential;
- failure compensates completed changes in reverse order.

Use `--adopt`, `--prune`, or `--allow-irreversible` only after reviewing the
JSON plan. Task Scheduler has no transaction primitive, so compensation is
best-effort when the remote service disappears or the caller explicitly allows
an incomplete snapshot.

## COM handlers

A handler is a `cdylib` with one macro-marked implementation:

```rust,no_run
use windows_task::{handler, handler::{HandlerContext, TaskHandler}};

#[derive(Default)]
struct Cleanup;

#[handler(clsid = "e4ef9b55-4f33-4dd2-a658-6eb2c58c576b")]
impl TaskHandler for Cleanup {
    fn run(self, context: HandlerContext) -> windows_task::Result<()> {
        context.reporter.report_with_message(50, "cleaning")?;
        context.control.wait_if_paused();
        if context.control.is_cancelled() {
            return Ok(());
        }
        context.reporter.report(100)
    }
}
# fn main() {}
```

The macro emits `DllGetClassObject` and `DllCanUnloadNow`. Generate a per-user
registry file without mutation, or register explicitly on Windows:

```powershell
cargo xtask handler reg-file --clsid e4ef9b55-4f33-4dd2-a658-6eb2c58c576b `
  --dll C:\absolute\cleanup.dll --output cleanup.reg
cargo xtask handler register --clsid e4ef9b55-4f33-4dd2-a658-6eb2c58c576b `
  --dll C:\absolute\cleanup.dll --scope user --yes
```

The DLL architecture must match the Task Scheduler host architecture. Build
x64 and ARM64 artifacts separately when targeting both systems.

## Features

| Feature | Default | Purpose |
| --- | --- | --- |
| `client` | yes | Live COM client and folder/task operations |
| `async` | yes | Runtime-neutral operation futures |
| `history` | yes | Event Log query, watch, and run correlation |
| `diagnostics` | yes | Target capability and rights diagnostics |
| `serde` | yes | Model serialization plus TOML/JSON/YAML manifests |
| `reconcile` | yes | Ownership-safe plan/apply/rollback |
| `recipes` | yes | Common schedule constructors |
| `cron` | yes | Exact five-field POSIX cron compiler |
| `tracing` | yes | Structured operation metadata; application owns output setup |
| `handler` | no | COM handler runtime and attribute macro |

Use `default-features = false` for a smaller portable model/XML-only build.

## Development

```sh
cargo +1.85.0 xtask ci
cargo +1.85.0 xtask check-windows
```

Architecture and security decisions live in [the ADR index](docs/ADR_INDEX.md).
See [verification and reproduction](docs/verification.md) and the
[reliability API migration](docs/migration-reliability.md) for strict run results,
structured recovery reports, shutdown behavior and diagnostic bundles.
Contribution and disclosure guidance are in [CONTRIBUTING.md](CONTRIBUTING.md)
and [SECURITY.md](SECURITY.md). Maintainer publication steps are documented in
[the release guide](docs/releasing.md).

## License

Dual-licensed under Apache-2.0 OR MIT, at your option. See
[LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT).
