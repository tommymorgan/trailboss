Feature: Renderer-neutral shared client contracts

  Scenario: Consume shared UI and pane state independently of the terminal renderer
    Given the shared Core owns the live session state and action semantics, each client owns its renderer, and the Core publishes widget declarations and pane Surface state through the shared client contract
    When an independent protocol consumer decodes that stream using the documented contract without the terminal renderer
    Then it obtains the declared structure, action identities, semantic styling and pane terminal state, the declaration payload itself contains no ratatui types, terminal escape sequences or fixed terminal-cell geometry, and Core-owned state and action semantics remain independent of each client-owned renderer
