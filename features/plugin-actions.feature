Feature: Plugin action dispatch

  Scenario: Dispatch plugin actions through stdin envelopes
    Given a long-lived producer declares a focus-tab action for an unfocused tab
    When I activate that declaration
    Then the producer receives the correlated action and tab context on stdin and the requested tab becomes focused after its successful action result
