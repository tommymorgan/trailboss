# Running palette commands by name

## Problem and goal

Tommy selects built-in and extension commands by visible name in the native palette. Agents cannot use that same surface: `command.invoke` handles custom keybindings, the API catalog omits Move to session while retaining client-local commands, and plugin CLI invocation lacks palette arguments/context.

**Goal (verbatim):** An agent running in a Trailboss pane can discover and run the same named commands Tommy sees in the palette, with the same meaning and the same context the palette would supply if opened on that pane, without redesigning existing commands.

## Accepted decisions and provenance

Source: **Tommy, 2026-11-09 grilling session**, with accepted Q12 follow-up on the same date. The confirmed interview is complete; this repository document preserves its original eleven decisions verbatim as R1–R11 and the accepted extension as R12 (number = requirement ID). No separate PRD exists. The source interview snapshot is `local://command-run-decisions.md` in the planning conversation; this checked-in record is the durable requirements source for this plan.

1. Agents run the same named palette commands, built-in and extension (plugin) commands.
2. Picker commands take their choice as an argument, validated like the picker (same destinations, same errors).
3. One CLI entry: `trailboss command run "<title>" [argument]`.
4. Resolve by visible title or stable ID; ambiguous title fails and lists matches.
5. Context = caller's own pane's space/tab/pane, exactly what the palette sends when opened on it. Outside a Trailboss pane, commands needing that context refuse with a clear error ("no Trailboss pane context; run from a Trailboss pane"); context-free commands (e.g. New space) still run. No per-command flags; existing commands unchanged.
6. API callers neither see nor run window-only commands (Detach, Switch session, Previous tab). The native window keeps them.
7. Browser bridge (herdr-web) and Repeater/phone app excluded for now ("Maybe later, but not yet").
8. Argument = text after the title exactly as typed in the palette box; for pickers, the choice's visible label (e.g. "beta / Work"); no/multiple label match fails and lists valid choices.
9. New `trailboss command list` shows runnable titles with argument hints.
10. No confirmation prompt (e.g. Close tab) when run by name.
11. `command run` returns once started, printing result or plugin log ID, like the palette; no waiting for background completion.
12. Answers in the order the palette asks; each answer decides the next question; a missing answer fails and shows the next question; an extra answer fails.

**R12 provenance:** Tommy, 2026-11-09, Q12 follow-up to the grilling session. Tommy answered **"i accept"** to the verbatim rule above. R12 extends R8 and generalizes R3's optional argument to positional answers: `command run "<title>" answer1 answer2 …`. Each answer is one shell-decoded string; branching follows the palette. Missing answers fail showing the next prompt and its valid choices; extra answers fail. `command list` shows only the first prompt's hint. The original R1–R11 quotations remain unchanged as interview provenance; R12 governs their extended argument contract.

## Local built-in command interface

Task 1.1 implements `command list` and `command run TITLE-OR-ID [ANSWER ...]` for New space, New tab or space, New tab, Rename space, Rename tab, Close tab, Focus tab and Restart idle agents. Names match exactly; Core IDs are the reserved `host:*` identities shown by `command list`. Each answer is one shell-decoded string, with whitespace preserved. Missing names show the next prompt, blank names fail, and surplus answers fail before dispatch.

Put launch options such as `--session work` before `command`. After `command run`, the selector and ordered answers are command text, not launch options: `trailboss --session daily command run "New space" "--session=work"` creates that exact name in daily rather than selecting work. One `--` separator can precede the selector or appear before an answer; the separator is not an answer. After it, another `--` is literal text, so `command run "New space" -- --` creates a space named `--`. Explicit empty answers remain distinct from omitted answers.

The CLI reads only `HERDR_PANE_ID` for caller identity; Core resolves its current location, including retained aliases after transfers. Context-dependent commands never fall back to global focus. Outside a pane, New space and Restart idle agents remain runnable. Results use the existing API response envelope and exit status; no confirmation is added.

`ui.command.list` accepts optional `pane_id` and returns only currently runnable commands with first-answer hints. `ui.command.run` accepts `command`, ordered `answers` (default empty), and optional `pane_id`. Stable plugin IDs use `plugin-id/action-id`; plugin and picker execution adapters remain unavailable until their planned successor tasks. Window-only actions cannot run through this API. Native palette inventory instead travels with its owner-bound fragments in the existing paired `EndpointPluginUi.commands` projection, preserving native window and extension behavior. `command.invoke` remains the separate custom-keybinding route.

## Proposed interface

Task-local engineering proposal implementing the accepted decisions, not additional standing policy:

```text
trailboss command list
trailboss command run "New space" "Research"
trailboss command run "Rename tab" "Planning"
trailboss command run "omp: add-dir" "/home/tommy/src/trailboss"
trailboss command run "Move to session…" "beta / Work"
trailboss command run "Move to session…" "beta / Create space…" "Build"
trailboss command run "Recently closed tabs" "Planning · Work · 2026-11-09 14:00:00 UTC"
```

CLI grammar takes the title/ID as one positional value followed by zero or more positional answers, consumed in the order the palette asks. Each answer determines the next prompt; do not flatten them into one string. Preserve each shell-decoded answer byte-for-byte, including leading/trailing whitespace, embedded newlines and explicit empty string versus omission. Shell quotation syntax itself is not part of the palette text; do not reconstruct a raw tail by joining argv or interpreting it as shell code. A raw-tail plugin command consumes one answer as its existing argument string, not one answer per word. Missing required answers fail with the next prompt and valid choices; extra answers fail before dispatch. Add no per-command context flags. Help/completion/machine forwarding must recognize the same grammar rather than creating another invocation entry point.

Proposed API: `ui.command.run` with `command` (visible title or stable ID), `answers` (an ordered string array, default empty) and optional generic `pane_id`. The CLI gets the pane locator from `HERDR_PANE_ID` and the existing socket route from `HERDR_SOCKET_PATH`; API callers can convey the same generic pane context. Use `ui.command.list` as the shared catalog, with the same optional pane locator. List reports only commands executable by this caller and implementation head and shows only each command's first prompt hint; it does not enumerate hypothetical later branches. Missing-answer errors expose the actual next prompt and its currently valid choices.

Exact stable identity comes from the catalog's typed key, not its boot-scoped hashed fragment locator. Core entries use reserved action IDs; plugin entries use the existing qualified plugin/action identity. Exact stable IDs take precedence over visible titles, including a title that spells another command's ID. Otherwise titles match exactly, including punctuation and case. A title collision returns all matching titles plus stable IDs and runs nothing. Unknown identity and unsupported window actions fail without side effects. Display IDs and argument hints in list output so collisions and picker choices are actionable.

Successful run prints the existing operation result or plugin log ID and exits zero after dispatch/start acknowledgment. Execution and validation errors print the underlying actionable error and exit nonzero. Plugin completion is not awaited. Preserve transfer operation identity and uncertain-outcome errors; do not silently retry a potentially committed move. No confirmation is inserted before dispatch, including Close tab and plugin confirmation fragments.

## Architecture grounded in the source base

Research base: `master@origin 6e8c692b`, 2026-11-09. Paths below are current code evidence, not implementation claims.

### One catalog, surface-specific availability

`crates/trailboss-runtime/src/app/api/plugins/commands.rs:29–49,299–479` already derives `UiCommandInfo` and native declarations from `CORE_COMMANDS` plus installed available plugin actions. Extend that projection/resolver rather than building a second title registry. Retain plugin enabled/platform/diagnostic restrictions. Keep native declaration capability filtering independent from API run eligibility: API excludes Detach from session, Switch session and Previous tab but native windows retain them. Enable API Move to session once its complete picker adapter exists, independent of native-client capability bits.

`core_actions.rs:89–129` inventories all built-in commands. Include New space; New tab or space; New tab; Rename space; Rename tab; Close tab; Focus tab; Restart idle agents; Move to session…; Recently closed tabs. Submit, Cancel and restore-control IDs are interaction controls, not additional named palette commands. `host:focus` is shared by targeted Focus tab and client-owned Previous tab: resolve the typed command key/mode and exclude Previous tab without disabling Focus tab.

### Core owns current context

Resolve the inherited pane locator using `app/ids.rs:106–143`, including existing aliases, then derive its current containing tab/workspace and the pane's identity. Do not trust stale `HERDR_WORKSPACE_ID`/`HERDR_TAB_ID` after a move or derive targets from Core-global focus. Reject stale or unresolved locators rather than targeting a decoy pane. This follows the existing Core-owned identity contract, not a new identity service.

No pane context means no implicit active-tab fallback. Context-dependent built-ins and plugins refuse with `no Trailboss pane context; run from a Trailboss pane`. Context-free commands (New space, Restart idle agents, Recently closed tabs, and genuinely context-free plugin actions) remain available. Use existing action context declarations to distinguish context requirements; do not invent a per-command CLI flag system. Outside-pane plugin dispatch must not acquire Core focus through context merging. Selection is absent, not fabricated.

Native palette dispatch supplies only workspace/tab/pane identity to plugins (`src/lib.rs:657–681`). `app/api/plugins/context.rs:5–38` replaces context for `plugin_ui` but merges missing fields with global focus otherwise. Use the exact palette-shaped context path for named execution; missing context must stay missing. Do not change legacy `plugin.action.invoke` semantics for existing callers.

### Reuse existing effects and handlers

`src/core.rs:870–901,904–1073` separates named palette preparation from immediate action effects. `prepare_palette` requires a nonblank name for New space, New tab or space, New tab, Rename space and Rename tab; submit maps to workspace.create, tab.create, workspace.rename and tab.rename. Other actions map to tab.close, tab.focus and agent.restart_idle. Extract/reuse this named preparation, validation and operation mapping in a runtime-owned helper usable by native Core and the new API. Do not embed a window or simulate keystrokes in the server. New tab or space uses the caller's space, not another window's selection. Missing/blank names fail rather than silently selecting the immediate shortcuts' default labels. Existing direct host:new-space/host:new-tab shortcuts retain their defaults unchanged. Supplied text goes through the same named palette meaning, not a new shell argument parser.

`app/api/plugins/mod.rs:201–275` already validates raw-tail support, forwards actions and starts a plugin command returning log/context. Reuse this production dispatch with the resolved plugin key, exact argument option and caller context. Existing `arguments: None` versus `Some("")` semantics and `unsupported_plugin_arguments` errors remain intact. Native confirmation UI stays intact; named dispatch bypasses it as R10 requires.

`cli/spec.rs:8–48` owns CLI discoverability; extend it and the existing CLI dispatcher/socket request path together. Update public API schema generation/documentation through existing conventions during implementation. Do not overload legacy custom-keybinding `command.invoke` or change its callers.

### Picker adapters, not redesigned commands

Move to session uses `server/tab_transfer/discovery.rs:19–98` and native `src/tab_transfer.rs:188–208,303–380`. Reuse current compatible-running-destination discovery and `tab.transfer` validation. Factor the visible label formatter so native and API both display `session / space` identically. Match the complete visible label, reject zero/multiple matches and include every currently valid label. Keep destination disappearance/incompatibility/ownership errors from the existing transfer operation; never invent a destination or restart the caller's process.

Recently closed tabs uses `commands.rs:482–515`: exact labels are `tab · space · YYYY-MM-DD HH:MM:SS UTC` (or existing Unknown close time). Factor its formatter/choice data and call `tab.closed.restore` with the chosen closed-tab ID. Duplicate visible labels fail instead of picking the first. Empty history returns no choices and performs no restore.

**Resolved nested-input contract (R12):** when a compatible destination session has zero spaces, discovery emits `session / Create space…`. The current picker next requires a new space-name text box (`src/tab_transfer.rs:198–208,315–335`), and `tab.transfer` rejects a missing/blank destination name (`server/tab_transfer/coordinator.rs:248–252`). Consume the next positional answer as that name: `"beta / Work"` consumes one answer and runs; `"beta / Create space…"` consumes a second answer for the new space name. Validate the entire selected branch, including absence of surplus answers, before dispatching any operation. Missing second answer shows `New destination space name`; invalid/missing picker answers show the current valid labels. Extra answers fail without mutation. This preserves every existing destination without a default, exclusion or new flag. The same ordered-prompt rule applies to command interactions generally; it does not redesign existing plugin handlers or wait for their background completion.

## Non-goals

Browser bridge/herdr-web; Repeater/phone app; remote surfaces; waiting for plugin completion; selection-dependent behavior; redesigning existing commands; new permissions, identity services or confirmation prompts. This planning change authorizes no implementation, runtime activation, push or deployment. No Linear projection applies.

## Planning procedure provenance

Loaded through Kizn on 2026-11-09: Default planning and delivery workflow revision 10; Feature planning procedure revision 8; Plan and PRD coverage format revision 3; Planning authoring checklist revision 3; Sequentially shippable review units revision 3; Plan decomposition procedure revision 4; PRD fidelity audit and planning learning revision 3; Plan build procedure revision 5. IDs are pinned in the linked plan.

Plan: [2026-11-09-command-run.md](../plans/2026-11-09-command-run.md).
