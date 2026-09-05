# Reliability API migration

## New task settings

`TaskSettings::default()` and `TaskDefinition::new()` now enable unified
scheduling. Current Windows x64 and ARM64 acceptance runs showed registration
changing an explicit false to true, causing the conservative apply verifier to
report a conflict. The new default avoids that mismatch for new modern tasks.
XML input still treats an absent element as false, as required by the schema.
Explicit false values are never ignored or silently normalized by reconciliation.

For schema 1.2, set both `definition.schema_version = TaskSchemaVersion::V1_2`
and `definition.settings.use_unified_scheduling_engine = false`. Verify that the
target preserves this legacy setting; a changed setting remains unconfirmed.

## Waiting and history

`WaitOptions::default()` now requires exact instance-correlated completion.
Set `allow_polling_fallback: true` deliberately to accept an estimate, or pass
`--allow-polling-fallback` to `task run --wait`. `RunOutcome.fallback_reason`
explains estimates. On provider versions that omit the terminal result field,
the last correlated action result is paired with the task completion event.

Watchers read forward with a bookmark and a bounded 256-event delivery queue.
`HistoryGap` means the anchor was cleared or expired; start a new watcher only
after recording the gap. Query scan-limit exhaustion returns `QueryLimit`.
Watch `HistoryQuery.limit` does not cap the lifetime stream; native page size is
fixed and bounded. Query direction applies to one-shot queries, not watchers.

`Scheduler::shutdown(timeout)` closes all clones to new operations and waits for
queued work and COM cleanup. Timeout means termination is unconfirmed. Drop no
longer waits indefinitely for native calls. `HistoryWatcher::shutdown(timeout)`
provides the corresponding helper-thread wait. Neither force-aborts COM RPCs.

## Reconciliation

Ownership now resides in `RegistrationInfo.Source`: the marker is followed by
a newline and the original Source text when supplied. Windows was observed to
rewrite URI to the task path, making URI-only ownership unreliable. Legacy URI
markers are still recognized if present. A path alone never establishes ownership;
tasks whose old marker was lost require explicit adoption after review.

Use `plan_live` for native plans: it resolves account names to SIDs and normalizes
SDDL before comparing. `plan_state` remains an offline comparison and cannot
resolve account aliases. Apply performs the same normalization. On the local
non-elevated test host, Windows forced unified scheduling even when false was
requested. Such a definition mismatch remains a conflict, not a successful apply;
use target-supported explicit settings. This restriction is recorded by the native
test artifacts rather than hidden by ignoring the field.

`ApplyReport` includes `journal`, `unresolved` and `irreversible_effects`.
Its required `status` field is `succeeded` or `failed`. Use `succeeded()` rather
than inferring success from an empty plan: preflight failures also have empty
plans, but retain `status: failed`. Complete compensation does not change a
failed apply into a successful one.
`rollback_failures` now contains structured `RollbackFailure` values with the
original `Error`, phase and change. The CLI emits `{ cause, report }` on failure
and retains exit code 1. A successful apply still emits its report directly.

`Plan::with_stop_running()` includes planned irreversible stop requests;
`plan --stop-running` exposes the same evidence. Read `rollback_complete()` in
addition to individual successful compensation steps. A stop cannot be undone
by restoring XML. Detected external drift prevents automatic overwriting.

## Diagnostics

Enable a `tracing` subscriber in your application to receive structured events;
the default library features include instrumentation but install no subscriber.
Use `--log-level debug --log-format json --log-file incident.jsonl` in the CLI.
`Scheduler::builder().diagnose()` returns a report even when connection fails.
`doctor --bundle new-directory` writes a redacted local bundle. It never probes
write permissions by creating a task and never uploads diagnostics.

Validation errors retain `Error::diagnostics()`. Wait errors carry last-observed
state in `Error::context()`. These fields are not automatically logged.

COM handler release builds use unwind so user panics can become failed
completion. Downstream panic=abort builds still abort the process.
Automatic terminal notification retries once after failure. Both attempts are
traced; two failures leave completion unconfirmed. If the first response was
lost after delivery, the receiver may see the same terminal code again.
An unavailable notification interface is recorded as `handler.unmarshal_status`
with its native error code. The user handler is not started, references are
released on the worker, and completion remains unconfirmed. `Start` acknowledges
packet transfer before unmarshalling, avoiding a caller-side wait on a COM RPC.
