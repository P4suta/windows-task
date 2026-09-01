# 2. Isolate Task Scheduler COM on a session worker

- Status: accepted
- Date: 2026-09-02
- Deciders: project maintainers
- Tags: windows, com, concurrency, security

## Context

Task Scheduler is an apartment-aware COM API. Rust applications may already
have initialized COM with a different apartment model or established
process-wide COM security. Passing raw proxies through async runtimes would mix
thread affinity, cancellation, and RPC lifetime concerns with every call.

`CoInitializeSecurity` is process-wide and normally succeeds only once. A
library cannot safely assume it owns that decision.

## Decision

Each `Scheduler` owns one private MTA worker. Connection and all interfaces
created from it remain on that worker. Blocking calls use a reply channel;
executor-neutral futures queue the same owned jobs and can cancel before a job
starts.

Do not call `CoInitializeSecurity`. Apply `CoSetProxyBlanket` to scheduler
proxies. Offer a strict policy that fails when the blanket cannot be applied and
a default policy that respects an embedding process's existing configuration.

Open a separate Windows Event Log RPC session for remote history because it is
not part of `ITaskService`.

## Consequences

- COM interface thread affinity is enforced structurally.
- No Tokio or async-std dependency is required.
- One slow RPC serializes later operations for that session; callers can open
  independent sessions for parallelism.
- Dropping an already-started future does not cancel an in-flight native RPC.
- Remote scheduler access and remote history access can succeed or fail
  independently and diagnostics must report both.

## Alternatives considered

- **Initialize COM on every caller thread.** Rejected because executors move
  futures and applications may own apartment setup.
- **Treat COM interfaces as `Send`.** Rejected because it would encode an
  invalid guarantee for arbitrary proxies.
- **Call `CoInitializeSecurity` automatically.** Rejected because a library
  cannot safely take process-wide ownership.
