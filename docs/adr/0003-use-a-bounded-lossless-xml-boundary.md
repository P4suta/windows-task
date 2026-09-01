# 3. Use a bounded, lossless Task XML boundary

- Status: accepted
- Date: 2026-09-02
- Deciders: project maintainers
- Tags: xml, compatibility, security

## Context

Task Scheduler's most complete interchange format is XML. Windows versions add
schema elements over time, existing tools attach vendor extensions, and the COM
object model does not expose every value uniformly. A serde-only typed decoder
would either reject future tasks or silently discard information.

XML also crosses a trust boundary when definitions come from files or remote
machines.

## Decision

Keep exact input bytes and detected encoding in every raw snapshot. Decode a
fully owned typed model for known schema 1.2 through 1.6 while retaining unknown
elements at explicit extension points. Provide deterministic canonical UTF-8
and UTF-16LE writers, but do not claim byte-identical typed round-trips.

Bound document bytes, nesting, and element count. Reject DTD declarations and
general entity references. Keep native validation separate from portable
structural and cross-field validation.

## Consequences

- Callers can export exact registered XML even when typed decoding fails.
- Reconcile can compare semantic models without treating Windows formatting or
  registration timestamps as drift.
- Future schema data is not silently lost.
- Canonical writers are maintainable custom mappings and require extensive
  round-trip tests.
- Opaque extensions are preserved but cannot receive semantic validation.

## Alternatives considered

- **Expose only generated XML schema types.** Rejected because the result would
  mirror XML complexity without task-oriented invariants.
- **Normalize all input immediately.** Rejected because unknown data and source
  encoding would be unrecoverable.
- **Use an unrestricted DOM parser.** Rejected because scheduler XML is a small,
  predictable format and should have explicit resource ceilings.
