# Compatibility and semantic boundaries

## Supported baseline

- Rust 1.85 or newer, edition 2024.
- Windows Task Scheduler 2.0 interfaces.
- Typed Task XML schema versions 1.2, 1.3, 1.4, 1.5, and 1.6.
- Windows x64 and ARM64 MSVC targets in continuous integration.

Portable model/XML/manifest code is tested on Linux. Live calls fail with the
stable `UnsupportedPlatform` error kind on non-Windows systems.

## Time and recurrence

`TaskDateTime` records whether a boundary is local wall-clock time or a fixed
offset instant. Recipes and the cron compiler preserve that distinction. They
do not emulate a Unix cron daemon: Task Scheduler owns DST gaps/folds, missed
starts, battery/network constraints, and multiple-instance policy.

The five-field cron compiler accepts exact minute/hour/day/month/weekday sets.
POSIX expressions that rely on day-of-month OR day-of-week behavior are
rejected because one Windows calendar trigger cannot represent that ambiguity
without surprising duplicate runs. Compilation also enforces the native
48-trigger ceiling.

## XML preservation

Raw snapshots retain their original bytes and detected UTF-8/UTF-16 encoding.
Typed round-trips preserve opaque elements at supported extension points.
Canonical output is deterministic UTF-8 or UTF-16LE, not byte-identical to
Windows-generated XML.

Convenience parsing is bounded to 8 MiB, depth 64, and 100,000 elements. Event
XML is bounded to 1 MiB and 10,000 elements. DTDs and general entity references
are rejected.

Unknown future schema versions remain visible through the raw snapshot. A live
reconcile that cannot obtain a safe typed definition fails instead of replacing
the task blindly.

## COM ownership

Every scheduler session owns one dedicated MTA thread. Interface pointers never
leave it, and executor-neutral futures only queue owned jobs. Dropping an async
operation cancels it before native execution when possible; an in-flight COM
RPC is not force-aborted.

The crate does not call `CoInitializeSecurity`, because that process-wide call
may already belong to the embedding application. `CoSetProxyBlanket` is applied
to returned scheduler proxies. `RequireProxyBlanket` turns blanket failure into
an error; the default tolerates hosts whose existing policy rejects a redundant
blanket call.

Remote Task Scheduler and remote Event Log are separate RPC services. A
successful scheduler connection does not imply history access. `doctor` reports
the distinction.

## Registration and passwords

Task Scheduler does not return a registered principal's password. Therefore a
password-backed update/delete cannot be guaranteed reversible from scheduler
state alone. Reconcile requires a separate credential resolver during
preflight or classifies the change as irreversible. Plaintext secrets are never
serialized.

Registration triggers are suppressed by default on create/update/rollback.
Callers must explicitly opt into their side effects.

## Event history

Known Task Scheduler event IDs map to stable categories while the original ID
and all named payload fields remain available. This lets newer Windows versions
add fields or IDs without data loss.

Exact run completion requires an instance GUID in Operational history. The
opt-in polling fallback reads `LastTaskResult` only after the requested instance is no
longer observed and a grace interval has passed; its confidence is explicitly
`PollingFallback` because another run can race that property.

## COM handlers

The handler macro generates a conventional in-process class factory and DLL
exports. `ITaskHandlerStatus` is marshaled from the Task Scheduler callback
apartment to a private MTA status worker. User code runs separately and talks to
that worker through channels, so `ProgressReporter` remains safe to move across
threads.

Pause, resume, and stop are cooperative. A handler that ignores
`HandlerControl` cannot be safely terminated by Rust code. Panics are contained
within the user worker, reported as failure, and do not unwind across the COM
ABI when built with `panic = "unwind"`. Downstream `panic = "abort"` cannot be
caught and terminates the host process.
