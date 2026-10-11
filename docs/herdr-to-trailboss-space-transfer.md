# Original Herdr → Trailboss live space transfer

**Historical interoperability evidence:** These handoffs predate the Trailboss name. Original Herdr paths/namespaces, the prior fork's `herdr-fork` executable/configuration, inherited `HERDR_*` variables, archive/evidence paths, hashes and human/fixture space names are preserved as observed. Source links now follow Trailboss's renamed tree. Current source uses `trailboss`, `TRAILBOSS_*`, `trailboss.sock` and `trailboss-client.sock`; no Trailboss handoff or human-session migration is claimed by these old results.

Investigated 2026-10-12. Primary-source trace plus two isolated executable handoffs. No human session, PTY, plugin registry, package, or installed binary was modified. This is an investigation, not authorization to migrate the live session.

## Answer

**One live space cannot currently be moved into an already-running fork session. The entire original server session can be handed to a fork replacement server, preserving actual shell processes, non-exported variables and jobs. That whole-session operation succeeded with both available original binaries and the current native fork binary.**

The existing operation is `server.live_handoff`, not `tab.move` or `workspace.move`. It replaces the source server; it does not split out the space named `herdr`, leave its siblings behind, or merge into the dogfood `spike` session. If `herdr` is the only space, whole-session replacement happens to carry just that space, but remains a server-level operation.

**Recommendation:** keep the human original session untouched. Selective transfer requires new engineering. Whole-session takeover is a demonstrated possible path, but namespace-dependent agent reporting and fork producer/configuration setup must be addressed before a human cutover.

## Supported mechanism and scope

| API | Actual scope | Primary evidence |
| --- | --- | --- |
| `tab.move` | Reorders a tab inside its existing workspace. Parameters are only `tab_id` and `insert_index`; there is no destination workspace/server/session. | [schema](../crates/trailboss-runtime/src/api/schema/tabs.rs#L34), [handler](../crates/trailboss-runtime/src/app/api/tabs.rs#L174). |
| `workspace.move`, `workspace.move_block` | Reorders one workspace or a block inside the same App state. No destination session/socket or transfer token. | [parameters](../crates/trailboss-runtime/src/api/schema/workspaces.rs#L35), [handlers](../crates/trailboss-runtime/src/app/api/workspaces.rs#L134). |
| `server.live_handoff` | Captures every workspace, tab and pane; pauses runtime readers; duplicates live PTY master descriptors; starts a replacement executable; transfers metadata and descriptors; commits ownership and exits the old server without terminating those pane processes. Disconnects all clients. | [parameters](../crates/trailboss-runtime/src/api/schema/server.rs#L7), [capture/spawn](../crates/trailboss-runtime/src/server/headless/lifecycle.rs#L24), [commit/preservation](../crates/trailboss-runtime/src/server/headless/lifecycle.rs#L165). |

The original checkout has the equivalent implementation at `/home/tommy/src/agents/herdr/src/server/headless/lifecycle.rs` and `src/server/handoff.rs`. `server.live_handoff` parameters are `import_exe`, `expected_protocol`, `expected_version`; there is no selected-space or destination-session parameter. The CLI bypasses its ordinary compatibility guard because this operation is itself an upgrade/recovery path ([CLI dispatch](../crates/trailboss-runtime/src/cli/server.rs#L205)).

The replacement is launched as `IMPORT_EXE server --handoff-import SOURCE_HANDOFF_SOCKET TOKEN`. It inherits the server environment and session name. Explicit source `--session` removes inherited API/client socket overrides when spawning the import child ([spawn](../crates/trailboss-runtime/src/server/handoff.rs#L65)). The fork native entry point forwards this server invocation into runtime bootstrap ([main](../src/main.rs#L61)). Bootstrap imports PTYs rather than creating fresh shells, restores the snapshot, binds replacement public sockets, waits for commit, and assumes ownership ([import bootstrap](../crates/trailboss-runtime/src/server/headless/bootstrap.rs#L212)). Disk snapshot copying/restoration alone is not a substitute: it creates new shells, not the original process memory; see [session survival](dogfood-session-survival.md).

## Namespaces and compatibility

Original release config/state namespace is `herdr`; the **actual original debug binary uses `herdr-dev`**, not `herdr`. Original `src/config/io.rs:22-40` selects that with `cfg!(debug_assertions)`. Fork always uses `herdr-fork` ([fork namespace](../crates/trailboss-runtime/src/config/io.rs#L22)). Named public sockets are `<config namespace>/sessions/NAME/herdr.sock` and `herdr-client.sock` ([session paths](../crates/trailboss-runtime/src/session.rs#L159)).

The temporary import socket belongs to the source namespace and is explicitly passed to the child. Replacement public sockets are recomputed by the fork from its own namespace. In both successful smokes the source sockets disappeared and the new sockets appeared under `herdr-fork/sessions/transfer-fixture`. This is a namespace transition, not socket aliasing, copying, or import into an existing Core.

The importer requires handoff manifest version 1 and checks optional expected application version/private protocol. It parses the complete snapshot/runtime manifest before receiving descriptors ([validation](../crates/trailboss-runtime/src/server/handoff.rs#L229)). Version equality alone cannot prove arbitrary future snapshot or runtime compatibility. Both exercised binaries answered version `0.9.3`, private protocol `22`, endpoint generation `1`, and `live_handoff: true`; actual successful descriptor import is the stronger evidence.

An existing listening fork destination is not a supported merge target. The importer waits for its computed public sockets to become unavailable before binding. The source-backed failure text is `old server sockets did not close before handoff import bind`, even though across namespaces these are the importer's destination paths ([wait](../crates/trailboss-runtime/src/server/headless/lifecycle.rs#L384)). This collision case was not exercised. Source rollback tries to restore old sockets/resume readers before commit; that is not a guarantee against every crash or failure.

### Preserved environment has a migration consequence

The debug smoke deliberately recorded `$HERDR_SOCKET_PATH` before and after. It remained the **old `herdr-dev` socket path**, which no longer existed after takeover. This is expected preservation of the live shell's environment, not a refreshed launch environment. Agent integrations read that inherited socket path directly, including OMP ([integration](../crates/trailboss-runtime/src/integration/assets/omp/trailboss-agent-state.ts#L11)). **[INFERENCE] Existing agents with cached old paths will no longer report to the new Core unless endpoint continuity is deliberately provided.** Editing an interactive shell variable does not rewrite the environment or cached configuration of already-running agent processes.

The fixture had no plugins. Import bootstrap loads the fork's config and starts its plugin hooks; it does not migrate the original plugin registry into the fork namespace. Native UI/producer installation, live client reattachment, SSH bridges, hooks, agents, scrollback fidelity, busy foreground programs, and failure rollback were not exercised here. Do not equate the process-preservation proof with a complete daily-driver migration proof.

## Observed isolated handoffs

All fixtures used private HOME, XDG config/state/runtime/data/cache directories beneath `/tmp/herdr-transfer-eudetzab`, private session `transfer-fixture`, and `/bin/bash`. Inherited `HERDR_*` overrides were removed. Each source session contained spaces `herdr` and `other-space`. No producer was registered.

| Source executable | Replacement | Outcome |
| --- | --- | --- |
| `/home/tommy/.local/bin/herdr` (installed release) | `/home/tommy/src/agents/herdr-fork/target/debug/herdr-fork` | API returned `{"id":"fixture","result":{"type":"ok"}}`; source server exited 0; fork ping succeeded. Shell PID `2624005`, background sleep PID `2625317`, and non-exported `TRANSFER_PRIVATE=preserved-not-exported` were identical before/after. `jobs -l` still showed `[1]+ 2625317 Running sleep 600 &`. |
| `/home/tommy/src/agents/herdr/target/debug/herdr` | Same fork | With `expected_protocol:22`, `expected_version:"0.9.3"`, API returned `ok`; source exited 0; fork ping succeeded. Shell PID `2644014`, sleep PID `2645811`, non-exported `TRANSFER_PRIVATE=current-debug`, and inherited API socket variable were identical before/after. `jobs -l` still showed `[1]+ 2645811 Running sleep 600 &`. |

Both fork `workspace.list` responses retained **both** `w1/herdr` and `w2/other-space`, with IDs, focus, tab counts and pane counts. Release fixture pane IDs remained `w1:p1`, `w2:p1`; terminal IDs changed after import, so terminal IDs are not a continuity identifier. Sending a new command through the fork's pane API executed inside the same original bash, proving preservation rather than a recreated shell with copied exported variables.

Read-only process evidence identified an existing original server PID `4132097` executing `/home/tommy/src/agents/herdr/target/debug/herdr`; it was not attached, queried, or changed. This identifies an available live original server binary, **not** which session contains the human space named `herdr`.

Binary SHA-256 at investigation time:

```text
18a8dc65f1c2fa485884344356dea1cfd911c6f06cf46fa78e193f4087f4dba7  ~/.local/bin/herdr
08c599693fb3bd3deb5eccb40b31208407b3eb46f5f25c79291763059bc41529  /home/tommy/src/agents/herdr/target/debug/herdr
5611188d9f3d97fcdd5b41c791b34030de11e9602fde82fdb842446bbbffff75  /home/tommy/src/agents/herdr-fork/target/debug/herdr-fork
```

### Runtime errors inspected

- An initial exploratory `system.ping` request returned `invalid_request`: the actual method is `ping`; its returned method list included `server.live_handoff`.
- First debug-fixture lookup incorrectly targeted the release `herdr` namespace and failed with `FileNotFoundError`; reading the owned server stdout identified `herdr-dev`.
- An installed-release CLI sent to the debug fixture returned `server_not_running` at the `herdr` socket. Using the actual original debug executable resolved it without touching the server or processes.
- After successful handoff the old socket returned `FileNotFoundError`, while `ping` at the fork socket returned the expected capabilities. No handoff API error occurred in either exercised pair.

These namespace errors matter operationally: do not choose the source CLI from its name alone.

### Safety and cleanup

The source fixture remained alive until successful ownership transfer; no source stop was used to make import succeed. After each success, the imported shell and job were verified before stopping **only the owned fork fixture** through its private API socket. Process inspection afterward showed neither recorded shell nor sleep remained. Temporary evidence files are retained under `/tmp/herdr-transfer-eudetzab`; no human process was stopped. No Cargo build, permanent test, or production implementation was added.

## New engineering versus a direct operation

A direct existing operation is whole-session replacement with `server live-handoff --import-exe PATH` using the correctly resolved original server identity. It creates a replacement fork Core for that inherited session name and environment; it does not transfer into an existing session. This document intentionally supplies no executable human migration command because the human source session identity and destination collision/endpoint-continuity policy were not authorized for mutation.

Selective `herdr` space transfer would need a scoped runtime export/import contract, source/destination ownership handshake, collision-safe identity remapping, restoration/rollback, and endpoint continuity for existing processes. Current `tab.move`/workspace APIs cannot provide that by changing a socket or label. Whole-session takeover avoids selective splitting, but still needs a deliberate answer for stale inherited socket paths and fork producer/configuration installation before live use.


## Accepted recreation instead of live transfer (2026-10-12)

Tommy subsequently authorized recreating the nine original Infilla tabs in the existing fork Infilla space, using the original directories and supported OMP resume commands. This was completed without moving or stopping any original process. The existing fork blank tab and selected pane were retained. All new tabs remained at bash; each matching full-path `omp --resume` command was staged without Enter.

Before pressing Enter in a recreated tab, exit its original OMP instance. Two OMP instances must not write the same transcript concurrently. The original transcript header, not its parent directory name, supplied the identity; this matters for the P0 child transcript.

Directories below are relative to `/home/tommy/src/agents/infilla-app`; `.` means that directory itself.

| Tab | Fork tab | Directory | Verified transcript ID |
| --- | --- | --- | --- |
| multi-permit | `w2:t2` | `multi-permit` | `01a0e8d3-1602-7174-af6a-a0f1dd535ecd` |
| mp-2 | `w2:t3` | `multi-permit` | `01a11f19-94a9-76e8-a1fb-32a8a39e461e` |
| mp3 | `w2:t4` | `multi-permit` | `01a12188-456b-7740-a290-d8a89bd15bbd` |
| de-cart | `w2:t5` | `de-cart` | `01a0db66-8bad-73ed-b99d-2abf3897a235` |
| review | `w2:t6` | `.` | `01a121b1-eb52-749d-bf9d-5da91e0df1ee` |
| halp! | `w2:t7` | `sonoma-email` | `01a11c15-b951-75b6-a1e0-84d017139ece` |
| standup | `w2:t8` | `multi-permit` | `01a12169-89c2-722e-9a56-c34978a49840` |
| P0 INF-5394 | `w2:t9` | `.` | `01a121d3-ae3e-7200-a4b2-0569d7fcf8e7` |
| mcp | `w2:tA` | `.` | `01a1220b-2f81-7097-ab33-b8107066f4ec` |

The exact supported command is already staged in each new tab. For the child transcript, use its full path, not the parent ID:

```sh
omp --resume /home/tommy/.omp/agent/sessions/-src-agents-infilla-app/2026-10-06T18-34-09-358Z_01a1127e-9a4e-7688-8cf1-0dfb9b7c1bad/INF-5394P0operationalunblock.jsonl
```

At the verified checkpoint, the original Core and nine original OMP/shell processes remained alive; fork Infilla had ten tabs including its original blank tab, and focus remained on `w1:p1`. This is a dated recreation checkpoint, not a live-process handoff or a claim that OMP has since been resumed.

