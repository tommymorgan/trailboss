Feature: Remote session attach

  Scenario: Attach from achy to one remote session
    Given the remote core owns a named session and achy has SSH access to it
    When I run the client with --remote on achy and it attaches over SSH to that session
    Then the client attaches to exactly that session and displays its live Surfaces while shell input executes on the remote core
