Feature: Pi agent status reporting
  The Pi integration reports the root TUI session's state to Trailboss.

  Scenario: SettledPiWithBackgroundRunsReportsWaiting
    Given a Pi TUI root session has reported idle
    When pi-subagents emits herdr:busy with active true and label "⏳ 1 subagent (worker)"
    Then Trailboss receives state "waiting" with message "awaiting background work"
    When pi-subagents emits herdr:busy with active false
    Then Trailboss receives state "idle"
