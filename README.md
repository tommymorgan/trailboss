# Trailboss

A fork of [herdr](https://github.com/herdrdev/herdr) — the runtime my coding
agents live on, shaped the way I run them.

This fork is maintained for my own workflow first, and published in case the
pieces are useful to anyone else. Upstream [herdr](https://herdr.dev) remains
the supported product; expect this line to diverge freely.


<p align="center">
  <img src="docs/assets/trailboss-demo.png" alt="Trailboss: agent sidebar with per-agent panes alongside a shell workspace" width="820">
</p>

## Why my own fork

I run a couple dozen coding agents in one long-lived session, attach to them
from whatever machine I am at, and steer them from outside the terminal. Two
architectural directions mattered enough to own the runtime for:

- **Plugin-first UI architecture** — the sidebar and pane widgets are plugins
  over core projections, not hardcoded TUI screens. The interface can be
  replaced piece by piece without touching the core, which is how I want every
  UI surface here to grow.
- **Renderer-neutral core contracts** — declarations and projections instead
  of terminal-only state, so GUI and web clients can arrive later without a
  rewrite.

On top of those, changes I wanted to make quickly and dogfood daily without
asking a public project to adopt my workflow:

- **Structured control socket** — drive running agents programmatically:
  `chat_submit` (text, images, files), `turn_abort`, `agent.shutdown`, with
  idempotency, attachment validation, and typed rejections.
- **Remote attach** — connect from a laptop to the same core session over SSH,
  with live handoffs that swap the server binary without dropping clients or
  agents.

Everything here is tested against real agent sessions before it lands; the
machine I am typing on is usually running the previous commit.

## Thanks

Trailboss stands on [herdr](https://github.com/herdrdev/herdr) and its
contributors — the terminal runtime, agent management, and design that this
fork builds on and could not exist without. Thank you for making it and for
making it open.

## License

Apache-2.0, inherited from herdr. See [LICENSE](LICENSE).
