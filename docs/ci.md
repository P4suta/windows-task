# Continuous integration

Every pull request and merge-group revision runs formatting, strict Clippy, source rules, portable tests, documentation, prose and workflow lints, and a fresh dependency audit.
`just test` runs the portable test suite without installing or invoking Kani.
`just ci` keeps the full local verification surface, including proofs on Linux.

The [path filters](../.github/filters/ci.yml) select additional contracts:

| Check | Selected inputs | Execution |
| --- | --- | --- |
| Windows native | Rust code, native fixtures, manifests, toolchain and CI configuration | Disposable x64 and ARM64 Windows runners |
| Implementation proofs | XML, model, verification, release identity, dependencies and CI configuration | The eight named Kani harnesses with one proof worker |
| Isolated features | Library source, feature declarations, dependencies and verification commands | Each supported library feature without default features |
| Package consumers | Crate contents, root README, licenses, manifests and tool configuration | Consumers built from the generated crate archives on Windows |

`just features`, `just prove`, and `just package` invoke those checks separately.
The proof inventory and its first-failure behavior live in [the verification command](../crates/xtask/src/verification.rs).
The required check accepts successful selected work and skipped unselected work.
It rejects failed, cancelled, missing or incorrectly skipped selected work, unknown jobs, and malformed selection flags.
[The CI contract tests](../crates/xtask/tests/ci_selection.rs) execute the production shell and jq expressions, including all supported selection and result combinations.

Release verification calls this workflow with an exact source revision.
A nonempty revision forces every contract after validating the path-filter outputs.
This preserves complete release verification even when the source matches the default branch.

Native mutation, account, and Event Log tests run only on disposable hosted Windows runners.
Keep their opt-in environment variables unset on personal machines.
Each run saves its commands, environment, results, stdout and stderr under `target/verification`.
An unsupported host or incomplete run supplies no passing native evidence.

CodeQL scans changed Rust code and workflow inputs.
The workflows do not repeat PR verification on default-branch pushes or recurring schedules.
Coverage reports, resource measurements, broad fuzzing, mutation campaigns, and portable Miri runs are no longer automatic hosted work.
Their project entry points remain available for a concrete investigation.
