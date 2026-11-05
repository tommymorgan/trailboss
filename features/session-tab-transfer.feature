Feature: Process-preserving live cross-session tab transfer

  # Acceptance is tracked in plans/2026-11-04-live-session-handoff.md.
  # Agent controls are the restricted app-profile controls (R4 decision, 2026-11-07).

  Scenario Outline: LiveTabRetainsProcessesAndStateInItsDestination
    Given alpha and beta are distinct running Trailboss sessions on the same host
    And alpha contains a named split tab with distinct pane labels, working directories, shell-local markers and a background job
    And both panes contain retained terminal content and one split is focused and zoomed
    And beta contains an unrelated live tab in space Work
    When the captured tab moves into beta Work through <entry>
    Then beta contains that tab with its split layout, labels, directories, retained content, zoom and focused split
    And the original shell and job PID, start identity and PTY remain unchanged
    And every original tab process, including foreground programs and descendant jobs, continues running throughout without suspension or recreation, allowing only a brief terminal-I/O pause
    And input in each moved pane reads its original shell-local marker
    And alpha no longer lists the moved tab or records it as recently closed
    And unrelated source and destination tabs remain usable and unchanged
    Examples:
      | entry |
      | public API |
      | CLI |
      | tab menu |
      | command palette |

  Scenario: MovingATabDoesNotSwitchAnyWindowToAnotherSession
    Given two native windows view alpha and another views beta
    And the invoking alpha window views the tab being moved
    When that tab moves from alpha into beta
    Then every window retains its existing session attachment
    And alpha viewers use their normal remaining-tab or empty-session view
    And the invoking window can explicitly choose Switch session to view the moved tab in beta

  Scenario: MovedTabAndAgentControlsSurviveSourceCoreExit
    Given a live tab containing a shell, background job and managed OMP enrolled in the restricted app profile has moved from alpha into beta
    And its processes still contain alpha's original socket and pane handles
    When alpha is stopped and its Core process exits
    Then the moved shell, job and OMP retain their original PID, start identity and PTY
    And the moved shell accepts input and retains its shell-local marker
    And genuine OMP reports through the inherited context update only beta's moved agent
    And fresh destination restricted reads, single-outstanding submit and input status reach that same OMP
    And abort through beta is refused exactly as on an unmoved restricted tab
    And alpha is reported stopped rather than falsely running because a routing address remains

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

  Scenario: LostResultIsResolvedWithoutRepeatingTheMove
    Given a cross-session transfer uses one known operation ID
    When the result connection is lost after the destination may have committed
    Then the caller reports an unresolved result rather than claiming failure or success
    And it does not submit a second move or resume a possibly relinquished source reader
    When the recorded operation outcome is queried
    Then it identifies the committed owner and final mapping or the authoritative aborted outcome
    And only one live owner and one tab instance exist

  Scenario: IndependentCoreIdsAndInheritedAliasesCannotMisroute
    Given alpha and beta have identical-looking public pane locators for different processes
    When alpha's tab moves into beta and requests use its original endpoint and locator
    Then those requests resolve to the moved terminal's current owner
    And beta's unrelated same-looking locator remains associated with its original process
    And neither focus nor label equality is used to resolve ambiguity

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

  Scenario: SourceRestartCannotStealMovedProcessContexts
    Given alpha is stopped after its tab moved into beta
    When a new alpha Core starts and creates a different tab
    Then the moved processes' original contexts still resolve to beta
    And new alpha's contexts resolve to its own different tab
    And reusing a numeric public locator cannot redirect moved-agent reports to new alpha

  Scenario Outline: StaleControlsAndConflictingRootReportsRemainRejected
    Given a managed OMP tab enrolled in the restricted app profile has moved into beta and its valid destination context has been read
    When a <request> is submitted through a retained context
    Then the existing authority checks reject it without invoking or rebinding the moved OMP
    And a fresh admitted restricted control still targets the original OMP process in beta
    And the rejected request cannot advance an unrelated terminal's report sequence
    Examples:
      | request |
      | second restricted submit while an earlier instruction is outstanding |
      | restricted submit bound to a conversation replaced by local /new |
      | report from a conflicting nested root |

  Scenario: EmptyDestinationAndSourceCleanupRemainUsable
    Given beta is running with no space and alpha has exactly one live tab
    When the explicit create-space destination is chosen for that tab
    Then beta gains the named destination space containing the original live tab
    And alpha remains an attached empty session without starting a replacement shell

  Scenario: DeletingAStoppedSourceCannotRemoveLiveRouting
    Given alpha is stopped while moved live processes in beta, including a managed OMP enrolled in the restricted app profile, still use its original endpoint
    When deletion of alpha would remove that endpoint
    Then deletion is rejected with an explanation that live routing still refers to it
    And beta's processes and restricted agent controls remain usable
    When those moved processes end and no live references remain
    Then ordinary stopped-session deletion succeeds without leaving an active route

  Scenario: TabOwnedPopupRetainsItsOriginalProcessAndOwner
    Given alpha and beta are distinct running Trailboss sessions on the same host
    And a tab in alpha owns a live popup terminal with its own shell-local marker
    And beta has an unrelated popup and live tab
    When the owning alpha tab moves into beta
    Then the moved popup retains its original PID, start identity, PTY and shell-local marker
    And the original popup process continues running throughout without suspension or recreation
    And its ownership refers to the moved tab rather than a coincidentally matching source tab
    And beta's unrelated popup and tab remain unchanged

  Scenario: DelayedSourceExitCannotRemoveUnrelatedSourceState
    Given a live pane has moved from alpha into beta
    And alpha retains a different tab with a live popup and overlay
    When the original moved shell exits and its source child waiter reports that exit
    Then the moved pane closes in beta through its actual imported runtime
    And alpha's different tab, popup and overlay remain alive and usable
    And neither Core removes another pane by a stale location or reused public locator

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
