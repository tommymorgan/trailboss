Feature: Session API compatibility

Scenario: Existing session automation works without protocol changes
  Given the new core is built with the fork’s server-side session API reused verbatim and the fork’s unmodified session API client and compatibility fixtures are available
  When the client exercises the reference session methods, response schemas and event subscriptions against the new core
  Then each result and event satisfies the original session API contract without rewriting the client or translating the protocol
