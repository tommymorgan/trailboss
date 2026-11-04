# Recently closed tabs

## Authority and goal

Tommy approved this feature on November 3, 2026 and its documented recovery support boundary on November 4, 2026. Accepted decisions are Kizn Item 01a1a410-5ef1-7714-906a-75b4054c5ee0 revision 4. Recover a closed tab's working context without reconstructing it manually. A tab is the terminal layout defined in [the domain model](../CONTEXT.md); a history entry is a completed closure, not a hidden live tab.

## Approved requirements

- R1: "Keep the 50 most recently closed tabs per session, newest first, across Trailboss restarts."
- R2: "Add Recently closed tabs to the session menu and command palette, using the existing searchable picker."
- R3: "Restore splits, names, working directories, and saved agent conversations into the original space; recreate the space if it’s gone."
- R4: "Keep failed restores in history with a clear error."
- R5: "Prevent duplicate agent conversations from running; remove an entry only after successful restoration."
- R6: "Verify through an isolated Core, real PTYs, and the native picker—not your running tabs."
- R7: "This requires persisted history records and additive tab.closed.list / tab.closed.restore API methods."
- R8: "history from installation onward is enough"
- R9: "Recreate the tab with its layout, working directories, and saved agent conversation. Closing still ends its processes."

These are direct-request acceptance criteria, not a formal external PRD. The initial request was: "I want a way to view recently closed tabs and restore them. I want to plan and build that now."

## Implementation decisions

Core owns the per-session history and opaque entry IDs. Capture only structural and supported agent-resume metadata before removing a tab. Do not retain terminal output, typed input, or arbitrary foreground launch commands. Record actual whole-tab removals, including workspace closure and final-pane removal; moves/transfers, Core shutdown/handoff, and rollback of a newly created failed layout are not tab closures.

Persist bounded entries through the existing atomic session-snapshot pipeline, including its raw/legacy migration and empty-session handling. Additive defaulted fields keep older snapshots readable. Handoff carries history with structural state; it does not log the handoff as closures. Old clients retain existing behavior.

List returns {closed_tabs:[{closed_tab_id,label,workspace_id,workspace_label,closed_at,pane_count}]}, newest first. closed_at is Unix milliseconds. Restore accepts {closed_tab_id}; successful response includes workspace_id and tab_id. Missing/stale IDs and unavailable prerequisites are structured errors, not silent fallback to an unrelated tab. Only consume the record after successful reconstruction; a repeated/stale invocation must not launch another writer.

Reuse persistence layout reconstruction with fresh pane/runtime/public identities, not resurrected aliases. Reuse the existing workspace if its original identity exists; otherwise recreate an ordinary space with its saved name and valid working directory. Never recreate a deleted Git worktree as a side effect. Validate all required directories and supported agent-session prerequisites before changing live state. Failed setup cleans up only newly owned resources and retains the history entry.

Use validated, agent-specific saved session references for conversation resume. Do not execute arbitrary saved launch_argv or reporter-supplied shell commands. Refuse a resume if that same conversation is already running or pending in this Core. Existing session-restore configuration still governs whether agents auto-resume; restoration must report any missing/unavailable supported conversation rather than claim success.

The UI reuses existing public command/widget declarations and searchable nested selection. Each entry shows its saved tab name, original space and close time. Escape/dismissal does not mutate history; empty history is explicit. No new visual framework or default keyboard binding is introduced.

## Verification and limits

Approved seams are an isolated owned Core's public socket API, real PTYs and the actual native picker. Exercise replacement process identities, resumed conversation identity, durable list/restore after restart and handoff, stale concurrent selections, missing prerequisites, 50-entry eviction, close-path coverage and exclusion of transfers/rollback. No live user tab input, real credential entry, or user-agent restart is authorized by testing.

There is no pre-install backfill, arbitrary job replay, new transport, dependency, schema-version cutover or security-boundary change. Existing renderer defects are outside scope. Source publication and live deployment mode are separate from this build approval.

## Native conversation availability checkpoint

Tommy approved this matrix on November 4, 2026: "finish the recently closed tabs build with the documented support matrix, leaving its listed gaps as explicit restore errors." This is the accepted recovery boundary for this build; it does not promise successful restore for every agent accepted by the general resume planner.

### Approved deviation for R3 and R9

The original requirements promise saved-conversation recovery without enumerating native-store limitations. This build restores the implemented paths below and returns explicit errors for the listed gaps, retaining the entry and all existing live tabs. Tommy accepted that narrower boundary so the build can finish without new dependencies, backend access or unsafe native-store queries. The original R3/R9 wording above remains unchanged.

The owned native-runtime proof covers OMP conversation recovery and a missing Codex conversation. Source-backed private-store fixtures additionally exercise exact native identity and storage selection for Claude, Copilot, Kimi, Droid, Qwen, local Letta and Grok. Those fixtures are not claims of native CLI execution or provider-backed recovery.

| Recovery path | Current constraint |
|---|---|
| OMP / Pi | Native JSONL header identity and canonical transcript path; live and pending writers are rejected. OMP has an exercised native recovery proof. |
| Codex / Claude | Native transcript identity is checked; Claude metadata parsing does not impose a prompt-size cap. Successful native Codex/Claude recovery has not been exercised. |
| Copilot / Kimi / Droid | Local readers follow official published event/storage contracts, including Kimi 2.4.1 filesystem discovery and Droid SDK 0.10.0 legacy/project stores. Native CLIs were unavailable for runtime proof. |
| Qwen | Exact JSONL identity and deterministic JSONC runtime-root precedence are implemented. Dotenv bootstrap, interpolation, legacy setting migration and trust-dependent workspace roots still require native resolution. |
| Grok | Exact URL-encoded cwd bucket and summary identity are implemented. Encoded cwd names over 255 bytes require native BLAKE3 encoding, which is not an approved dependency. |
| Letta | Source-backed local backend, ordinary/default conversation keys and transcript manifests are implemented. Cloud recovery needs an authenticated lookup; original explicit backend flags are not persisted. |
| Hermes / Kilo / Mastra Code / OpenCode | A suitable read-only store reader or native lookup contract is still required. SQLite support is not an approved dependency; Mastra can also select remote storage. |
| Devin / QoderCLI / Cursor / Antigravity CLI | An authoritative arbitrary-ID lookup or compatible native store contract remains unresolved. Existing picker/last-session caches do not prove arbitrary saved-ID availability. |

Do not substitute a CLI command solely because its output is JSON. Current [Kilo database initialization](https://github.com/Kilo-Org/kilocode/blob/080ca6c41399e014cfcc8657202863bb8e3ab6d6/packages/core/src/database/database.ts) enables WAL and applies migrations before executing a query. Current [OpenCode session listing](https://github.com/anomalyco/opencode/blob/59e95cc5394b7714451608099ef4db7bffc904b6/packages/cli/src/commands/handlers/session/list.ts) uses a managed server and limits root-session results. Neither is an exhaustive, side-effect-free local lookup merely because the requested operation is a list or `SELECT`.

Missing prerequisites and the approved recovery gaps produce explicit restore errors and retain history. That behavior satisfies the accepted R3/R9 deviation, not the original unrestricted conversation-recovery promise. No new dependencies, native-store migrations, backend calls or service launches are authorized by this approval. The completion boundary remains a verified local build; publication and installation require separate authorization.
