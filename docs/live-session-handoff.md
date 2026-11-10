# Live tab transfer and explicit session switching

Status: full-plan implementation authorized. Build explicit window switching and live tab transfer in order, including this plan's documented routing helper, ownership model, interfaces and public/native verification seams. Release automation remains outside scope; authorization is not implementation or activation evidence.

## Problem and goal

Tommy needs to reorganize live work between Trailboss sessions without reconstructing tabs or restarting their agents, shells and jobs. Moving a tab must not force the current window to leave its source session. The window also needs an explicit way to switch sessions.

Success means the original tab processes and terminal state are usable in the destination, even after the source session stops, while unrelated tabs and windows continue unchanged.

There is no separate PRD. The direct request and accepted follow-up decisions are the source requirements. Canonical decision provenance: Kizn Item 01a1a410-5ef1-7714-906a-75b4054c5ee0. This repository document defines the implementation contract within the execution authorization below.

## Full-plan execution authorization

Tommy instructed: “Don't stop at 1.1. Build every fucking thing unless you get truly blocked”. Execute all current tasks in dependency order, including the documented helper/model/interface work and public/native test seams, without another approval at each task boundary. This grants implementation judgment within this plan; it is not a claim that Tommy independently selected each internal design detail. A failed check, incomplete implementation or local discovery need is resumable work, not an external blocker. Preserve the documented process continuity, source-stop independence, ownership and security invariants. Do not add dependencies, change security boundaries, introduce release automation or alter unrelated live sessions. Build and verify on owned fixtures before any separately authorized live activation.

## Accepted requirements

R1. Original request: “is it possible to move a tab from one session to another and keep the processes in that tab running throughout the handoff?” Tommy subsequently instructed “Direct to master then build the tab/session handoff feature”. The agreed target is two sessions on the same host. Moving operating-system processes to another computer is not promised. Brief terminal I/O quiescence is allowed; recreating or suspending the processes is not the implementation.

R2. Asked whether the initiating window should automatically follow the moved tab to the destination session, Tommy answered “no”. That window stays attached to its source session. Other windows keep their existing attachments.

R3. Tommy added “though there should be a command to switch sessions”. A separate explicit command changes only the initiating window's session attachment, without restarting either session's processes.

R4. Asked “Must moved tabs and their agent controls keep working after the source session is stopped?”, Tommy answered “yes”. The source session cannot remain a hidden prerequisite for the moved tab or its agent controls.

R4 decision, 2026-11-07: asked whether “agent controls” means the restricted controls he actually uses (the Repeater/phone restricted path on stock OMP) or the strict guarded `conversationControl` controls that stock OMP 18.16 does not provide, Tommy was told to “reply "restricted" to redefine R4 that way” and answered “restricted”. R4's agent controls are therefore the restricted app-profile controls of ADR 0007: metadata reads, single-outstanding submit and input status, bound by the opt-in grant to the OMP PID, process start ticks, Core terminal ID and conversation. Abort stays refused exactly as on an unmoved restricted tab. Guarded `conversationControl` controls are out of scope until OMP provides them; this decision adds no guarantee beyond the restricted profile.

R5. Existing identity contract, not a new proposal: a tab's identity is Core-managed; a terminal environment is a cached handle. Preserve or resolve supported live associations rather than restarting agents or rewriting their environments. Kizn Item 01a1805c-b160-7fe9-9791-b31cd679999d, revision 5.

## Proposed user interface

Reuse the current session menu, tab menu, command palette and searchable picker. These are additions to settled interactions, not a sidebar redesign or a new shortcut system.

- **Move to session…** appears in the tab menu and command palette. The picker identifies both the destination session and destination space. Selection moves the captured tab, not whichever tab happens to be focused later. The invoking window stays in the source session and shows its normal remaining-tab or empty-session view.
- **Switch session…** appears in the session menu and command palette. Selection prepares the destination attachment, then changes only that window. Failure or cancellation retains the current attachment and view. A remote window selects sessions on its existing remote host and retains its SSH executable/host context.
- Destination choices are existing running sessions/spaces. Missing, stopped or incompatible destinations produce a clear error without starting a replacement session or moving part of a tab. The picker must represent an empty running destination with an explicit create-space destination rather than choosing an arbitrary space; this creates only the ordinary destination workspace needed to contain the moved tab.
- No default keyboard binding is added. Existing `session attach NAME` remains a new CLI attachment; it is not silently redefined as retargeting an already-running window.

## Proposed public interface and seams

Existing same-session `tab.transfer` requests retain their current meaning and behavior.

- Extend `tab.transfer` with optional `destination_session`, `destination_workspace_label` and `operation_id`. A cross-session request requires an operation ID and exactly one of `workspace_id` or `destination_workspace_label`; the label explicitly requests a new destination workspace. Existing same-session requests still require `workspace_id`. Reject both-or-neither targets rather than inferring one from a label collision or focus.
- Add `tab.transfer.status` with `operation_id` to resolve a lost result. Return an authoritative preparing, committed, aborted or unresolved outcome, with the final mapping when committed. Unresolved does not authorize replay, source unfreeze or a success message.
- CLI: extend `trailboss tab move TAB --workspace SPACE` with `--to-session SESSION`; `--new-space LABEL` is an explicit alternative to `--workspace` only for cross-session transfer. The CLI creates one operation ID and reports/query-resolves that same operation; it never submits a second move merely because a response was lost.
- Add read-only `session.list` through the existing public Core API, sharing the existing named-session discovery contract. Native remote discovery runs on the attached host, not the display computer.
- `host:switch-session` is a client-local semantic command discovered through the existing catalog. Do not introduce a Core-global switch operation that retargets other clients. The selected destination is validated before releasing the old attachment.

Verification seams are the manual command's output/exit status, public Core JSON API and CLI against isolated real Cores/PTYs, and the actual native renderer/menu/palette interactions. The full-plan build instruction authorizes testing this documented contract. No private-method seam, implementation-text assertion, fake terminal or human-session input substitutes for those observations.

## Proposed architecture: per-user routing helper

A moved running agent retains its original socket path and pane locator. The current in-Core alias map cannot route that address to another Core. Keeping the source Core as a forwarder would violate R4 after that source session stops.

Use a small on-demand routing helper implemented by the same Trailboss executable. It owns protected local routing/ownership records and the stable API entry points needed by inherited contexts; session Cores remain owners of their PTYs, terminal state and agent admission checks. The helper outlives an individual session and does not own or restart agents or shells.

The authorized full-plan implementation includes this local process and persisted model. No package, TCP port, remote service, system service installation or security relaxation is authorized. Reuse existing local Unix-socket and descriptor-transfer primitives. Restrict directories and sockets to the current user and validate peer ownership, endpoint identity and transfer generation. If those guarantees require a choice outside this plan's constraints, identify the actual decision and incompatible outcomes rather than introducing a fallback.

The helper must distinguish a stopped session from a stable legacy routing address. `session list`, normal stop/start and socket cleanup cannot report an old source as running merely because moved agents still use its old address. Restarting that source registers a new Core without stealing inherited contexts from the moved terminals. Deleting a stopped source must not erase a live moved process's required endpoint; refuse the deletion with an explanation while such references remain. Record cleanup follows actual terminal/process lifetime, not an arbitrary timeout.

### Proposed private registration and front-door contract

The helper uses a versioned control socket under the existing user configuration root, in a mode-0700 directory with mode-0600 sockets and records. Core registration names the exact session, boot ID, PID/start identity and owned private API endpoint. Endpoints must be within that configuration root and have the expected owner and socket identity; neither labels nor arbitrary caller-supplied socket paths grant a route.

Before its first cross-session transfer, a participating Core opens its private API endpoint against the same App/event hub, then offers a duplicate of its existing public API listening descriptor through the existing Unix descriptor transport. The helper initially holds that descriptor without accepting. The Core quiesces and acknowledges only the public acceptor; already accepted connections retain their existing handler lifetime. The helper then becomes the sole public acceptor of the same listening socket, forwarding ordinary requests to the registered private Core endpoint. There is no unlink/rebind interval at the inherited public address. Failure before adoption resumes the original acceptor and closes only provisional descriptors.

Listener adoption separates accept-loop control from accepted-connection control and delegates public socket cleanup explicitly. The old Core handle must not unlink the helper-owned entry point on stop. The existing API listener wrapper needs a Unix descriptor-adoption adapter, not a replacement transport or changes to user socket permissions. Requests already accepted by a Core are completed or reported conservatively under that Core's normal lifecycle; no uncertain mutation is replayed.

Routing records contain the original endpoint namespace and boot, original pane/terminal handles, destination session/boot/current terminal, shell PID/start and PTY identity, ownership generation, operation ID and transaction phase. Updates use protected atomic replacement. The committed generation, not a cached forwarding chain, identifies the current owner. Registration of a restarted source reserves still-live inherited handles before allocating new identities. Repeated moves update the same origin records; the helper cannot infer ownership from a matching numeric locator.

Pane-addressed legacy requests resolve inside their original endpoint namespace. New CLI requests carry their explicit-session choice or inherited pane context, validated against the live process/PTY association, so a default command from a moved pane can reach its current owner while an explicit `--session alpha` still means alpha. Ambiguous or stale context is rejected. Forwarding carries the admitted origin and operation generation to the destination, which independently validates its current terminal/process association and all existing reporter/control fences.

The helper remains available after the source Core exits. Normal source-session status, stop/start and new-client attachment describe the registered Core, not mere reachability of its retained API address. Existing legacy reporter packets keep working without reload. New feature participants must advertise the new coordination capability; old Cores reject before preparation. Old native clients must not be offered unusable new commands: use an additive capability message over the existing endpoint-control envelope and per-client command/declaration availability, rather than changing the generation-1 binary frame layout.

A source Core must actually exit in the R4 proof, rather than masquerading as a stopped session while secretly remaining the router. Create helper processes, sockets and routing records only in isolated owned fixtures during the build; reviewed live activation is separate from implementation proof.

## Selective ownership transaction

Prepare only the selected source tab and destination insertion. Keep unrelated source/destination tabs responsive. Snapshot its split layout, labels, zoom/focus, terminal state/scrollback, runtime identity and agent bindings. Allocate destination IDs without colliding with existing panes or aliases. Freeze selected PTY readers, not the operating-system processes.

Reuse descriptor duplication and strict imported-runtime construction. Provisional destination runtimes remain non-owning and quiesced. Validate every required runtime and routing admission before commit. The committed record identifies one terminal owner and one reader/writer path. Remove the source tab without invoking ordinary close/history capture or terminating its processes. Destination ownership, source removal and public result must agree on the same operation/generation.

Failures before commit close only provisional resources, resume the source reader and preserve both sessions. A lost response after a possible commit is unresolved until the recorded owner is established; do not blindly retry or recreate a source runtime. Reverse transfers and a third-session move update the original contexts directly, without forwarding cycles or unbounded chains.

Original source child waiters can remain reapers while that process exists. Imported PTYs already report EOF as `ChildExitReason::Handoff`; preserving exact original child exit status is not added scope. Clean source popup/overlay/pending-worktree ownership before delayed source waiter events. After source Core exit, ordinary OS reparenting/reaping must not kill the moved processes.

## Identity and agent controls

Namespace inherited locators by their original endpoint, not only `wN:pN`. Resolve their current terminal owner using Core-owned runtime/process/PTY identity and ownership generation. Reject ambiguity, stale ownership and foreign routes rather than targeting the focused pane.

Preserve report sequences, nested-OMP/root authority and live process binding. Destination restricted reads, single-outstanding submit and input status remain authoritative; abort keeps the restricted profile's refusal. Existing grant, binding, reporter/context epoch and outstanding-instruction fences are revalidated after movement; stale phone/control requests must fail safely rather than bypassing those checks. No OMP-core patch, reporter restart or terminal-environment edit is a repair.

## Compatibility check for each unit

The manual release-compatibility command is shipped separately on master; Tommy said tab/session handoff and SemVer compatibility "need to be separately shipped". Each review unit in this plan runs that command, as documented in `docs/release-compatibility.md` and the README, against its exact head and the retained baseline. Its findings inform Tommy's version choice; this plan does not change the command.

## Using explicit window switching

Open **Switch session** in the session menu or command palette, search for a running session and select it. The picker identifies the attached host. An SSH window lists that host's sessions and keeps its remote host and executable when switching. No new shortcut is assigned.

Only the invoking window changes attachments. The destination must already be running and support the negotiated V1 contracts. The current attachment stays usable until the destination handshake and initial snapshot succeed. Escape cancels without an error; a stopped, incompatible or unreachable destination reports a failure without starting a replacement session. Other windows and both sessions' original shells and jobs keep running.

`session.list` exposes the existing named-session inventory through the public Core API. It is read-only. Core issues the Switch session command only to capable endpoints; the native client places that issued export in its session menu and palette. Producer-owned declarations and the general capability-neutral API catalog remain unchanged by that placement, including when a retained client links older stock producers.

Task 2.1 verification exercised sixteen tests across six suites against owned real Cores, native renderers and SSH: both entrypoints, ordinary and explicit default attachment, named/default round trips, spectators, process identity, cancellation before commit, source responsiveness during a delayed handshake, failed destinations, stopped-after-discovery SSH rejection, retained-client startup ordering and queued source terminal/menu/Kitty graphics. The final concurrent affected-consumer gate passed 161 tests with nine explicitly skipped in 42.813 seconds; the two actual SSH tests were run separately and passed. All 3,773 library tests passed with thirteen ignored; formatting, Clippy with warnings denied and product-binary builds passed.

An isolated mutation removing attachment-state reset made F6 select the wrong original destination shell; restoring it passed. The source-fencing witness verifies image bytes and later metadata-only placement reuse, an in-flight action's original endpoint, destination selection and colliding tab-history identities. The four admitted code-review defects were fixed and independently cleared; the scoped R3 living-text fidelity audit passed. Selection evidence used terminal OSC52 output without accessing the physical clipboard. These are isolated implementation checks, not human-session activation or acceptance.

The exact-production-code manual compatibility run verified all five retained native/API/plugin/endpoint/negotiation probes. It classified `session.list` as an additive method but left the extended `success_response` schema unclassified, returning exit 1 without a version recommendation. The report is `/tmp/trailboss-switch-compat-bfa7bcf1.NHBOML/report.json`; source/binary correspondence, graphics, popup and optional Surface-codec compatibility remain unverified. No complete compatibility claim or automatic release decision follows.

## Out of scope

Cross-host process migration, new agent resumption, arbitrary job replay, new sidebar layouts, new global shortcuts, unrelated phone/voice/Auto activation, a generalized event/workflow framework, and changes to unrelated live sessions. This feature does not promise surviving a destination Core crash beyond the existing Core lifecycle contract. Normal source-session stop is explicitly in scope.

## Evidence and current limits

The transfer implementation is in `app/live_transfer.rs` and `server/tab_transfer/`. The private coordinator handles selected-runtime preparation and ownership commit; the routing helper retains original endpoint namespaces. `api/server/front_door.rs` handles public-listener quiescence and adoption. The OMP reporter still captures its original socket and pane context at startup.

The product native attachment loop is `src/main.rs`, not the vendored endpoint UI. Task 2.1 owns each attachment's receiver, input shutdown and worker lifetime. Cancellable background preparation keeps the source usable until commit; commit clears source-bound native state before consuming destination updates. Default-session addressing retains its existing root namespace, and existing-only SSH attachment cannot start a stopped destination.

Explicit native switching is implemented, independently reviewed and runtime-verified on owned fixtures. All fifteen transfer scenarios are locally verified, bringing the plan to twenty of twenty. Task3 is not published or activated for live use. API, CLI, actual native tab-menu and palette movement pass the live split-tab checks. A three-window check verifies that movement does not switch attachments and that explicit switching can reach the moved tab.

An owned source Core has exited successfully while the moved shell and background job retain their PID, start identity, PTY and shell-local state. Inherited current-pane CLI requests now resolve the owning Core through final terminal/process/generation admission; identity inspection does not pause the active PTY reader. Explicit session selection and unrelated explicit targets retain their original routing. Missing or stopped sessions, an older incompatible Core, removed spaces, stale captured source tabs and invalid destination paths are rejected without changing unrelated live work.

If a cross-session CLI result connection is lost, the caller reports an unresolved result with the original operation ID and a public `tab.transfer.status` query. It does not submit another move. An owned socket-interruption check verifies the emitted query against the actual committed mapping and the single surviving tab and terminal.

Public transfer errors distinguish definitive rejection (`tab_transfer_failed`), uncertain ownership (`transfer_unresolved`) and conflicting reuse of an operation ID (`transfer_operation_conflict`). Uncertain results retain the original operation ID and query instruction; neither the CLI nor native picker repeats the move. A definitive native rejection releases the pending operation so a later explicit attempt is possible.

Original endpoint routing remains usable after movement through a third Core and both previous Cores exiting. If the original Core is confirmed dead, no live replacement has claimed its name and live routes remain, the helper answers compatibility pings with its own actual build and protocol, without Core capabilities. This does not mark the original session running. Ordinary source restart also retains separate routing for the moved shell and the new source's own shell.

The unmodified installed OMP reporter publishes a fresh session identity after a local `/new` plus encoded Enter following source exit, source restart, return to the original owner and movement through a third Core. These owned checks retain the original OMP process, PTY, inherited environment and protected endpoint inode, while unrelated destination shells retain their metadata and usable input. They exercise genuine reports without a model turn or conversation-control substitute.

Return and third-owner checks also send a deliberately stale packet through the former owner's endpoint using the genuine old session and a lower sequence. It cannot overwrite the current session or lifecycle. Fresh sessions before and after the old Core exits still come from the unchanged installed OMP reporter, not that adversarial packet.

Owned Linux preparation/adoption checks exercise PTY-descriptor delivery failure, imported-runtime construction failure, lost destination preparation coordination, routing-admission failure, successful listener adoption, substituted socket identity, listener-quiescence failure and helper exit before adoption commit. External PID-restricted syscall injection requires exact fault-stage receipts; no product debug API or dependency was added. The destination-loss witness keeps the destination Core alive and does not prove Core-death recovery. The socket-identity witness does not impersonate another Unix user.

When retained state is absent, adoption first requires an acknowledged Cancel under the per-source adoption lock, before preparing the source acceptor. The helper's serialized control loop drains any earlier Activate before answering Cancel; an already-active or unreachable helper refuses adoption without preparing or resuming the source. After that fence, Stage holds only a passive listener descriptor and this adopter has not Activated, so Stage failure can cancel best-effort and resume even if EOF precedes observable helper death. After Activate is issued, acknowledgement loss still requires verified death of the exact helper PID/start before source resumption. The owned Linux EOF regression closes Stage/control and listener descriptors while holding that helper alive behind a FIFO until refusal, then verifies reaping, unchanged endpoint identity, snapshots and usable input. The concurrent-adopter regression holds an earlier Activate before retained-state publication, loses its acknowledgement, and queues the next adopter before completing activation. With helper acceptance gated, a queued public request must remain unanswered until that sole helper acceptor is released. These fixtures use PID-restricted syscall boundaries, not global signals or product fault hooks.

Adopted destination discovery validates the original listener's recorded inode, ownership, mode and listen-time credentials, then authenticates the live helper and its active-listener record. Descriptor adoption does not change Linux `SO_PEERCRED` to identify the new acceptor. The native discovery witness fails when that incorrect helper-PID requirement is deliberately restored outside the candidate and passes after restoration.

Preparation reserves the destination's containing workspace for tab/pane-targeted structural requests. Commit validates its insertion point before consuming prepared state. If a natural shell exit removes that destination during preparation, recovery aborts destination ownership before resuming the source; a late queued commit cannot override the abort. Repeated moves admit cached aliases only within the endpoint namespace that originally issued them, leaving unrelated matching source locators usable.

Held-preparation checks observe the original shell/job PID, start identity and PTY while their readers are quiesced. Neither process is OS-stopped, and the original background job completes further work iterations before transfer is released. This supplements the before/after identity checks; terminal I/O quiescence is not process suspension.

Under the R4 decision, agent controls are the restricted app-profile controls of ADR 0007. `tests/tab_transfer_restricted_controls.rs` runs the installed stock OMP with this repository's reporter extension through isolated owned Cores and PTYs, with an owned loopback model provider. Transfer keeps the Core terminal ID, so the restricted opt-in grant stays valid without re-enrollment. After the source Core stops, the moved OMP keeps its PID, start identity and PTY; metadata reads, single-outstanding submit and input status work through the destination, and abort is refused exactly as on an unmoved restricted tab. A second submit is rejected while the first instruction is still running. Core's live capability probe refuses it, because the reporter withholds `restricted_submit_v1` whenever an instruction is outstanding or OMP is busy. The reporter's own outstanding check only decides a concurrent caller whose probe finished just before another caller's submit; real OMP gives no deterministic way to hold that window open, so no witness isolates that check. A submit bound to a replaced conversation and a conflicting root-session report are rejected, while a fresh admitted control still reaches the original process. Deleting the stopped source is refused while the moved process route is live and succeeds after it exits. Guarded `conversationControl` controls remain out of scope until OMP provides them. All five witnesses passed on their first run; no product defect was found. Each was shown to fail against a deliberately broken copy of the relevant guard outside the candidate tree, then pass after restoration. The broken guards were the restricted capability family; the reporter's submit advertisement together with its outstanding and idle checks; Core and reporter binding/context checks; root-session cross-talk routing; and the deletion/release route guards. Breaking only the reporter's outstanding check leaves all five passing.

The final serial library gate passed 3,773 tests with thirteen ignored in 40.6 seconds. Fifteen transfer and switching consumer suites passed 61 tests with fourteen explicit-run tests ignored in 55.7 seconds; the explicit OMP report and restricted-control run passed nine in 74.9 seconds; `trailboss-restricted-conversation.test.ts` passed seventeen. Formatting, the product build and warnings-denied Clippy pass. Runs unset the invoking pane's inherited `TRAILBOSS_SESSION`/`HERDR_*` variables, which otherwise break unrelated session-environment unit tests. The manual compatibility run against the retained `trailboss-tabs-7c713486` installation, at `/tmp/trailboss-transfer-compat-r4-20261107/report.json`, verified all five retained probes and classified `session.list` and `tab.transfer.status` as compatible additions, but exits 1 for unclassified `tab.transfer` request and success-response schema differences; it does not recommend or select a version.

No human session has been switched, transferred or restarted as a build side effect.
