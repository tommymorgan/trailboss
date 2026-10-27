# Trailboss vendored runtime provenance

Trailboss derives its native PTY, persistence, server/session API and runtime implementation from upstream [Herdr](https://github.com/herdrdev/herdr). The original Apache-2.0 license and third-party notices remain in [LICENSE](../LICENSE) and the vendored sources; renaming this fork does not transfer upstream authorship.

**Historical implementation evidence:** The task records below predate the Trailboss naming cutover. Recorded `herdr-fork` binaries/crates, bookmarks, commands, hashes, source comparisons and evidence paths describe those builds, not released or deployed Trailboss artifacts. Source-tree links/paths now point to their renamed locations. Current source-built commands use `cargo build -p trailboss --locked`, `target/debug/trailboss` and `cargo test -p trailboss --test session_api --locked`. The original `{herdr: "widget_tree", ...}` declaration wire contract remains current; clipboard staging uses `/tmp/trailboss-clipboard-images-UID/` for a new Trailboss Core. Historical gate results are not verification of the renamed tree.

## Trailboss naming cutover

The current product, executable and root Rust crate are `trailboss`; its vendored runtime is `trailboss-runtime`, and its shared Bun package is `@trailboss/store`. The rename preserves the original wire contract in both directions: `{herdr: "widget_tree", ...}`, `herdr` action/action-result envelopes, `min_herdr_version`, stable API enum/error identifiers, protocol 22 and endpoint codecs, and original `HERDR_*` client/plugin context and socket overrides. Rust internal identifiers may use Trailboss without changing their serialized names. Source plugin manifests are named `trailboss-plugin.toml`; explicit original manifest file paths remain loadable. Application configuration/session/socket filenames use the separate `trailboss` namespace, while selecting an existing Herdr-fork remote executable/session retains that existing Core and its namespace. Name-only changes must not require a Core restart, producer replacement or a web-bridge upgrade.

Upstream Herdr authorship, license notices, real repository URLs and historical evidence remain intact. The retained `crates/trailboss-runtime/distribution/` scripts and manifests describe **upstream Herdr releases**, not Trailboss installers or releases. Production and handoff policies disable upstream binary-update checks; the Trailboss CLI does not expose that updater. Remote startup requires an explicitly selected or discoverable compatible Trailboss executable and never downloads an upstream Herdr binary as Trailboss. The inherited agent-detection catalog URL remains a real upstream resource, not a fabricated Trailboss service.

The local binary-package recipe is `packaging/trailboss-bin/PKGBUILD`. Current rename verification and package evidence are recorded in [spike-validation.md](spike-validation.md). No human Core, installed launcher, existing release bundle or companion web service was changed by this source cutover.

## Dedicated Core-discovered command palette

The bundled default `command-palette` provider is the separate Rust executable `trailboss-command-palette`, declared by `plugins/command-palette/trailboss-plugin.toml`. It consumes the existing `ui.command.list` catalog and Core-issued, owner-bound command fragments. Stock UI and sidebar menus retain their other declarations without publishing competing bundled palette roots. The Core registry, session API and wire layouts remain unchanged; command-reference encoding and producer-line capacity change as described below.

Build source executables with `cargo build --bins --locked` before starting `target/debug/trailboss`. Source registration launches the dedicated debug executable through its relative manifest path; the binary package installs its compiled executable beside a manifest using `./command-palette`. The package also retains the three compiled Bun producers and does not require Bun at runtime.

The producer resolves its API target once and derives the matching observer socket using Core's API-stem mapping. An inherited `HERDR_SOCKET_PATH` therefore selects both connections, even when an unrelated session or legacy client-only socket is inherited; an explicit `--session` retains the existing API-selection precedence.

The producer groups large catalogs and resource lists into bounded columns without adding menu levels, keeping every supported command searchable instead of invalidating the complete palette at the per-container limit.

Issued command names retain the reserved prefix and encode the full SHA-256 of the typed boot-ID/command-key tuple as unpadded base64url. Names are deterministic within a Core boot and need no mutable lookup registry. Core admits producer lines up to 512 KiB while retaining the separate 2 MiB complete merged-frame limit, including framing overhead. Exceeding either limit rejects the complete publication rather than substituting a partial catalog. Local optimized-native verification exercised a 5,000-action owner catalog: every export resolved, first and last actions executed, and unrelated Core search remained available. Native input witnesses must wait for the client’s publication and completed filtering, not just an observer response or a label already visible in unfiltered results.

F1 and the existing prefix-p shortcut open native search without focusing the sidebar. Named creation, rename and confirmation commands hand off to the shared interaction controller; owner-collected input remains with its owner. Existing immediate creation entries remain immediate. Captured resources and client-local navigation history remain separate. Invalid captured owner workspaces/tabs are omitted. Owner-context wrappers show current workspace, tab and pane labels before invocation, ahead of the command label so a long title cannot hide its target. Separate Label nodes retain Core's bounded, escaped command text without exceeding the per-label limit. Private issued-command identity survives displayed-title collisions; full titles and structured execution identities remain in the catalog.

Resource captions use the same 200-scalar, whole-control-escape display policy as Core command captions. Pane-label changes invalidate the producer's semantic inputs, so renamed targets update both the visible caption and native search without changing their captured IDs.

Shared confirmations render owner-supplied labels and retain Cancel as the initial choice. A refreshed confirmation follows the same unique issued-command locator and captured context; changed confirmation text updates in place, while missing or ambiguous commands close the popup. Confirmation action order, supplying owners and captured targets must remain unchanged, including through referenced fragments; a change closes the popup before another selection can activate. The guard uses the native controller's traversal and geometry, including descendants of actionable rows. Inline and referenced name interactions retain their validation, draft and ordered input/action identities across label-only refreshes. Reordering a form or replacing a focused submit cancels before another activation.

Local runtime verification on October 21, 2026 exercised four source-manifest/native-client cases and the same four cases from an actual `makepkg` extraction: empty-session initialization, passive observation and PTY geometry, both palette shortcuts, configured enabled states and custom-provider priority. The extracted payload ran with Bun absent. Four old/current-Core and original/annotated-producer cases passed through the independent generation-1 consumer. Four supplied compiled Core fixtures added, retitled and removed a command while the candidate consumer artifacts remained unchanged. Reproduction drivers are `tests/support/command_packaged.py`, `command_compatibility.py` and `command_definitions.py`; each requires explicit owned artifact paths and creates isolated sessions.

These local build and extracted-package checks are separate from installed-release and live-session handoff verification.

## Initial vendored snapshot (historical)

Task 1.1 snapshots `/home/tommy/src/agents/herdr` at jj commit `6361885a53ed65628a8ab585695bdcb824d45dca` (observed again immediately before copying). The reference checkout was read-only.

- `crates/trailboss-runtime/src`: upstream runtime, server-side session API, PTY actors, terminal decoding, persistence and their coarse dependencies. Original executable entrypoint is excluded; the new `src/main.rs` implements its own client, input loop and renderer. The vendored headless-server runner and, from Task 1.2, the unchanged session automation CLI are invoked. Retained legacy client/remote code is not the new client and no remote option is exposed.
- `crates/ghostty-vt` and `vendor/libghostty-vt`: exact source and bindings, native Zig build and upstream license. Vendored patch records and upstream revision JSON remain alongside them.
- `vendor/portable-pty`: original patched 0.9.0 dependency and license.
- Root `LICENSE`: original Apache-2.0 license. `Cargo.lock` originated from the source lock; workspace package records are updated without runtime dependency upgrades in Task 1.1. Task 1.2 adds the standard `jsonschema` validator as a development-only dependency; its transitive requirement advances `zmij` from 1.0.21 to 1.0.23.

The runtime crate retains ratatui internally. New public client Surface/Cell values contain only terminal-state data, not ratatui types or terminal-control bytes. Terminal-only geometry and escape output live in the client renderer. In Task 1.1, list/footer/footer-right Zones were unoccupied and the focused PTY was the only rendered content. Task 2.1 makes agent-tree the first real widget-tree consumer; stock UI remains unimplemented.

## Task 2.1: First agent-tree widget consumer

The flat source files in `plugins/agent-tree` and `packages/trailboss-store` are ported from the same read-only reference checkout. The retained TanStack DB adapter, entity collections and commands keep their existing behavior. `Bun.connect` carries the existing session API transport and `Bun.stdin.stream()` receives the original action envelopes; neither is replaced by Node networking/readline. Package versions remain TanStack DB 0.12.3, Zod 4.6.5, TypeScript 5.9.3 and `@types/bun` 1.4.2. Package lint/format commands use pinned Biome 2.3.15 without disabled rules. The root directly depends on the already-resolved `unicode-width` 0.2.2 for terminal-only text measurement.

Agent-tree now emits only a fresh version-1 `{herdr: "widget_tree", version: 1, zones: [...]}` declaration. The list placement contains a Column of Rows, whose children are semantic Labels or elastic Leaders. Action IDs and workspace/tab/pane context travel with the rows. The shared Rust types carry no ratatui types, escape sequences, cell coordinates or fixed widths. Core validates declaration structure and declared plugin actions, then forwards the tree without flattening it into legacy rows. The new client renders the tree beside, not inside, the focused PTY Surface. The original session API files still compare byte-identically to the reference.

The current variant C producer emits section headers, rules, flat tab rows and collapsed counts. Active statuses use solid dots with semantic working/blocked/waiting/done roles; idle and unknown use hollow dots with status-idle. The terminal renderer owns RGB colors, cell spacing and the selected-row background/left-edge accent. Tab clicks retain the original plugin.action.invoke, correlated stdin action/result and commands.focusTab path; Core state drives selection and subsequent PTY input. Stock controls use Core-owned reserved actions, including host:new-space through the existing workspace.create API. Variant C native verification and package delivery are recorded in docs/spike-validation.md.

### Task 2.1 verification

- Behavioral RED: the lowest TypeScript test failed against the real flat producer output, 0 passed / 1 failed, 0.143s wall; the failure was the tree-shape assertion, not an import error. The nonempty, deliberately unordered fixture contains two workspaces, five tabs and a pane; literal expected declarations cover status glyphs, roles, leaders and action context.
- Mutation: changing the production working glyph from `●` to `○` made the lowest TypeScript test fail (0/1, 0.103s wall, `artifact://547`) and both Rust lowest/living checks fail (0 passed / 2 failed / 1 skipped, 0.505s tests / 0.792s wall, `artifact://549`). Restoring the glyph returned the lowest TypeScript test and all three Rust checks to green.
- Final focused Rust gates: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo nextest run -p herdr-fork --test agent_tree --locked --no-fail-fast --status-level fail --final-status-level fail --failure-output final --success-output never` all passed: 3 tests, 0 failures/skips, 0.759s tests / 14.67s chain wall. The exact living Given/When/Then is production-bound in `tests/agent_tree.rs`, alongside the lowest integration seam and actual executable-under-PTY test. Each core isolates all HOME/XDG roots and inherited HERDR variables and uses a unique named session.
- Retained package gates: in each package, `bun install --frozen-lockfile`, `bun test`, `bun run check`, `bun run lint` and `bun run format:check` passed. Agent-tree: 6 tests, 18 assertions, 0 failures, 1.02s chain wall. Store: 7 tests, 15 assertions, 0 failures, 1.47s chain wall. The migrated store runtime smoke also observed the first declaration and stdin-driven focus through the actual new core and Bun producer.
- Standalone executable/PTY smoke outside the test harness: built `target/debug/herdr-fork` and launched an isolated named core plus 100×25 client. Enabling the first plugin displayed Alpha/Beta, Build/Review, the hostname title, working/blocked glyphs and elastic leaders. A real SGR mouse click on Review changed core focus and rendered its bold highlight; typing a shell command then produced `standalone-live` in the selected live PTY Surface. Smoke passed in 0.562s; build plus smoke took 28.14s wall. The disposable smoke script was removed.

### Task 2.1 bounded review repair

The lead's one basic review of revision 6cbe20a0 found that the row-child cap of 16 rejected a retained workspace header with 15 active tab glyphs plus its label and Leader (17 children), clearing the whole plugin projection. The validator now admits up to 128 row children, matching the existing bounded Column capacity; it does not clip producer statuses, exempt a fixture or remove structural validation. Both the lowest integration and exact living test additionally create 15 real working/blocked tabs through the unchanged core API and render the complete literal header at width 50. An initial fixture mistakenly sent API state waiting, which the original API rejects; that setup error was corrected to valid working/blocked reports and is not counted as behavioral RED.

With the original cap, both regression checks failed because core forwarded no valid declaration: 0 passed / 2 failed / 1 skipped, 6.995s tests / 14.34s wall (artifact://634). Raising only the structural bound returned both to green: 2 passed / 1 skipped, 1.159s tests / 6.75s wall. Mutating the exact bound back to 16 again failed both: 0 passed / 2 failed / 1 skipped, 6.775s tests / 12.38s wall (artifact://639). The bound was restored to 128.

The focused executable check then exposed a separate major input race, authorized by the lead for the additional major-fix pass. Client::receive already stores snapshot tab and pane together. Before repair, main forwarded only snapshot tab/boot, painted the selected bold row, but changed Input's pane target only from a later Surface. Immediate typing could split a command across the old and new panes (artifact://642). Production now forwards and detects both snapshot tab and pane changes, updates the Input target before painting that same focus update, and never lets later Surface data overwrite authoritative input routing. The test types immediately after observing the selected bold row, with no private Surface-ready marker or added delay. A temporary marker-wait experiment was removed because it concealed the consumer-visible race.

Mutation of the exact snapshot-to-Input update made the executable check fail despite the selected tab and bold row: 0 passed / 1 failed / 2 skipped, 10.751s tests / 11.62s wall (artifact://658). Restoring routing returned the final complete focused suite to 3 passed / 0 failed/skipped in 1.167s. Format, workspace all-target check, strict Clippy, focused nextest and executable build all passed in a 5.04s chain; retained package frozen-install/test/type/lint/format gates again passed (agent-tree 6 tests / 18 assertions / 1.01s, store 7 tests / 15 assertions / 1.45s; artifact://659).

A fresh standalone 100×30 executable/PTY smoke created 15 real active tabs before linking the first plugin. All 15 named working/blocked rows plus Alpha/Beta appeared after core validation. A real Review click rendered focus, immediate outer-PTY typing produced boundary-live in the selected live Surface, and the unchanged API confirmed Review focus. No private readiness marker was used. Smoke passed in 0.655s (1.41s command wall, artifact://663); its disposable script was removed. The repairs remain folded into the same scenario under the lead-renamed bookmark herdr-fork-slice-2-task-1a. Final full-workspace nextest remains the lead's post-repair gate, not an engine claim.

The lead’s final process-isolated full-workspace nextest layer passed all 3,707 tests, with 13 skipped, in 4.707s test runtime and 35.11s wall time against repaired revision `d0d5f382`. Exactly one basic review ran (55s reviewer-reported time; 70.525s worker duration); its boundary finding and the subsequently demonstrated major input-routing defect were repaired within the permitted fix passes. No PR, push, demo or recording occurred. Later widget consumers, later living scenarios, latency and full spike exit criteria remain unclaimed.

Local adaptations: library module visibility, package/path manifests, independent `herdr-fork` configuration/session namespace, and the new client adapter. Server-side session API files are unchanged (`diff -qr` against the reference `src/api/` produced no differences). Build caches are ignored, not source. The root `clippy.toml` preserves the source workspace’s argument threshold (11); no new lint suppression is added. Excluded original executable launch code is removed; its retained compatibility-test helpers are test-only. An unused duplicate original client-test buffer and unnecessary cast are removed; an original wire documentation blank is corrected. The original build-time contributor-policy reminder is excluded because its referenced governance files do not apply to this vendored library.

## Task 1.1 evidence

`cargo test -p herdr-fork --test local_pty --locked -- --nocapture` executes three real-process checks: the lowest client/PTY seam test, the living Given/When/Then binding in `features/session.feature`, and the actual interactive executable under an outer PTY. All three pass (0.27s test runtime; 3.40s final command-chain wall time). Each uses an isolated HOME, XDG roots and named session, with the new executable starting the vendored headless core. The interactive check decodes its 100×25 terminal output using Ghostty, sees shell-produced `console-live`, checks empty list/footer/footer-right Zones and detaches with Ctrl-Q. These checks do not use the original executable, a fake runtime or a plugin consumer.

Initial behavioral red: the new client’s legacy hello was rejected by the stable endpoint protocol, preventing live PTY attach. After negotiating the real endpoint, normal pane patches also had to be applied: disabling optional compact delta/reuse encodings does not disable base `PaneSurfacePatch` messages.

Mutation: replace the production adapter’s PTY cell symbol with a blank. All three checks fail (0 passed, 3 failed, 8.24s): lowest and living steps cannot see shell-produced `spike-ready`; the interactive screen also times out without that output. Restoring the exact cell symbol projection returns all three to green. `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked` and `cargo clippy --workspace --all-targets --locked -- -D warnings` pass with no remaining warnings.

## Bounded review repair

The single basic review found that the terminal renderer omitted the inherited PTY DIM, SLOW_BLINK and CROSSED_OUT attributes. The real shell now prints a combined SGR `2;5;9` styled line; both the lowest seam/living binding and the actual interactive executable decode the displayed cells through Ghostty and assert faint, blink and strikethrough. Before repair, all three checks failed with `renderer lost shell DIM attribute` (0.49s tests; 1.09s wall). The renderer now maps exactly those three carried-over bits. All three checks pass (0.49s tests). Mutating the exact production DIM mapping to normal intensity makes all three fail at the same displayed-style assertion (0.34s tests; 1.14s wall). Restoring DIM returns all three to green; final focused test/format/check/strict-Clippy chain passes in 3.33s with no warnings. No additional review, consumer or feature was introduced.

The lead’s broader `cargo test --workspace --locked` attempt failed after 38.19s: inherited runtime tests shared process environment and the process terminated with SIGPIPE (evidence `artifact://286`). The upstream justfile uses process-isolated `cargo nextest run --locked`. Its first run passed 3,701 of 3,702 tests in 3.191s; the sole failure asserted fixed changelog prose in an inherited fake-update test. That wording-only test was removed, not re-pinned. The final format/check/strict-Clippy/isolated-workspace chain passes: 3,701 tests passed, 13 skipped, 3.164s test runtime and 36.27s total wall time. API files and production update behavior remain unchanged. Later task/latency/exit gates remain unclaimed.

## Task 1.2: Existing session automation

The new executable routes session API automation commands through the original `cli::maybe_run` dispatcher and its original `ApiClient`; the server transport, schemas, error variants and handlers are not rewritten. Commands exposed here are `api`, `status`, `server`, `session`, `workspace`, `worktree`, `tab`, `pane`, `agent`, `notification`, `integration` and `plugin`. Legacy terminal/remote client entrypoints, updater channels and configuration-maintenance commands are not exposed. The original client and schema modules are re-exported from the runtime library for typed automation; every file under `crates/trailboss-runtime/src/api/` remains byte-identical to the read-only reference. The protocol bundle remains schema version 1, protocol 22, with all 102 request variants and its five original schemas.

`tests/session_api.rs` contains the lowest meaningful executable/client/core contract test, the exact three production-bound living steps in `features/session-api.feature`, and an executable automation smoke. Each core uses a unique temporary HOME, all XDG roots, an isolated configuration/socket namespace and the named `contract` session. It restores the original `legacy-pre-tabs-v2.json` compatibility fixture byte-for-byte through production persistence. Original reference sources and the installed daily driver remain untouched.

The contract flow uses the unchanged CLI client to create a real workspace, then the unchanged typed `ApiClient` to inspect restored session state, negotiate protocol 22, rename a Unicode workspace and observe the updated snapshot. It subscribes using the original typed Request and event models, observes a real workspace lifecycle event and a PTY-output-matched subscription event, and validates each against the complete original schema using the standard JSON Schema validator. The shell constructs the expected output marker from separate arguments, so input echo cannot satisfy the output assertion. Typed missing-workspace errors preserve `workspace_not_found` and request correlation.

The added schema-derived adversarial corpus in `tests/fixtures/session-api-methods.json` sends every one of the 102 method variants through the unchanged typed client into the real core. Every actual success/error response is validated against the complete reference schema and original Rust response/error types; coverage is compared against the entire original method union. Missing targets, nonexistent plugin/import paths, nonempty key lists and bounded wait timeouts exercise rejection/default boundaries. This is universal method/schema coverage, not a claim that every positive branch of all 102 operations has been exercised. Positive state changes, persistence restore, real PTY output, lifecycle events and parameterized subscription events are exercised explicitly. No mock, custom protocol parser or translation layer is used. The original API client lacks a streaming iterator, so the persistent subscription uses a Unix socket with the original Request serializer and original event deserializers, not a replacement client/parser.

### Task 1.2 verification

- RED: all three checks failed at the actual workspace-create assertion because the new executable rejected existing automation arguments with usage; 0 passed, 3 failed, 0.02s test runtime / 27.10s command wall time including the first validator build (`artifact://395`). The already-vendored server API itself was present; the missing production entrypoint behavior supplied the truthful red.
- GREEN: original command dispatch restored all three checks; first full green was 3/3 in 2.84s / 3.55s wall (`artifact://401`). An intermediate assertion mistakenly expected generic `not_found`; it was corrected to the authoritative original `workspace_not_found`, without changing production error behavior.
- MUTATION: pass only the executable name, instead of the real arguments, to the exact production CLI dispatcher. Lowest, living and executable tests all failed at workspace-create, 0 passed / 3 failed, 0.01s / 0.73s wall (`artifact://423`). Restore the real arguments: final focused suite 3/3 in 2.73s; focused/format/check/strict-Clippy/immutability chain 5.46s (`artifact://427`).
- Standalone executable smoke, outside the test harness: 1 passed in 0.71s. Under separate temporary HOME/XDG roots and named `smoke` session, `target/debug/herdr-fork server --session smoke`, `workspace create --label "external automation" --focus`, `workspace rename ID "external updated"`, and `api snapshot` proved changed server state. `workspace get w999999` returned exit 1 and the original typed error; `server stop` shut down cleanly. All automation commands select the same session using `--session smoke`.

**Historical reproduction instruction (pre-rename):** Reproduce focused verification with `cargo test -p herdr-fork --test session_api --locked -- --nocapture`, then `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked` and `cargo clippy --workspace --all-targets --locked -- -D warnings`. The final chain passes with no remaining format, type or lint errors. `diff -qr crates/herdr-runtime/src/api /home/tommy/src/agents/herdr/src/api` and `cmp` of the original protocol schema return no differences; the original legacy fixture also compares identically.

One gate collection lost its Eval response when the worker timed out after 30s; the bounded retry collected the complete focused/strict gate output (`artifact://418`, 31.51s wall), followed by the final restored chain above. Split-check measured 523 reviewable lines and 338 lock-file lines; the single cohesive scenario stays whole as `herdr-fork-slice-1-task-2a`. Exactly one basic reviewer returned no findings in 28.213s. The final process-isolated workspace layer gate passed all 3,704 tests, with 13 skipped, in 4.129s test runtime and 41.48s wall time. No PR, push, demo or recording occurred. Latency and full spike exit criteria remain unclaimed.

## Task 2.2: Workspace collapse acceptance

The retained producer already implements workspace collapse/expand: real sidebar actions traverse the client endpoint, Core forwarding and producer stdin, persist session-scoped collapsed state, and republish declarations. No missing production behavior was invented. Tests create three Alpha tabs plus an unrelated Beta tab and observe rendered rows, not setter self-reads. Both lowest and living checks initially passed (2/2, 0.552s). Negating only the production hidden-tab branch made both fail (0/2, 0.573s; artifact://703); restoring it returned green. The actual executable PTY client receives mouse header clicks and shows all three tabs disappearing, 3 hidden, and all three reappearing while Beta stays visible (artifact://706).

Scenario-one gates passed: Rustfmt, locked all-target workspace check, strict all-target Clippy, and six focused agent-tree tests (6 passed, 0 skipped, 1.235s; 25.24s chain, artifact://712). Frozen install/test/typecheck/lint/format and strict Biome CI passed for agent-tree (6 tests/18 assertions, 1.19s chain) and retained store (7 tests/15 assertions, 1.52s chain). No workspace nextest layer or reviewer was run by this engine.

### Task 2.2: All eight session-event examples

The retained store already subscribes to all eight outline events. New production-bound checks create, rename and close a workspace; create, focus and close a tab; detect an agent; and change agent status. Every operation is observed through a separate original events.subscribe connection, so fixtures cannot invent event names or satisfy assertions by echoing requests. The rendered sidebar must change after each operation, and the selected tab must become bold. An executable PTY client observes the same changes. Producer PID plus /proc start time and the attached client boot identity remain unchanged throughout.

The original pane.report_agent status-only fixture did not emit pane.updated: it emits detection/status events instead. The fixture now assigns an agent name through agent.rename, then changes ownership and status; the original lifecycle clears the owned name and emits pane.updated with the changed status. One invalid uppercase agent name was corrected to the API's lowercase naming contract. These were fixture failures, not manufactured behavioral RED. All eight supported cases initially passed the unchanged production paths (3/3 including executable PTY, 2.050s; artifact://747).

Mutation disabled only the producer's reactive collection-change emission. Both lowest and exact living outline checks failed while awaiting the newly created workspace (0 passed / 2 failed / 13 filtered, 6.551s tests / 9.36s wall; artifact://753). Restoring the callback returned green. A focus-style assertion additionally required enabling terminal styles for the non-TTY buffer fixture, following the existing renderer-test convention; no production focus semantics changed.

Standalone executable smoke, outside nextest: built the real binary, launched isolated named cores, live Bun producers and 120×25 PTY clients, clicked Alpha collapse/expand, observed 3 hidden and disappearing/reappearing tabs, and drove all eight real event examples without restarts. Passed in 2.896s smoke / 14.28s build-plus-run wall (artifact://765). The disposable example was removed. Tests now stop Core through its unchanged server.stop API so supervised producers exit before the bounded forced-kill fallback. Only this engine's old orphan producers, matched to exact unique socket paths recorded in its own artifacts, were terminated; other producers were untouched.

Final per-scenario gates passed after fixture cleanup: Rustfmt, locked all-target workspace check, strict all-target Clippy and focused agent-tree nextest (9 passed / 0 failed / 0 skipped, 2.634s tests / 17.67s chain; artifact://773). Agent-tree frozen install/test/typecheck/lint/format/strict Biome CI passed (6 tests/18 assertions, 1.20s chain); retained store passed the same gates (7 tests/15 assertions, 1.48s chain). Biome CI used --error-on-warnings --enforce-assist=true. Only Task 2.2 living scenarios/metadata changed; original session API, renderer-neutral contracts, row bound and immediate-focus input routing remain unchanged. No reviewer, full workspace nextest layer, PR, push, demo or recording was run by this engine.

Task-level acceptance: split-check measured 497 reviewable lines and no lock/generated changes, so no split was needed. Exactly one basic review of 436e9f4a..b59726b5 returned no findings in 25.022s. The final process-isolated workspace layer passed 3,713 tests, with 13 skipped, in 4.727s test runtime and 13.06s wall time. No PR, push, demo or recording occurred.

## Task 3.1: Public stock composition

Stock UI is now a supervised public plugin (`plugins/stock-ui/trailboss-plugin.toml`), with its live declaration projection in `plugins/agent-tree/stock.ts`. It reuses the existing Bun/store transport and producer host. The shared widget contract carries an optional priority-selected composition tree, proportional flex allocation, semantic conditions and references to runtime-owned pane Surfaces; it contains no fixed terminal-cell layout requirements or plugin-rendered cell-grid region. Core owns reserved new-tab/menu/focus/create/cancel semantics and draft input. The terminal adapter only traverses declarations, maps structure to terminal geometry and paints existing Surface cells. No fallback stock shell or privileged stock rendering remains.

Only the exact Task 3.1 four-row outline is implemented in `features/declarations.feature`: no pane yet, live session, new-tab interaction open and menu open. The production-shaped lowest check consumes the actual TypeScript stock projection under an arbitrary plugin identity, checks every visible group, reverses the public arrangement to move sidebar/tab strip/menu/Surface placement, and removes declarations to require a blank application. The living outline starts actual isolated Core and supervised Bun processes, observes the neutral declaration channel and the executable outer PTY, creates the first real pane through the declared new-tab affordance, types/cancels drafts, opens/closes the menu, and observes actual shell output. Existing agent-tree tests retain collapse/events, tab focus/input and all fifteen active clusters. Original session API contracts are unchanged. Tasks 3.2–3.5, second-consumer portability and GUI/web implementations remain unclaimed.

- RED: before the cutover, the lowest four-state test observed a runtime Surface despite zero public declarations; 0 passed / 1 failed, 0.005s tests / 5.46s wall (`artifact://837`). This exposed the privileged fixed-shell rendering path, not a fixture/import failure.
- MUTATION: negate only the production application walker’s public composition selection. Lowest and exact living outline both failed (0 passed / 2 failed, 8.561s tests / 9.73s wall, `artifact://973`); the lowest lost the declared sidebar and the actual executable became blank. Restore that selection: final focused declaration/living/agent-tree/local-PTY nextest passed 14/14, 0 skipped, in 4.154s (`artifact://976`).
- Final strict Rust chain: `cargo fmt --all -- --check`, locked all-target workspace check, strict all-target Clippy (`-D warnings`), executable build and focused process-isolated nextest all passed in 10.63s wall (`artifact://976`). The obsolete executable empty-shell assertion was migrated to the declared stock UI while retaining actual PTY input/output and DIM/BLINK/CROSSED_OUT styling. It waits for the real declaration’s tab strip instead of relying on replay of an unterminated pre-attach printf across PTY resize.
- Changed producer package: frozen install, six Bun tests / eighteen assertions, typecheck, lint, format and Biome CI with `--error-on-warnings --enforce-assist=true` passed under isolated HOME/XDG/config roots in 1.14s wall (`local://herdr-fork-task31-ts-gates.txt`). The retained store package is unchanged. No warnings were disabled.
- Standalone smoke outside nextest: the built real executable, live named Core sessions, supervised stock producers and 80×24 outer PTYs observed all four states and real Core menu/new-tab actions. Passed in 3.887s smoke / 4.66s wall (`local://herdr-fork-task31-smoke.txt`). The disposable runner was removed. Every Core fixture has its own HOME/XDG/config roots and sockets; cleanup calls the original `server.stop` with bounded forced fallback, allowing supervised producers to exit.

One cohesive Scenario Outline requires one atomic scenario commit even though its clean-cutover change exceeds the usual 500-line split threshold. No engine review, full workspace nextest layer, PR, push, demo, recording or Kizn write was performed. Only Task 3.1 metadata is DONE / Living updated YES.

### Task 3.1: Single bounded review fix

The lead’s one basic review (46.487s) found a shared-list collision: the public Zone walker passed the identical rectangle to every producer, so later declarations overwrote earlier rows and stole their hit targets. The bounded fix allocates nonoverlapping vertical rectangles in declaration order, using each tree’s declared extent/relative weight and preserving its producer identity for both painting and hit testing. No additional Zone kinds or Task 3.3 acceptance were introduced.

- Regression RED: two valid public list declarations lost the first visible/actionable row; the exact living outline’s real two-producer PTY check showed only one tree. Both failed before the fix (0/2, 8.446s tests / 9.21s wall, `artifact://1028`). A fixture string-quoting compile error was corrected before this behavioral RED; it was not counted as evidence.
- Exact collision mutation: remove only the cumulative vertical origin from the production placement rectangle. Both lowest and living checks failed again (0/2, 8.412s tests / 9.59s wall, `artifact://1034`). Restoring the origin returned green.
- Standalone real-executable smoke: two separately supervised instances of the retained agent-tree producer declare into the same stock list. Clicking each visible workspace row collapses only that producer’s tree and independently restores it, demonstrating actual producer stdin routing rather than request echo. The same run observes all four original stock states. Passed in 4.973s smoke / 11.29s build-plus-run wall (`local://herdr-fork-task31-review-fix-smoke.txt`); disposable runner removed.
- Final bounded-fix gates: Rustfmt check, locked all-target workspace check, strict Clippy and focused declarations/living/agent-tree/local-PTY nextest passed warning-free (15/15, 0 skipped, 5.272s tests / 20.37s chain, `artifact://1039`). Fifteen active clusters and immediate tab-focus input remain covered. No TypeScript package changed in this fix; the preceding isolated frozen-install/test/typecheck/lint/format/strict-Biome evidence remains applicable.

The fix and evidence are folded into the existing single Scenario Outline commit at `herdr-fork-slice-3-task-1a`. Exactly one basic review and one bounded fix pass completed. The final lead-owned process-isolated workspace layer passed 3,716 tests, with 13 skipped, in 6.020s test runtime and 42.76s wall time. Initial split-check measured 2,269 reviewable lines and no lock/generated changes; this single cohesive outline remains an atomic lettered part. No PR, push, demo or recording occurred.

## Task 3.2: Each supported nested widget node

The public Rust Node and retained producer Zod schema add Spacer logical spacing and ordered List entries. Core defines horizontal Row and vertical Column/List traversal; the client projects their nested contents without fixture-specific layout. Leaders now paint and allocate elastically outside the optimized flat-row path without claiming vertical flex. Empty containers consume no implicit space. Stock UI now declares its intentional empty tab-strip gap as a Spacer, preserving its prior composition in all four states. Session API/transport, existing Surface semantics, producer action routing and the read-only daily-driver checkout remain unchanged. No additional zones/styles/hot-apply, core actions or second consumer are delivered.

- Initial lowest-seam baseline: Row/Column/Label inherited green; Leader/Spacer/List failed (3 passed, 3 failed, 2 skipped, 0.005s tests / 0.824818s wall, `artifact://1072`). Unknown missing protocol variants were real boundary failures; an earlier wide-cell expected-string mistake and a painter-closure compile error are not counted as behavioral RED. Six independent cases render real declarations with nested/empty containers, zero/nonzero spacing, wide text and widths 1, 2, 4, 13 and 80.
- Exact six node-specific mutations each failed both its lowest renderer and living producer/executable check (0 passed, 2 failed, 13 skipped): Row axis (`artifact://1116`, 0.588s / 1.740290s wall), Column axis (`artifact://1119`, 8.422s / 9.548248s), List axis (`artifact://1122`, 0.698s / 1.816253s), Leader painting (`artifact://1125`, 0.365s / 1.452688s), Spacer extent (`artifact://1128`, 0.611s / 1.765014s), Label painting (`artifact://1136`, 8.281s / 8.550672s). Each was restored and its two checks passed. An additional nested-traversal mutation failed all twelve lowest/living cases (0/12), then restoration passed all twelve (0.733s / 1.878151s wall, `artifact://1101`). An interrupted label-mutation invocation was recovered without leaving owned processes; only the completed rerun is counted.
- Preserved-stock regression: zero-extent empty rows exposed its previously implicit tab-strip blank line; existing lowest/living stock checks failed (`artifact://1140`). Replacing that intentional gap with a producer-declared Spacer restored all four states and separate-producer routing (15/15 declarations/living tests, 5.261s / 5.554443s wall, `artifact://1145`).
- Final Rust gates passed: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked`, strict workspace/all-target Clippy, focused process-isolated declarations/living/agent-tree/local-PTY nextest: 27 passed, 0 failed/skipped, 5.379s tests / 7.802157s gate-chain wall (`artifact://1148`). Coverage retains fifteen active clusters and immediate tab-focus input. No full-workspace test layer or code review was run by the engine.
- Changed TypeScript package passed frozen install, 6 tests / 18 assertions / 0 failures (54ms test runtime), typecheck, lint, format and strict Biome CI (`--error-on-warnings --enforce-assist=true`): 1.128588s chain wall. No rule suppression, dependency version change or store-package implementation change.
- Final standalone built-executable outer-PTY smoke passed outside the test harness at 40×24 and 80×24. The same validated generic nested producer declaration displayed all six node kinds: horizontal/vertical text positions, wide text, ordered list entries, zero/three-unit blank spacing and leaders growing from 31 to 71 dots between LEFT/RIGHT. Smoke 0.703s, build-plus-run 6.608834s wall; evidence `local://herdr-fork-task32-smoke.txt`. Isolated named sockets and all HOME/XDG/config roots; original `server.stop`, bounded fallback and client shutdown; temporary roots and runners removed.

The living feature contains exactly the implemented six-example outline. Only Task 3.2 metadata is completed; Tasks 3.3–3.5 remain TODO. LSP reference queries resolved the public Node consumers and both Core flow call sites (`artifact://1110`). Split-check measured 331 reviewable lines with no lock/generated changes, so no split was needed. Exactly one basic review of 88628092..700a7610 returned no findings in 89.045s. The final process-isolated workspace layer passed 3,728 tests, with 13 skipped, in 6.394s test runtime and 28.74s wall time. No PR, push, demo or recording occurred.

## Task 3.3: Styled declarations in zones

Shared declarations add Footer/FooterRight and optional palette-neutral named tones on Labels/Leaders. The terminal client owns tone-to-palette mapping, one-row footer clipping and right alignment of nonelastic footer-right contents. The public stock producer declares both slots using its existing New-tab/Menu fallbacks; all four stock states and the separate list-producer routing remain exercised. No session API, action contract, GUI/web renderer or second-consumer implementation changed.

- Lowest RED: the real shared declaration parser rejected the missing `tone` field (0 passed / 1 failed, 0.004s tests / 6.03s wall, `artifact://1201`). Test-observation repairs for Unicode indexing, decoder default-color semantics and inherited `NO_COLOR=1` are not counted as behavioral RED. Palette gates explicitly remove NO_COLOR.
- Exact mutation: map only BgLime to DarkGreen instead of Green in the production segment palette. Both lowest rendered-cell and exact living supervised-producer/outer-PTY checks failed (0/2, 0.764s tests / 1.91s wall, `artifact://1235`); the mapping was restored. Assertions check decoded cell foreground/background palette indices, label/muted/leader/status roles, list/footer/footer-right origins and Surface boundaries at 80×24 and 52×11. Additional boundary checks cover short right-aligned footer content, clipped excess footer nesting, duplicate zone rejection and plugin x/width/pixels rejection.
- Strict Rust gates: Rustfmt --all --check, locked workspace all-target check, workspace all-target Clippy -D warnings, process-isolated focused declarations/living/agent-tree/local-PTY nextest and a locked executable build passed (29/29, no skipped, 25.848s tests / 56.24s chain wall, `artifact://1255`). Existing fifteen-active-cluster and immediate-focus-input checks are retained.
- Isolated frozen installs, tests, typechecks, lint, format and strict Biome CI (--error-on-warnings --enforce-assist=true) passed for agent-tree (7 tests / 20 assertions / 80ms) and the retained store (7 tests / 15 assertions / 457ms); 2.84s chain wall. No warnings were suppressed or dependencies changed.
- Final-layer palette setup repair: artifact://1628 reported the lowest styled check decoding default background instead of Palette(10) under inherited NO_COLOR=1. Crossterm suppresses its ANSI palette commands in that environment; the original Task 3.3 gate had removed NO_COLOR, while living Console startup already removes it. The lowest check now explicitly enables color output using the existing agent-tree test convention, without changing production NO_COLOR behavior or weakening cell assertions. With NO_COLOR still inherited, lowest/living passed 2/2 (0.714s tests / 8.25s fmt/build/run wall). Replacing only the production BgLime mapping with DarkGreen then failed both seams on actual Palette(2) versus expected Palette(10), 0/2, 0.859s tests / 2.98s wall (artifact://1648); the original mapping was restored. The standalone runtime palette/padding proof below remains unchanged: this repair changes test setup only.
- Restored palette-repair gates: Rustfmt --all --check, locked workspace all-target check, workspace all-target Clippy -D warnings, locked executable build and the two styled plus four action tests passed with inherited NO_COLOR=1 (6/6, 16 unrelated styled tests filtered, 2.463s tests / 11.97s chain wall, artifact://1653). No second review or full-layer rerun was performed here; the lead owns the final full-layer gate.
- Standalone non-test smoke correction: artifact://1256 is a failed decoder assertion, not retained evidence of a pass; the earlier passing-run reference was wrong. The restored bounded-fix pass is retained verbatim at local://herdr-task33-review-smoke.txt (0.903s smoke / 1.23s wall). Built executable, supervised validated producer and stock composition rendered L/F/R at (0,1)/(0,22)/(0,23), actual lime 00ff00, muted 7f7f7f, cyan 00ffff and working-status ffff00 cells. Final bg-lime-label padding stayed default when unfocused and palette slot 4 (pyte xterm 0000ee) when focused; the clipped wide-label/final-bg-lime-leader padding stayed default. HOME/XDG/config and named sockets were isolated, original server.stop plus bounded fallback cleaned the root/processes, and the disposable runner was removed. Decoder-only wrong expectations for focused-color naming and emission line splitting were corrected before this retained passing run; they are not production RED.

The first scenario is DONE / Living updated YES; hot replacement is not claimed by this commit. Its existing basic review found final-segment background leaking into padding. Regression RED failed both seams causally: lowest actual padding Palette(10) instead of default, living Palette(10) instead of focused Palette(4), 0/2, 0.811s tests / 3.68s wall (artifact://1342). Resetting renderer-owned background before padding fixed the bug. Removing exactly that background reset after GREEN again failed both rendered-padding assertions, 0/2, 0.777s combined style test time, in the 4-check mutation batch (artifact://1354; 15.257s batch tests / 31.04s build-plus-run wall); it was restored. Strict restored gates for both bounded fixes passed: fmt check, locked worksp…

### Task 3.3: Running whole-tree hot replacement

This behavior was inherited GREEN: no fabricated RED or extra production convention. Real supervised producer and built client checks drive horizontal OLD/TREE followed by complete changed-text/nesting NEW and nested NESTED/ONLY, checking every old cell absent, unchanged producer/client PID and sockets and unchanged Core boot. The lowest check now reuses one terminal decoder across frames instead of allocating a fresh decoder per message. Its existing five-second receive timeout and eight-second observation deadline explicitly report the last actual decoded rows, so a blocked replacement fails with the visible old tree rather than a raw resource error.

- Baseline GREEN: lowest 1/1, 0.382s tests / 6.23s wall; both lowest/living 2/2, 1.049s tests / 2.03s wall. Bounded-fix GREEN passed all four style/hot seams, 4/4, 1.975s tests / 5.91s wall.
- Mutation evidence correction: artifact://1285 did not prove a causal two-seam failure: its lowest check failed with Resource temporarily unavailable during fresh Terminal allocation, not an old-contents assertion. A subsequent run still hit the public receive timeout and is also not claimed as causal evidence (hot subset of artifact://1354). After reusing the decoder and reporting the last rendered tree at the receive deadline, the exact producer-host insert(key, rows) -> entry(key).or_insert(rows) mutation failed BOTH seams because OLDTREE remained visible: living hit its observation deadline at 8.352s, lowest explicitly reported replacement NEW not visible before receive deadline with last rendered rows OLDTREE at 5.785s (0/2, 14.138s tests / 26.44s build-plus-run wall, artifact://1359). The original insert was restored before gates. The earlier client-cache-None guard failed only the lowest check and remains unclaimed and fully restored.
- Restored strict bounded-fix chain passed: Rustfmt --all --check, locked workspace/all-target check, strict workspace/all-target Clippy -D warnings, executable build BEFORE focused declarations/living/hot-apply/agent-tree/local-PTY nextest, 31/31 with no skipped, 27.987s tests / 66.50s chain wall (artifact://1365). Earlier pre-review 31/31 evidence remains artifact://1293; it is not substituted for the corrected checks.
- Both TypeScript packages repeated isolated frozen-install/test/typecheck/lint/format/strict Biome CI: agent-tree 7 tests / 20 assertions / 64ms; store 7 tests / 15 assertions / 437ms; 2.71s total chain wall, no warnings. No TypeScript implementation changed in the bounded fix.
- Fresh standalone non-test proof is retained verbatim at local://herdr-task33-review-smoke.txt: 0.903s smoke / 1.23s runner wall; actual outer PTY OLD/TREE changed to NEW/NESTED/ONLY, every old string absent, same live client 3099969 and producer 3099968, endpoint inode 64124718 and executable mtime 1791684427037151062 unchanged. Four validated whole-tree emissions came from the same producer and API socket. This same run also proved default/focused palette padding. All HOME/XDG/config roots and named sockets were isolated; original server.stop plus bounded fallback cleaned the processes/root, and the owned disposable runner was removed.

Both Task 3.3 scenarios are DONE / Living updated YES; Tasks 3.4 and 3.5 remain TODO. The lead performed exactly one basic review per lettered unit (55.754s and 46.963s), then one batched bounded-fix pass and the demonstrated layer startup-race correction. Final full-workspace nextest passed 3732 tests with 13 skipped in 6.253s tests / 15.07s wall on final behavioral content 982a4607. Only this evidence paragraph was folded afterward; no second review, PR, push or demos.

#### Initial boot-ready baseline follow-up

The lead’s full-layer run on 7414ad50 exposed a genuine test startup-order race: 3731 passed / 1 failed / 13 skipped, 6.599s tests / 42.03s wall (artifact://1393), failing only the initial nonempty boot assertion after OLDTREE was already rendered. This reported RED was not rerun to confirm it. Client::attach initializes boot identity empty; Client::receive sets the actual identity only on ClientShellSnapshot or its ENDPOINT_SNAPSHOT_KIND control. Neutral declarations can arrive earlier. The existing bounded observation now waits for BOTH the rendered marker and the received initial shell snapshot’s boot identity before recording the baseline and triggering the producer’s replacement. The redundant post-observation nonempty assertion was removed, not the restart contract: exact equality with that ready, pre-replacement boot identity is still required after the visible replacement. No production behavior or protocol changed.

- Focused GREEN after this startup fix: both lowest and living passed, 2/2, 1.071s tests / 9.18s build-plus-run wall. Exact host insert(key, rows) -> entry(key).or_insert(rows) mutation still failed BOTH because OLDTREE remained rendered: living observation deadline 8.381s, lowest replacement NEW receive deadline with decoded OLDTREE 5.592s; 0/2, 13.973s tests / 30.13s build-plus-run wall (artifact://1406). The original host insert was restored before final gates.
- Restored startup-fix strict chain passed: fmt --all --check, locked workspace/all-target check, workspace/all-target Clippy -D warnings, executable build before focused five-binary nextest, 31/31 with zero skipped, 27.011s tests / 63.17s chain wall (artifact://1412). Isolated frozen installs/tests/typechecks/lint/format/strict Biome CI again passed both TypeScript packages: 14 tests / 35 assertions, agent-tree 61ms and store 433ms, 2.61s total chain wall.
- Fresh non-test standalone proof is retained verbatim at local://herdr-task33-boot-smoke.txt: actual built-client outer PTY OLDTREE changed to NEW/NESTED/ONLY with all old strings absent, same live client 3171034 and supervised validated producer 3171033, endpoint inode 64124830 and executable mtime 1791684896336098151 unchanged; 0.262s smoke / 0.50s runner wall. HOME/XDG/config and named sockets were isolated; original server.stop plus bounded fallback cleaned owned processes/root, and the disposable runner was removed. This supplements, rather than replaces, the retained styled-padding smoke.

## Task 3.4: Correlated producer stdin focus

The retained producer transport and focus semantics were already implemented: inherited GREEN, not a fabricated behavioral RED. New lowest public Input::dispatch and literal living outer-PTY checks observe the real agent-tree child through a transparent byte-forwarding boundary recorder. They require an initially unfocused nonempty target, the original action envelope with tab/workspace context and plugin_ui invocation source, matching successful result ID, Core's succeeded command log, requested session focus and the still-running producer. The living Given waits for the actual agent-tree header, avoiding stock fallback activation. The existing Console driver is shared without production API changes; its existing callers retain their child/screen access. Existing fifteen-active-tab focus/input and independent-producer routing tests are unchanged.

- Exact mutation: change only the production forwarder's action_id to mutated-focus-tab. Both checks fail the explicit received action identity assertion (0/2, 0.760s tests / 14.39s build-plus-run wall; artifact://1478), not allocation, timing or runtime startup. Restore returns both to green. Initial harness escaping, wrong row selection and extra premature shell input were setup errors, not claimed behavioral RED; the test stays on the assigned action contract.
- Final scenario gates: all-workspace fmt check, locked workspace/all-target check, strict workspace/all-target Clippy -D warnings, built executable before focused nextest: 2/2 passed, zero skipped, 0.880s tests / 6.00s chain wall. Frozen install, tests, typecheck, lint, format and strict Biome CI --error-on-warnings --enforce-assist=true passed both unchanged retained packages: agent-tree 7 tests / 20 assertions / 1.28s chain; store 7 tests / 15 assertions / 1.83s chain.
- Standalone non-test 80x24 built-client outer-PTY smoke: actual producer declaration click changed focus w2:t1 to unfocused w2:t2; producer PID 3261683 received focus-tab, workspace w2, tab w2:t2 and correlation ID, returned action_result plugin-log-1 ok:true, and Core's original plugin.log.list reported succeeded. Client PID 3261665 and the producer stayed live. Exact evidence: artifact://1512; 0.704s smoke / 3.03s runner wall. Every HOME/XDG/config root and named socket was isolated; original server.stop preceded bounded owned-process cleanup. Early standalone harness attempts clicked stock fallback or stopped draining the outer PTY; readiness/draining were corrected without production changes.

### Task 3.4a: One bounded review repair — invocation context correlation

The lead's single basic review found the P2 missing assertion on context.correlation_id, which is distinct from action_result.id. The original endpoint rewrites invocation IDs to endpoint:{Core boot}:{endpoint client}:{request ID} (server/headless/endpoint_requests.rs79-83); the original plugin_ui context merger carries that request ID (app/api/plugins/context.rs14-20). The fixture now independently receives the current Core boot through the public Client contract before activation, requires that exact boot prefix and a numeric endpoint client, and requires the exact activated first request ID widget-action-1. Existing result.id == stdin.id == command-log.log_id and succeeded-result/focus assertions remain. No production contract or behavior changes; no second review or full-workspace test layer is run by this engine.

- Initial stronger checks inherit GREEN: 2/2, 0.767s tests / 3.64s build-plus-run wall. Exact production mutation removes only correlation_id from the forwarded context while retaining the request/result envelope IDs and tab action. BOTH lowest and living fail producer invocation correlation missing (0/2, 0.817s tests / 15.20s build-plus-run wall; artifact://1604). The failure names the missing causal field, not allocation, startup or scheduling; the original forwarder is restored.
- Restored bounded-fix strict gates: fmt --all --check; locked workspace/all-target check; workspace/all-target Clippy -D warnings; built executable before focused Task 3.4 nextest; 4/4 passed, zero skipped, 2.174s tests / 36.46s chain wall. Both retained package frozen installs/tests/type/lint/format/strict Biome CI pass again: agent-tree 7 tests / 20 assertions / 1.16s chain; store 7 tests / 15 assertions / 1.54s chain.
- Fresh non-test 80x24 actual outer-PTY declaration click observes expected Core boot 3342086-1791686049430572408 independently through a disposable public Client reader. The actual producer receives endpoint:3342086-1791686049430572408:2:widget-action-1, with workspace w2 and initially unfocused tab w2:t2; matching plugin-log-1 successful result and Core succeeded log accompany focus w2:t1 -> w2:t2. Producer 3342188/client 3342186 stay live. Exact output: local://herdr-task34-context-correlation-smoke.txt (artifact://1615); 0.637s smoke / 2.98s runner wall. All HOME/XDG/config/named sockets are isolated; original server.stop precedes bounded cleanup. Disposable reader/runner and owned Python environment are removed. This supplements the original focus and reserved-action proofs.
## Task 3.4: Plugins place Core-defined new/menu actions

Both outline rows inherit existing shared Core semantics: no fabricated behavioral RED and no production API change. An installed, long-lived adversarial plugin declares host:new or host:menu with its own initial label in List, then hot-replaces the label and placement into FooterRight. Its installed reserved handler really calls the retained store's original tab.rename API to hijack a live tab; a nonreserved probe executes that exact same handler body before AND after the replacement, proving HIJACKED-before and HIJACKED-after state changes. The real client activation must instead open only the Core new-tab/menu interaction. New-tab text/Enter creates and focuses the explicitly named tab. Menu opens and closes the declared menu. Both reserved activations leave the original tab Safe, and the nonempty handler journal contains exactly the two probes, never a reserved action. Both declarations come from the same producer PID. The lowest public Input dispatch seam and every literal living step cover both example rows in order through actual production, without echo mocks or source assertions. A mistaken full-viewport footer-width assertion was corrected to the public stock composition's sidebar-contained right alignment; that harness mismatch is not behavioral RED.

- Mutating only shared Core precedence for host:new to return Effect::Plugin fails BOTH lowest and living: explicit missing Core new interaction and the actual HIJACKED-before tab rename visible in the outer PTY (0/2, 0.951s tests / 2.95s build-plus-run wall, artifact://1533). Separately mutating host:menu fails BOTH on missing Core menu and actual HIJACKED-before after the complete new row passes (0/2, 2.154s tests / 4.02s wall, artifact://1537). These failures are semantic hijack assertions, not scheduling/allocation/startup. Both exact mutations are restored.
- Final scenario content passed all-workspace fmt, locked workspace/all-target check, strict workspace/all-target Clippy -D warnings, built executable before focused nextest: all four Task 3.4 tests pass, zero skipped, 2.554s tests / 7.22s chain wall. Both retained packages again pass frozen install/tests/type/lint/format/strict Biome CI: agent-tree 7 tests / 20 assertions / 1.22s chain; store 7 tests / 15 assertions / 1.55s chain. Earlier fully gated tasks are not rerun here; the lead owns the one final full-layer run and one basic review per scenario-cutpoint unit.
- Standalone non-test outer-PTY proof of both outline rows, before AND after hot replacement: new producer 3285040/client 3285041, initial placement [0,1] and replacement [11,23], Name interaction and two actually created/focused Standalone-before/Standalone-after tabs; menu producer 3286191/client 3286192, initial [0,1] and replacement [10,23], Session menu opened/closed both times. Every installed hijack handler is positively exercised via the original API twice, yet all four actual reserved mouse activations avoid it. Nonempty journals contain exactly before/after probe records, and each pair of declarations has the same producer PID. Exact complete output: local://herdr-task34-reserved-smoke.txt (artifact://1530); new 4.380s, menu 1.483s, combined runner 10.45s wall. HOME/XDG/config and unique named sockets were isolated; original server.stop preceded bounded owned-process/root cleanup. The focus standalone output is also retained at local://herdr-task34-focus-smoke.txt.

Both Task 3.4 scenarios are DONE / Living updated YES. The lead split at scenario boundaries (553/375 reviewable lines; first is a single-scenario exception) and ran exactly one basic review per part, 46.927s/39.211s. One bounded correlation-assertion repair passed exact mutation and runtime proof. Final layer found inherited NO_COLOR=1 in the lowest styled test; color setup was repaired in its original Task 3.3 commit, with Palette(2) versus Palette(10) mutation failing both seams. Final full-workspace nextest passed 3736 tests with 13 skipped in 6.228s tests / 15.20s wall on 7428a640. Only this acceptance paragraph was folded afterward. Task 3.5 remains TODO; no PR, push or demos.

## Task 3.5: Independent renderer-neutral wire consumption

The new Python standard-library consumer connects directly to the actual Core client socket, implements generation-1 framing/variable-integer decoding independently, and imports neither application code nor the terminal renderer/Rust codec. ADR 0005 documents its existing handshake, tags, declaration fields, action transport and plain-PTY Surface layout. Lowest and literal living tests run the same production-bound fixture: real agent-tree and stock producers, public semantic-tone producer, actual shell ANSI output decoded by Core into cells, exact RGB/bold/cursor/pane state, action identity/context obtained from the stream, independently encoded invocation, unfocused→focused snapshot transition and original session.snapshot confirmation. Every received declaration is recursively checked, not source-searched or re-serialized with the client codec. No GUI/web client, dependency, public symbol or schema change is introduced.

- Inherited happy-path GREEN: 1/1, 0.852s tests / 1.66s wall. Early wrong fixture root_pane/tag/color assumptions and shell-prompt cursor movement were setup errors, not claimed behavioral RED. Explicit unconditional SGR output in the real PTY fixture supplies color despite inherited NO_COLOR=1; no production color override was added.
- Actual malicious producer input exposed the remaining boundary gap: a Label role containing ESC+[31m was forwarded, violating declaration neutrality. Both lowest and exact living tests failed terminal control in declaration (0/2, 0.839s tests / 1.64s wall, artifact://1713). Core now rejects controls in Label role as it already did in text. GREEN: 2/2, 0.894s tests / 6.80s wall. Exact mutation removing only role.chars() from the production guard again failed BOTH for the same decoded declaration assertion (0/2, 0.828s tests / 6.51s wall, artifact://1718); guard restored.
- Final strict Rust chain on complete behavioral content: cargo fmt --all -- --check; cargo check --workspace --all-targets --locked; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo build -p herdr-fork --locked BEFORE focused nextest for plugin_actions and declarations. All 15 tests passed, zero skipped, 2.272s tests / 10.65s chain wall. One earlier enclosing Eval timeout discarded gate evidence; no result from it is claimed. The original session API directory remains byte-identical to the read-only reference (diff -qr, no differences, 0.054s).
- Both retained producer packages passed frozen install, tests, TypeScript check, lint, format check and Biome CI --error-on-warnings --enforce-assist=true: agent-tree 7 tests / 20 assertions / 1.17s chain; store 7 tests / 15 assertions / 1.53s chain. No TypeScript source changed.
- Standalone non-test-built Core/independent consumer smoke: Core PID 3488146, named session herdr-task35-t0_zgqmp, actual stock/agent-tree/tone declarations, received focus-tab context workspace w2/tab w2:t2, actual focus w2:t1→w2:t2, pane w2:p2, neutral-live output, RGB 0x02112233, bold bit, cursor [6,3,1,6]. Python consumer imports/compiles no terminal renderer. Smoke 0.629s / 0.97s runner wall; decoded runtime artifact retained at /tmp/herdr-task35-wire.json. All HOME/XDG/config roots and named sockets isolated; original server.stop and bounded cleanup removed the owned Core/root. Disposable driver removed.

Task 3.5 is DONE / Living updated YES. Reviewable diff 433 lines, no split. Exactly one basic review returned no findings in 49.107s. Final full-workspace nextest passed 3738 tests with 13 skipped in 6.237s tests / 20.21s wall on 95caa0e6; only this acceptance paragraph was folded afterward. No PR, push or demos; Task 4.1 is next.

## Task 4.1: Same-PTY continuity after client disconnect

The original runtime already owns session topology and PTYs independently of attached clients: inherited GREEN, not a fabricated missing-behavior RED. No production code, dependency, session API, schema or persistence format changed. The direct public Client attach-stream test and the exact living scenario create three workspaces, four tabs and four real panes. A foreground shell command records its PID and monotonically increasing ticks while emitting live PTY output. Assertions preserve its Linux process start time, controlling-terminal number and slave-PTY path across disconnection and require progress with no client attached, followed by newer output through a newly attached client. The direct stream additionally reattaches after Core focus moves to a different tab, then follows a Core-driven switch back without restoring stale client focus. The living check launches two different actual executable clients under outer PTYs, detaches the first through normal Ctrl-Q, and demonstrates that every other retained pane still accepts input and renders fresh output. The existing local-PTY living binding remains scoped to its original scenario.

- Initial continuity GREEN: 2/2, zero skipped, 2.088s tests / 4.82s build-plus-run wall (artifact://1788). An earlier living matcher failed because sidebar content precedes the usable-pane output; this fixture correction is not claimed as behavioral RED.
- Exact production mutation: insert self.app.state.terminals.clear() at HeadlessServer::remove_client. BOTH lowest and actual-client living checks fail the causal detach destroyed topology assertion: all four original pane identities disappear (0/2, zero skipped, 1.830s tests / 17.90s build-plus-run wall; artifact://1792). The original runtime is restored byte-for-byte. The mutation proves topology/live-pane retention; the independent PID/start/PTY and disconnected-progress assertions also remain required.
- Restored strict gates passed: cargo fmt --all -- --check; cargo check --workspace --all-targets --locked; cargo clippy --workspace --all-targets --locked -- -D warnings; locked executable build before focused session_continuity/local_pty nextest. Five tests passed, zero skipped, 2.242s tests / 11.93s chain wall (artifact://1798). No workspace/full nextest or cargo test --workspace was run by the engine. TypeScript packages and their previously tested dependency locks are unchanged, so their gates were not rerun. Original src/api compares byte-identical with the read-only daily-driver checkout.
- Standalone non-test-built Core and two actual executable outer-PTY clients passed in 2.405s smoke / 2.70s runner wall (artifact://1815): stable named session continuity-1791687401569339443, Core PID 3588141/start 18324075, clients 3588218 then 3588602, command PID 3588207/start 18324093, tty_nr 34844 and slave /dev/pts/28 unchanged. Ticks advance 2 to 7 while disconnected and to 9 after reattach; new client output, unchanged 3/4/4 topology, all other usable panes and unchanged socket inodes 123208532/123208535 are observed. Exact JSON, first/second raw outer-PTY output, detach output and decoded second output are retained in /tmp/herdr-task41-smoke-wuds2fn9. All HOME/XDG/config roots and named endpoints were isolated. Both clients exited normally and were reaped; original server.stop stopped Core and the command process was confirmed gone. Early standalone attempts failed because the driver stopped draining the outer PTY; the driver was corrected without production edits and removed after the successful smoke.

Task 4.1 is DONE / Living updated YES. Reviewable diff 277 lines, no split. Exactly one basic review returned no findings in 32.912s. Final full-workspace nextest passed 3740 tests with 13 skipped in 6.265s tests / 15.28s wall on 7c995f5c. Only this acceptance paragraph was folded afterward. Task 4.2 remains TODO; no PR, push or demos.

## Task 4.2: Persisted topology restore after Core relaunch

Inherited persistence already implements this behavior: truthful inherited GREEN, with no production implementation, API, schema, dependency or persistence-format change. The lowest meaningful process-boundary test drives the original session API, normally stops Core 1 with server.stop, awaits successful exit, reads its actual session.json before Core 2 starts, and relaunches the same executable with the same isolated HOME/XDG/config/session paths. The literal living scenario uses actual executable outer-PTY clients before and after that relaunch. Both cover three workspaces, four tabs and six panes, including a nested right/down split at ratios 0.6/0.35, an unfocused sibling tab, and existing directories containing spaces and Unicode. Labels, workspace/tab membership, ordered leaf cwds, split directions/ratios and exact pane geometry are compared across relaunch after equal-size attach. Every restored pane must have a new shell PID, the actual Linux process cwd, and executable shell input/output in that cwd; the living commands enter through the actual client keyboard stream. Short split-literal tokens exclude input-echo false positives, while real pwd output is separately captured without narrow-pane wrapping. A foreground sleep PID must disappear on shutdown: Task 4.1 continuity is not falsely claimed across a Core restart.

- Initial inherited GREEN: 2/2 tests, zero skipped, 3.691s tests / 6.50s build-plus-run wall. Stale-socket startup readiness, wrapped output matching and waiting for the actual focused client Surface were harness corrections, not missing-behavior RED.
- Exact production mutations in persist/restore.rs each defeat BOTH lowest and living tests causally: remap_inner ratio: *ratio to 1.0 - *ratio changes nested geometry (0/2, 1.913s, artifact://1878); saved_cwd to saved_cwd.join("..") restores shells in the wrong directories (0/2, 1.959s, artifact://1881); terminal_runtimes.clear() immediately after restored-pane creation destroys replacement PTYs and removes their topology (0/2, 1.972s, artifact://1891). Each fails restore changed saved topology/layout/cwds, not build/startup. All exact mutations were restored; no runtime diff remains.
- Restored strict chain passed cargo fmt --all -- --check; cargo check --workspace --all-targets --locked; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo build -p herdr-fork --locked BEFORE focused session_restore/session_continuity/local_pty nextest: seven passed, zero skipped, 4.041s tests / 14.13s chain wall (artifact://1898). An earlier enclosing Eval timeout discarded gate evidence and is not claimed. Original src/api is byte-identical to the read-only daily-driver checkout (diff -qr in that chain). No full-workspace nextest, cargo test --workspace or unchanged TypeScript gates were run by this engine.
- Standalone non-test driver, using the already locked-built production executable: Core 3735652 stopped normally, then Core 3735863 restored the same named session at /tmp/at-18dd5b117ace508a. Actual restored client 3735940 displayed usable output from all six replacement shells 3735897/3735900/3735903/3735906/3735910/3735913 in their respective saved directories; stale foreground command 3735724 ended. The real 3165-byte session artifact was read between Core instances and never substituted as a fixture. Smoke passed in 4.0816s / 4.63s compile-plus-run wall. Evidence copies retained only after real restore: /tmp/herdr-task42-session.json and /tmp/herdr-task42-smoke.txt. The initially reported /tmp/herdr-task42-restored.json was not retained; that evidence claim is withdrawn. Original server.stop, bounded owned-process cleanup and root removal completed; all listed Core/client/shell/command PIDs were absent. The disposable driver/source were removed.

Task 4.2 is DONE / Living updated YES. Reviewable diff 363 lines, no split. Exactly one basic review took 59.482s and found one missing evidence-file claim; the bounded fix withdraws that claim without changing behavioral content. Final full-workspace nextest passed 3742 tests with 13 skipped in 6.386s tests / 16.57s wall. Acceptance text and the evidence correction were folded into the scenario commit. Task 5.1 remains TODO; no PR, push or demos.

## Task 5.1: Actual achy client attaches to one remote Core

The new UI client handles --remote itself, using the vendored SshStdioBridge, RemoteSsh, endpoint metadata/path validation, binary capability probes, named-session command quoting, and owned bridge teardown. Optional --remote-executable selects an existing compatible executable; automatic PATH/known-path resolution remains the default. No installation, old-terminal child, new dependency, auth/security change, wire/Surface-format change, or session API rewrite. The remote bridge enables the same public stock producer on its Core; the local renderer receives the unchanged endpoint Surface stream. Original src/api remains byte-identical (diff -qr passed).

- Actual-achy RED: cargo nextest run --locked --test remote_attach --run-ignored only --no-capture failed at the visible-output assertion because the executable rejected --remote (7.087s test / 7.95s wall, artifact://1968). First GREEN: both real-SSH transport and actual achy UI tests passed (9.407s tests / 50.37s strict-chain wall, artifact://1984). The literal features/remote-attach.feature is executable at Given/When/Then; both SSH tests are explicitly ignored unless opted in, so normal suites do not depend on achy availability.
- Exact production mutations defeated BOTH seams: RemoteConnection passing decoy instead of the requested session displays decoy-forbidden and misses selected-live (0/2, 21.171s tests / 32.42s wall, artifact://1992); dropping received PaneSurface frames removes live output (0/2, 21.287s / 32.34s, artifact://1995); Input::type_text sending an empty TextCommit prevents actual shell execution after the initial live Surface (0/2, 21.567s / 23.50s, artifact://2001). All restored; routing/stream restoration passed 2/2 in 9.144s / 11.11s (artifact://1998), and final input restoration passed 2/2 below. Nonempty selected and decoy sessions, exact Core boot/pane identity, host/session shell output, no decoy input leakage, and missing/incompatible executable rejection are exercised.
- Restored gates passed in order: cargo fmt --all --check; cargo check --locked --workspace --all-targets; cargo clippy --locked --workspace --all-targets -- -D warnings; cargo build --locked --bin herdr-fork BEFORE cargo nextest run --locked --test remote_attach --run-ignored only --no-fail-fast --no-capture (2/2, 9.287s); focused local_pty/session_continuity/session_restore nextest (7/7, 9.942s). Combined chain 43.78s wall, artifact://2005. No full-workspace nextest, cargo test --workspace, or unchanged TypeScript checks by this engine.
- Independent non-test PTY smoke: actual achy executable PID 1618931 at disposable /tmp/herdr-task51-smoke.V4OPuNCD/herdr-fork used --remote junkman --remote-executable pointing to a real exec wrapper, with isolated HOME/XDG/config roots. Named Core PID 3963958, session remote51-1791690049556983866, displayed its seeded Surface and rendered actual-ssh:junkman:remote51-1791690049556983866 from typed shell input; decoy Core PID 3964007 coexisted. Ctrl-Q detached normally with exit 0 while Core still answered; server.stop stopped both owned Cores with exit 0. Passed 7.9821s runtime / 8.34s wall. Evidence retained at /tmp/herdr-task51-smoke.txt; binaries/roots and throwaway driver removed. Earlier smoke-driver attempts failed observation because incremental ANSI chunks were parsed incompletely and the accumulated-output parser imposed backpressure; a background PTY drain with bounded cross-chunk decoding corrected the driver without changing production.

Task 5.1 is DONE / Living updated YES. Reviewable diff 496 lines, no split. Exactly one basic review returned no findings in 43.703s. Final full-workspace nextest passed 3742 tests with 15 skipped in 6.016s tests / 31.55s wall on d0f492b1; the two actual-achy SSH contracts passed separately in the strict chain above. Only this acceptance paragraph was folded afterward. Task 5.2 remains TODO; no PR, push, demos or daily-driver changes.

## Task 5.2: Client clipboard images in the focused pane

The new terminal client enables bracketed paste and routes the retained empty-image paste trigger through public Input::paste. It captures the current pane ID before reading the client clipboard and sends the original ClipboardImage message. A narrow runtime adapter reuses the original platform image reader, MIME/signature and payload-size boundaries, and write_remote_image_to_server. The original server staging and explicit-pane injection code are unchanged: server effective UID selects /tmp/herdr-clipboard-images-UID/, directory permissions are 0700 and new files are 0600. Session API sources compare byte-identically to the read-only reference (diff -qr crates/herdr-runtime/src/api /home/tommy/src/agents/herdr/src/api). No dependencies, authentication, security or image formats changed.

features/clipboard-images.feature contains exactly the planned single outline and both local / remote from achy examples. Five opt-in checks cover the lowest public Input::paste seam, actual executable UI paste on both machines, and literal Given/When/Then bindings for each example. Each proof uses a complete nonempty 43-byte 1x1 GIF89a offered by native xclip on a newly owned Xvfb display; no existing human clipboard is read or replaced. Two real panes are focused in turn, with snapshot/rendered readiness before paste, immediate nonfocused-pane exclusion, byte/checksum and server-UID/path/permission assertions. Terminal text wins over a stale image, and image/gif containing non-image bytes does not cross the retained signature boundary.

- Behavioral RED: actual local executable paste failed the required staged-path assertion (0/1, 5.08s tests / 5.788s wall, artifact://2090). Earlier response-shape setup errors are not behavioral RED. Initial GREEN passed 1/1 in 1.12s tests / 1.773s wall (artifact://2098).
- Exact production mutation: Input::paste targets missing-clipboard-target instead of its captured self.pane_id. The core rejects that target before staging/injection. The lowest Input seam, both executable checks, and BOTH literal living examples fail because no staged path arrives (0/5, 36.85s tests / 39.433s wall, artifact://2121). Original target restored.
- Restored strict gates passed in order: cargo fmt --all -- --check; cargo check --workspace --all-targets --locked; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo build --workspace --all-targets --locked BEFORE HERDR_TEST_XVFB=/tmp/herdr-task52-tools.Z5qxXm0y/Xvfb cargo test --locked --test clipboard_images -- --ignored --nocapture --test-threads=1. Result 5/5, zero skipped, 18.16s tests / 23.115s chain wall (artifact://2126). Xvfb was copied to an owned throwaway root from achy because local Xvfb was absent; the checks also accept an installed Xvfb or HERDR_TEST_XVFB. No unchanged TypeScript gates or full-workspace test suite ran in the engine.
- Independent non-test 160x32 PTY proof ran the built executable locally on junkman and actually on achy with --remote junkman and an isolated executable wrapper. Native system clipboards and real SSH were used, not file-drop/mock images. Both modes pasted into w1:p1 and w1:p2, with no other-pane injection, server UID 1000, 0600 files / 0700 directory, 43 bytes and SHA-256 e35f718ebdf9d601ada213eb594e9a5c671d36a272390d68dd645e3f93af6af1. Exact four server paths and client/server identities are in artifact://2149; 13.427s wall. Ctrl-Q detached and server.stop exited normally in both modes. Initial smoke-driver failures came from stripping the server producer PATH and not draining terminal output, not from production changes.
- Owned staged files, local/remote roots, display servers, clipboard owners and disposable smoke driver are cleaned; original directories are never removed. Two orphan displays from an interrupted early driver were identified by their owned task52 root command lines, and the orphan clipboard owner was matched by DISPLAY=:2 to the owned X2 socket before cleanup. The existing SSH mux caches are not treated as fixture-owned processes.

Task 5.2 is DONE / Living updated YES. Single outline with both examples remains scenario-atomic as part herdr-fork-slice-5-task-2a: 597 reviewable lines, no lock/generated lines. Exactly one basic review returned no findings in 33.325s. Final full-workspace nextest passed 3742 tests with 20 skipped in 6.991s tests / 7.96s wall; five real clipboard contracts already passed opt-in above. Only this acceptance paragraph was folded afterward. Task 6.1 remains TODO; no PR, push, demos or daily-driver changes.

## Task 6.1: Local and actual-achy click-to-repaint acceptance

The durable `examples/click-to-repaint-15-panes.rs` executable creates an isolated Core and 15 live shell PTYs across three workspaces, two tabs each (three and two panes). Every pane prints four unique identity rows and accepts input probes. Agent-tree is the real sidebar producer; multi-pane tabs now declare pane rows and route their `focus-pane` action through the retained store command. Stock remains the real tab-strip producer. No API, schema, dependency or security boundary changed.

Acceptance uses optimized executables and a 100x45 outer PTY. Each mode measures 36 actual SGR mouse clicks: 12 workspace, 12 tab and 12 pane selections, with no discarded warmups or outliers. Timing starts immediately before input dispatch and includes producer and transport delays. The observer decodes through the renderer’s final cursor/palette reset, checks every visible Surface cell (including blank padding) and selected tab identity, then independently proves input reaches the selected live PTY. Remote observation really runs on achy with `--remote junkman`; no transport duration is subtracted. JSON records machine, timestamp, session, commit snapshot, boot/Core/client/producer identities, dimensions, all 15 populations, every sample, completed-frame identities and retained producer perf log.

The causal fixes are a persistent buffered terminal writer and prompt Core API wakeups. EventHub uses a condition variable with an observed sequence captured before polling to avoid missed notifications. Unix initial request reads use the existing native fd readiness helper instead of an unconditional 100ms sleep; Windows initial-read behavior is unchanged. Existing bounded subscription probing/disconnect deadlines remain. Temporary stock instrumentation and the unsuccessful native Bun stdout experiment were reverted.

- Behavioral RED: actual pre-fix release acceptance exceeded 100ms in both modes (raw `/tmp/herdr-latency-release-local.json` and `-remote.json`). The new real-socket event-arrival and initial-request tests separately failed their 50ms deadlines before their respective fixes (artifacts 2364 and 2370). After the fixes, six subscription socket tests and two EventHub tests passed; initial full strict chain was 141.91s, with actual acceptance 3/3 in 18.69s (artifact://2387).
- Exact mutation: adding an unconditional 120ms sleep in production `Input::invoke` failed the lowest dispatch-to-completed-Surface seam and BOTH literal living examples: 0/3, 24.18s tests / 55.27s chain (artifact://2396). All display/focus proofs still matched, but 24/36 samples in each mode violated latency; maxima 194.892ms local and 207.218ms remote. Raw mutation files: `/tmp/herdr-latency-201233-local.json` and `-remote.json`. The mutation was removed.
- Restored final gates passed in order: fmt check; locked workspace/all-targets check; clippy with `-D warnings`; locked workspace/all-targets debug build; optimized all-targets build; `cargo test --release --locked --test latency -- --include-ignored --test-threads=1 --nocapture`. All three checks passed with neither living mode skipped, 18.46s tests. Including the two independent benchmark invocations below, the chain took 65.93s (artifact://2401). Final living raw files `/tmp/herdr-latency-212894-local.json` and `-remote.json` each contain 36/36 passing samples: maxima 83.001182ms local and 83.175482ms remote; every complete-visible-frame and input-focus proof passed.
- Independent non-test invocations of `target/release/examples/click-to-repaint-15-panes local|remote OUTPUT_JSON` passed outside Cargo tests: `/tmp/herdr-task61-smoke-local.json` and `-remote.json`, 36/36 samples each, maxima 71.999710ms and 89.208765ms. Observer machines are junkman and achy respectively. Both use actual executable mouse paths, real producers and live PTYs, not API-only timing. Owned clients detach with Ctrl-Q, Core stops through `server.stop`, and owned local/remote roots are removed. Daily-driver sessions and original checkout remain untouched.
- Changed TypeScript strict check and existing Biome lint/format gates passed (artifacts 2278 and 2337). No engine basic review, full-workspace test run, PR, push, demo or Kizn operation ran.

### Task 6.1 bounded review repair: all 15 populations must be ready

The lead’s one basic review took 77.387s and identified that the third panes in three-pane tabs were never selected by the click schedule. The measurements above are pre-review evidence; the final acceptance below supersedes them. The bounded repair now proves four exact visible identity rows, an independent unique live-PTY input response, and restoration of those rows for ALL 15 panes under a shared five-second readiness deadline. This happens before plugin startup, client creation or any measured click. Every population’s observed text, verified response and completion timestamp are recorded in the raw JSON. The 36-click schedule is unchanged, with no removed or discarded samples.

- Behavioral RED: a real live but unpopulated Pane002 input loop bypassed the original prerequisite, so the new expected-failure regression failed (0/1, 0.80s tests / 6.13s strict chain, artifact://2438). With the repair, that fixture fails specifically at `population readiness failed for Pane002` before measurement. The restored all-populated setup passes. Initial repair GREEN was 4/4, 24.28s tests / 57.39s strict chain including both non-test smokes (artifact://2443).
- Re-exercised exact production mutation: unconditional 120ms `Input::invoke` delay, after all 15 readiness checks. The lowest seam and BOTH literal living modes failed for latency (the negative readiness regression still passed), 29.95s tests / 59.51s chain, artifact://2449. Lowest dispatch took 163.177645ms; local/remote maxima were 192.087377ms / 209.371444ms, with 24/36 latency failures in each mode and all visible/focus proofs correct. Both mutation reports `/tmp/herdr-latency-273246-local.json` and `-remote.json` prove 15/15 readiness before the first measured dispatch. The mutation was removed.
- Final restored strict gates passed: fmt check; locked workspace/all-targets check, clippy `-D warnings`, debug and optimized builds; agent-tree `bun run check`; pinned Biome `ci --error-on-warnings --enforce-assist=true agent_tree.ts project.ts project.test.ts`; then the opt-in, serial optimized latency suite. Result 4/4, zero skipped, 24.24s tests. Including both independent non-test smokes, the final chain took 72.03s (artifact://2454). No second review or engine full-workspace run occurred.
- Final living reports `/tmp/herdr-latency-288945-local.json` and `-remote.json`: 15/15 populated/live-input/restore proofs before first dispatch, 36/36 passing click samples in each mode, maxima 87.732661ms local and 82.960293ms remote. Every completed-visible-Surface and independent input-focus proof passed.
- Final independent non-test reports `/tmp/herdr-task61-final-smoke-local.json` and `-remote.json`: 15/15 readiness before measurement in each mode; 36/36 passing samples; maxima 71.327873ms local and 83.913756ms remote, with every visible/focus proof passing. The UI observer actually runs on achy for remote mode. Owned clients/Core/roots detach, stop and clean normally; daily-driver state remains untouched.

Task 6.1 is DONE / Living updated YES with one exact outline and both literal-bound examples. Part herdr-fork-slice-6-task-1a retains the scenario-atomic exception: final 826 reviewable lines, no lock/generated changes. Exactly one basic review (77.387s) and one bounded readiness repair completed. Final full-workspace nextest passed 3746 tests with 22 skipped in 6.819s tests / 7.18s wall on 8bc1f2b5; both actual latency modes already passed opt-in above. Only this acceptance paragraph was folded afterward. Task 6.2 remains TODO; no PR, push or demos.

## Cross-space tab movement: T01 command and terminal continuity

The CLI now routes `trailboss tab move <tab-id> --workspace <space-id> [--index N]`
through the new `tab.transfer` API rather than overloading the existing in-space
`tab.move`. Core moves the owned tab and assigns destination public locators
without recreating its terminal runtimes. Clients viewing the transferred tab
follow its destination; the retained layout keeps the same focused split pane.

The isolated `cli_transfers_complete_split_tab_without_restarting_terminals`
contract passed (1/1, 0.804s tests / 28.330614s build-and-run). It verifies
destination order, split layout, labels, zoom, client focus, TerminalIds, original
Linux processes and PTYs, shell-local variables and background jobs. A separate
real CLI continuity witness passed in 0.243590s. An outside-candidate mutation
inserting the transferred tab at slot zero failed the exact order assertion
(1 failed, 0.494s tests), and the restored candidate passed. Generated API schema
refresh passed 1/1; workspace/all-target check, strict Clippy and Rustfmt passed.
These are local source proofs, not installation or human-session activation.
Events/store convergence, remaining transfer cases and handoff compatibility
are not claimed by this T01 checkpoint.

### T02: Explicit destination insertion slots

The real CLI outline `cli_transfer_uses_each_destination_insertion_slot`
exercises slots 0, 1 and 2 against fresh isolated Cores and checks exact
destination order, the reported index and removal from the source. Its initial
baseline passed: T01 already implemented these slots, so no missing-behavior
RED or production fix is claimed. Forcing slot zero outside the candidate
failed the slot-one order assertion (1 failed, 1.119s tests / 28.310275s total).
The restored related suite passed 2/2 (1.316s tests / 27.193911s total), with
Rustfmt and strict test-target Clippy/typecheck passing. A standalone real CLI
`--index 1` continuity witness passed in 0.257908s.

### T03: Shared plugin API, events and live-store convergence

Registered plugin actions can transfer through the existing store's generic
request API. A completed cross-space operation emits one `tab.transferred`
summary carrying the authoritative transfer result, source/destination
`workspace.updated` (or source `workspace.closed`) and existing `pane.moved`
notifications. No terminal creation or exit is implied. The event catalog,
subscription dispatch, plugin context/invalidation, store subscriptions,
generated schema and existing endpoint method advertisement include the new
contract; same-space `tab.moved` retains its separate semantics.

The production-boundary plugin/store/subscriber regression first failed on
the unknown `tab.transferred` subscription. After implementation it passed;
an owned plugin manifest missing its required minimum version was a fixture
setup error, not a second behavioral RED. The restored transfer suite passed
3/3 (1.810s tests / 28.773927s total). Strict Rust Clippy, Rustfmt, store/plugin
and fixture TypeScript checks, strict scoped Biome CI, all 27 existing Bun
package tests and generated-schema refresh passed.

An independent owned runtime proved the registered action, automatic store
refresh, production sidebar projection and exact five public notifications
with unchanged TerminalIds (0.300325s operation). Suppressing only the completed
summary outside the candidate let the store assertions pass but failed the
required summary observation at the subscriber timeout (6.261s tests /
32.862194s total). The mutation was restored before final candidate gates.

### T05: Background moves preserve viewed and remembered focus

One production Client outline covers Diagnostics viewed in Tools, Main and
Logs, including Tools' remembered active Build while another space is viewed.
Fresh post-transfer output frames, client/global pane focus, each unaffected
tab's focused split and non-source workspaces' active tabs remain unchanged.
The initial baseline passed all three rows; no production fix was needed.
Removing only the global active-space guard outside the candidate passed the
Tools row but failed Main's exact global focused-pane assertion (1.118s tests /
27.970657s total). The restored full transfer target passed 4/4 (1.661s tests /
27.527539s total), with strict targeted Clippy/typecheck and Rustfmt passing.
An independent three-row production Client witness also retained native-input
targeting, original PIDs, shell-local state, TerminalIds and remembered focus
(2.201213s operation / 2.215180s wall).

### T06: Removing the final source tab preserves destination identity

One outline covers Tools before and after Main, with only Build in Tools
and Diagnostics displayed in Logs. It reuses the registered plugin's production
store fixture and compares API/store topology, original TerminalIds, shell PIDs,
start times, PTYs, executable identities and shell-local state. Both rows passed
the initial baseline; no production fix was needed.

An owned mutation emulating a cached destination index after source removal
put Build under Logs rather than the original Main and failed subscribed-store
convergence (5.857s tests / 31.730225508s total). The restored full transfer target
passed 5/5 (1.669s tests / 27.777293092s total); strict targeted Clippy/typecheck
and Rustfmt passed. An independent production-store and Client witness covered
both orders, fresh Logs frames/native input, both Build shells' process/PTY,
local/CWD continuity and all five original TerminalIds without extra terminals
(1.504641327s operation / 1.546076902s wall).

### T07: Invalid transfers leave topology, view and live targets unchanged

One five-row public-wire outline rejects closed source and destination locators,
index 3 into two tabs, a malformed negative index and an old tab locator after
a valid transfer. Each rejection identifies its offending token and retains
semantic topology, selection, pane layouts, original pane targets, processes,
PTYs and shell-local state. A fresh production Client frame confirms Diagnostics
remains displayed. Attaching the Client before layout capture corrected an
initial fixture-only resize mismatch; the corrected baseline passed without a
production change.

An owned silent-index-clamping mutation passed the first closed-locator rows
but failed the index row by successfully moving Build instead of rejecting it
(1.843s tests / 27.860924395s total). The restored full target passed 6/6
(3.120s tests / 29.45445884s total), including semantic error-token assertions,
with strict targeted Clippy/typecheck and Rustfmt passing. An independent
five-row API/native-input witness retained fresh Diagnostics frames, layouts,
TerminalIds, original PID/start/PTY/executable identities and local/CWD state
(3.284154192s operation / 3.303525253s wall).

### T08: Same-space transfers retain identities and silent no-ops

One four-row outline moves Build within `[One, Build, Two]` through insertion
slots 0–3. It checks exact order, stable tab and pane locators, TerminalIds,
response mappings, and a fresh production Client Diagnostics frame in Logs.
Actual public topology subscriptions receive one `tab_moved` for slots 0/3
and no notifications for no-op slots 1/2. The inherited baseline passed all
four rows without a production change (3.704s tests / 4.547331711s total).

An owned mutation removing only the same-space event's `changed` guard failed
at slot 1: the response still reported no change and identities/view remained
correct, but an illegal `tab_moved` notification arrived
(1.853s tests / 27.776036755s total). The restored full target passed 7/7
(3.829s tests / 29.931240421s total), with strict targeted Clippy/typecheck
and Rustfmt passing. An independent real CLI/native-input witness covered
all slots, fresh Logs frames, all original public tab/pane IDs and TerminalIds,
and the nine-kind topology subscription, including 250ms no-op observation
(3.231439304s operation / 3.246254002s wall).

### T09: Inherited pane context survives live Core handoff

Transferred panes carry their old public locators in private, serde-optional
per-runtime handoff metadata. Import remaps each old internal pane ID to its
restored live pane and rejects duplicate, invalid, missing-owner or shadowing
aliases. A nonempty alias manifest requires an exact `restored-aliases <count>`
receipt at the existing restored-runtime acknowledgment, before public socket
removal and ownership commit. Alias-free handoffs retain plain `restored`.
No cold-persistence format, public API schema or inherited process environment
changed. T18/T19 compatibility scenarios remain separate evidence gates.

The real-Core regression first failed when an existing shell's actual
`pane current --current` CLI call lost its inherited locator only after handoff
(0.962s tests / 1.823045330s total). It now checks both split shells before the
move, after the move and after supported handoff, retaining PID/start/PTY,
executable, shell-local state and original `HERDR_PANE_ID`, with alias cleanup
after close and no retargeting to a replacement pane. The fixture initially
overconstrained TerminalId survival across handoff: existing restore code imports
the same FD but allocates a fresh logical TerminalId. The correction compares
aliases with the current imported terminal while retaining original TerminalId
checks across the move itself.

An owned mutation replacing the old-to-imported PaneId remap with the old raw
ID failed after handoff by targeting `w3:p1` instead of `w3:p3`
(0.724s tests / 27.141114821s total). The restored full target passed 8/8
(3.727s tests / 30.329029096s total), with strict root/runtime Clippy
(0.187020307s) and Rustfmt (2.089661171s) passing.

An independent witness ran two genuine OMP 18.13.0 TUI roots in owned PTYs,
using the actual reporter and a local streaming model fixture without host
credentials or an external model. The original copied binary preserved the
OMP/shell processes and completed its fifth model turn but stopped publishing
fresh Working state after handoff (13.187067436s operation / 13.202048578s wall).
The fixed candidate completed five actual Working-to-Done turns before/after
transfer and after handoff in the same moved OMP session
(8.717692793s operation / 8.736796244s wall). A foreign genuine root's attempted
session takeover with sequence 1000000 was ignored, and both roots continued
their own turns. OMP/shell/companion PID/start/PTY/executable identities survived;
all recorded owned processes were gone after stopping the new Core through its
API. These are isolated local proofs, not installation or human-session changes.

### T10: Ordinary restart restores transferred tab placement

The real CLI moves a split Build tab from Tools into `[One, Build, Two]` in Main.
After ordinary Core shutdown, the test reads the actual saved session, restarts
the same owned session, and checks space order, tab order, labels, distinct CWDs,
split geometry, zoom and selected Tests pane. Tools is absent from both saved
and restored topology. New shell PIDs, actual process CWDs and fresh native
Client output distinguish cold restoration from live-process continuity.

The existing implementation passed the baseline without production changes
(1/1, 0.898s tests / 1.893661339s total). An owned mutation reversing persisted
tab order failed causally with `[Two, Build, One]` instead of `[One, Build, Two]`
(0.873s tests / 26.948457523s total). After restoring the source, the full target
passed 9/9 (3.736s tests / 29.695521271s total), strict root/runtime Clippy passed
(0.487924829s), and Rustfmt check passed (2.076868464s). Restart ownership remains
local to this test file, without a shared dead-code lint suppression.

An independent ordinary-shutdown/relaunch witness checked the saved ordering,
layout, focus and missing Tools space, observed a new Core and Tests shell,
and received fresh native input/output (0.678930588s operation / 0.931078235s
total). Its initial API-ready/client-socket startup race was a witness correction;
the permanent harness already waits for the actual Client socket. These remain
isolated local proofs, not installation or activation in human sessions.

### T11: An older Core rejects transfer without source reordering

An independent witness used the actual pre-feature Core binary (0.9.3, protocol
22) and a transparent wire observer. The new CLI sent `ping` and exactly one
`tab.transfer` request, with ID `cli:tab:move`. The older Core returned its
correlated `invalid_request` for unknown `tab.transfer`; the CLI exited 1 and
reported that error as JSON on stderr. No fallback `tab.move` was sent. Tools
remained `[One, Build, Two]`, and Main and the current pane/view were unchanged.
A separate existing `tab.move` caller against the candidate Core reordered Build
to `[Build, One, Two]`. The witness passed in 0.360630982s operation /
0.954637083s total.

Existing permanent bindings cover generic executable CLI error reporting and
typed request/response correlation (`executable_session_automation_smoke` and
`existing_session_automation_preserves_typed_wire_contracts`: 2/2,
2.829s tests / 3.880730908s total), plus the actual legacy reorder handler and
its event (`api_tab_move_reorders_tabs_in_target_workspace`: 1/1,
0.019s tests / 16.501357163s total). This was inherited GREEN, with no production
change or redundant mock test. The old binary stays an owned external witness
artifact, not a permanent test dependency.

The required owned mutation routed the CLI transfer to legacy `TabMove`.
The permanent real-CLI transfer test failed because it received `tab_list`
instead of `tab_transfer` (0.461s tests / 26.668573832s total). The actual
older-Core witness also failed: the mutated CLI exited 0 instead of nonzero
(0.661381589s total). After removing the mutation, all nine transfer tests passed
(3.755s tests / 29.937279188s total), and the older-Core wire witness passed again
(0.373940688s operation / 0.934879895s total). Strict Clippy (0.171949206s) and
Rustfmt check (2.023386314s) passed on the restored candidate.

### T18: Reject an alias-blind importer before ownership commits

One opt-in regression launches an explicitly supplied actual legacy Core to
compare its advertised version/protocol with the candidate, then attempts a
real handoff of a transferred shell tab without optional version/protocol
expectations. The alias-restoration error is correlated and reports no success.
Both public socket inodes, the original Core process and boot, TerminalId,
shell PID/start/PTY/executable, inherited locator and shell-local state survive.
A newly attached native Client receives fresh shell output after reader rollback.
The existing handoff disconnects attached clients before preparation; uninterrupted
client connections are not claimed. The default harness still launches the
candidate; only this explicit opt-in supplies a legacy executable.

The baseline passed without production edits (1/1 explicitly requested ignored
test, 0.970s tests / 1.884263396s total). Initial fixture failures were corrected
by focusing the source before transfer and reattaching after the intentional
handoff client shutdown; neither was a production RED.

The required owned mutation passed zero expected aliases into the restored
receipt check. The permanent regression failed on unexpected success instead of
`handoff_failed` (0.951s tests / 27.231443836s total), and the independent genuine
OMP witness failed on the same unexpected successful handoff (7.614214845s total).
After removing that mutation, all ten transfer tests passed, including explicitly
running this ignored regression (3.720s tests / 29.558430473s total). Strict Clippy
(0.459107303s) and Rustfmt check (2.043529914s) passed.

The restored independent witness compared actual source/importer version and
protocol, retained the public socket inode and original shell/OMP process,
PTY, executable, session and inherited alias, and completed all five genuine
Working-to-Done turns, including fresh turns after rejection (8.840292440s total).
All owned processes were cleaned up. The restored acknowledgment remains before
public socket removal and ownership commit; no production change beyond T09 was
needed. These are isolated proofs, not changes to human sessions or installations.

### T19: Keep alias-free legacy handoff compatible

One opt-in regression launches the explicitly supplied actual legacy Core with
an unmoved running tab and no carried public aliases. Its existing manifest and
plain restored acknowledgment complete a real handoff to the candidate. The
correlated request succeeds and the legacy Core exits successfully; a fresh
native Client observes the candidate's new boot and the same focused pane.
The original shell PID/start/PTY/executable, shell-local state and inherited
locator survive. Actual inherited CLI targeting resolves to the imported
runtime's current TerminalId, and fresh native output proves that runtime is
usable. Existing handoff may allocate a new logical TerminalId and disconnect
old Client connections; neither is treated as OS process loss.

The baseline passed without production edits (1/1 explicitly requested ignored
test, 0.898s tests / 1.935782887s total). The required owned mutation changed
only the importer's zero-alias receipt from `restored` to `restored-aliases 0`.
The permanent regression failed with the old source's `handoff_failed`, because
the importer did not report restored runtimes in its expected format
(0.557s tests / 26.520184370s total). The genuine legacy-source OMP witness failed
for the same reason (7.371979041s total), and all owned processes were cleaned up.

After removing the mutation, all eleven transfer tests passed, including both
explicitly requested ignored mixed-version regressions (3.179s tests /
29.152138580s total). Strict Clippy (0.322770987s) and Rustfmt check (2.096111877s)
passed. The independent actual legacy-to-candidate witness completed all five
genuine OMP Working-to-Done turns, including fresh turns after import
(9.035638217s total), preserving the original shell/OMP processes, PTYs and
session and cleaning up all owned processes. The plain receipt remains necessary
for an alias-free legacy manifest; no production change beyond T09 was needed.
