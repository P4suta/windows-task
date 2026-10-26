---
date: 2026-10-26
status: proposed
---

# Release-only heavy checks

## Status

Proposed.
This replaces automatic path-based scheduling in [the previous decision](select-checks-by-changed-contract.md).

## Context

Path-based selection makes a CI workflow edit launch native Windows suites, all implementation proofs, isolated features, and package consumers after portable checks pass.
The ordinary gate already supplies portable regression, lint, documentation, dependency, and workflow checks.
Heavy verification belongs to the exact release source rather than every ordinary source change.

## Decision

Keep the portable gate mandatory on every pull request and merge-group revision.
Derive all four heavy-selection outputs only from the validated `FULL` input in the [actual workflow](../../.github/workflows/ci.yml).
Admit only the strings `true` and `false`, rejecting malformed or missing values without writing a successful selection.
Remove the obsolete path-filter action, flags, and configuration.

The existing release caller resolves the selected tag to a commit and passes that revision into full verification before build and publication.
Preserve all five heavy jobs, the eight existing Kani harnesses, their inventory, verifier pins, and first-failure behavior.
The required summary continues to demand successful portable and selected work, skipped unselected work, complete job identities, and valid selection outputs.

The [actual-source CI tests](../../crates/xtask/tests/ci_selection.rs) evaluate the embedded shell and jq over both admitted mode values and the complete finite summary domain.
Negative cases cover malformed and missing mode inputs, invalid outputs, failed or unfinished jobs, and incorrectly skipped selected work.
The regression retains the former path flags as hostile ambient inputs to show that they cannot override ordinary mode.
The [release tests](../../crates/xtask/tests/release_workflow.rs) bind full verification and downstream build to the production caller and resolved source.

Kani checks the existing Rust proof inventory, not the embedded jq or GitHub expression interpreter.
No suitable existing project verifier models those interpreters.
The bounded alternative evaluates the actual jq implementation over its complete admitted finite domain and checks invalid inputs at the shell boundary.
The jq engine, shell execution, GitHub input interpretation, and hosted runner isolation remain external assumptions.
Unrun evaluations and Kani campaigns remain unrun.
Reconsider the boundary if the selector moves into a language covered by the project's proof tools or gains a larger input domain.

## Consequences

Ordinary changes use one portable gate and a strict summary without launching native suites or proof campaigns.
Release verification still demands the full inventory on the selected source.
Local investigation commands remain available for a concrete question.

Removing only the CI workflow path from filters was rejected because other source paths would still launch heavy PR work.
Moving heavy work to a schedule was rejected because it does not bind verification to a release source.
Accepting every skipped job was rejected because it would hide missing selected work.
A separate Rust mirror of the jq selector was rejected because proving the mirror would not establish the production decision.
