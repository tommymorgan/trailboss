Feature: Agent-tree declarations

  Scenario: Port the agent tree as the first widget-tree consumer
    Given no plugin has yet consumed the new widget-tree API, and the fork’s agent-tree producer and TanStack DB store are ported without replacing the store or stdin-envelope transport
    When I enable agent-tree as the first real plugin for a session with named workspaces and tabs
    Then the first real widget-declaration stream is emitted by agent-tree, its sidebar shows those workspaces and tabs with the fork’s status glyphs, roles and elastic leaders, and its focus-tab action focuses the selected tab

  Scenario: Expand and collapse workspace declarations
    Given a workspace in the agent tree contains three tabs
    When I collapse the workspace and expand it again
    Then its tabs disappear and reappear under that workspace and the collapsed header shows the hidden-tab count
