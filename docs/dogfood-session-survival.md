# Trailboss dogfood session survival

**Naming cutover:** Trailboss is the current product name. This dated investigation retains historical installed `herdr-spike` launchers, `target/debug/herdr-fork` smoke commands, temporary paths, hashes and session/process evidence. They are not Trailboss releases or a migration recipe. Current source builds use `target/debug/trailboss --session NAME` in the new `trailboss` namespace; that command does not attach to an old installation's Core. Preserve the human Core/PTYs and use its existing compatible installation unless a separate migration is explicitly authorized. Source links follow the renamed runtime tree; historical test results have not been rerun for this cutover.

**Current remote executable policy:** Trailboss discovers a compatible executable already installed on the SSH host, or uses the explicitly supplied `--remote-executable PATH`. Install a genuine Trailboss build remotely before attaching if none is available. Automatic remote release download/install/update and the old remote-binary upload override are not supported. The historical vendored preparation/install flow described below is provenance, not an available Trailboss installation mechanism.

Investigated 2026-10-12. This is a source-backed investigation plus an isolated native-binary smoke, not a production lifecycle change. Source line references describe the working tree inspected that day; concurrent UI work may move them.

## Recommendation

Keep the existing named session for dogfood, detach the client, and relaunch the updated client with the **same name, host, and configuration home**. For the historical installed remote spike session, the recorded launcher was (do not choose a new name):

```sh
~/.local/bin/herdr-spike --remote junkman --remote-executable /home/tommy/.local/lib/herdr-spike/herdr-fork --session spike
# Detach: Ctrl-B, release, q; or the visible Detach action.
# After installing client UI changes, run the same command again.
```

The installed launcher/remote bundle command is documented in [spike-validation.md:125](spike-validation.md); it was not changed or smoke-tested here. For the historical local installation, the command was `~/.local/bin/herdr-spike --session NAME` with its existing name. Actual runtime evidence below uses `target/debug/herdr-fork`. Do not use `session stop`, `server stop`, `session delete`, or the old fresh-test-session reset workflow on dogfood. Those are not client relaunch operations. A separate owned disposable fixture is still appropriate for destructive tests.

**Client restart preserves a live Core, its pane PTYs, and their processes. Disk layout restore does not preserve shell memory or jobs.** A rebuilt client also does not replace the running Core's code. Core-side feature changes need a separate deliberate upgrade strategy; an explicit Unix live handoff exists, but was not exercised in this investigation.

## Lifecycle and compatibility

| Operation | Source-backed behavior | Evidence |
| --- | --- | --- |
| Relaunch same local name | Connects to that session's client socket; starts a Core only when the connection reports NotFound or ConnectionRefused. No stop/delete/update is performed on a listening Core. | `main`, `run_client`: [src/main.rs:31-35, 102-151](../src/main.rs); `session::data_dir_for`, `client_socket_path_for`: [session.rs:159-186](../crates/trailboss-runtime/src/session.rs). |
| Explicit client detach | Writes `ClientMessage::Detach`, shuts down only its socket, returns successfully, and restores the outer terminal. Core removes client connection/input/geometry ownership, not panes or PTY runtimes. | `Input::detach`: [src/lib.rs:380-383](../src/lib.rs); client exits [main.rs:234-237, 395-400](../src/main.rs); Core `ClientDetach` [headless.rs:2628-2637](../crates/trailboss-runtime/src/server/headless.rs), `remove_client` [934-980](../crates/trailboss-runtime/src/server/headless.rs). |
| Abrupt client process exit | Socket disconnect follows the same client-removal path. A client is not the owner of the pane shells. Closing the client process is distinct from sending `exit` to a shell or stopping Core. | Transport EOF/disconnect [client_transport.rs:1018-1040](../crates/trailboss-runtime/src/server/client_transport.rs); `ClientDisconnected` [headless.rs:2634-2637](../crates/trailboss-runtime/src/server/headless.rs). |
| Relaunch same SSH host/name | Native `RemoteConnection::connect` uses the host/session-specific SSH bridge, then the ordinary native client. Host bridge reuses a listening compatible Core; otherwise starts a detached daemon. Dropping the connection stops its bridge, not remote Core. | Native dispatch [main.rs:92-110](../src/main.rs); [remote.rs:1-13](../crates/trailboss-runtime/src/remote.rs); `RemoteConnection` [remote/attach.rs:42-90](../crates/trailboss-runtime/src/remote/attach.rs), `SshStdioBridge::drop` [2938-2948](../crates/trailboss-runtime/src/remote/attach.rs); `ensure_remote_server_running` [remote/host.rs:44-66](../crates/trailboss-runtime/src/remote/host.rs); detached daemon [server/autodetect.rs:193-232](../crates/trailboss-runtime/src/server/autodetect.rs). |
| Core stop / host restart / crash | Not covered by live-client-survival guarantee. Ordinary restart recreates shells from persisted layout/cwd; it cannot reconstruct shell variables or running jobs. | `persist::restore` explicitly creates fresh shells [persist/restore.rs:65-93](../crates/trailboss-runtime/src/persist/restore.rs); stop warning [session.rs:123-130](../crates/trailboss-runtime/src/session.rs). |

The namespace includes configuration location, not just session name: `XDG_CONFIG_HOME/trailboss/sessions/NAME`, with `trailboss.sock` and `trailboss-client.sock` underneath. Default has a separate top-level directory. `--session` overrides inherited session selection. See [config/io.rs:22-31](../crates/trailboss-runtime/src/config/io.rs) and [session.rs:31-103, 159-186](../crates/trailboss-runtime/src/session.rs). Remote identity additionally includes the SSH target and the remote configuration location.

### New client versus old Core

`Client::attach` sends the stable endpoint generation and required snapshot/surface/input/blob codecs, not an exact application-version equality requirement ([src/lib.rs:53-82](../src/lib.rs)). Endpoint generation is intentionally separate from the private same-install protocol ([protocol/endpoint.rs:1-7](../crates/trailboss-runtime/src/protocol/endpoint.rs)). The Core rejects unsupported generations or missing required codecs, closes that connection, and does not upgrade/reset itself ([client_transport.rs:738-774](../crates/trailboss-runtime/src/server/client_transport.rs)). Native `run_client` does not retry a compatibility rejection by stopping Core. Its startup timeout kills only the newly spawned child, not an existing Core ([main.rs:124-151](../src/main.rs)).

This supports compatible client-only UI iteration; it is not a promise that every future client build works with every prior Core. Declaration changes depend on the running producer/Core, and private CLI/API or handoff compatibility is a separate concern. The native receiver currently ignores endpoint controls other than plugin UI and endpoint snapshots; it handles ordinary welcome errors and Core shutdown, but does not expose every endpoint negotiation rejection as a dedicated diagnostic ([src/lib.rs:160-194](../src/lib.rs)). Failure to attach is not permission to reset the live session.

Native SSH is narrower than the vendored full launch/update flow. `RemoteConnection::connect` finds an installed compatible executable (or validates `--remote-executable`) and starts a bridge; it does **not** call `prepare_remote_trailboss`/`ensure_remote_server_ready`. The host bridge refuses an incompatible existing Core with an upgrade hint, without stopping it ([remote/attach.rs:48-85](../crates/trailboss-runtime/src/remote/attach.rs), [remote/host.rs:44-66](../crates/trailboss-runtime/src/remote/host.rs)).

The separate vendored `run_remote` flow can prepare/install binaries and request an old-Core upgrade. Compatible endpoint/daemon capabilities keep Core running even when versions differ ([remote/restart_policy.rs:16-61](../crates/trailboss-runtime/src/remote/restart_policy.rs)). A required destructive restart is interactive, warns that shells/agents/dev servers/tests stop, and defaults to No; noninteractive required upgrades fail instead ([remote/attach.rs:2329-2400](../crates/trailboss-runtime/src/remote/attach.rs)). Explicitly opted-in live handoff can be attempted first; failure may proceed to the restart-confirmation path, or reuse previously approved stop permission ([1999-2047](../crates/trailboss-runtime/src/remote/attach.rs)). Do not describe that entire vendored flow as incapable of stopping sessions, but do not attribute it to native `RemoteConnection` either.

### Live handoff is not disk restore

Unix `HeadlessServer::perform_live_handoff` captures runtime metadata, pauses readers, duplicates live PTY descriptors, and transfers them to a replacement Core ([server/headless/lifecycle.rs:24-160](../crates/trailboss-runtime/src/server/headless/lifecycle.rs)). `restore_handoff` imports runtimes through a strict path rather than silently substituting fresh shells ([persist/restore.rs:95-119, 189-226](../crates/trailboss-runtime/src/persist/restore.rs)). This is a distinct, explicit upgrade operation: Core PID changes and clients disconnect/reconnect. Neither ordinary client relaunch nor disk persistence is live handoff. No runtime handoff success is claimed here.

## Actual native smoke

Executed the existing binary without Cargo, builds, installation, or production edits:

```text
binary: target/debug/herdr-fork
sha256: 11d2ea4ead1361648b96c337e8d09d8c48bf751a205a515c3e6e1c29a5b6101c
session: survival-2488707
XDG_CONFIG_HOME: /tmp/herdr-survival-pkotn4zn/config
XDG_STATE_HOME: /tmp/herdr-survival-pkotn4zn/state
viewport: 100 columns × 30 rows
```

A bounded Python `pty` harness launched the **actual native client** with a controlling terminal, continuously drained rendering during detach, and issued these actual CLI operations against only its named fixture:

```sh
target/debug/herdr-fork --session survival-2488707
target/debug/herdr-fork workspace create --cwd /tmp/herdr-survival-pkotn4zn --label survival-owned --focus --session survival-2488707
target/debug/herdr-fork pane list --session survival-2488707
target/debug/herdr-fork pane run w2:p1 'cd /tmp; survival_token=alive_survival-2488707; sleep 180 & survival_job=$!; printf "%s|%s|%s|%s\n" "$$" "$PWD" "$survival_token" "$survival_job" > /tmp/herdr-survival-pkotn4zn/before' --session survival-2488707
# Ctrl-B, release, q sent to the native client PTY; client exited 0.
target/debug/herdr-fork --session survival-2488707
target/debug/herdr-fork pane run w2:p1 'printf "%s|%s|%s|%s\n" "$$" "$PWD" "$survival_token" "$survival_job" > /tmp/herdr-survival-pkotn4zn/after; kill -0 "$survival_job" && printf "job_alive\n" >> /tmp/herdr-survival-pkotn4zn/after' --session survival-2488707
# SIGKILL only the second native client, then launch a third native client.
target/debug/herdr-fork --session survival-2488707
target/debug/herdr-fork pane list --session survival-2488707
# Detach third client, then clean up only this owned fixture:
target/debug/herdr-fork session stop survival-2488707 --session survival-2488707
```

Observed before and after graceful detach/relaunch:

| Identity/state | Before | After |
| --- | --- | --- |
| Core PID | 2488709 | 2488709 |
| Shell PID (`$$`) | 2489145 | 2489145 |
| Shell cwd (`$PWD`) | `/tmp` | `/tmp` |
| Unexported shell variable | `alive_survival-2488707` | `alive_survival-2488707` |
| Background `sleep 180` PID | 2489367 | 2489367; `kill -0` succeeded |
| Pane / tab / workspace | `w2:p1` / `w2:t1` / `w2` | unchanged |
| Terminal runtime ID | `term_65d9dfebb863a2` | unchanged |

After SIGKILL of only the second client, the third client attached to Core PID 2488709 with the same pane and terminal IDs. Shell-variable/job assertions were made after graceful detach; the abrupt-exit leg checked Core and pane/runtime identity rather than repeating those shell assertions. The fixture was stopped successfully afterward; no preexisting Core was stopped, deleted, or reset.

Initial harness attempts were not successful detach evidence: the first used the wrong CLI response shape; subsequent attempts used the wrong shortcut or did not drain the PTY while waiting, so rendering backpressure prevented client exit. Each owned fixture was cleaned up. The corrected run above completed in 4.52 seconds. These were harness corrections, not production-code fixes.

## Investigation boundaries

1. **SSH runtime smoke blocked:** `ssh -o BatchMode=yes -o ConnectTimeout=3 localhost true` failed strict host-key verification because localhost's recorded key changed. No known-hosts modification, verification bypass, or remote launch was attempted. SSH survival above is source-backed, not an exercised remote guarantee.
2. **Version matrix not exercised:** binary evidence is same installed debug binary across launches. No mixed-version client/Core, network-drop, logout, host reboot/crash, disk restore, or live-handoff smoke was run.
3. **Code intelligence:** rust-analyzer 1.96.1 answered LSP `textDocument/documentSymbol` for main/client/host bridge/headless lifecycle/restore sources, confirming cited symbol ranges. It ran with no linked Cargo projects, build scripts/proc macros disabled, and checks disabled; this was syntax-level navigation, not compilation or cross-crate type verification.
4. **Durable guidance corrected:** Kizn’s “Herdr-inspired app: confirmed design record” (`01a12810-3a69-7f17-96f2-010ade0b85af`) revision 47 now records the explicit dogfood continuity requirement. Earlier fresh-test reset authorization is historical and limited to disposable fixtures, not human dogfood sessions.

No production source, package, human session state, or user pane contents were changed. For daily dogfood, **reattach the same named live Core; do not reset it to refresh client UI**.
