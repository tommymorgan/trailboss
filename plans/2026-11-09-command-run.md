# Feature: Run palette commands by name

**Created**: 2026-11-09
**Goal**: An agent running in a Trailboss pane can discover and run the same named commands Tommy sees in the palette, with the same meaning and the same context the palette would supply if opened on that pane, without redesigning existing commands.
> Plan home: repository
> Target repository: git@github.com:tommymorgan/trailboss.git
> Source PRD: accepted decisions R1–R12 and Goal in docs/command-run.md; Tommy, 2026-11-09 grilling session and accepted Q12 follow-up; no separate PRD
> PRD coverage: plans/command-run.prd-coverage.md
> Context: docs/command-run.md; CONTEXT.md
> Source base: master@origin 6e8c692b
> Execution base: master@origin `5056d54a`, followed by `transfer-helper-rollback-race`; the planning research base above is retained as provenance.
> Readiness: All 24 scenarios in Tasks 1.1, 2.1 and 3.1 implemented locally. Formal revision-17 review approved Tasks 2.1 and 3.1; the Task 1.1 argv finding and helper listener-ownership finding are repaired and await their independent re-review. Task bookmarks each contain one feature-level commit, as subsequently authorized. Exact-head boundary and final gates passed after restacking the repairs; no push, deployment, daily-runtime activation or implementation-fidelity verification is claimed.

## Slice 1: Run built-in commands from the caller's pane

**Tracer bullet**: Create a named space from the CLI without a native window or pane context.
**Touches**: shared Core descriptors/projection, runtime API schema/dispatch, native effect reuse, CLI parser/help/completion, pane lookup.
**Off-limits**: custom-keybinding semantics, new identity services, native window command behavior, plugin dispatch redesign, picker execution until Slice 3.

### Task 1.1: Deliver named built-in execution and contextual discovery
<!-- estimate: ~750–1100 reviewable lines, medium confidence from current projection/Core/CLI handlers | feature-files: features/command-run.feature | demoable: yes -->
> Depends on: none
> Shippable state: All non-picker built-in commands run by title/ID with caller context; window-only commands remain native-only. Plugin and picker rows are not advertised as runnable until their adapters exist. Existing native palette and custom-keybinding execution remain usable; no unusable new command is exposed.
> Boundary verification: At this exact task head without successors, create Research outside a pane; run the listed built-ins from an owned pane with decoy global focus; observe only caller targets through workspace/tab APIs. Check native-only actions remain native and fail through the API. Confirm unimplemented picker/plugin adapters are absent from runnable discovery.

<!-- DONE -->
# Living: features/command-run.feature::CreateSpaceByTitle
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: CreateSpaceByTitle
  Given a running owned Core with no active pane
  When I run trailboss command run "New space" "Research"
  Then the CLI exits zero with the workspace creation result
  And workspace.list contains the new space named Research
  And no native window or confirmation prompt is required
<!-- DONE -->
# Living: features/command-run.feature::RunCoreCommandsInCallerContext
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: RunCoreCommandsInCallerContext
  Given caller pane w1:p1 belongs to tab Planning in space Work
  And Core-global focus and a native window are on decoy tab Other in space Personal
  When the caller runs trailboss command run "<title>" <argument>
  Then <effect> occurs through the public API in the caller context
  And Other and Personal remain unchanged
  Examples:
    | title | argument | effect |
    | New tab or space | "Notes" | a Notes tab is created in Work |
    | New tab | "Notes" | a Notes tab is created in Work |
    | Rename space | "Build" | Work is renamed Build |
    | Rename tab | "Build" | Planning is renamed Build |
    | Focus tab | omitted | Planning becomes the focused tab |
    | Close tab | omitted | Planning is closed without confirmation |
<!-- DONE -->
# Living: features/command-run.feature::PreserveCoreInputMeaning
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: PreserveCoreInputMeaning
  Given caller pane w1:p1 belongs to Planning in Work
  When command run submits <title> with <argument>
  Then its result is <outcome>, matching the native palette's existing input semantics
  Examples:
    | title | argument | outcome |
    | New space | omitted | missing-answer error showing its name prompt and no space creation |
    | New tab | omitted | missing-answer error showing its name prompt and no tab creation |
    | Rename tab | omitted | missing-answer error showing its name prompt and no rename |
    | Rename space | whitespace only | invalid-name error and no rename |
    | Rename tab | "  Planning v2  " | rename uses exactly the palette-submitted name |

<!-- DONE -->
# Living: features/command-run.feature::RejectSurplusCoreAnswersBeforeDispatch
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: RejectSurplusCoreAnswersBeforeDispatch
  Given caller Planning's pane in Work and the command's native prompt sequence
  When command run receives <title> followed by <answers>
  Then it exits nonzero with an extra-answer error
  And no operation, name change, close or restart is started
  Examples:
    | title | answers |
    | New space | Research surplus |
    | Rename tab | Build surplus |
    | Close tab | surplus |
    | Restart idle agents | surplus |

<!-- DONE -->
# Living: features/command-run.feature::RestartIdleAgentsByName
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: RestartIdleAgentsByName
  Given a running owned Core with an eligible idle agent and a protected busy agent
  When command run "Restart idle agents" is called without pane context
  Then it returns the existing agent.restart_idle result
  And the existing restart eligibility rules preserve the busy agent
<!-- DONE -->
# Living: features/command-run.feature::ResolveExactTitlesAndStableIds
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: ResolveExactTitlesAndStableIds
  Given command list exposes New space and its stable ID host:new-space
  When command run uses <selector> with argument Research
  Then it starts exactly the New space operation and returns its result
  Examples:
    | selector |
    | New space |
    | host:new-space |
<!-- DONE -->
# Living: features/command-run.feature::RejectMissingOrAmbiguousCommands
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: RejectMissingOrAmbiguousCommands
  Given two enabled palette commands are titled Shared with different stable IDs
  When command run receives <selector>
  Then it exits nonzero and starts no operation
  And it prints <diagnostic>
  Examples:
    | selector | diagnostic |
    | Shared | ambiguous title with both titles and stable IDs |
    | does-not-exist | command not found |
<!-- DONE -->
# Living: features/command-run.feature::RequireRealCallerPane
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: RequireRealCallerPane
  Given Core-global focus is on decoy tab Other
  And the caller has <pane_context>
  When command run "Rename tab" "Build" is requested
  Then it refuses without changing Other or any tab
  And the missing context error is "no Trailboss pane context; run from a Trailboss pane" or the stale-locator error identifies the unavailable pane
  Examples:
    | pane_context |
    | no HERDR_PANE_ID |
    | an inherited locator no longer resolved by Core |
<!-- DONE -->
# Living: features/command-run.feature::ResolveMovedPaneRatherThanCachedLocation
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: ResolveMovedPaneRatherThanCachedLocation
  Given the same running shell's inherited HERDR_PANE_ID resolves through a Core alias to Planning in space Build
  And inherited HERDR_WORKSPACE_ID and HERDR_TAB_ID still name its old location
  When that shell runs command run "Rename tab" "Current"
  Then only its current containing tab is renamed Current
  And neither the old location nor the globally focused decoy is changed
<!-- DONE -->
# Living: features/command-run.feature::ExcludeWindowOnlyCommandsFromApi
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: ExcludeWindowOnlyCommandsFromApi
  Given a native window supports its existing <title> command
  When an API caller requests ui.command.list and then attempts ui.command.run by that command's title and stable key
  Then the command is absent from the API list and both run attempts fail without a window effect
  And the native window's command remains available with its original meaning
  Examples:
    | title |
    | Detach from session |
    | Switch session |
    | Previous tab |
<!-- DONE -->
# Living: features/command-run.feature::ListRunnableCoreCommandsAndHints
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: ListRunnableCoreCommandsAndHints
  Given caller pane Planning in Work and the complete non-picker Core adapter
  When trailboss command list is run
  Then it lists New space, New tab or space, New tab, Rename space, Rename tab, Close tab, Focus tab and Restart idle agents with stable IDs
  And name-taking commands display their argument hints
  And outside a pane the same list retains context-free commands and omits context-required commands
  And command run accepts no per-command workspace, tab or pane context flags
  And the legacy custom-keybinding command.invoke route retains its existing behavior

## Slice 2: Run installed extension commands with palette arguments

**Tracer bullet**: Run omp: clear by visible title against the caller's pane and immediately receive its plugin log ID.
**Touches**: shared catalog plugin rows, existing plugin.action.invoke dispatch/context/raw-tail handling, CLI result display.
**Off-limits**: plugin handler redesign, background completion polling, selection fabrication, OMP core changes, browser/phone surfaces.

### Task 2.1: Deliver extension title/ID invocation and exact arguments
<!-- estimate: ~300–500 reviewable lines, medium confidence from existing plugin invoke/context/raw-tail paths | feature-files: features/command-run.feature | demoable: yes -->
> Depends on: Task 1.1
> Shippable state: Enabled available extension commands join the runnable catalog and execute using the same context/arguments as the palette. Built-ins retain Task 1.1 behavior. Pickers remain unadvertised until Task 3.1; no handler-completion wait or new authority is introduced.
> Boundary verification: At this exact head without Task 3.1, run owned omp-shaped actions by title and qualified ID; inspect handler context and raw-tail bytes through their logs, retain a running handler while CLI exits, and prove the globally focused decoy is unchanged. Re-exercise New space through the same CLI route.

<!-- DONE -->
# Living: features/command-run.feature::RunExtensionByVisibleTitle
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: RunExtensionByVisibleTitle
  Given enabled plugin omp exposes the visible palette title "omp: clear" and a qualified stable action ID
  And caller pane Planning in Work differs from the global focused pane
  When trailboss command run "omp: clear" is requested
  Then the existing plugin action starts exactly once on the caller's workspace, tab and pane
  And the CLI prints its plugin log ID and exits without waiting for the action to finish
  And no on-screen selection is supplied
<!-- DONE -->
# Living: features/command-run.feature::PreserveRawPluginArgumentText
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: PreserveRawPluginArgumentText
  Given enabled plugin omp exposes "omp: add-dir" with raw-tail input
  When command run selects it by <selector_kind> with <argument>
  Then plugin.action.invoke receives that same arguments option without trimming, tokenizing or shell evaluation
  And the CLI prints the started plugin log ID
  Examples:
    | selector_kind | argument |
    | visible title | omitted (None) |
    | qualified stable ID | explicit empty string (Some empty) |
    | visible title | two leading spaces, /tmp/a b, two trailing spaces |
    | qualified stable ID | literal dollar sign, quotes and newline |
<!-- DONE -->
# Living: features/command-run.feature::RejectUnsupportedPluginExecution
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: RejectUnsupportedPluginExecution
  Given the current plugin catalog has <state>
  When command run attempts the affected title or stable ID
  Then it exits nonzero with the existing actionable error and no handler is started
  Examples:
    | state |
    | action not declared for the current platform |
    | disabled plugin |
    | invalid confirmation declaration that the native palette cannot execute |
    | supplied argument on an action that does not support raw-tail input |
    | two answers supplied to a raw-tail action whose palette accepts one text answer |
<!-- DONE -->
# Living: features/command-run.feature::PluginContextNeverFallsBackToGlobalFocus
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: PluginContextNeverFallsBackToGlobalFocus
  Given global focus is Other in Personal
  And a context-required plugin action and a context-free plugin action are installed
  When command run invokes the context-required action outside a pane
  Then it returns "no Trailboss pane context; run from a Trailboss pane" and starts nothing
  When command run invokes the context-free action outside a pane
  Then it starts with absent workspace, tab, pane and selection rather than Other's context
  And its plugin log ID is returned without background-completion waiting
<!-- DONE -->
# Living: features/command-run.feature::ListPluginsWithArgumentsAndBypassConfirmation
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: ListPluginsWithArgumentsAndBypassConfirmation
  Given enabled available plugins omp and pi expose palette commands and a confirmation-fragment action
  When command list is requested from Planning's pane
  Then every executable omp and pi command is discoverable by its native visible title and qualified stable ID
  And each raw-tail command retains its declared argument hint
  When command run selects the confirmation-fragment action
  Then it starts without requesting a confirmation and immediately returns its plugin log ID

## Slice 3: Supply picker choices by visible label

**Tracer bullet**: Move Planning to beta / Work through command run without opening a picker.
**Touches**: shared picker choice data/formatters, compatible destination discovery, tab.transfer, closed history and tab.closed.restore, API choices and CLI hints.
**Off-limits**: transfer protocol redesign, hidden retries, default destination invention, process restarts, new remote surfaces.

### Task 3.1: Deliver both complete picker adapters
<!-- estimate: ~750–1150 reviewable lines, medium confidence after accepted R12 resolves nested input; existing transfer discovery and restore handlers reused | feature-files: features/command-run.feature | demoable: yes -->
> Depends on: Tasks 1.1 and 2.1
> Shippable state: Complete same-label Move to session and Recently closed tabs adapters are exposed together, including ordered branching answers, missing/duplicate choices, current validation and errors. Empty destinations accept a second positional space-name answer. All answers are validated before dispatch; no selectable destination is silently dropped.
> Boundary verification: At this exact head, use two owned Cores; move the caller's existing tab to beta / Work and restore an exact closed-tab label, proving caller process continuity and same results as existing handlers. Reject missing, duplicate, stale and surplus choices without fallback. Move into empty beta with answers "beta / Create space…" and "Build"; omit the second answer to observe the name prompt without mutation.

<!-- DONE -->
# Living: features/command-run.feature::MoveTabToVisibleDestination
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: MoveTabToVisibleDestination
  Given caller Planning's live shell is in alpha and compatible beta contains space Work
  And command list exposes "Move to session…" with only its first choice-prompt hint
  When command run "Move to session…" "beta / Work" is requested
  Then the same tab is transferred through tab.transfer to beta's Work
  And the CLI prints the existing transfer result or start acknowledgment with its operation identity
  And the tab's shell and running agent are not restarted
  And no native picker window is opened
<!-- DONE -->
# Living: features/command-run.feature::RestoreClosedTabByVisibleLabel
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: RestoreClosedTabByVisibleLabel
  Given Recently closed tabs lists "Planning · Work · 2026-11-09 14:00:00 UTC"
  When command run "Recently closed tabs" uses that exact label outside a pane
  Then tab.closed.restore receives the ID of that choice
  And the CLI prints the existing restore result
  And the restored tab has the same meaning and destination as selecting that row in the native picker
<!-- DONE -->
# Living: features/command-run.feature::RejectInvalidPickerLabels
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: RejectInvalidPickerLabels
  Given <picker> has the same currently valid choices and visible labels as its native picker
  When command run receives <selection>
  Then it exits nonzero, lists all currently valid labels and changes no tab
  And a missing answer identifies this picker's current selection question alongside its valid labels
  Examples:
    | picker | selection |
    | Move to session… | omitted |
    | Move to session… | no matching label |
    | Move to session… | label matching two destination spaces |
    | Recently closed tabs | omitted |
    | Recently closed tabs | no matching label |
    | Recently closed tabs | label matching two history entries |
    | Recently closed tabs with empty history | any label |
<!-- DONE -->
# Living: features/command-run.feature::PickerUsesCurrentValidationAndErrors
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: PickerUsesCurrentValidationAndErrors
  Given a command-list choice was valid when discovered
  When command run selects it after <change>
  Then it returns the same current-choice validation or operation error as the native picker
  And it does not fall back to another choice or automatically repeat a transfer
  Examples:
    | change |
    | destination beta stops |
    | beta is incompatible |
    | destination workspace disappears |
    | selected closed-tab entry is no longer available |
    | transfer outcome is unresolved after dispatch |
<!-- DONE -->
# Living: features/command-run.feature::CreateDestinationSpaceWithNextOrderedAnswer
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: CreateDestinationSpaceWithNextOrderedAnswer
  Given caller Planning's live tab is in alpha and compatible running beta has no spaces
  And beta / Create space… is a valid visible destination label in the native picker
  When command run "Move to session…" "beta / Create space…" "Build" is requested
  Then the first answer selects the create-space branch and the second supplies its New destination space name
  And the existing tab.transfer operation creates Build in beta and moves Planning into it
  And it prints the normal result or start acknowledgment with operation identity
  And Planning's shell and running agent are not restarted

<!-- DONE -->
# Living: features/command-run.feature::MissingFollowupShowsActualNextQuestion
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: MissingFollowupShowsActualNextQuestion
  Given compatible beta has no spaces and alpha contains caller Planning
  When command run "Move to session…" "beta / Create space…" omits the next answer
  Then it exits nonzero and shows the next palette question New destination space name
  And it identifies a text answer as required with no enumerated valid choices for that text prompt
  And no space is created and Planning remains in alpha unchanged

<!-- DONE -->
# Living: features/command-run.feature::RejectSurplusPickerAnswersOnSelectedBranch
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: RejectSurplusPickerAnswersOnSelectedBranch
  Given alpha contains caller Planning and beta has the selected destination state
  When command run receives "Move to session…" followed by <answers>
  Then it exits nonzero with an extra-answer error before starting tab.transfer
  And no destination space or tab is created or moved
  Examples:
    | answers |
    | "beta / Work" "Build" |
    | "beta / Create space…" "Build" "surplus" |

<!-- DONE -->
# Living: features/command-run.feature::ListShowsOnlyFirstPromptHint
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: ListShowsOnlyFirstPromptHint
  Given beta contains no spaces and a context-valid caller requests command list
  Then Move to session… shows only its first destination-choice prompt hint
  And it does not show New destination space name as an unconditional second argument
  When command run "Move to session…" omits the first answer
  Then it fails showing the destination-choice question and all current valid labels including beta / Create space…
  When command run "Move to session…" "beta / Create space…" omits the second answer
  Then it fails showing New destination space name rather than repeating the destination-choice question

#### Local build evidence for Task 3.1

Each row has a scenario commit and an observed failing production mutation,
followed by restored GREEN. Commands use
`cargo test --test command_run <test-name> -- --exact`; exact test names are in
`tests/command_run.rs`. Durations below are test runtime, not compilation.

| Scenario | Initial test | GREEN | Mutation RED | Restored GREEN |
|---|---:|---:|---:|---:|
| MoveTabToVisibleDestination | RED 1.14s | 4.65s | 1.14s, discard selected workspace ID | 4.71s |
| RestoreClosedTabByVisibleLabel | RED 0.73s | 0.80s | 0.74s, corrupt selected history ID | 0.80s |
| RejectInvalidPickerLabels | RED 1.46s | 1.43s | 4.75s, remove uniqueness guard | 1.42s |
| PickerUsesCurrentValidationAndErrors | inherited GREEN 1.53s | 1.53s | 1.15s, omit operation identity | 1.55s |
| CreateDestinationSpaceWithNextOrderedAnswer | RED 1.20s | 4.70s | 4.74s, use first answer for name | 4.72s |
| MissingFollowupShowsActualNextQuestion | RED 1.17s | 1.19s | 1.20s, repeat first question | 1.19s |
| RejectSurplusPickerAnswersOnSelectedBranch | RED 4.75s | 1.13s | 4.65s, admit one extra answer | 1.12s |
| ListShowsOnlyFirstPromptHint | inherited GREEN 1.18s | 1.18s | 1.19s, advertise branch-only name prompt | final focused layer |

The current-validation and list-hint regressions were already satisfied by the
earlier shared adapters; their initial runs were GREEN, not missing-behavior RED.
The mutation failures above are real, but are not relabeled as initial TDD RED.
The unresolved-outcome fixture denies operation-lock creation after discovery;
the incompatible fixture changes only an owned Core's advertised protocol.
These are bounded failure fixtures, not a new compatibility or fault campaign.

The two successful destination tests additionally run the existing owned
foreground OMP reporter fixture on Planning's PTY. They compare original shell
and reporter `/proc` start/TTY identities, retain the terminal ID and agent
session, and accept the original locator's lifecycle report after movement.
Only fixture Cores, shells, reporters and per-home transfer helpers are used.
No daily Core or actual user agent is attached or restarted.

Focused final layer: `cargo test --test command_run -- --test-threads=1`
passed 26/26 in 48.56s. Native history selection passed 1/1 in 4.46s;
native empty-destination creation passed 1/1 in 5.16s. Strict focused Clippy
(`cargo clippy --test command_run -- -D warnings`) passed with no warnings
in 10.15s; `cargo fmt --all` completed. The generated protocol schema was
updated with `TRAILBOSS_UPDATE_API_SCHEMA=1` and its current-artifact test
passed 1/1 in 0.02s. No broad workspace gates or review were run here.

Review fixes preserve the original pane-bound request through deferred discovery
and re-admission; terminal generation/process checks remain intact and the current
tab is re-resolved before dispatch. Three regressions cover a retained shell's
second named move after its original Core exits, a successful response's operation
identity, and a real committed move whose response is dropped by an owned proxy.
The proxy observes one request and one persisted operation, with no automatic retry.
All three were observed RED before the fixes and GREEN together in 17.08s.
Mutations restoring premature request replacement, removing success identity, and
removing recovery identity from CLI transport errors each failed their regression.
The retained-shell witness waits for the owned original Core process to exit,
not merely its stop acknowledgment. With that condition satisfied, the request
replacement mutation fails specifically with `stale_terminal_route` / “forwarding
requires an explicit terminal target” (5.74s). Lost-response recovery also queries
the returned operation ID through `tab.transfer.status` and observes Committed.

The combined fix includes the native-click fixture repair in
`tests/support/action_producer.rs` and `tests/plugin_actions.rs`: generated owner
command fragments are not proof that the real owner action row is ready. The
fixture now waits for the owner/action/context-bound row and verifies the actual
rendered hit. Its independently executed gated readiness mutation failed; the
three restored focused regressions passed in 10.220s. Detailed peer receipt:
`/tmp/trailboss-command-run-native-click-fix-evidence.json`. This is a fixture
readiness repair, not a production native command change.

Combined review-fix focused proof: command-run 28/28 passed in 61.06s;
the three native producer/click regressions passed in 0.92s; strict focused
Clippy for both test targets passed with zero warnings in 11.25s; formatting
completed; generated API schema freshness passed 1/1 in 0.02s. These are local
focused checks, not parent final gates, delivery, or independent fidelity.

Formal review repairs preserve shell-decoded selector/answer text before global
session/remote option extraction and consume one `--` separator. The two-owned-Core
regression was RED before the fix and under separate boundary/separator mutations,
then GREEN. The prerequisite helper now fences unresolved prior activation with a
serialized Cancel preflight before source preparation. Its controlled concurrent
adopter witness was RED before the fence and when only the fence was removed;
the restored nine-test failure suite passed, including Stage EOF before helper death.

Restacked boundary proof on 2026-11-09: Task 1.1 `7ef35664` passed 20/20
command-run/native-owner integrations, bins, formatting and workspace strict Clippy.
Task 2.1 `b82d1fd8` passed 113/113 integrations across eight binaries and the same
build/style checks. Final code head `fcd5ad6e` passed bins (13.310s), formatting
(2.517s), workspace strict Clippy (16.730s), library tests (3,785 passed, 13 skipped;
21.352s wall), and the 20 affected integration targets (180 passed, 16 skipped;
66.492s wall). Evidence: `/tmp/trailboss-command-run-review-repair-gates.json`.
This subsequent amendment changes only this evidence/readiness text.



## Procedure pins

- Default planning and delivery workflow: `01a14125-6806-713d-ab55-4c6b92f13f64`, revision 10.
- Feature planning procedure: `01a14125-680b-7993-8109-a1055a77d3d8`, revision 8.
- Plan and PRD coverage format: `01a14125-680f-76df-b77f-b272487d94e9`, revision 3.
- Planning authoring checklist: `01a14125-6849-7ca7-aaeb-a00776adc5b5`, revision 3.
- Sequentially shippable review units: `01a14125-684b-7aa3-a421-b3a3ed80661a`, revision 3.
- Plan decomposition procedure: `01a14125-6821-73a7-895f-f9a9cefb0bdf`, revision 4.
- PRD fidelity audit and planning learning: `01a14125-681c-7742-854c-95ec0619f326`, revision 3.
- Plan build procedure: `01a14125-6812-7d36-9b62-f66b5b886e10`, revision 5.
- Task build procedure: `01a14125-6815-7dbb-9d59-ea4c63ddd2b4`, revision 3.
- Workflow block policy: `01a14125-6848-754f-a006-a18f046ae68d`, revision 2.
- Review unit splitting procedure: `01a14125-6825-73e2-9382-1814779b894b`, revision 4.

## Planning evidence

Scenarios specify only CLI/API-visible behavior. Technical contracts and rationale are in docs/command-run.md. Living scenarios now reflect the locally implemented tasks; DONE/YES records build progress, not production delivery or independent fidelity. Estimates use the existing catalog projection, native Core invoke, plugin dispatch and picker sizes. Build remains sequential by task with exact-head boundary evidence; no push or deployment is authorized.

## Coverage and independent fidelity audit

Source enumeration: Goal plus R1–R12, 13/13 source units mapped to 35 atomic rows. Ledger completeness: 35/35, 0 extraction misses. Covered rows with authored scenario pointers: 34/34. Dispositions: 34 covered, 1 out-of-scope (R7), 0 no-plan, 0 deferred, 0 deviations. R12 resolves the nested destination-name contradiction; no open product question remains.

The planning-time independent audit found no living artifact and therefore verified 0/34 covered rows. The subsequent local build has created `features/command-run.feature` and completed the plan's scenarios, but no new independent implementation-fidelity audit has run: all 34 covered rows remain fidelity-unverified and the PRD is not declared satisfied. Planning-time findings were 0 prior-audit escapes, 0 benchmark gaps, 0 extraction gaps and 0 proposed deviations. Build evidence does not replace an independent scenario-versus-source audit.
