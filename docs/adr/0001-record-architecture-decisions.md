# 1. Record architecture decisions

- Status: accepted
- Date: 2026-04-30
- Deciders: project maintainers
- Tags: process, architecture

## Context

Architectural decisions made during the lifetime of a project shape
the system long after the original authors have moved on. Without a
written record, the rationale behind a choice degrades into folklore
and the team eventually rebuilds the same trade-off from scratch — or
worse, walks back into a known dead end.

## Decision

Record architecturally significant decisions as Architecture Decision
Records under `docs/adr/`, formatted as
[MADR 4.0](https://adr.github.io/madr/). One file per decision; never
edit a decision after it is accepted — supersede it with a new ADR
that links back.

The template lives at `docs/adr/0000-template.md`.

## Consequences

* New contributors have a written trail of *why* the system looks the
  way it does, which lowers onboarding cost and reduces churn from
  re-litigation.
* Each decision incurs a small documentation cost; the cost is paid
  once and amortised across every future reader.
* Superseding an ADR rather than editing it preserves history;
  reviewers can always see what the previous reasoning was.

## Alternatives considered

* **No formal process.** Rejected: the decisions still get made, but
  the rationale evaporates.
* **A single rolling design document.** Rejected: linear narrative
  obscures which choices are still load-bearing versus historically
  interesting.
* **Tickets in the issue tracker.** Rejected: search and discoverability
  are weaker than files in the repository, and tickets are routinely
  closed and forgotten.

## References

- Michael Nygard, "Documenting Architecture Decisions" (2011),
  <https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions>.
- MADR 4.0 specification, <https://adr.github.io/madr/>.
