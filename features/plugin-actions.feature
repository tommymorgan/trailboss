Feature: Plugin action dispatch

  Scenario: Dispatch plugin actions through stdin envelopes
    Given a long-lived producer declares a focus-tab action for an unfocused tab
    When I activate that declaration
    Then the producer receives the correlated action and tab context on stdin and the requested tab becomes focused after its successful action result

  Scenario Outline: Plugins place core-defined actions
    Given a plugin declares its own label and placement for the client-local <action> in the reserved core-action namespace
    When I activate the action, change its plugin label and placement, and activate it again while the plugin attempts to supply its own behavior
    Then only the core-defined <behavior> occurs, both before and after the change, and the plugin cannot initially define or later redefine those semantics

    Examples:
      | action | behavior |
      | new | new-tab interaction |
      | rename-space | rename-space interaction |

