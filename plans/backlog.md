# Backlog

Current product: **Trailboss**. Dated revision results and literal screenshot labels below are historical pre-rename evidence; they do not identify newly released Trailboss artifacts.

## Space-header connecting-line contrast

Requested 2026-10-12; initially deferred, then authorized and implemented with revision 15.

- Increase the contrast of the horizontal line connecting each space name to its status dots.
- Keep the line dimmer than the space name, but clearly visible with terminal background transparency enabled.
- Preserve the accepted sectioned-flat sidebar layout, status dots and selection styling.
- Verify against Tommy's transparent Ghostty setup when this is included in a later UI change.

Evidence: the user-provided screenshot showed the lines beside `herdr` and `Infilla` nearly disappearing through the transparent background. Revision 15 increases only the section rule/leader contrast; native PTY and palette hierarchy checks passed. The user's physical transparent Ghostty setup was not exercised.

## Distinct status dots at space level

Requested 2026-10-12; implemented in revision 15.

- Show one space-header dot per distinct status represented by its tabs, rather than one dot for every tab.
- Repeated statuses must not consume the header width or unnecessarily truncate the space name.
- Preserve individual tab status indicators and the existing status meanings.

## OMP modified-key forwarding

Reported 2026-10-12; implemented in revision 15 through the original Herdr-derived native semantic input path, not isolated shortcut patches.

- Ctrl+Left/Right word navigation and Alt+Backspace word deletion fail inside OMP under the fork.
- Audit modifier encoding and key forwarding across terminal → fork client → pane PTY → OMP, including other modified keys.
- The fork's manual key encoder dropped modifiers; keys now use native semantic pane events and existing Core encoding.
- Verify these examples and the broader affected key set when addressed.

Tommy authorized all three items. Revision 15 implements them together with the urgent mouse-wheel fix and the one-cell ellipsis margin below. See docs/spike-validation.md for exercised checks and limits.

## Ellipsis boundary margin

Requested 2026-10-12; implemented in revision 15.

- Leave one terminal cell between every ellipsis and its region boundary, including more-menu controls and truncation/status-overflow ellipses.
- Keep the displayed controls and their hit geometry aligned at session, space and tab levels.
- Do not draw outside a region when it is too narrow to fit the ellipsis and margin.

## Delivered revision28 — 2026-10-13

- Command-palette text entry has a highlighted field, typing prompt, insertion marker and action-specific footer. Native Rename tab Save/Cancel passed.
- Backlog item 1: double-click a pane word to select and automatically copy it. Single clicks do not copy; existing drag behavior is retained.
- Backlog item 5: space/tab ordering is shared Core state, reflected in two attached clients and retained on detach/reattach to the surviving session. Restart persistence remains unspecified.

Canonical requirement status: Kizn Item 01a13172-ea3c-7618-8541-3902dbb71375. Runtime evidence and side-by-side delivery paths: docs/spike-validation.md. No human Core/PTYS or default launcher was changed.

## Action-trigger architecture review

Requested 2026-10-13. Review how actions are triggered and dispatched, and whether the boundaries fit the intended plugin-driven design. Keep this distinct from shortcut definition and full UI pluginification. Review is pending; no replacement design or rewrite is authorized. Canonical tracking: Kizn backlog Item 01a13172-ea3c-7618-8541-3902dbb71375.

## Metrics collection exploration

Requested 2026-10-13. Explore useful metrics to collect and present. Active-agent count is the initial example; other metrics, the meaning of active, collection scope, retention and presentation remain undecided. This is a backlog idea, not authorization to implement telemetry now. Canonical tracking: Kizn backlog Item 01a13172-ea3c-7618-8541-3902dbb71375.
