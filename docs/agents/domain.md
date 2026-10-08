# Domain Docs

## Layout

This repository uses a single-context layout:

- `CONTEXT.md`: domain glossary at the repository root.
- `docs/adr/`: architectural decisions.

These documents are created lazily when domain terms or decisions
are resolved. If absent, proceed silently.

## Before exploring

Read `CONTEXT.md` and relevant ADRs when present.
Read the existing documents relevant to the task:

- `docs/architecture.md`
- `docs/cleaning-architecture.md`
- `docs/cleaning-rules.md`
- `docs/reporting-rules.md`

Keep business rules in their existing documents rather than
duplicating them in the glossary.

## Vocabulary and decisions

Use domain terms as defined in `CONTEXT.md`.
Reconsider unfamiliar terminology or note a real glossary gap
for domain modeling.

Explicitly flag proposals that conflict with an existing ADR.
Do not silently override recorded decisions.
