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

Portable construction and validation:

```rust
use windows_task::model::{Action, ExecAction, TaskDefinition};

let definition = TaskDefinition::new(Action::Exec(
    ExecAction::new(r"C:\Program Files\Acme\agent.exe")
        .args(["--once"])
));
let report = definition.validate();
assert!(report.is_valid(), "{:#?}", report.diagnostics);

let xml = windows_task::xml::to_string(&definition)?;
let decoded = windows_task::xml::from_bytes(xml.as_bytes())?;
assert_eq!(decoded, definition);
# Ok::<(), windows_task::Error>(())
```

Live Windows use:

```rust,no_run
use std::str::FromStr as _;
use windows_task::{TaskPath, client::{RunOptions, Scheduler, WaitOptions}};

let scheduler = Scheduler::builder().local().connect_blocking()?;
let task = TaskPath::from_str(r"\Acme\Backup").expect("absolute task path");
let run = scheduler.blocking().run(&task, RunOptions::default())?;
let outcome = scheduler
    .blocking()
    .wait_for_run(&run, WaitOptions::default())?;
println!("result={} ({:?})", outcome.result_code, outcome.confidence);
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

See [the CLI guide](docs/cli.md) and the checked
[manifest example](examples/desired-state.toml).

## Desired state and rollback

Each managed task gets a deterministic registration URI containing the
manifest owner UUID and task path. By default:

- an unowned collision is an error;
- pruning is off and can delete only tasks carrying the same ownership URI;
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
| `handler` | no | COM handler runtime and attribute macro |

Use `default-features = false` for a smaller portable model/XML-only build.

## Development

```sh
cargo +1.85.0 xtask ci
cargo +1.85.0 xtask check-windows
```

Architecture and security decisions live in [the ADR index](docs/ADR_INDEX.md).
Contribution and disclosure guidance are in [CONTRIBUTING.md](CONTRIBUTING.md)
and [SECURITY.md](SECURITY.md). Maintainer publication steps are documented in
[the release guide](docs/releasing.md).

## License

Dual-licensed under Apache-2.0 OR MIT, at your option. See
[LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT).
