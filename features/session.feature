Feature: Local session compatibility

  Scenario: Use a local PTY through the Rust client
    Given a named local session is started by the vendored core with one shell pane
    When I attach the Rust client with the default workspace focused and type "printf spike-ready" into its primary working pane
    Then the client’s primary focused working Surface is the live PTY-backed pane, it shows "spike-ready" from that PTY, and it accepts further shell input while plugin UI remains in reserved Zones

  Scenario: Disconnect without losing the running session
    Given a named session has workspaces, tabs and panes including a running shell command
    When I disconnect the client and attach a new client to that session
    Then the workspaces, tabs and live panes remain and the command continues in the same PTY
