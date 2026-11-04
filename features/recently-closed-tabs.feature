Feature: Recently closed tabs

  Scenario: ClosedTabRecreatesItsLayoutAndWorkingDirectories
    Given a named space contains a two-pane tab with distinct labels and working directories
    When that tab is closed and its public closed-tab entry is restored
    Then replacement panes preserve the tab name, split geometry, pane labels and directories in the original space
    And its old processes have ended and its new panes accept terminal input
    And the restored entry is no longer in history

  Scenario: HistorySurvivesRestartWithNoOpenSpaces
    Given closed-tab history exists and every space has been closed
    When the Core restarts using its persisted session
    Then the history remains available without restarting any closed job
    And restoring an entry recreates its missing original space and usable tab

  Scenario: HistoryIsBoundedAndNewestFirst
    Given 51 differently named tabs have been closed in one session
    And a different session has its own closed tab
    When closed-tab history is listed
    Then exactly the latest 50 closures appear in newest-first order
    And the oldest evicted entry cannot be restored
    And the other session's history is unchanged and its entry cannot be restored in this session

  Scenario: ActualTabRemovalIsRecordedWithoutRecordingMoves
    Given tabs are removed by explicit tab close, workspace close, final-pane close and natural final-pane exit
    When each completed removal is listed in its owned session
    Then each removed tab has exactly one restorable history entry
    And a tab transfer, Core handoff and failed-creation rollback add no closure entries

  Scenario Outline: UnavailableRestoreRetainsHistoryAndCurrentTabs
    Given a closed tab requires a <prerequisite> that is now missing
    When restore is requested
    Then a clear structured error identifies the unavailable prerequisite
    And history and all existing live tabs remain unchanged
    And retry succeeds after the prerequisite is restored
    Examples:
      | prerequisite |
      | working directory |
      | supported agent transcript |

  Scenario: SavedConversationResumesWithoutDuplicateWritersOrJobReplay
    Given a closed tab has a saved supported agent conversation and a different pane had an ordinary foreground job
    When the tab is restored
    Then the replacement agent resumes that exact saved conversation and the ordinary job is not relaunched
    And a running or pending copy of the conversation prevents a duplicate writer
    And a repeated restore request cannot launch another agent
    And disabled resumption or replacement-shell startup failure retains history and all live tabs with a clear error
    And two panes of one closed tab cannot start duplicate writers for the same conversation

  Scenario Outline: RecentlyClosedTabsPickerRestoresTheChosenEntry
    Given two closed tabs share a name but belong to different spaces
    When Recently closed tabs is opened from the <entry point>
    Then the existing searchable picker shows newest-first entries with their original spaces and close times
    When an entry is chosen with <input>
    Then only that selected entry is restored and focused
    And dismissal leaves history unchanged and empty history is shown explicitly
    Examples:
      | entry point | input |
      | session menu | keyboard |
      | session menu | mouse |
      | command palette | keyboard |
      | command palette | mouse |

  Scenario: HistorySurvivesHandoffAndOldSnapshotsStartEmpty
    Given an older snapshot has no closed-tab history field
    When the new Core loads it
    Then history starts empty and its existing tabs remain usable
    When a tab is closed and the session hands off to the same feature-enabled release
    Then its history and remaining live process identities survive without recording extra closures
