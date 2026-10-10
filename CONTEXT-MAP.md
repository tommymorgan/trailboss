# Context map

One application domain; canonical terms are in [CONTEXT.md](CONTEXT.md). This map describes responsibility boundaries, not separate domain models.

| Boundary | Owns | Consumes |
|---|---|---|
| Core/runtime | Session lifecycle, PTYs, terminal state, persistence, session API | Terminal input and session commands |
| Rust client | Store, projections, geometry, rendering; executes core-defined reserved actions | Renderer-neutral Core events, Surfaces and Declarations |
| Plugin producer | Extension identity, declared actions and widget trees | Session API/events and stdin action envelopes |
| Stock UI | Default sidebar and tab-strip declarations | Public plugin API |

## Accepted decisions

- [Reuse the session API](docs/adr/0001-reuse-session-api.md)
- [Widget-tree declarations](docs/adr/0002-widget-tree-declarations.md)
- [Rust and vendored runtime](docs/adr/0003-rust-vendored-runtime.md)

Source: Kizn design record `01a12810-3a69-7f17-96f2-010ade0b85af`, revision 7; glossary `01a12817-28e1-72e6-aba5-cb08bbb9124b`, revision 5. The shared Core owns state and action semantics; each client owns its renderer. See [renderer-neutral contracts ADR](docs/adr/0005-renderer-neutral-client-contracts.md). Repo documentation is the implementation contract; these files project the accepted Kizn decisions.
