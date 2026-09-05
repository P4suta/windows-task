# Development priorities

Prioritize user safety and correctness, reproducibility and observability,
usable APIs, maintainability, then development speed. Make routine design
decisions without asking the user to choose between implementation details.

Every behavioral change must include a reproducer or a meaningful regression
test, allowlisted diagnostic context, failure and recovery behavior, and updated
usage documentation. Exercise the production algorithm through internal fault
boundaries; do not substitute a separately implemented algorithm in tests.

Use `cargo xtask ci` as the common verification entry point. Keep failed runs,
seeds, fixed verification-tool arguments and logs under `target/verification`.
These commands must contain only controlled fixture paths and non-secret build
options, never user task arguments or credential values. Never count an
unexecuted native test as passed or hide an initial failure with a retry.

Library code must not initialize global tracing, COM security or panic hooks.
Never log credentials, raw XML, command arguments or arbitrary error bodies.
Do not claim rollback or shutdown completed when native state is unconfirmed.

Native mutation tests run only in explicitly selected suites with isolated
namespaces. CI uses disposable hosts. Preserve foreign tasks and report cleanup
failures separately from the original failure.
