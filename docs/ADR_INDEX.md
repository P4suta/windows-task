# Architecture Decision Records

This directory holds [MADR 4.0](https://adr.github.io/madr/)
Architecture Decision Records. Each file documents one decision; once
accepted an ADR is never edited — it is *superseded* by a later ADR
that links back.

| ADR | Title | Status |
| --- | --- | --- |
| [0001](./adr/0001-record-architecture-decisions.md) | Record architecture decisions | accepted |
| [0002](./adr/0002-isolate-com-on-a-session-worker.md) | Isolate Task Scheduler COM on a session worker | accepted |
| [0003](./adr/0003-use-a-bounded-lossless-xml-boundary.md) | Use a bounded, lossless Task XML boundary | accepted |
| [0004](./adr/0004-make-reconciliation-owned-and-compensating.md) | Make reconciliation owned and compensating | accepted |
| [0005](./adr/0005-generate-a-contained-com-handler-server.md) | Generate a contained COM handler server | accepted |
| [0006](./adr/0006-make-failures-observable-and-reproducible.md) | Make failures observable and reproducible | accepted |

## Authoring a new ADR

1. Copy `adr/0000-template.md` to `adr/NNNN-short-slug.md` with the
   next sequential number.
2. Fill in the sections; keep paragraphs short and action-oriented.
3. Add a row to the table above.
4. Open a PR. ADRs are normally accepted on merge; controversial ones
   are landed as `proposed` and flipped to `accepted` once the
   discussion concludes.
