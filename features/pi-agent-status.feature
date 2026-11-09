Feature: Pi agent status reporting
  The Pi integration reports the root TUI session's state to Trailboss.

  Scenario: SettledPiWithBackgroundRunsReportsWaiting
    Given a Pi TUI root session has reported idle
    When pi-subagents emits herdr:busy with active true and label "⏳ 1 subagent (worker)"
    Then Trailboss receives state "waiting" with message "awaiting background work"
    When pi-subagents emits herdr:busy with active false
    Then Trailboss receives state "idle"

  Scenario: PiOwnTurnOutranksBackgroundWait
    Given a Pi TUI root session is waiting on background runs
    When the parent agent starts a turn
    Then Trailboss receives state "working"
    When the parent turn settles while the background runs remain
    Then Trailboss receives state "waiting" with message "awaiting background work"

  Scenario: PiBlockedOutranksBackgroundWait
    Given a Pi TUI root session is waiting on background runs
    When herdr:blocked is raised with label "approval"
    Then Trailboss receives state "blocked" with message "approval"
    When herdr:blocked is cleared while the background runs remain
    Then Trailboss receives state "waiting" with message "awaiting background work"

  Scenario: PiBackgroundLabelChangeSendsNoIdle
    Given a Pi TUI root session is waiting on background runs
    When pi-subagents emits herdr:busy active false and then active true in the same tick
    Then Trailboss receives no further state report

  Scenario: PiReloadRestoresBackgroundWait
    Given a Pi reload restores an active background run before the Trailboss extension's session_start runs
    When the Trailboss extension's session_start runs in TUI mode with the parent idle
    Then the first state Trailboss receives is "waiting" with message "awaiting background work"
