---
date: 2026-10-22
status: proposed
---

# Select checks by changed contract

## Status

Proposed.
This changes CI cadence while preserving the diagnostic and proof contracts in [ADR 0006](0006-make-failures-observable-and-reproducible.md) and [ADR 0007](0007-verify-absolute-xml-extension-ordering.md).

## Context

Portable tests previously invoked Kani on Linux, so the same test command depended on the host and optional verifier installation.
Every source revision also repeated packaging, coverage and platform checks, alongside recurring fuzzing, mutation and resource campaigns.
The required check accepted skipped dependencies without distinguishing selected work.

## Decision

Separate portable tests, isolated feature tests, the existing eight implementation proofs, package consumers and disposable native suites.
Use the [CI path filters](../../.github/filters/ci.yml) to select the affected contracts while keeping portable checks mandatory.
Validate selector flags before forcing all checks for an explicit release revision.
Require success for selected jobs and a skipped result for unselected jobs.
Check the actual workflow decision expressions over their complete supported finite input domain.

Keep pinned tools and the Rust contributor floor.
Remove duplicate push runs and recurring verification without a concrete defect hypothesis.
Preserve local investigation commands and per-run failure artifacts.

Running every check on every revision was rejected because it repeats unrelated work.
Moving broad campaigns to a schedule was rejected because a schedule alone supplies no missing correctness argument.
Accepting any skipped job was rejected because it can turn missing required evidence into a successful check.

## Consequences

Documentation-only changes avoid native suites and proofs unless they change package contents.
Changes to proof inputs still execute every named implementation harness.
Release verification still checks every supported contract at the exact requested revision.
Path filters are part of the gate and must change when a contract gains a new source dependency.
External action behavior and hosted runner isolation remain trust assumptions outside the finite jq decision checks.
