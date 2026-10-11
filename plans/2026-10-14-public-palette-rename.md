# Feature: Public command palette and reusable rename form

**Created**: 2026-10-14
**Goal**: Discover commands through a plugin-declared native palette and rename spaces or tabs through one consistent, independently reusable form.
> Source PRD: docs/palette-rename-source.md (accepted interview source plus identified task-local provisions; no standalone PRD)
> PRD coverage: plans/public-palette-rename.prd-coverage.md
> Context: [domain model](../CONTEXT.md), [context map](../CONTEXT-MAP.md), [public palette/form ADR](../docs/adr/0006-public-palette-and-reusable-rename.md), [renderer-neutral contracts](../docs/adr/0005-renderer-neutral-client-contracts.md)

Prototype reference: [variant B fixture](../plugins/agent-tree/prototype/palette-rename.prototype.html), http://192.168.1.133:8892/?variant=b. No fig2json design exists; the HTML is a comparison reference, not a production renderer. This is one complete bounded feature plan, not phase one of all-TUI pluginification. All scenarios below are unbuilt.

## Slice 1: Public native palette and authoritative command invocation

**Tracer bullet**: Replace palette text and entries from an ordinary producer, then invoke an existing command in the actual native client.
**Touches**: public widget declarations and validators; existing fragment resolution; native palette; existing Core/producer invocation routes
**Off-limits**: rename workflows, general keybinding redesign, themes, terminal/PTY mechanics

### Task 1.1: Public palette presentation
<!-- estimate: ~430 reviewable lines | feature-files: features/command-palette.feature | demoable: yes -->

<!-- TODO -->
# Living: features/command-palette.feature::Replace palette presentation through a public producer
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Replace palette presentation through a public producer
  Given an isolated native client has stock-ui, agent-tree and sidebar-menus enabled
  And a non-stock producer declares the palette title "Actions", prompt "Find action", help "Enter to run", empty text "Nothing matches", and an entry "Inspect Atlas"
  When I open the palette and search for "Inspect"
  Then the native palette displays the declared title, prompt, help and entry
  When I search for "no-such-entry"
  Then the native palette displays "Nothing matches"
  When that producer changes its title to "Workspace actions" through the same public interface
  Then the open palette displays "Workspace actions" without rebuilding or restarting the client

### Task 1.2: Authoritative command dispatch
<!-- estimate: ~390 reviewable lines | feature-files: features/command-palette.feature | demoable: yes -->

<!-- TODO -->
# Living: features/command-palette.feature::Invoke one authoritative action from a selected command
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Invoke one authoritative action from a selected command
  Given the palette displays a command owned by <owner> for tab "orbit-build"
  And another provider has a similarly labelled command
  And tab "atlas-build" is focused
  When I select the command and press Enter once
  Then only <owner> receives one invocation for "orbit-build"
  And the native UI displays the outcome of <effect>
  And the similarly labelled provider receives no invocation
  Examples:
    | owner | effect |
    | Core | focusing orbit-build through the existing focus action |
    | declaring producer | marking its Orbit command as completed |

## Slice 2: One rename form from palette and sidebar

**Tracer bullet**: Rename an explicit tab through the independent shared form, then extend to spaces and both entry points.
**Touches**: public form declarations and interaction controller; stock/sidebar-menu rename callers; native modal projection
**Off-limits**: new/create workflow redesign, non-rename confirmations, official web/GUI clients

### Task 2.1: Shared form handoff and rename matrix
<!-- estimate: ~480 reviewable lines | feature-files: features/rename-form.feature | demoable: yes -->

<!-- TODO -->
# Living: features/rename-form.feature::Rename both entity kinds from each existing entry point
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Rename both entity kinds from each existing entry point
  Given "Atlas" contains tab "Build" and "Orbit" contains a different tab "Build"
  And "atlas-build" is focused
  When I invoke Rename <kind> for <target> from <entry>
  Then one reusable rename form opens with the current name selected for editing
  And the form displays <target> as its explicit rename target
  And the palette is closed
  When I enter <name> and save
  Then the session reports <target> renamed to <name>
  And the form closes
  And the other workspace and tabs retain their labels
  And the focused tab remains "atlas-build"
  Examples:
    | kind | target | entry | name |
    | space | space-orbit | palette | Orbital |
    | space | space-orbit | sidebar menu | Orbital |
    | tab | orbit-build | palette | Release |
    | tab | orbit-build | sidebar menu | Release |

### Task 2.2: Cancellation and original focus
<!-- estimate: ~350 reviewable lines | feature-files: features/rename-form.feature | demoable: yes -->

<!-- TODO -->
# Living: features/rename-form.feature::Cancel rename without a mutation or palette resurrection
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Cancel rename without a mutation or palette resurrection
  Given I invoked Rename <kind> from <entry> while control "origin" held focus
  And I replaced the form name with "Unsubmitted draft"
  And no save has been sent
  When I press Escape
  Then the rename form is closed
  And the palette is closed
  And control "origin" has focus
  And the session label is unchanged
  When I invoke the same rename again
  Then its field contains the current session name rather than "Unsubmitted draft"
  Examples:
    | kind | entry |
    | space | palette |
    | space | sidebar menu |
    | tab | palette |
    | tab | sidebar menu |

## Slice 3: Target-safe and invocation-safe rename lifecycle

**Tracer bullet**: Keep the original rename target through a focus change and expose a known failure without losing the draft.
**Touches**: form instance state; authoritative rename results; existing provider invalidation; input/pending state
**Off-limits**: general command-state registry, mutation auto-retries, distributed rollback, unrelated plugin lifecycle rewrites

### Task 3.1: Stable targets and visible failures
<!-- estimate: ~480 reviewable lines | feature-files: features/rename-form.feature | demoable: yes -->

<!-- TODO -->
# Living: features/rename-form.feature::Keep rename target stable when another client changes focus
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Keep rename target stable when another client changes focus
  Given a rename form is open for <kind> <target>
  And "atlas-build" was selected when the form opened
  When another attached client focuses "notes-reading"
  And I save "Release" in the open form
  Then only <target> receives the new name "Release"
  And "notes-reading" retains its name
  Examples:
    | kind | target |
    | space | space-orbit |
    | tab | orbit-build |

<!-- TODO -->
# Living: features/rename-form.feature::Report a removed target without renaming a replacement
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Report a removed target without renaming a replacement
  Given a rename form holds draft "Release" for <kind> <target>
  When another client removes <target>
  And I attempt to save
  Then the form reports that its original target is unavailable
  And no remaining entity is renamed
  And the draft "Release" remains available until I cancel
  Examples:
    | kind | target |
    | space | space-orbit |
    | tab | orbit-build |

<!-- TODO -->
# Living: features/rename-form.feature::Keep a failed rename draft available for explicit retry
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Keep a failed rename draft available for explicit retry
  Given the form holds "Release" for tab "orbit-build"
  When the authoritative rename returns an error
  Then the form remains open with "Release"
  And it displays the returned failure without claiming success
  And no automatic retry is sent
  When I explicitly save again and the authoritative rename succeeds
  Then the session label becomes "Release" and the form closes

### Task 3.2: Validation and pending submissions
<!-- estimate: ~440 reviewable lines | feature-files: features/rename-form.feature | demoable: yes -->

<!-- TODO -->
# Living: features/rename-form.feature::Reject an empty name without sending a rename
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Reject an empty name without sending a rename
  Given a rename form is open for tab "orbit-build"
  When I replace its field with <draft>
  And I press Enter
  Then no rename request is sent
  And the form remains available for editing
  And the Save control is disabled
  Examples:
    | draft |
    | empty string |
    | three spaces |

<!-- TODO -->
# Living: features/rename-form.feature::Submit once while a rename is pending
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Submit once while a rename is pending
  Given "orbit-build" has an open form with draft "Release"
  And the authoritative rename response is delayed
  When I press Enter twice and click Save once before the response
  Then exactly one rename invocation is sent for "orbit-build"
  And the form visibly reports that the save is pending
  When the successful response arrives
  Then the form closes and "orbit-build" is named "Release"

<!-- TODO -->
# Living: features/rename-form.feature::Dismiss pending UI without pretending to cancel a sent mutation
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Dismiss pending UI without pretending to cancel a sent mutation
  Given a rename of "orbit-build" to "Release" has been sent and is pending
  When I press Escape
  Then the form closes and the original invocation focus is restored
  And the UI does not claim the sent rename was cancelled or rolled back
  When the authoritative success arrives
  Then "orbit-build" becomes "Release"
  And neither the old form nor the palette reopens

### Task 3.3: Interaction and provider lifetime
<!-- estimate: ~500 reviewable lines | feature-files: features/rename-form.feature, features/plugin-actions.feature | demoable: yes -->

<!-- TODO -->
# Living: features/rename-form.feature::Ignore a stale form result after a new invocation opens
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Ignore a stale form result after a new invocation opens
  Given a pending rename form for "orbit-build" was dismissed
  And I opened a new form for "notes-drafts" with draft "Today"
  When the old rename completes
  Then the new form still targets "notes-drafts" with draft "Today"
  And its pending/error state is unchanged by the old result
  And authoritative session updates remain visible independently

<!-- TODO -->
# Living: features/plugin-actions.feature::Invalidate removed or replaced provider interactions
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Invalidate removed or replaced provider interactions
  Given producer instance "provider-old" supplied <interaction>
  When that same instance publishes an updated declaration
  Then its updated presentation applies and the interaction remains usable
  When that instance is <change>
  Then its interaction closes and restores valid focus
  And the stale interaction cannot submit
  When a new instance reuses the same textual plugin ID
  And a queued action for the old interaction is delivered
  Then no old action is invoked on the replacement
  And no old completion changes a new interaction
  Examples:
    | interaction | change |
    | an open rename form | unloaded |
    | an open rename form | replaced |
    | an open palette with a selected producer-owned action | unloaded |
    | an open palette with a selected producer-owned action | replaced |

## Slice 4: Public replacement and native stock cutover proof

**Tracer bullet**: Use an ordinary replacement producer for the same form from both entry points, then prove stock commands and narrow keyboard interaction.
**Touches**: bundled declarations, public schemas/contracts, native integration verification and affected specs
**Off-limits**: other TUI pluginification, web/GUI implementation, r31 Surface/ANSI/PTY rendering

### Task 4.1: Public form replacement and clean stock migration
<!-- estimate: ~380 reviewable lines | feature-files: features/rename-form.feature | demoable: yes -->

<!-- TODO -->
# Living: features/rename-form.feature::Replace rename presentation without stock privileges
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Replace rename presentation without stock privileges
  Given a non-stock producer supplies the reusable form with title "Change name" and Save label "Apply" through the public declaration interface
  When I invoke space rename from the palette
  Then the native form displays "Change name" and "Apply"
  When I cancel and invoke tab rename from the sidebar menu
  Then the same public form declaration supplies "Change name" and "Apply"
  And saving renames only the explicit tab through Core's existing operation
  And no stock producer identity is required to render or submit it

### Task 4.2: Native keyboard fit
<!-- estimate: ~260 reviewable lines | feature-files: features/rename-form.feature | demoable: yes -->

<!-- TODO -->
# Living: features/rename-form.feature::Keep rename keyboard-operable in narrow and wide native terminals
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Keep rename keyboard-operable in narrow and wide native terminals
  Given an isolated native session has three workspaces, six tabs and fifteen populated panes
  And its client is <columns> columns wide and 32 rows high
  When I open the palette, search for Rename tab, choose "orbit-build", and edit its name by keyboard
  Then the form is separate from the sidebar rows
  And its name field, feedback and save/cancel affordances are reachable without horizontal overflow
  And Tab and Shift-Tab stay within the open interaction
  When I save "Release" by keyboard
  Then the session reports "orbit-build" renamed to "Release"
  Examples:
    | columns |
    | 60 |
    | 120 |

### Task 4.3: Retained command workflows and cancellation
<!-- estimate: ~300 reviewable lines | feature-files: features/command-palette.feature | demoable: yes -->

<!-- TODO -->
# Living: features/command-palette.feature::Preserve the existing non-rename command paths
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Preserve the existing non-rename command paths
  Given the existing stock composition, agent-tree sidebar and sidebar-menus declarations are enabled in an isolated session
  When I select <command> through the public palette and complete <interaction>
  Then <outcome> occurs through its existing authoritative action route
  And the sidebar hierarchy and menu actions remain available in the existing composition
  Examples:
    | command | interaction | outcome |
    | New space | its name prompt with "Planning" and submit | one space named "Planning" is created |
    | New tab | its name prompt with "Work" and submit | one tab named "Work" is created in its selected workspace |
    | New space | its name prompt with "Unsubmitted" and cancel | no space is created |
    | New tab | its name prompt with "Unsubmitted" and cancel | no tab is created |
    | Switch tab | its target selection | its explicit tab is focused |
    | Previous tab | its existing invocation | the previous tab is focused |
    | Close tab | confirmation for its captured tab and confirm | only its captured tab is removed |
    | Close tab | confirmation for its captured tab and cancel | no tab is removed |
    | Close space | confirmation for its captured space and confirm | only its captured space is removed |
    | Close space | confirmation for its captured space and cancel | no space is removed |
    | Expand space | its existing invocation | its sidebar children become visible |
    | Collapse space | its existing invocation | its sidebar children become hidden |
    | Detach from session | its existing invocation | this client detaches while the session survives |

## Estimate and verification notes

Task metadata was derived by grouping whole authored scenarios in dependency order; no scenario was split. Estimates use the existing palette/menu tree, input controller, Core action routing and stock declaration shapes inspected during planning. Generated schema output and lockfiles are excluded. All estimates are low-confidence until the public form invocation seam is implemented; no task exceeds the ~500-line soft cap as estimated. If an atomic scenario exceeds that budget during build, flag it rather than splitting the behavior or hiding its breadth.

Build smoke: run the real native client against an isolated Core, with an ordinary replacement producer and the populated fixture; use public invocation/result traffic and observed native UI as evidence. Preserve Tommy's sessions. Existing schema/contract tests affected by the change must be migrated without repinning incidental wording. No test run is claimed by this plan. Follow ADR 0006 for clean cutover and unchanged new/create workflows.

Plan-time source/row/scenario checks are separate from delivery fidelity. The fidelity audit must not stamp these rows verified until the corresponding living specs have been built; never pre-create feature files to satisfy that gate.
