# Keep shared client contracts independent of the renderer

Tommy approved renderer-neutral shared contracts as a requirement of this spike on 2026-10-10, to preserve future GUI and web clients without implementing them now. The shared Core owns session state and action semantics; each client owns its renderer, pixels and mapping of Zones to geometry. Shared widget declarations carry structure, actions and semantic styling, and shared Surface data carries terminal state. Do not expose ratatui types, terminal escape sequences or fixed terminal-cell geometry as widget-layout requirements in shared contracts. A Surface remains a cell grid; that inherent terminal geometry is not a TUI chrome/layout contract.

Keep the accepted coarse vendored runtime, including internal ratatui data types, behind a client-neutral adapter. The terminal renderer consumes the shared contracts instead of defining them. Public declarations determine app UI composition, including where existing pane Surfaces are placed; a reference to a runtime-owned Surface is not a plugin-rendered cell-grid region. Producers run outside browser clients; do not require a client to spawn Bun or accept producer stdin directly.

A future browser transport adapter can preserve session API semantics while using a browser-compatible connection. Browser transport/authentication and GUI toolkits remain unselected and outside this spike. No bridge service, GUI client, web client, multi-client synchronization policy or additional dependency is introduced by this decision.

Source: Kizn design revision 6 and glossary revision 4. Number allocated in the new repository with no remote default branch; recheck before the first PR.
