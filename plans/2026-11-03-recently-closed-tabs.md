# Feature: Recently closed tabs

**Created**: 2026-11-03
**Goal**: View recently closed tabs and restore their saved working context without reconstructing it manually.
> Plan home: repository
> Target repository: git@github.com:tommymorgan/trailboss.git
> Source PRD: none; direct approved request and docs/recently-closed-tabs.md requirements R1–R9
> PRD coverage: plans/recently-closed-tabs.prd-coverage.md
> Context: docs/recently-closed-tabs.md; CONTEXT.md
> Source base: 7c5fe57ff43fc7296aeddfd6a4ed11c421dd6932

## Slice 1: Recover closed working context
**Tracer bullet**: Close a real split tab, list its history, restore it and use the replacement panes in the saved directories.
**Touches**: Core close/capture/persistence/restore, additive public methods, command declarations, existing session menu and native picker, owned public integration tests.
**Off-limits**: live user tabs, arbitrary command replay, hidden live jobs, pre-install backfill, new dependencies/transports/security boundaries, unrelated rendering repairs.

### Task 1.1: Complete durable history and picker restore
<!-- estimate: ~1200 reviewable lines, low confidence until constructor reuse is exercised | feature-files: features/recently-closed-tabs.feature | demoable: yes -->
> Depends on: none
> Shippable state: Complete 50-entry durable history, guarded recreation within the approved native recovery matrix, and usable picker; existing close/restore behavior remains valid. No successor required.
> Boundary verification: Exact-head owned Core/native client closes, lists and restores real PTYs, survives restart/handoff, and exercises failure/duplicate guards without touching user sessions.
> Build status: COMPLETE within the approved native recovery matrix (Kizn decision revision 4). Combined recovery/sidebar source de20c4694668d6305e6ff650c47cfa3614abd5d0 is published on master. Junkman's daily Core uses the verified release after a process-preserving handoff; Achy's installed executable and bundled producers are verified. Beauty remains inaccessible. Existing clients and unrelated session Cores were not restarted. Tommy's hands-on acceptance remains outstanding.
> Release verification: 4,058 workspace tests and four explicit native restoration checks passed; strict workspace/all-target Clippy and formatting passed. The optimized artifact passed an isolated handoff rehearsal and non-test CLI/API/PTY close/list/restore smoke. Daily handoff preserved 35 panes, 69 captured processes and 523 command keys. Achy's isolated native client rendered its owned workspace and registered the packaged recovery action. Artifact SHA-256: 718444cbe83522f436c1872fbbf4efa055dae67533aeaa5a7d688618c4f83502. Operational receipts: /tmp/trailboss-tabs-release-wswcf7en. No all-host activation or human acceptance is claimed.

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::ClosedTabRecreatesItsLayoutAndWorkingDirectories
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: ClosedTabRecreatesItsLayoutAndWorkingDirectories
  Given a named space contains a two-pane tab with distinct labels and working directories
  When that tab is closed and its public closed-tab entry is restored
  Then replacement panes preserve the tab name, split geometry, pane labels and directories in the original space
  And its old processes have ended and its new panes accept terminal input
  And the restored entry is no longer in history

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::HistorySurvivesRestartWithNoOpenSpaces
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: HistorySurvivesRestartWithNoOpenSpaces
  Given closed-tab history exists and every space has been closed
  When the Core restarts using its persisted session
  Then the history remains available without restarting any closed job
  And restoring an entry recreates its missing original space and usable tab

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::HistoryIsBoundedAndNewestFirst
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: HistoryIsBoundedAndNewestFirst
  Given 51 differently named tabs have been closed in one session
  And a different session has its own closed tab
  When closed-tab history is listed
  Then exactly the latest 50 closures appear in newest-first order
  And the oldest evicted entry cannot be restored
  And the other session's history is unchanged and its entry cannot be restored in this session

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::ActualTabRemovalIsRecordedWithoutRecordingMoves
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: ActualTabRemovalIsRecordedWithoutRecordingMoves
  Given tabs are removed by explicit tab close, workspace close, final-pane close and natural final-pane exit
  When each completed removal is listed in its owned session
  Then each removed tab has exactly one restorable history entry
  And a tab transfer, Core handoff and failed-creation rollback add no closure entries

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::UnavailableRestoreRetainsHistoryAndCurrentTabs
# Action: creates
# Status: DONE
# Living updated: YES
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

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::SavedConversationResumesWithoutDuplicateWritersOrJobReplay
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: SavedConversationResumesWithoutDuplicateWritersOrJobReplay
  Given a closed tab has a saved supported agent conversation and a different pane had an ordinary foreground job
  When the tab is restored
  Then the replacement agent resumes that exact saved conversation and the ordinary job is not relaunched
  And a running or pending copy of the conversation prevents a duplicate writer
  And a repeated restore request cannot launch another agent
  And disabled resumption or replacement-shell startup failure retains history and all live tabs with a clear error
  And two panes of one closed tab cannot start duplicate writers for the same conversation

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::RecentlyClosedTabsPickerRestoresTheChosenEntry
# Action: creates
# Status: DONE
# Living updated: YES
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

<!-- DONE -->
# Living: features/recently-closed-tabs.feature::HistorySurvivesHandoffAndOldSnapshotsStartEmpty
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: HistorySurvivesHandoffAndOldSnapshotsStartEmpty
  Given an older snapshot has no closed-tab history field
  When the new Core loads it
  Then history starts empty and its existing tabs remain usable
  When a tab is closed and the session hands off to the same feature-enabled release
  Then its history and remaining live process identities survive without recording extra closures
