Feature: Run palette commands by name

Scenario: CreateSpaceByTitle
  Given a running owned Core with no active pane
  When I run trailboss command run "New space" "Research"
  Then the CLI exits zero with the workspace creation result
  And workspace.list contains the new space named Research
  And no native window or confirmation prompt is required

Scenario Outline: RunCoreCommandsInCallerContext
  Given caller pane w1:p1 belongs to tab Planning in space Work
  And Core-global focus and a native window are on decoy tab Other in space Personal
  When the caller runs trailboss command run "<title>" <argument>
  Then <effect> occurs through the public API in the caller context
  And Other and Personal remain unchanged
  Examples:
    | title | argument | effect |
    | New tab or space | "Notes" | a Notes tab is created in Work |
    | New tab | "Notes" | a Notes tab is created in Work |
    | Rename space | "Build" | Work is renamed Build |
    | Rename tab | "Build" | Planning is renamed Build |
    | Focus tab | omitted | Planning becomes the focused tab |
    | Close tab | omitted | Planning is closed without confirmation |

Scenario Outline: PreserveCoreInputMeaning
  Given caller pane w1:p1 belongs to Planning in Work
  When command run submits <title> with <argument>
  Then its result is <outcome>, matching the native palette's existing input semantics
  Examples:
    | title | argument | outcome |
    | New space | omitted | missing-answer error showing its name prompt and no space creation |
    | New tab | omitted | missing-answer error showing its name prompt and no tab creation |
    | Rename tab | omitted | missing-answer error showing its name prompt and no rename |
    | Rename space | whitespace only | invalid-name error and no rename |
    | Rename tab | "  Planning v2  " | rename uses exactly the palette-submitted name |

Scenario Outline: RejectSurplusCoreAnswersBeforeDispatch
  Given caller Planning's pane in Work and the command's native prompt sequence
  When command run receives <title> followed by <answers>
  Then it exits nonzero with an extra-answer error
  And no operation, name change, close or restart is started
  Examples:
    | title | answers |
    | New space | Research surplus |
    | Rename tab | Build surplus |
    | Close tab | surplus |
    | Restart idle agents | surplus |

Scenario: RestartIdleAgentsByName
  Given a running owned Core with an eligible idle agent and a protected busy agent
  When command run "Restart idle agents" is called without pane context
  Then it returns the existing agent.restart_idle result
  And the existing restart eligibility rules preserve the busy agent

Scenario Outline: ResolveExactTitlesAndStableIds
  Given command list exposes New space and its stable ID host:new-space
  When command run uses <selector> with argument Research
  Then it starts exactly the New space operation and returns its result
  Examples:
    | selector |
    | New space |
    | host:new-space |

Scenario Outline: RejectMissingOrAmbiguousCommands
  Given two enabled palette commands are titled Shared with different stable IDs
  When command run receives <selector>
  Then it exits nonzero and starts no operation
  And it prints <diagnostic>
  Examples:
    | selector | diagnostic |
    | Shared | ambiguous title with both titles and stable IDs |
    | does-not-exist | command not found |

Scenario Outline: RequireRealCallerPane
  Given Core-global focus is on decoy tab Other
  And the caller has <pane_context>
  When command run "Rename tab" "Build" is requested
  Then it refuses without changing Other or any tab
  And the missing context error is "no Trailboss pane context; run from a Trailboss pane" or the stale-locator error identifies the unavailable pane
  Examples:
    | pane_context |
    | no HERDR_PANE_ID |
    | an inherited locator no longer resolved by Core |

Scenario: ResolveMovedPaneRatherThanCachedLocation
  Given the same running shell's inherited HERDR_PANE_ID resolves through a Core alias to Planning in space Build
  And inherited HERDR_WORKSPACE_ID and HERDR_TAB_ID still name its old location
  When that shell runs command run "Rename tab" "Current"
  Then only its current containing tab is renamed Current
  And neither the old location nor the globally focused decoy is changed

Scenario Outline: ExcludeWindowOnlyCommandsFromApi
  Given a native window supports its existing <title> command
  When an API caller requests ui.command.list and then attempts ui.command.run by that command's title and stable key
  Then the command is absent from the API list and both run attempts fail without a window effect
  And the native window's command remains available with its original meaning
  Examples:
    | title |
    | Detach from session |
    | Switch session |
    | Previous tab |

Scenario: ListRunnableCoreCommandsAndHints
  Given caller pane Planning in Work and the complete non-picker Core adapter
  When trailboss command list is run
  Then it lists New space, New tab or space, New tab, Rename space, Rename tab, Close tab, Focus tab and Restart idle agents with stable IDs
  And name-taking commands display their argument hints
  And outside a pane the same list retains context-free commands and omits context-required commands
  And command run accepts no per-command workspace, tab or pane context flags
  And the legacy custom-keybinding command.invoke route retains its existing behavior

Scenario: RunExtensionByVisibleTitle
  Given enabled plugin omp exposes the visible palette title "omp: clear" and a qualified stable action ID
  And caller pane Planning in Work differs from the global focused pane
  When trailboss command run "omp: clear" is requested
  Then the existing plugin action starts exactly once on the caller's workspace, tab and pane
  And the CLI prints its plugin log ID and exits without waiting for the action to finish
  And no on-screen selection is supplied

Scenario Outline: PreserveRawPluginArgumentText
  Given enabled plugin omp exposes "omp: add-dir" with raw-tail input
  When command run selects it by <selector_kind> with <argument>
  Then plugin.action.invoke receives that same arguments option without trimming, tokenizing or shell evaluation
  And the CLI prints the started plugin log ID
  Examples:
    | selector_kind | argument |
    | visible title | omitted (None) |
    | qualified stable ID | explicit empty string (Some empty) |
    | visible title | two leading spaces, /tmp/a b, two trailing spaces |
    | qualified stable ID | literal dollar sign, quotes and newline |

Scenario Outline: RejectUnsupportedPluginExecution
  Given the current plugin catalog has <state>
  When command run attempts the affected title or stable ID
  Then it exits nonzero with the existing actionable error and no handler is started
  Examples:
    | state |
    | action not declared for the current platform |
    | disabled plugin |
    | invalid confirmation declaration that the native palette cannot execute |
    | supplied argument on an action that does not support raw-tail input |
    | two answers supplied to a raw-tail action whose palette accepts one text answer |

Scenario: PluginContextNeverFallsBackToGlobalFocus
  Given global focus is Other in Personal
  And a context-required plugin action and a context-free plugin action are installed
  When command run invokes the context-required action outside a pane
  Then it returns "no Trailboss pane context; run from a Trailboss pane" and starts nothing
  When command run invokes the context-free action outside a pane
  Then it starts with absent workspace, tab, pane and selection rather than Other's context
  And its plugin log ID is returned without background-completion waiting

Scenario: ListPluginsWithArgumentsAndBypassConfirmation
  Given enabled available plugins omp and pi expose palette commands and a confirmation-fragment action
  When command list is requested from Planning's pane
  Then every executable omp and pi command is discoverable by its native visible title and qualified stable ID
  And each raw-tail command retains its declared argument hint
  When command run selects the confirmation-fragment action
  Then it starts without requesting a confirmation and immediately returns its plugin log ID
