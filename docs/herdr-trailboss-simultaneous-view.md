# Original Herdr and Trailboss simultaneous viewing

**Historical interoperability evidence:** The probes below ran before this fork was named Trailboss. Original Herdr executables/configuration, the prior fork's `herdr-fork` namespace and `HERDR_*` variables, the linked `libherdr_fork` artifact, temporary evidence paths and process/session details remain literal records. Source links now follow Trailboss's renamed runtime tree. Current Trailboss uses `trailboss`, `TRAILBOSS_*`, `trailboss.sock` and `trailboss-client.sock`; the historical namespace discussion is not a command to redirect or migrate a human Core.

**Keep one Core as the PTY owner. Multiple clients can observe its live pane, but the current fork TUI is not a usable second renderer for an original Herdr space.** Its transport works; its declaration-driven screen has no composition to draw. A separate fork session is not another view of the same shells.

Investigated 2026-10-12. This is an evidence-backed interoperability investigation, not authorization to migrate, attach to, restart, or alter a human session. No application code, installed package, or real plugin registry was changed.

## The four different cases

| Case | Finding | Status |
| --- | --- | --- |
| Two compatible clients connected to the **same original Core** | Both can receive the same live pane state and output. Core retains PTY ownership when either client disconnects. | Observed with two fork `Client` connections against installed original Herdr 0.9.3. |
| Original TUI plus current fork TUI, same original space | The endpoint transport is compatible, but original Core does not supply the fork's widget-tree composition. Receiving a Surface is not enough to paint it. Normal fork launch also selects a different namespace. | Source-backed limitation; actual probe received Surfaces but computed `surface_rect=None`. Not a completed two-TUI visual demo. |
| Original Core plus independent fork Core/session | Separate runtime ownership, terminal emulators, PTYs and processes. Reusing cwd, names, persisted layout or public IDs does not share the original live shell. | Source-backed; no migration or independent fork fixture was needed for this slice. |
| New read-only mirror/compatibility bridge | Plausible direction: consume original snapshots/Surfaces and provide a fork-local composition without moving PTYs. Needs explicit interaction/geometry policy and protocol adaptation. | Proposal only; no bridge or observer mode implemented. |

## What accepts multiple clients

Original checkout discovered read-only at `/home/tommy/src/herdr`; installed executable discovered through `command -v herdr` at `/home/tommy/.local/bin/herdr`, reporting `herdr 0.9.3`. A separate plugin-enhanced original checkout exists at `/home/tommy/src/agents/herdr`. These are distinct source trees: the installed executable was exercised, not assumed to be the build output of either checkout.

Original `src/server/client_accept.rs:11-56` loops over accepted sockets, assigns monotonically increasing client IDs and starts separate handshake readers. `src/server/headless.rs:1884-2018` creates and inserts a `ClientConnection` for each endpoint-shell client; it does not evict an earlier client. The corresponding vendored fork paths are [client_accept.rs](../crates/trailboss-runtime/src/server/client_accept.rs) and [headless.rs](../crates/trailboss-runtime/src/server/headless.rs).

Both original and fork use **endpoint generation 1** and the `shell.snapshot.v1`, `shell.surface.v1`, `shell.input.semantic.v1`, `shell.blob.v1` codecs. Original `src/protocol/endpoint.rs:1-23` separates this stable endpoint contract from private same-install protocol versioning. Handshake validation checks generation and required codecs (`src/server/client_transport.rs:738-774`), not exact application-version equality. The fork [Client::attach](../src/lib.rs#L53-L92) sends that endpoint hello. Private CLI, live-handoff and producer contracts must not be inferred compatible merely because this handshake works.

### Names and sockets do not select the same Core automatically

Original release config namespace is `herdr` (debug builds use `herdr-dev`); fork always uses `herdr-fork`. Sources: original `src/config/io.rs:22-34`, fork [config/io.rs:22-35](../crates/trailboss-runtime/src/config/io.rs). A named session's client socket is `<config namespace>/sessions/NAME/herdr-client.sock`, distinct from its API socket `herdr.sock` ([session.rs:159-186](../crates/trailboss-runtime/src/session.rs)).

The native fork main computes its local socket from that namespace and session name, not `HERDR_CLIENT_SOCKET_PATH` ([main.rs:102-109](../src/main.rs)). `HERDR_SOCKET_PATH` selects API context in some paths, not the native renderer socket. Giving both programs the same `--session NAME` therefore does **not** attach them to the same live Core. On a missing/refused native socket, fork startup spawns its own Core ([main.rs:112-151](../src/main.rs)). The smoke deliberately used `Client::attach(explicit_original_socket, ...)`, avoiding startup and producer-registration side effects. No real socket was symlinked or registry re-linked.

## Why successful transport does not give a working fork screen

The plugin-enhanced original's `src/protocol/endpoint.rs:95-134` describes flat sidebar rows: title, action, scoped IDs and semantic segments. Its producer parser expects `Vec<PluginUiRowJson>` (`src/app/api/plugins/ui.rs:707-758`). The upstream checkout's endpoint file does not expose that plugin companion at all. Neither inspected original schema supplies the fork's composition/fragments/widget-node declaration contract.

Fork adds optional `declaration` to each [EndpointPluginUiItem](../crates/trailboss-runtime/src/protocol/endpoint.rs#L110-L126). Its [widget schema](../crates/trailboss-runtime/src/protocol/widgets.rs) represents composition/fragments and `Surface`/`Zone` nodes. The fork receiver **filters out row entries without a declaration** ([lib.rs:160-174](../src/lib.rs)). The terminal's application walker only visits the selected declaration composition; `surface_rect` only exists when that walk encounters `Node::Surface` ([terminal.rs:274-296](../src/terminal.rs)). Actual painting likewise requires a visited Surface node ([terminal.rs:437-451](../src/terminal.rs)).

Thus an old-style sidebar row is not a substitute for a declaration, and an original pane Surface does not create its own fallback layout. The smoke observed exactly this distinction: matching pane output with zero declaration entries and no Surface rectangle. Merely pointing the current fork client at an original Core cannot provide its stock/sidebar/menu UI. Re-linking widget producers into a live original registry is not a safe solution: the old Core parses a different producer format, and its registry is shared across its namespace's sessions.

## Surface interests, navigation and focus are not read-only isolation

Original source behavior below is retained in the vendored [client_views](../crates/trailboss-runtime/src/server/headless/client_views.rs), [surface_interest](../crates/trailboss-runtime/src/server/headless/surface_interest.rs) and [headless](../crates/trailboss-runtime/src/server/headless.rs) modules.

1. **Surface interest is a per-connection active lease, not exclusive access to a pane.** `surface_active=true` permits live Surface delivery; two active clients can coexist. Deactivation discards pending render, drains held input and releases geometry ownership. Reactivation advances the projection floor and may promote the connection/claim geometry (original `surface_interest.rs:9-98`). Inactive is not a live read-only mirror.
2. **Workspace/tab location is per client, initially seeded from shared state.** `ClientShellLocation` selects its tab (`client_views.rs:54-80,128-200`). Endpoint navigation can target one connection. The focused pane within a selected tab comes from that tab's shared layout (`render.rs:4-33`); clients viewing the same tab do not get independent shell processes or independent layout/pane selection universes.
3. **Foreground and geometry have shared effects.** There is one `foreground_client_id` and one geometry controller per stable tab (`headless.rs:202-205`). Active attachment, input, activation and focus can promote a client. Resizing the controlling viewer resizes the actual shared PTYs; other viewers receive the resulting state (`client_views.rs:541-610`). A second viewer is not inherently non-invasive.
4. **Outer-terminal focus is per client but reconciled across viewers.** A focus-lost event is not forwarded to the child while another active, focused viewer of that tab remains; focus gained promotes foreground and may claim geometry (`headless.rs:2268-2307`). Native fork sends an active hello and supports semantic pane input; it exposes no read-only role or independent mirror policy ([lib.rs:53-74,229-241](../src/lib.rs)).

These focus/geometry policies were source-traced, **not** exhaustively exercised in a resize/focus/input-race matrix. The smoke used equal 80×24 connection sizes and only fixture shell input.

## Actual isolated smoke

Fixture: `/tmp/herdr-simultaneous.hYlg5l`. HOME, XDG config/state/runtime/data/cache and `HERDR_CONFIG_PATH` all pointed inside that directory; inherited `HERDR_*` was removed, shell explicitly `/bin/bash`. Only this fixture's Core/client processes were started or stopped. No human session or plugin registry was contacted.

A throwaway Rust executable linked the existing `libherdr_fork-538fe00d1db11529.rlib`, called the real `Client::attach`/`receive`/`type_text` and `terminal::surface_rect`, and opened two simultaneous connections to an actual installed original Core. This was not a mock protocol server or permanent test, and no Cargo build/test run occurred.

Observed bounded probe, exit **0**:

```text
attach boot=2650535-1791784255310615707 space=Some("w1") tab=Some("w1:t1") pane=Some("w1:p1") surface=79x24 declaration_entries=0
attach boot=2650535-1791784255310615707 space=Some("w1") tab=Some("w1:t1") pane=Some("w1:p1") surface=79x24 declaration_entries=0
shared_output=true surface_rect=None
shared_output=true surface_rect=None
survivor_after_other_disconnect=true
owned original core stopped 0
```

The first connection injected a unique printf marker into the fixture shell; both received it in their projected Surface. After dropping that connection, the survivor injected and observed a second marker. Both had identical Core boot, space, tab and pane IDs. This proves shared live output and disconnect continuity, not full original/fork UI interoperability.

An earlier run additionally launched the actual original TUI in an owned PTY. Core log records original endpoint client ID 2 and the two fork-library clients IDs 3/4 overlapping; only one pane child was spawned (PID 2644095). Its controller tool timed out during cleanup, so captured stdout was unavailable. Core log shows shutdown and fixture child exit; that run is supporting connection evidence only, not the successful probe cited above. No owned process from it remained. The bounded second run above avoided that cleanup path and shut its Core down normally.

Runtime evidence retained inside the fixture: `probe-result.txt`, `server-output.log`, `second-server.log`, and `config/herdr/sessions/viewprobe/herdr-server.log`. Throwaway probe source/executable were removed after recording findings; no scaffold remains in the repository.

## Safe recommendation

**For simultaneous viewing now, use compatible original Herdr clients on the original Core. Keep dogfood fork sessions separate; do not call that shared live-space viewing.**

For a future fork rendering the same original live space, implement an explicit compatibility/mirror client boundary: original snapshot/Surface input, fork-local declaration composition, explicit socket/remote selection, no producer registration or Core auto-start against the original namespace. For a genuinely non-invasive read-only mirror, simply suppressing keystrokes is insufficient: prevent mutating endpoint/API requests and avoid stealing foreground/PTY geometry while retaining live Surface delivery. Existing active-lease behavior must be adapted or a dedicated observer capability added. This is proposed engineering work, not an implemented or approved bridge.

Do not start a second Core against the first Core's live sockets/state, duplicate ownership of PTY masters, replace the original registry, or restart the human Core to investigate this. A live handoff transfers ownership; it is not two independent Cores safely driving the same PTYs. See [dogfood-session-survival.md](dogfood-session-survival.md) for the separate detach/runtime-preservation boundary.
