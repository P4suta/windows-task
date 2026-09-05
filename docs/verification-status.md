# Reliability implementation and verification status

Recorded on 2026-09-05, Windows x64, Rust 1.85.0. The local runs below preceded the commit;
each run records the base revision and working-tree status. Paths below are
relative to `target/verification/`. This is local evidence, not a claim that the
every GitHub workflow has executed successfully. The PR evidence below records
the subsequent hosted checks separately.

## Pull request verification

[PR 5](https://github.com/P4suta/windows-task/pull/5) is the development review.
No release, publication or version tag has been performed.

The first CI run, `33954240164`, passed Linux checks and package consumers but
measured only 56.52% region coverage against the existing 70% floor. Windows
exposed a running-instance disappearance race and native ACL normalization.
The next run, `33955285052`, measured 61.88% after including integration processes
and adding CLI/verification contracts. It confirmed exact instance correlation;
the native fixture still incorrectly expected raw exit 7 instead of its possible
HRESULT encoding, and compared the informational ACL inheritance bit literally.
Both sets of first-failure artifacts are retained under `target/verification`.
Run `33956342439` reached 67.59% regions / 70.77% lines and passed the SYSTEM
execution and apply-idempotence fixtures. The ACL fixture still needed explicit
conversion of inherited ACEs before changing DACL protection.

On revision `8243c96`, run `33957154658` passed the complete Linux job, including
package consumers and the unchanged 70% region floor: **70.45% regions and
73.55% lines**. Its evidence is saved in `github-33957154658-portable/`.
CodeQL and extended native-resource and Miri jobs also passed. Windows passed
29 commands, including all native fixtures; its final Markdown tool could not
load `unicorn-magic`. The pinned npm installation now passes locally, with the
same installer configured for CI. The initial installation error is retained.
The commitlint failure on that revision exposed ignored hyphenated configuration
keys; the corrected underscore keys now enable the documented `ci`/`build`/
`revert` types, with all PR commits checked locally.

The latest local shared CI run, `4fc709d8-27c4-4dcf-bf2c-3a20919a164b`, passed all
26 commands. The later compound-ACE fixture has separate passing regression,
Clippy and native task/folder ACL checks in `pr5-compound-ace-fixed.log`,
`pr5-ace-clippy-fixed.log` and `pr5-compound-ace-native.log`. Local resource
measurement `resources-d8ff1be0-1fe2-460b-8337-eda3170b0cee` passed 1,000 cycles;
`resources-b44b618f-6243-46e0-9208-b650825b60f9` deliberately exceeded its one-second
deadline and correctly preserved a failed result. These deliberate fixture
failures are not substitutes for the enclosing test or workflow verdict.

Extended run `33957154685` tested 205 reconciliation mutants: 91 caught,
91 missed and 23 unviable. The missed cases are retained in
`github-33957154685-mutations/missed.txt`; additional production-boundary tests
now cover report completeness, ownership/pruning, observation failures,
registration credentials/flags and multiple changes to one target. The first
mutation result remains a failure until a subsequent run is inspected.

The same run found a YAML serialization defect after approximately 8.5 million
guided executions: a trailing colon was emitted as an ambiguous plain scalar.
The original crash is saved in `github-33957154685-fuzz/` and checked in as
`fuzz/seeds/yaml-trailing-colon.bin`. A minimized normal test exercises that
case plus nested strings, control characters and whitespace; output now uses
quoted YAML 1.2 scalars. The original fuzz run failed and must not be described
as a successful 20-minute run. The expanded release DLL fixture also passed
concurrent progress/completion and a failed-then-retried completion callback.

## Completed local checks

| Check | Evidence | Result |
| --- | --- | --- |
| Shared CI entry point | `3395cb41-4309-4ed5-a0c7-f8c7db2839ef/results.json` | All 26 commands passed: formatting, Clippy, tests, feature configurations, docs, x64/ARM64 compilation, source rules and dependency/tool checks |
| Unit tests | Same run, `02.stdout.log` | 71 library, 2 CLI and 2 macro unit tests passed; native DLL test visibly ignored here and executed separately |
| CLI processes | Same run, `04.stdout.log` | Invalid apply JSON/exit code and preconnection doctor/bundle failure contracts passed |
| Macro contracts | Same run, `05.stdout.log` | Valid expansion and expected compiler diagnostics passed |
| Linux compilation | `final-linux-check.log` | All-feature library and test source compiled for Linux; this is not Linux execution |
| History page fault boundary | `history-page-algorithm.log` | Six tests passed, including shared page logic, anchor reuse, permission errors, batch cleanup and bounded delivery |
| Handler startup fault boundary | `handler-startup-injection-fixed.log` | Worker creation, apartment initialization, user-thread creation, marshal preparation and constructor panic; restart and reference/counter cleanup passed |
| Native marshal packet lifetime | `marshal-reference-fixed.log` | Real marshaled status reference released in its originating apartment |
| Connection cancellation | `connect-cancellation.log` | Dropped connection future prevents an unclaimed connection; a claimed operation retains its started state |
| Native registration and ACLs | `native-acl-final.log` | Isolated current-user disabled task, task/folder ACL read/write and cleanup passed |
| Native apply and idempotence | `native-apply-final.log` | Source payload retained; first apply and second empty plan passed; namespace removed |
| Native lifetime/resource stress | `resources-a5f4e028-1223-4f6d-a82f-659fda070e7c/results.json` | 10,000 connection/confirmed-shutdown cycles passed |
| Packaged release consumers | `4071aae4-58b7-4613-b73e-f2db189d6406/results.json` | All 10 commands passed, including extracted-package executable, CLI and handler DLL |
| Packaged handler execution | Same package run, `08.stdout.log` | Stop/Resume, restart, notifications, panic, retained reporter and unload checks passed |
| Input corpus replay | `seed-replay-first.log` | 1,000 deterministic mutated inputs passed through the fuzz oracle |
| Windows unit coverage | `23e68231-7228-4001-95c3-02579e9b5d35/coverage-windows.json` | Lines 52.00%; regions 46.20% |

Resource samples compare ten observations after warm-up with the final ten:
handles +3.9, private memory +309,248 bytes, threads -1.2. These remained within
the configured bounds (32 handles, 64 MiB, 8 threads). This is a bounded
regression check, not proof of zero leaks under every operation or workload.

The original Windows percentage in the table covers unit tests only. Current
coverage includes normal workspace integration tests and CLI contract processes.
Ignored mutation tests and the separate DLL process remain independent evidence.
Linux's 70% region floor remains configured independently and has now passed on
the hosted Linux runner; Windows measurements must not be substituted for it.

## Reproduced defects and corrections

- XML rejected normal predefined/numeric references found in registered tasks.
  Parser regression tests and native readback now pass.
- Native registration rewrote URI and user identities. Ownership now uses a
  Source marker with the original payload preserved; live plans resolve SIDs.
  A task path alone never grants ownership.
- Inactive IdleSettings incorrectly implied RunOnlyIfIdle. The parser now uses
  the actual setting. Other native behavioral changes remain conflicts.
- COM security setters received security-information flags as registration
  flags. Selected SDDL sections and the proper COM flags now have native tests.
  ACE order, access rights and protection remain significant when comparing ACLs.
- Registration and readback were one operation. The internal commit boundary
  and journal now distinguish committed registration from failed readback.
- Delayed completion could become an unrelated last-result estimate. Waiting
  now defaults to exact GUID correlation and records explicit fallback reasons.
- Worker destruction could wait forever; shutdown now distinguishes a request
  from confirmed completion. Gated tests cover queued, running and completed
  response destruction and timeout followed by successful shutdown.
- Connection futures now share the queued/started/cancelled transition and
  report worker creation failure without panicking.
- Handler stop, completion and reporter lifetime races now have deterministic
  tests plus a release DLL process test.
- Handler startup transfers a marshal packet only after COM initialization;
  failed startup releases the unconsumed packet in the originating apartment.
- The bookmark page algorithm is shared with injected sources. Tests verify
  that parsing/bookmark failures release all 256 handles, including unprocessed
  handles, and that a changed timestamp detects reused record IDs after clearing.
- Packaged documentation depended on the workspace root. Crate-local README
  inclusion now builds outside the workspace.
- The verification consumer's executable and DLL collided on the same Windows
  PDB name. Distinct target names fix the collision.

First failures remain available, including `baseline/test.log`,
`native-apply-inspect.log`, `native-apply-normalized-second.log`,
`native-acl-explicit-inspect.log`, and package run
`08b88175-73d0-4610-8089-1f334507662a`. Coverage run
`9a9b5907-73db-450d-aca8-6fccafb570ef` failed because the disk filled; only older
generated compiler caches were removed before the new run. Logs, input sources
and package archives were preserved. Intermediate CI failures also remain;
they are not replaced by the final passing run.
Coverage run `cac08d62-51ed-424b-8a5f-7b2762cd1c58` collided with a concurrently
loaded trybuild DLL. Run coverage after CI, not alongside it: LLVM coverage
cleanup touches test artifacts outside its instrumented build directory.

## Acceptance work still outstanding

The complete seven-phase acceptance criteria are **not yet fully verified**.
Do not describe this report as certification of every listed failure scenario.

- Finish the full Windows workflow after the successful hosted SYSTEM execution
  fixture. The local account can register per-user tasks but SYSTEM registration
  failed with access denied. The fixture restores the original history-enabled
  setting and records cleanup failures.
- Extend native Event Log acceptance tests for actual provider log clearing,
  retention and stale-bookmark errors. The shared production page and delivery
  algorithms now have deterministic paging, parse-failure, handle-release,
  1,024-event/backpressure/gap tests; OS-provider behavior needs separate evidence.
- Native provider/unmarshal errors and additional combinations of concurrent
  completion notification failures still need acceptance coverage. Startup
  allocation/initialization failures and constructor panic are now injected
  through private test boundaries; the release fixture covers lifecycle modes.
- Rerun guided fuzzing after the YAML fix and inspect remaining missed mutants.
  Miri passed on the hosted Linux runner. Local fixed-seed replay is not a
  replacement for guided fuzzing. Direct MSVC libFuzzer linking failed
  (`fuzz-smoke-first.log`); hosted fuzzing runs on Linux.
- Verify remote authentication, password-backed restoration and ARM64 execution
  on dedicated hosts. ARM64 compilation passed; ARM64 execution did not run.
- Expand measured native coverage and executable end-to-end usage scenarios as
  these environments become available. Keep unexecuted scenarios distinct from
  passed commands and preserve every first-failure artifact.
