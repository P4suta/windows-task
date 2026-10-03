# 7. Verify absolute XML extension ordering

- Status: accepted
- Date: 2026-10-03
- Deciders: project maintainers
- Tags: xml, verification

## Context

Unknown XML children carry absolute sibling ordinals.
Counting an inserted extension twice, or counting an absent known child, changes subsequent canonical output.
Round-trip fuzzing exposed this shared boundary across Task, Settings, and RegistrationInfo.

## Decision

Use one private generic ordering core for all three writers.
Represent absent children with `Option` and absolute positions with a type whose index saturates at the current child count without arithmetic.
Move opaque child values through this core and retain parent filtering in the XML adapter.

Run pinned Kani proofs against the production core in the required Linux verification gate.
Check every expected harness individually with exact matching, retaining memory, overflow, unwinding, and assertion reachability checks.
Prove the index contract for every `usize` ordinal and child count, and the merger contract for two optional known children and two extensions with arbitrary `usize` ordinals.
Include the original reachable counterexample and verify that a representative broken implementation fails the gate.

## Consequences

The three extension points share one enforceable ordering rule.
The bounded merger proof establishes length, known-child order, and distinct valid absolute positions; it does not prove arbitrary XML parsing or unbounded collection sizes.
The XML adapter and public canonicalization retain regression tests and coverage-guided fuzzing.
Linux verification requires the pinned Kani toolchain in addition to the supported Rust MSRV.

## Alternatives considered

Separate writer fixes duplicate the same positional invariant and allow them to drift.
Tests alone leave symbolic overflow and ordering cases outside the exercised examples.
A separately rewritten proof model would require a refinement argument that directly importing the production core avoids.

## References

- [Bounded, lossless XML boundary](0003-use-a-bounded-lossless-xml-boundary.md).
- [Production ordering core](../../crates/windows-task/src/xml/ordering.rs).
- [Production verification entry point](../../crates/xtask/src/verification.rs).
- [Kani documentation](https://model-checking.github.io/kani/).
