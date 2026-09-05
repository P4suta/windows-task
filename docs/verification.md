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

The production reconciliation backend, run observer and watcher delivery loop
are exercised with deterministic faults. Response loss after mutation, failed
compensation, inaccessible history and delayed completion must remain regression
tests. Keep the first failure even if a later attempt succeeds.

Coverage on Linux measures portable execution. Windows execution and ARM64
compilation are distinct evidence. An ARM64 build is not an ARM64 runtime test.
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
