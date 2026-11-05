# Feature: Live session handoff

**Created**: 2026-11-04
**Goal**: Reorganize live tabs between sessions, or explicitly switch the viewing window, without restarting the work.
> Plan home: repository
> Target repository: git@github.com:tommymorgan/trailboss.git
> Source PRD: direct request and accepted decisions R1–R5 in docs/live-session-handoff.md; no separate PRD
> PRD coverage: plans/live-session-handoff.prd-coverage.md
> Context: docs/live-session-handoff.md; CONTEXT.md
> Source base: master 67dbb238c489988ff133c20a977167918c3f04ef + repeater-app-bridge aba034b68f9dea7df90603121e2b7aa96ddcc8f6
> Readiness: Full-plan implementation authorized by “Don't stop at 1.1. Build every fucking thing unless you get truly blocked”. Execute Tasks 2.1 and 3.1 in order through the public Core API/CLI, real owned Core/PTY and actual native-renderer seams; task boundaries are not approval gates. The documented routing helper, ownership model and interface work are included in this execution scope, not a new unrelated architecture. No release automation, new dependency or security relaxation is authorized. Scenario status and runtime evidence, not authorization alone, establish implementation.
> Compatibility tool: the separately shipped manual release-compatibility command on master (`docs/release-compatibility.md`, README "Manual release compatibility check"). Each unit runs it; this plan does not build or change it.

## Slice 2: Explicitly switch the viewing window

**Tracer bullet**: Choose Switch session in an actual native window and continue typing into the destination's existing shell while another window remains in the original session.
**Touches**: public session discovery, Core command catalog, session-menu declaration, existing searchable picker, native local/SSH attachment lifecycle.
**Off-limits**: Core-global retargeting, stopping sessions, restarting shells/agents, new shortcut defaults, tab ownership transfer, unrelated UI redesign.
> Design: existing session menu, command palette and searchable picker; no new layout.

### Task 2.1: Deliver complete explicit session switching
<!-- estimate: ~800–1400 reviewable lines, low confidence until native attachment lifetime is exercised | feature-files: features/session-switching.feature | demoable: yes -->
> Depends on: none (uses the shipped release-compatibility command)
> Shippable state: Explicit window switching and read-only host session discovery are usable, including failure/cancel behavior. Existing session attach, detach, tab navigation and Core processes retain their contracts. No transfer command or routing helper is exposed yet.
> Boundary verification: With this exact head and no later work, run two isolated real Cores and native clients; switch one client, type into the original destination shell, reject a failed destination, and prove spectator attachments and both sessions' PID/start/PTY identities unchanged. Exercise the same command through the existing SSH attachment path without touching human sessions.
> Compatibility check: Run the shipped release-compatibility command manually against this exact unit and the retained baseline, including older-client command availability. Use its findings and limits to inform Tommy's version choice; no automated publication hook or decision file is required.

<!-- DONE -->
# Living: features/session-switching.feature::ExplicitSwitchChangesOnlyTheInvokingWindow
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: ExplicitSwitchChangesOnlyTheInvokingWindow
  Given running sessions alpha and beta contain distinct live marker shells
  And two native windows view alpha and a third native window views beta
  When the first window opens Switch session from the <entry> and chooses beta
  Then that window displays beta and its input reaches beta's original shell
  And the other windows retain their session attachments and viewed panes
  And both sessions retain their original shell and background-job PID, start identity and PTY
  Examples:
    | entry |
    | session menu |
    | command palette |

<!-- DONE -->
# Living: features/session-switching.feature::FailedOrCancelledSwitchRetainsSourceAttachment
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: FailedOrCancelledSwitchRetainsSourceAttachment
  Given a native window is attached to alpha and its shell has a local marker
  When switching to beta encounters <outcome>
  Then the window remains attached to alpha with its original pane view
  And subsequent input reaches alpha's original shell and reads the marker
  And no replacement session or shell is started
  And failure is shown clearly while cancellation has no error
  Examples:
    | outcome |
    | picker cancellation |
    | destination stops before attach |
    | incompatible destination |
    | connection failure |

<!-- DONE -->
# Living: features/session-switching.feature::RemoteSwitchDiscoversSessionsOnTheAttachedHost
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: RemoteSwitchDiscoversSessionsOnTheAttachedHost
  Given an owned native window is attached through SSH to host A session alpha
  And host A has running beta while the display computer has an unrelated session named local-only
  When Switch session is opened and beta is selected
  Then the picker identifies sessions on host A rather than offering local-only
  And the new attachment retains host A and the selected remote executable
  And typing reaches beta's existing remote shell without stopping either remote session

<!-- DONE -->
# Living: features/session-switching.feature::OldAttachmentUpdatesCannotOverwriteTheNewView
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: OldAttachmentUpdatesCannotOverwriteTheNewView
  Given alpha has queued terminal, menu and graphics updates while a window switches to beta
  When the destination attachment becomes ready and switching commits
  Then later alpha updates cannot repaint beta or target beta with alpha's pending action
  And beta's pane, menu, selection and tab history use only the new attachment
  And the old attachment is released without stopping alpha's work

<!-- DONE -->
# Living: features/session-switching.feature::OlderClientsDoNotExposeUnsupportedSessionCommands
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: OlderClientsDoNotExposeUnsupportedSessionCommands
  Given an older native client and a capable new native client view the same upgraded Core
  When their session menus and command palettes are opened
  Then the new client can invoke the supported explicit session commands
  And the older client is not offered commands it cannot execute
  And ordinary older-client terminal input, tab navigation and detach remain usable

## Slice 3: Move a live tab independently of its source session

**Tracer bullet**: Move a split tab with a shell-local marker, background job and managed OMP into beta, stop alpha completely, then use the same original processes and guarded agent controls through the inherited context.
**Touches**: selective PTY ownership, same-host Core coordination, protected per-user routing helper and persisted ownership records, public transfer/status interface and CLI, tab-menu/palette declarations, source/destination view reconciliation, source stop/start/delete behavior.
**Off-limits**: cross-host process migration, process recreation, process suspension as a transfer technique, reporter restart/environment rewriting, weaker agent/phone admission, arbitrary job replay, new packages/TCP services, unrelated live sessions.
> Design: existing tab menu, command palette and searchable destination picker.

### Task 3.1: Deliver complete live transfer and independent inherited routing
<!-- estimate: ~4000–6500 reviewable lines, low confidence until the approved ownership/front-door protocol is exercised | feature-files: features/session-tab-transfer.feature | demoable: yes -->
> Depends on: Task 2.1
> Shippable state: Whole-tab movement, source-independent inherited routing and all exposed failure/ownership checks are complete together. The helper is not published as an unused prerequisite. Existing same-session transfer and ordinary session lifecycle remain valid. No successor repairs a deployed partial move.
> Boundary verification: At this exact head, exercise API/CLI and native menu/palette movement between two isolated real Cores; record original shell/job/OMP PID/start/PTY and terminal state, prove source Core process exit after transfer, then read/send/control via destination and original inherited context. Exercise invalid/precommit-failure/uncertain-result paths, reverse/third-session moves, ID collisions, root/report fences and unaffected spectator windows. Run focused checks per behavior and the complete delivery gate once at the unit boundary.
> Compatibility check: Run the shipped release-compatibility command manually against this exact unit and the retained baseline. Exercise legacy same-session API/CLI behavior and unsupported cross-session coordination before mutation; report compatibility changes and likely version impact without automatically selecting or publishing a version.

<!-- TODO -->
# Living: features/session-tab-transfer.feature::LiveTabRetainsProcessesAndStateInItsDestination
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: LiveTabRetainsProcessesAndStateInItsDestination
  Given alpha contains a named split tab with distinct pane labels, working directories, shell-local markers and a background job
  And both panes contain retained terminal content and one split is focused and zoomed
  And beta contains an unrelated live tab in space Work
  When the captured tab moves into beta Work through <entry>
  Then beta contains that tab with its split layout, labels, directories, retained content, zoom and focused split
  And the original shell and job PID, start identity and PTY remain unchanged
  And input in each moved pane reads its original shell-local marker
  And alpha no longer lists the moved tab or records it as recently closed
  And unrelated source and destination tabs remain usable and unchanged
  Examples:
    | entry |
    | public API |
    | CLI |
    | tab menu |
    | command palette |

<!-- TODO -->
# Living: features/session-tab-transfer.feature::MovingATabDoesNotSwitchAnyWindowToAnotherSession
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: MovingATabDoesNotSwitchAnyWindowToAnotherSession
  Given two native windows view alpha and another views beta
  And the invoking alpha window views the tab being moved
  When that tab moves from alpha into beta
  Then every window retains its existing session attachment
  And alpha viewers use their normal remaining-tab or empty-session view
  And the invoking window can explicitly choose Switch session to view the moved tab in beta

<!-- TODO -->
# Living: features/session-tab-transfer.feature::MovedTabAndAgentControlsSurviveSourceCoreExit
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: MovedTabAndAgentControlsSurviveSourceCoreExit
  Given a live tab containing a shell, background job and managed OMP has moved from alpha into beta
  And its processes still contain alpha's original socket and pane handles
  When alpha is stopped and its Core process exits
  Then the moved shell, job and OMP retain their original PID, start identity and PTY
  And the moved shell accepts input and retains its shell-local marker
  And genuine OMP reports through the inherited context update only beta's moved agent
  And fresh destination conversation reads and guarded submit/abort controls reach that same OMP
  And alpha is reported stopped rather than falsely running because a routing address remains

<!-- TODO -->
# Living: features/session-tab-transfer.feature::InvalidDestinationDoesNotPartiallyMoveTheTab
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: InvalidDestinationDoesNotPartiallyMoveTheTab
  Given alpha has a live split tab and beta has unrelated live work
  When transfer encounters <problem> before ownership commit
  Then a clear error identifies the rejected destination or context
  And the original source tab, destination tabs, focus and running processes remain unchanged
  And the source panes still accept input
  Examples:
    | problem |
    | missing session |
    | stopped session |
    | incompatible Core |
    | removed destination space |
    | stale captured source tab |
    | foreign or invalid endpoint |

<!-- TODO -->
# Living: features/session-tab-transfer.feature::PreparationFailureRollsBackOnlyProvisionalResources
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: PreparationFailureRollsBackOnlyProvisionalResources
  Given alpha and beta contain live marker tabs with recorded process identities
  When <failure> prevents the selected transfer from committing
  Then alpha retains the selected tab and its original processes and terminal content
  And beta retains its original topology and processes
  And no destination replacement shell, duplicate reader or recently-closed entry is created
  And source input works after rollback
  Examples:
    | failure |
    | descriptor transfer fails |
    | imported runtime construction fails |
    | destination disappears during preparation |
    | routing admission fails |

<!-- TODO -->
# Living: features/session-tab-transfer.feature::LostResultIsResolvedWithoutRepeatingTheMove
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: LostResultIsResolvedWithoutRepeatingTheMove
  Given a cross-session transfer uses one known operation ID
  When the result connection is lost after the destination may have committed
  Then the caller reports an unresolved result rather than claiming failure or success
  And it does not submit a second move or resume a possibly relinquished source reader
  When the recorded operation outcome is queried
  Then it identifies the committed owner and final mapping or the authoritative aborted outcome
  And only one live owner and one tab instance exist

<!-- TODO -->
# Living: features/session-tab-transfer.feature::IndependentCoreIdsAndInheritedAliasesCannotMisroute
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: IndependentCoreIdsAndInheritedAliasesCannotMisroute
  Given alpha and beta have identical-looking public pane locators for different processes
  When alpha's tab moves into beta and requests use its original endpoint and locator
  Then those requests resolve to the moved terminal's current owner
  And beta's unrelated same-looking locator remains associated with its original process
  And neither focus nor label equality is used to resolve ambiguity

<!-- TODO -->
# Living: features/session-tab-transfer.feature::ReturningOrMovingAgainDoesNotCreateRoutingCycles
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: ReturningOrMovingAgainDoesNotCreateRoutingCycles
  Given a tab has moved from alpha into beta while retaining its original inherited context
  When it next moves to <destination>
  Then the original context resolves directly to its current owner without a forwarding cycle
  And its original processes remain alive and accept input after the old source is stopped
  And stale previous-owner reports cannot overwrite the current terminal's authority
  Examples:
    | destination |
    | running alpha |
    | running gamma |

<!-- TODO -->
# Living: features/session-tab-transfer.feature::SourceRestartCannotStealMovedProcessContexts
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: SourceRestartCannotStealMovedProcessContexts
  Given alpha is stopped after its tab moved into beta
  When a new alpha Core starts and creates a different tab
  Then the moved processes' original contexts still resolve to beta
  And new alpha's contexts resolve to its own different tab
  And reusing a numeric public locator cannot redirect moved-agent reports to new alpha

<!-- TODO -->
# Living: features/session-tab-transfer.feature::StaleControlsAndConflictingRootReportsRemainRejected
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: StaleControlsAndConflictingRootReportsRemainRejected
  Given a managed OMP tab has moved into beta and its valid destination context has been read
  When a <request> is submitted through a retained context
  Then the existing authority checks reject it without invoking or rebinding the moved OMP
  And a fresh admitted control still targets the original OMP process in beta
  And the rejected request cannot advance an unrelated terminal's report sequence
  Examples:
    | request |
    | control with an old work fence |
    | report from a conflicting nested root |

<!-- TODO -->
# Living: features/session-tab-transfer.feature::EmptyDestinationAndSourceCleanupRemainUsable
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: EmptyDestinationAndSourceCleanupRemainUsable
  Given beta is running with no space and alpha has exactly one live tab
  When the explicit create-space destination is chosen for that tab
  Then beta gains the named destination space containing the original live tab
  And alpha remains an attached empty session without starting a replacement shell

<!-- TODO -->
# Living: features/session-tab-transfer.feature::DeletingAStoppedSourceCannotRemoveLiveRouting
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: DeletingAStoppedSourceCannotRemoveLiveRouting
  Given alpha is stopped while moved live processes in beta still use its original endpoint
  When deletion of alpha would remove that endpoint
  Then deletion is rejected with an explanation that live routing still refers to it
  And beta's processes and agent controls remain usable
  When those moved processes end and no live references remain
  Then ordinary stopped-session deletion succeeds without leaving an active route

<!-- TODO -->
# Living: features/session-tab-transfer.feature::TabOwnedPopupRetainsItsOriginalProcessAndOwner
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: TabOwnedPopupRetainsItsOriginalProcessAndOwner
  Given a tab in alpha owns a live popup terminal with its own shell-local marker
  And beta has an unrelated popup and live tab
  When the owning alpha tab moves into beta
  Then the moved popup retains its original PID, start identity, PTY and shell-local marker
  And its ownership refers to the moved tab rather than a coincidentally matching source tab
  And beta's unrelated popup and tab remain unchanged

<!-- TODO -->
# Living: features/session-tab-transfer.feature::DelayedSourceExitCannotRemoveUnrelatedSourceState
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: DelayedSourceExitCannotRemoveUnrelatedSourceState
  Given a live pane has moved from alpha into beta
  And alpha retains a different tab with a live popup and overlay
  When the original moved shell exits and its source child waiter reports that exit
  Then the moved pane closes in beta through its actual imported runtime
  And alpha's different tab, popup and overlay remain alive and usable
  And neither Core removes another pane by a stale location or reused public locator

<!-- TODO -->
# Living: features/session-tab-transfer.feature::RoutingAdoptionCannotBreakTheOriginalPublicEndpoint
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: RoutingAdoptionCannotBreakTheOriginalPublicEndpoint
  Given alpha serves real API clients through its original protected socket
  When routing adoption encounters <outcome>
  Then every acknowledged mutation has one authoritative result
  And subsequent requests through that same public endpoint still reach alpha
  And no foreign socket is unlinked or accepted under another user's ownership
  And alpha retains the selected tab's ownership and its original usable input
  Examples:
    | outcome |
    | helper starts and adoption commits |
    | acceptor quiescence fails |
    | helper exits before adoption commits |
    | peer ownership or socket identity does not match |
