# Planning source inventory

The accepted design is not a formal PRD. This inventory separates its externally observable promises from architectural decisions. Every behavioral acceptance clause and explicit exclusion is enumerated in the coverage ledger by source section; quotations remain verbatim. There are no hidden AC/details blocks in either source Item.

## Architectural and operating constraints

| Source | Constraint | Repository contract |
|---|---|---|
| Decisions 1 | Personal daily driver; product option open, not pursued | ADR 0004 |
| Decisions 2–3 | Small Core; UI composition as data; plugin declares, client renderer draws | ADR 0002, CONTEXT.md |
| Decisions 4 | New repo; runtime as vendored crates; fresh UI/plugin protocol only | ADRs 0001 and 0003 |
| Decisions 5 | Rust client; event-fed reactive store; pure projections | ADR 0003, CONTEXT-MAP.md |
| Decisions 6 | Performance, customizability, type safety, authoring ease | ADR 0003 |
| Decisions 8 | Explicit spike exclusions, not later phases of this plan | ADR 0004 |
| Decisions 9 | TanStack DB store and stdin envelopes retained | Agent-tree and action scenarios |
| Decisions 10 and Relationship | 2–3 week spike; keep fork until exit; freeze plugin-API investment except fixes | ADR 0004, spike-validation.md |
| Glossary | Session/Workspace/Tab/Pane/Surface/Attach; Core/Plugin/Producer; Declaration/Projection, Zone/Tone/Role/Stock UI | CONTEXT.md copied verbatim from revision 5 |

Decision 11 adds four shared-contract acceptance clauses and four explicit future-client exclusions, each separately inventoried in the ledger. ADR 0005 governs ownership and renderer-neutrality.

The runtime extraction inventory is implementation evidence, not additional user requirements. The accepted design record’s old Host spelling is historical; glossary revision 5 governs current naming. The reserved-action prefix is a build decision, not a requirement to preserve `host:`.
