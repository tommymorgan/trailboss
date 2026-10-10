# Herdr-inspired app — domain model

Ubiquitous language for the app designed in the 2026-10-10 grilling session (see linked design record). Mirrored verbatim in `/home/tommy/src/agents/herdr-fork/CONTEXT.md`. All naming decisions are settled by Tommy as of 2026-10-10; nothing remains contested.

## Sessions and terminals

**Session**: A named, persistent set of workspaces owned by the core; survives client disconnects and restores its panes on relaunch.
_Avoid_: conversation (that is an agent's own session), workspace (smaller unit).

**Workspace**: A named group of tabs sharing one working context inside a session.
_Avoid_: space, project.

**Tab**: One terminal layout within a workspace.
_Avoid_: window.

**Pane**: One live PTY together with its decoded terminal state; the surface agents run on.
_Avoid_: terminal (that names the emulator/protocol), split.

**Surface** (settled 2026-10-10, kept distinct from Zone): One pane's cell grid plus its terminal state (cursor, scrollback); the unit the wire protocol streams to the client.
_Avoid_: using surface for plugin layout areas (that is a Zone), region, view.

**Attach**: A client connecting to exactly one session, locally or over SSH (`--remote`). One client, one session in v1.
_Avoid_: connect, endpoint (reserved for deferred multi-endpoint aggregation).

## Declaration model (the thesis)

**Declaration**: Structured data a plugin emits that describes UI; a client renderer draws it under core-defined semantics. One direction only: plugin declares, client renderer draws.
_Avoid_: render, draw, component (implies code, not data).

**Widget tree**: The declaration model: nested row/column/label/leader/spacer/list nodes, with zones, tones, and core actions attached. Flat rows are the degenerate case.

**Plugin**: The unit of extension — a manifest plus declared actions plus UI declarations. A plugin declares; it never draws.

**Producer**: The long-lived process a plugin runs to emit declarations and receive action envelopes on stdin. Plugin is the identity; producer is the process.
_Avoid_: using plugin and producer interchangeably.

**Core** (settled 2026-10-10; rejected: host, container): The small semantic center that owns sessions, panes, focus and reserved action semantics. Each client owns its renderer and pixels; visual UI composition is declared through the public plugin API.
_Avoid_: host, container, shell.

**Core action**: A client-local affordance in the reserved-action namespace (fork precedent: `host:new`, `host:menu`; the new protocol's prefix spelling is a build decision). A plugin may place and label it; only the core defines what it does.
_Avoid_: builtin action, system action.

**Zone** (settled 2026-10-10, kept distinct from Surface): A named layout slot with client-renderer-defined geometry under core semantics that declarations pin into (list, footer, footer-right). The client renderer maps a zone to geometry; the plugin owns what is in it. A Surface is terminal-state data from a foreign terminal, rendered by the client; a Zone is a reserved layout slot awaiting plugin data. Neither can do the other's job.

**Tone**: A palette-agnostic color name a declaration requests (bg-lime); the client renderer maps tones to concrete colors.
_Avoid_: color, theme.

**Role**: The semantic name on a text segment that selects its styling (label, muted, rule, leader, status-*).

**Stock UI**: The default sidebar and tab strip, shipped as an in-repo plugin on the public API. No privileged built-in rendering exists.
_Avoid_: native UI, default chrome (implies privilege the design removes).

## Client state

**Store**: The client's single authoritative reactive state, synchronized from core events. Screens are pure projections of it; nothing mutates screen state directly.

**Projection** (settled 2026-10-10, kept as a directional pair with Declaration): A pure derivation of screen state from the store. Declarations flow plugin→core; projections flow store→screen.

## Validation

**Spike**: The 2–3 week build whose exit test runs Tommy's real session end-to-end with the stock UI fully plugin-declared, remote attach, image paste, and ≤100ms click-to-repaint at 15 panes. Failing it returns work to the herdr fork.

## Architectural decisions (recorded in the repository’s docs/adr/)

1. Reuse herdr's session API verbatim; design only the UI/plugin protocol fresh.
2. Widget tree as data, core-rendered; no plugin pixel access in v1 (cell-grid region node deferred).
3. Rust client and server with vendored herdr runtime; performance is the top priority and drove the stack choice.

## Naming provenance

All three contested terms settled by Tommy on 2026-10-10: Core (not host, not container); Surface and Zone both kept with distinct meanings (pane terminal-state data vs a declaration layout slot); Declaration and Projection kept as the directional pair.