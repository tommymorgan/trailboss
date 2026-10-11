Feature: Daily-driver spike exit

  Scenario: Keep the fork when the spike fails its exit test
    Given at least one required daily-driver exit result is failing or absent
    When the exit report evaluates the spike
    Then the report marks the spike failed and directs work back to plugin development on the existing herdr fork rather than switching daily drivers
