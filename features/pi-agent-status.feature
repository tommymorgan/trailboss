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
