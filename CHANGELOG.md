# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/).

## [Unreleased]

### Added

- Recorded verification suites, deterministic native-fault boundaries, compile
  contract tests, input fuzzing, and release DLL/packaged-consumer fixtures.
- Optional structured tracing, connection-failure diagnostics, redacted CLI
  bundles, and classified apply/rollback journals with unresolved-state reports.

### Changed

- New task settings enable the unified scheduling engine by default. XML input
  retains the schema's false default; explicit false requests remain significant.
  Schema 1.2 definitions must explicitly disable the engine.
- Run waits require exact instance correlation by default. Polling estimates
  require an explicit option and include their reason.
- Session/watcher destruction no longer joins an unbounded RPC; explicit
  shutdown methods wait for confirmed termination with a deadline.
- Handler stop remains terminal, progress/completion are serialized, and live
  reporter references prevent DLL unloading. Release panic mode uses unwind.
- Apply failures now include structured compensation errors and irreversible
  stop effects. See [migration notes](docs/migration-reliability.md).

### Fixed

- Failed handler status unmarshalling now retains classified trace evidence;
  native failure tests verify packet release, counters and a healthy restart.
- Apply reports distinguish preflight failures from successful empty plans.
- Automatic handler completion retries one failed notification and retains an
  unconfirmed state when both attempts fail.
- YAML output quotes ambiguous strings, including trailing colons, whitespace
  and control characters found by guided fuzzing, while retaining Rust 1.85.
- Stop requests during compensation are included in irreversible-effect reports,
  so removing a newly created task cannot falsely imply complete restoration.
- Recovery includes observed SACL sections for newly created folders. Conditional
  ACE strings no longer cause unrelated security sections to be requested.
- Standard XML entities and valid numeric references no longer fail parsing.
- Event batches release all handles on early exit; bookmark watchers detect
  lost anchors and apply bounded backpressure.
- Registration response loss is reobserved before compensation; detected
  external changes are not overwritten during recovery.
- Native ownership uses preserved Source markers because Windows rewrites URI.
  Live planning resolves account names to SIDs before comparing definitions.
- ACL setters use COM registration flags instead of security-information flags;
  comparison preserves ACE order and skips equivalent writes.
- Inactive native IdleSettings no longer imply idle-only execution.
- Doctor preserves completed checks when saving a bundle fails. Packaged release
  consumers verify both executable and handler DLL linking and execution.

### Initial implementation

- Complete owned Task Scheduler 2.0 model for schema versions 1.2 through 1.6.
- Bounded, lossless UTF-8/UTF-16 Task XML parsing and canonical writing.
- Dedicated-MTA local and remote scheduler clients with blocking and
  runtime-neutral async APIs.
- Typed Operational Event Log history, watchers, and run-instance correlation.
- Ownership-safe manifest planning, apply, credential preflight, and
  reverse-order compensation.
- Schedule recipes, an exact five-field cron compiler, and stable diagnostics.
- Official CLI and safe COM handler runtime/proc macro with registry tooling.
