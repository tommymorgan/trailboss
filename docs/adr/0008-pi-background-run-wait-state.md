# 0008: Pi reports background runs as waiting

**Status**: Proposed 2026-11-09

## Context

pi-subagents emits `herdr:busy` on `pi.events` while async runs remain: `{ active: true, label }` when work starts or its label changes, `{ active: false }` when it ends. A label change emits `false` then `true` in the same synchronous call. After `/reload` or `/resume` it re-emits `true` from its own `session_start` handler, which can run before Trailboss's handler. The Pi integration (v9) ignored `herdr:busy`, so a settled parent with running subagents reported `idle`.

The OMP integration already reports `waiting` with message `awaiting background work` for the same condition.

## Decision

- The Pi integration counts `herdr:busy` raises and clears, floored at zero, like `herdr:blocked`.
- State precedence: `blocked` > `working` (parent turn active) > `waiting` (`awaiting background work`) > `idle`.
- The message is the fixed OMP string, not the pi-subagents label. pi-subagents already publishes its label as pane metadata.
- Busy events received before the TUI root session starts update the count but publish nothing. The root `session_start` report includes them.
- Busy-driven publishes are coalesced to the end of the current microtask, so a same-tick `false`/`true` label swap sends no transient `idle`.
- Integration version moves from 9 to 10 so installed v9 copies show as outdated.

## Consequences

A Pi tab with active background runs shows waiting instead of idle, matching OMP. Any extension emitting `herdr:busy` affects the Pi pane state, not only pi-subagents.
