# Palette and reusable rename — bounded planning source

Date: 2026-10-14. No separately authored PRD exists for this feature. This document is the local source for the accepted interview scope and the task-local implementation provisions below, not a replacement for the full application design. Kizn backlog `01a13172-ea3c-7618-8541-3902dbb71375`, revision 38, retains the decision provenance. [ADR 0006](adr/0006-public-palette-and-reusable-rename.md) owns the implementation design.

## Direct interview decisions

Tommy: “I don't like C at all.  A and B seem indistinguishable as a user and I like them both, so I think it's a matter of which makes for a cleaner, more maintainable implementation.” Agent choice under that delegation: B, independent reusable form ownership. A is not asserted inherently unmaintainable.

Tommy answered “yes” to: “For Esc, I recommend closing rename and restoring the original focus—not reopening the palette. Does that match what you want?” The preceding recommendation specified discarding the unsubmitted draft; it does not promise rollback of a request already sent.

Accepted scope: plugin-driven palette, one reusable space/tab rename form, public interfaces with no stock-only privileges; preserve existing declarations. Official web work and other TUI pluginification remain outside this feature. Native Rust remains the product. Prototype comparison: [HTML fixture](../plugins/agent-tree/prototype/palette-rename.prototype.html), variant B, served at http://192.168.1.133:8892/?variant=b. Its screenshots prove fixture interactions only.

## Atomic source criteria

R rows express accepted scope/interaction requirements. E rows are explicit agent-selected implementation provisions within that scope; they are not new claims about what Tommy personally requested. O rows retain explicit scope exclusions. Each numbered bullet and edge case below must have one ledger row. The ledger quotes this source verbatim; its extracted criteria do not claim to be verbatim user quotations.

- **R01**: A plugin-driven command palette uses public interfaces with no stock-only privileges.
  - Basis: Accepted planning scope, Kizn backlog revision 38.

- **E01**: Palette invocation preserves the selected action owner and explicit target rather than broadcasting or substituting current focus.
  - Basis: Task-local safety provision of ADR 0006; not a separately requested command-system rewrite.

- **R02**: One reusable rename form is used for both spaces and tabs, from the palette and sidebar/menu entry points.
  - Basis: Accepted scope and delegated B ownership choice, Kizn backlog revision 38.

- **R03a**: Escape closes the rename form.
  - Basis: Tommy directly accepted the recommendation with yes on 2026-10-14.

- **R03b**: Escape discards the unsubmitted draft.
  - Basis: Tommy directly accepted the recommendation with yes on 2026-10-14.

- **R03c**: Escape restores the original invocation focus.
  - Basis: Tommy directly accepted the recommendation with yes on 2026-10-14.

- **R03d**: Escape does not reopen the command palette.
  - Basis: Tommy directly accepted the recommendation with yes on 2026-10-14.

- **E02**: Changing selection after a rename starts must not change its captured target.
  - Basis: Task-local safety provision; stable target is part of the selected ownership design.

- **E03**: If the captured target disappears before submission, report failure without renaming any replacement target.
  - Basis: Task-local error provision of ADR 0006.

- **E04**: A known save failure keeps the form open with the entered name and an error, and does not automatically retry.
  - Basis: Task-local error handling matching the exercised B prototype.

- **E05**: An empty or whitespace-only name cannot submit a rename.
  - Basis: Task-local name-form validation matching the prototype; do not add unrelated name rules.

- **E06**: Repeated save input while a rename is pending must not duplicate the invocation.
  - Basis: Task-local invocation safety; no automatic mutation retries.

- **E07**: Escape may dismiss a pending form but does not promise to cancel an already-sent mutation; its completion must not reopen that form.
  - Basis: Task-local consequence of accepted Escape dismissal and Core-owned mutations; no new cancellation operation.

- **E08**: A completion from an old invocation cannot overwrite the state of a newer form.
  - Basis: Task-local lifecycle safety for reusable form instances.

- **E09**: Removing or replacing the supplying producer invalidates its open interaction; stale actions cannot silently reach its replacement.
  - Basis: Task-local public-provider lifecycle safety, not a new plugin lifecycle subsystem.

- **R04**: The reusable rename form must use public interfaces with no stock-only privileges.
  - Basis: Accepted public-plugin requirement, Kizn backlog revision 38.

- **E10**: The native palette and rename form remain usable by keyboard in populated narrow and wide layouts, without moving editing into sidebar rows.
  - Basis: Accepted prototype/narrow-layout scope; 60 and 120 columns are bounded build proof cases, not a promise for every terminal size.

- **E11**: Existing composition, sidebar and menu/form declarations are reused rather than rebuilding unrelated UI or breaking existing palette commands.
  - Basis: Accepted scope; existing declaration source inventory supplies this concrete regression matrix.

- **O01**: Other TUI surfaces remain outside this plan.
  - Basis: Explicit accepted scope or direct prototype rejection; ADR 0006.

- **O02**: The official web UI remains later work; HTML is the rapid-iteration prototype medium only, not the production implementation.
  - Basis: Explicit accepted scope or direct prototype rejection; ADR 0006.

- **O03**: Sidebar-inline rename (prototype C) is rejected.
  - Basis: Explicit accepted scope or direct prototype rejection; ADR 0006.

## Delivery boundary

This is planning only. All new living scenarios stay in the plan until their delivering task implements them. Preserve live sessions and r31 native Surface/browser ANSI rendering, application-driven shared PTY sizing, pane identity and process lifetimes. Do not conflate the independent companion web app with an official declaration renderer. No Linear projection, PR, commit, production deployment or human-session mutation is authorized by this plan.
