# Public palette and one reusable rename interaction

Status: accepted planning decision, 2026-10-14. Not an implementation or deployment claim.

## Authority and scope

Tommy accepted a plugin-driven command palette and one reusable rename form for spaces and tabs, using public plugin interfaces. Other TUI pluginification, official web/GUI clients, and terminal rendering changes are outside this feature. Source: Kizn backlog `01a13172-ea3c-7618-8541-3902dbb71375`, revision 38, and [the bounded planning source](../palette-rename-source.md).

Tommy rejected prototype C (sidebar-inline editing), liked A and B equally, and delegated their implementation choice to maintainability. The agent selected B: a reusable form independent of the palette. Tommy then explicitly accepted Escape closing rename and restoring original focus without reopening the palette. These are different provenance: delegated architecture choice versus direct interaction approval.

## Decision

1. Retain existing widget composition, named fragments, menu contexts, action authorization and provider resolution. Extend the public declaration/interaction contract only where palette presentation and reusable form state require it. Do not create a parallel command registry, general workflow scripting engine or event-bus mutation route.
2. The palette module owns command discovery, search, selection and target navigation, not rename drafts or rename-specific submission branches. Its visible title, prompt, help, empty state and entry presentation come from public plugin declarations; the client retains generic filtering/navigation, input decoding, clipping and layout. Existing command declarations and existing authoritative action routes are reused. No fixed private stock provider is required.
3. The reusable form module owns one invocation's editable draft, validation, pending/error state and cancellation. Space and tab rename use the same interface with an explicit target kind and stable identity. Palette, sidebar and menu invocations hand off to that interface. The palette closes before the form opens; the form remains usable when no palette was opened.
4. The public form declaration supplies visible contents, field identity, initial name, submit/cancel actions and semantic feedback. The interaction controller owns editing, focus and the invocation lifecycle; producers declare data, not terminal cells. Bind form state to its invocation, not a process-global input string or current selection. Field validation is bounded to this name form; do not build a general validation language.
5. Core retains authoritative `workspace.rename` and `tab.rename` semantics and revalidates the captured target when submitting. Producer-owned actions retain their resolved supplying-plugin provenance. Do not broadcast mutations to every matching provider. Public declaration replacement must not change the authoritative owner or silently retarget an existing interaction.
6. Completion is correlated with the originating interaction and provider instance. Pending submission cannot be duplicated by repeated Enter. A known failure leaves the draft and error available for explicit retry; do not auto-retry an unknown-outcome mutation. Target removal is a visible failure, never a rename of the newly focused entity. Provider removal/replacement invalidates its open UI; stale completions cannot mutate a newer form's draft or state.
7. Escape before submission discards the draft, closes the form and restores the original invocation focus; it never reopens the palette. While a submission is pending, Escape dismisses local UI but does not claim to cancel or undo an already-sent mutation. A late result may update authoritative session state but cannot reopen the dismissed form. If the original focus target no longer exists, restore a valid existing focus without creating/focusing a replacement entity.
8. Renderer geometry remains client-owned and renderer-neutral declarations contain no ratatui types, escapes or fixed cell coordinates. The form must remain keyboard-operable in a narrow terminal; preserve visible field, feedback and save/cancel affordances through fitting/scrolling rather than moving editing into sidebar rows.

## Clean cutover

Migrate every palette/sidebar/menu space/tab rename caller to the reusable interaction. Remove obsolete rename-only palette variants, standalone rename variants, duplicate submission branches, stock-only rename conditions/form placement and old aliases once unused. Keep shared mechanisms still used by New space/New tab: this feature does not rewrite those workflows merely because they currently share `host:create` or input helpers. Update bundled producers, Rust/TypeScript declaration validators, negotiated/schema contracts and affected contract tests together; an unrecognized declaration must fail visibly rather than silently presenting a broken form. Do not alter frozen wire field layouts without the existing generation/capability rules in ADR 0005.

## Source evidence and verification boundaries

- `src/core.rs:7-27,281-297,350-370`: standalone and palette-specific rename variants and submission branches currently duplicate the operation.
- `plugins/sidebar-menus/menus.ts:84-176,179-252`: existing command and menu declarations are reusable inputs; preserve non-rename commands.
- `plugins/agent-tree/stock.ts:27-83`: existing conditional rename composition is to be migrated, not duplicated beside the new form.
- `crates/trailboss-runtime/src/protocol/widgets.rs:17-125`: public contexts/fragments/modal/input exist; the reusable invocation and feedback contract still needs implementation.
- Prototype: `plugins/agent-tree/prototype/palette-rename.prototype.html?variant=b`, served independently at LAN port 8892. Fixture-only browser checks cover keyboard, cancellation/focus, target isolation, failure retention and desktop/narrow layouts. They do not prove production plugin contracts or runtime implementation.

Build verification must exercise a non-stock producer replacing palette/form presentation through the public interface, both authoritative Core and producer-owned command invocation, all space/tab × palette/sidebar rename combinations, and actual native terminal behavior. Use isolated sessions; never replace, stop or send test input to Tommy's running applications. Keep r31 application-driven PTY resizing and native Surface/browser ANSI behavior unchanged.

## Alternatives

A can also be implemented with a reusable form; visual similarity is not evidence of bad architecture. B makes the ownership handoff explicit and avoids adding rename-specific state to the palette. C was explicitly rejected and is not an alternative to deliver. Keep the comparison artifact as design evidence, not a second production path.

## Number allocation

Local ADRs 0001–0005 existed. GitHub's latest default `master` recursive tree was checked on 2026-10-14 and contained no `docs/adr/` entries (`truncated: false`); 0006 is free in both that source and the local target directory. This JJ workspace currently has no configured remotes; direct GitHub metadata supplied the remote-default check. Recheck at publication time.
