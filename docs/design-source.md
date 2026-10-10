# Accepted planning source

Snapshot of Kizn Item `01a12810-3a69-7f17-96f2-010ade0b85af`, revision 6, captured 2026-10-10. This is an accepted design record, not a separately authored PRD. The numbered decisions below are the source of verbatim coverage quotes. Glossary revision 5 supplies the current domain contract; see [CONTEXT.md](../CONTEXT.md).

# Herdr-inspired app — confirmed design record

Accepted by Tommy on 2026-10-10 at the end of a structured grilling session. This is a standing decision set, not a proposal. Authority: Tommy answered every frontier round and confirmed the final record in one word ("I confirm").

## Decisions

1. **Identity**: personal daily driver; product option deliberately held open, not pursued now.
2. **Thesis**: UI composition as data. Core is a small semantic core (sessions, panes, focus, reserved `host:` action semantics); everything visual — including the stock sidebar and tab strip — is declared through the public plugin API. No privileged built-in rendering.
3. **Declaration model**: widget tree as data (row/column/label/leader/spacer/list, zones, tones, `host:` actions). Flat rows are the degenerate case; cell-grid plugin-rendered regions are a possible future node type, not built now.
4. **Architecture**: new repo; herdr runtime vendored as crates (PTY management, libghostty-vt, session persistence/restore). Server-side session API reused verbatim; the UI/plugin protocol is designed fresh.
5. **Client**: Rust. Reactive store fed by server events; screens are pure projections. Widget declarations hot-apply without client rebuilds. Latency gate: ≤100ms click-to-repaint with 15 populated panes, for both local and remote sessions (Tommy confirmed remote scope on 2026-10-10).
6. **Priority order** (settled): performance, customizability, type safety, authoring ease. This order is why the client is Rust, not TS.
7. **Retained from herdr**: `--remote` single-attach SSH sessions; clipboard-image paste (client intercept, server stages under /tmp/herdr-clipboard-images-<uid>/, path injection into the pane); PTYs as the primary surface.
8. **Deferred**: multi-endpoint aggregation, update channels, Windows.
9. **Acceptance demo**: port the fork's agent-tree plugin onto the new widget-tree API as the first real consumer; the TanStack DB store and stdin-envelope producer transport carry over.
10. **Spike**: 2-3 weeks. Exit test = runs Tommy's real session end-to-end with the stock UI fully plugin-declared, `herdr --remote`-style attach from achy, image paste working, latency gate met. Failing the exit test returns work to plugin development on the herdr fork.
11. **Future-client compatibility** (Tommy approved 2026-10-10): Renderer-neutral shared contracts are a spike requirement. The shared Core owns session state and action semantics; each client owns its renderer. Shared declarations express structure, actions and semantic styling without ratatui types, terminal escape sequences or fixed terminal-cell geometry. Pane Surfaces remain terminal-state data consumable by any client renderer. GUI and web clients, browser transport and browser authentication are not implemented in this spike.

## Future client direction

Tommy intends to add GUI and web clients for this fork in the future (2026-10-10). No GUI toolkit, web transport, browser authentication model or implementation scope has been selected. Renderer-neutral shared contracts are now approved for the spike; the future GUI and web implementations remain outside its delivery scope.

## Relationship to existing work

The herdr fork work (bookmark plugin-reactive-store) remains the daily driver until the spike passes its exit test. Further herdr-side plugin-API investment freezes at spike start except bug fixes.

## Runtime extraction inventory (read-only scout, 2026-10-10)

Vendors cleanly into crates: `src/pty/` (~1 KLOC, portable-pty backend + async actor); `crates/ghostty-vt` + `vendor/libghostty-vt` (VT engine, already a crate); `src/api/` schema + transport (~5–8 KLOC, only lightly TUI-coupled — this is the session API reused verbatim); `src/persist/` (~2–4 KLOC); `src/server/` (~8–12 KLOC, includes `--remote` attach in client_transport.rs and clipboard-image staging in clipboard_image.rs).

Coupling zone needing surgery: `src/app/` handlers use ratatui Direction, ui helpers and pane chrome; `src/pane/terminal.rs` (7.6 KLOC) depends on ratatui Rect/styles; `src/protocol/wire.rs` (3.7 KLOC) mixes server and client messages with no clean crate split today.

Spike approach: vendor a coarse `runtime` crate that keeps ratatui as a data-types dependency rather than chasing the clean split up front; isolate renderer-specific types behind the runtime adapter as the fresh shared client contract is introduced; do not export ratatui types in that contract.

## Open follow-ups

- New repository is provisionally named `herdr-fork` (Tommy, 2026-10-10); do not spend planning time renaming it.
- Domain glossary terminology is settled: Core, distinct Surface and Zone, and Declaration/Projection. The reserved-action prefix remains a build decision.
- ADR candidates listed in the domain-model Item; promote to docs/adr/ when the repo exists.
