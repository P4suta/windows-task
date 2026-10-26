# Continuous integration

Every pull request and merge-group revision runs formatting, strict Clippy, source rules, portable tests, documentation, prose and workflow lints, and a fresh dependency audit.
The portable gate runs for every change, including CI and dependency updates.
`just test` runs the portable test suite without installing or invoking Kani.
`just ci` keeps the full local verification surface, including proofs on Linux.

Heavy checks run through the existing release caller with an explicit resolved revision.
The full inventory includes disposable x64 and ARM64 Windows native suites, the eight named Kani harnesses, isolated feature combinations, and consumers built from generated package archives.
Ordinary changes select none of those jobs, regardless of their paths.
The [release-only decision](adr/release-only-heavy-checks.md) records the scheduling and assurance boundaries.

The CI selector admits only the strings `true` and `false` for `FULL`.
The empty revision default selects ordinary mode, while the release caller's resolved commit selects full mode.
Every heavy-job output equals that validated mode.
Malformed or missing mode inputs fail before a successful selection is written.
No path-filter action or configuration participates in selection.

The required check demands successful portable work, successful selected jobs, and skipped unselected jobs.
It rejects failed, cancelled, missing, unfinished, unknown, or incorrectly skipped work and malformed selection outputs.
The [CI contract tests](../crates/xtask/tests/ci_selection.rs) execute the production shell and embedded jq for both admitted modes and every supported summary selection and result combination.
The existing proof inventory and first-failure behavior remain in [the verification command](../crates/xtask/src/verification.rs).
Finite jq evaluation is source-bound decision evidence, not a proof of the jq engine or GitHub's event and input interpretation.
Unrun checks and proofs supply no passing evidence.

Release verification resolves a version tag to a commit ID and calls the reusable workflow with that revision.
The explicit revision forces every heavy contract, and build depends on successful verification before publication.
Checkouts use the same resolved source.
The [release workflow tests](../crates/xtask/tests/release_workflow.rs) check the production caller and source references.
An environment name in the workflow does not establish remote approval protection.

`just features`, `just prove`, and `just package` remain available for a concrete local investigation.
Native mutation, account, and Event Log tests run only on disposable hosted Windows runners.
Keep their opt-in environment variables unset on personal machines.
Each run saves its commands, results, stdout and stderr under `target/verification`.
An unsupported host or incomplete run supplies no passing native evidence.

CodeQL scans changed Rust code and workflow inputs.
The workflows do not repeat PR verification on default-branch pushes or recurring schedules.
Coverage reports, resource measurements, broad fuzzing, mutation campaigns, and portable Miri runs remain outside automatic hosted work.
