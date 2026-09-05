# Verification and incident reproduction

Run `cargo xtask ci` (also `just ci`) for the shared checks. This defaults to
portable execution and Windows cross-compilation. Use `cargo xtask ci --suite all`
to include native mutation tests on a disposable Windows host. The promised
toolchain is Rust 1.85.0. Each invocation creates a new directory under
`target/verification` with environment.json, results.json, original stdout and
stderr logs, and reproduction instructions. Commands are stored as argument
arrays so shell quoting cannot change their meaning. No environment dump is
collected. A missing tool is `not_run`, never passed.

Verification-run unit tests deliberately create failed and unstarted fixture
commands to test that evidence contract. Their enclosing test result determines
whether the regression passed; a recursive search for any failed results.json
entry is not a CI verdict. Compiler caches are excluded from uploaded evidence,
while logs, source inputs and actual `.crate` archives are retained.

Run coverage after CI has finished. The coverage tool cleans some shared test
artifacts, so running it concurrently with trybuild can collide with loaded DLLs
on Windows. Retain that first failure if it occurs and rerun sequentially.

`cargo xtask test --suite portable` runs deterministic library/CLI tests and
documentation. `--suite windows` selects native integration tests, including
explicitly ignored mutation tests, on an isolated disposable Windows host.
`--suite all` runs both. Mutation tests have unique names and must report cleanup
errors; running ordinary `cargo test` leaves them visibly ignored.

The native suite now also verifies a real Operational log clear. This requires
`GITHUB_ACTIONS=true`, `RUNNER_ENVIRONMENT=github-hosted` and the explicit
`WINDOWS_TASK_CLEAR_EVENT_LOG=1` acknowledgement supplied by CI. The test refuses
local and self-hosted execution before connecting or changing any state. It uses
`wevtutil cl` only for the Task Scheduler Operational channel, then checks a
terminal `HistoryGap` through the public watcher. Native mutation tests run
serially so clearing cannot race another fixture's exact execution wait.
Other mutation tests retain isolated task paths and cleanup reports; clearing
the disposable host's log is intentionally irreversible.

The same gate protects the retention-overwrite fixture. It saves the original
channel size and log mode, selects a 1 MiB circular log and clears it before
establishing an anchor. A slow receiver holds the bookmark while bounded changes
to one fixture task overwrite old events. The test verifies the oldest retained
record has advanced past the buffered window, then requires terminal `HistoryGap`.
Cleanup restores the original size, mode and enabled state and removes the task.
The fixture does not simulate a clock-based expiry or alter a local user's log.

Read-only native tests separately exercise missing bookmarks and invalid event
handles. Handler tests combine sixteen failing concurrent completion requests,
failed progress, panic, terminal notification retries and a retained reporter;
unconfirmed completion and resource release are checked independently.
Notification commands carry their caller's operation context to the COM worker,
so native progress/completion timings and failures remain children of the reporter
operation. Status message text is excluded from those trace fields.

Password-backed native restoration uses a separate `WINDOWS_TASK_ACCOUNT_TESTS=1`
acknowledgement on GitHub-hosted CI. It creates a UUID-named temporary account,
passes its generated password only through subprocess stdin, and suppresses raw
subprocess output. Failure evidence retains only exception type, numeric HRESULT
and category, native status, exit code and stage; a sentinel test checks secret exclusion.
The packaged fixture script uses `NetUserAdd`/`NetUserDel` directly, so it does not
depend on optional LocalAccounts cmdlets. Both the Rust entry and script enforce
the disposable-host gate; the script also restricts names to the generated namespace.
A wrong credential on the second update must restore the first
update with its backup credential; valid retry and a zero-diff repeat follow.
The fixture removes both tasks and the account even after an assertion failure.
Local and self-hosted execution is refused before account or scheduler changes.

The production reconciliation backend, run observer and watcher delivery loop
are exercised with deterministic faults. Response loss after mutation, failed
compensation, inaccessible history and delayed completion must remain regression
tests. Keep the first failure even if a later attempt succeeds.

Coverage on Linux measures portable execution. Windows execution and ARM64
compilation are distinct evidence. An ARM64 build is not an ARM64 runtime test.
The separate `Windows ARM64 native` job uses the public `windows-11-arm` hosted
runner and explicitly selects the ARM64 Rust 1.85 host. It executes the common
test suites and packaged consumers, including the native DLL and destructive
fixtures, and uploads `verification-windows-arm64` independently. Its actual
runtime outcome must be checked; x64 cross-compilation is not a substitute.
Remote RPC, credentials and Event Log access need a dedicated remote host.

Run the fixed input corpus locally with
`cargo run --manifest-path fuzz/Cargo.toml --example seed-replay`.
The example shares the fuzz oracle but is not a libFuzzer target. Native resource
checks accept `-TimeoutSeconds` and preserve a failure report when their disposable
test process exceeds the deadline. This does not change production COM shutdown.

The initial local baseline is recorded in
`target/verification/baseline/test.log`: 28 library tests passed, but the native
read-only smoke failed because standard XML references were rejected. The old
mutation test returned success without executing; the new suite marks it ignored.

The implementation and local execution status is recorded in
[the reliability verification report](verification-status.md). That report also
lists acceptance scenarios still requiring additional verification. A passing
portable suite alone does not establish completion of the full reliability plan.
