# 5. Generate a contained COM handler server

- Status: accepted
- Date: 2026-09-02
- Deciders: project maintainers
- Tags: com, handler, ffi

## Context

Task Scheduler `ComHandler` actions require an in-process COM DLL implementing
`ITaskHandler`, a class factory, and the conventional unload exports. Correct
reference counting, apartment transitions, status callbacks, and panic
containment are repetitive and unsafe for every application to rebuild.

## Decision

Provide a `TaskHandler` trait and `#[handler(clsid = "...")]` macro. Generate
`DllGetClassObject` and `DllCanUnloadNow`; keep vtables, class factories, raw ABI
pointers, and module counts inside the Windows-only runtime.

Marshal `ITaskHandlerStatus` into a private MTA worker. Run user code on a
separate Rust thread and route progress/completion through channels. Expose
pause, resume, and stop as a cooperative atomic control token. Catch panics
before they can cross the COM ABI.

Registry mutation remains an explicit `cargo xtask handler` command with
per-user scope by default and mandatory acknowledgement.

## Consequences

- A handler implementation contains safe application logic rather than COM
  plumbing.
- Progress reporting remains usable from user-created threads without moving a
  raw apartment-bound callback pointer.
- Stop is cooperative; the runtime cannot safely kill code that ignores the
  token.
- One macro-marked handler is supported per DLL because export names are unique.
- DLL architecture must match the scheduler host, so x64 and ARM64 builds are
  separate artifacts.

## Alternatives considered

- **Require every user to implement windows-rs COM traits.** Rejected because it
  repeats fragile ABI and lifetime work.
- **Call the status interface directly from user threads.** Rejected because
  apartment requirements would be undocumented and unsafe.
- **Register automatically at build time.** Rejected because builds must not
  mutate user or machine registry state.
