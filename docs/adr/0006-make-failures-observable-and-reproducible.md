# 6. Make failures observable and reproducible

- Status: accepted
- Date: 2026-09-05
- Tags: testing, diagnostics, recovery, lifecycle

## Decision

Correctness, user safety, reproduction and diagnosis take priority over delivery
speed. Production algorithms use internal fault boundaries; tests execute those
algorithms with deterministic clocks and scripted native responses. The public
API does not expose injection switches.

Trace spans propagate the caller's dispatcher across native worker threads.
Only operation names, generated IDs, phase, duration, error kind and native code
are logged. The library installs no global subscriber, panic hook or COM security.

Apply records attempts and observations, rechecks state before mutation and
compensation, and reports ambiguous responses conservatively. Stop requests are
irreversible effects even when definition rollback succeeds. Task Scheduler
offers no compare-and-swap primitive, so rechecking reduces but cannot eliminate
external races. Never overwrite detected external changes.

Exact run results require instance-correlated terminal events and, on provider
versions without a terminal result field, the correlated final action result.
Polling estimates require explicit consent. Watchers use bookmarks with anchor
identity checks, bounded delivery and explicit gap errors.

Drop detaches rather than blocking on arbitrary RPC duration; explicit shutdown
can wait for confirmed completion. Native objects are still destroyed on their
owning COM thread. Handler stop is terminal for an invocation; a new invocation
gets a fresh token. Reporter references prevent premature DLL unload.

## Consequences

The report and result APIs evolve before 1.0. Consumers must handle unresolved
states, structured compensation errors and strict wait defaults. Diagnostic
bundles omit message bodies and target names; users supply those separately when
they choose to disclose them. Capture the first failing run, never only retries.

Unwinding panics in handler construction/execution are reported as failure.
Applications built with panic=abort cannot receive that containment guarantee.

Startup retains ownership of an unconsumed marshal packet until the worker's
COM initialization succeeds. Failed startup releases marshal data in the
originating apartment, as required by
[CoReleaseMarshalData](https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-coreleasemarshaldata).
