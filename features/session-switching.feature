Feature: Explicit window-local session switching

  Scenario Outline: ExplicitSwitchChangesOnlyTheInvokingWindow
    Given running sessions alpha and beta contain distinct live marker shells
    And two native windows view alpha and a third native window views beta
    When the first window opens Switch session from the <entry> and chooses beta
    Then that window displays beta and its input reaches beta's original shell
    And the other windows retain their session attachments and viewed panes
    And both sessions retain their original shell and background-job PID, start identity and PTY
    Examples:
      | entry |
      | session menu |
      | command palette |

  Scenario Outline: FailedOrCancelledSwitchRetainsSourceAttachment
    Given a native window is attached to alpha and its shell has a local marker
    When switching to beta encounters <outcome>
    Then the window remains attached to alpha with its original pane view
    And subsequent input reaches alpha's original shell and reads the marker
    And no replacement session or shell is started
    And failure is shown clearly while cancellation has no error
    Examples:
      | outcome |
      | picker cancellation |
      | destination stops before attach |
      | incompatible destination |
      | connection failure |

  Scenario: RemoteSwitchDiscoversSessionsOnTheAttachedHost
    Given an owned native window is attached through SSH to host A session alpha
    And host A has running beta while the display computer has an unrelated session named local-only
    When Switch session is opened and beta is selected
    Then the picker identifies sessions on host A rather than offering local-only
    And the new attachment retains host A and the selected remote executable
    And typing reaches beta's existing remote shell without stopping either remote session

  Scenario: OldAttachmentUpdatesCannotOverwriteTheNewView
    Given alpha has queued terminal, menu and graphics updates while a window switches to beta
    When the destination attachment becomes ready and switching commits
    Then later alpha updates cannot repaint beta or target beta with alpha's pending action
    And beta's pane, menu, selection and tab history use only the new attachment
    And the old attachment is released without stopping alpha's work
    And an action already submitted in alpha completes against alpha's original pane

  Scenario: OlderClientsDoNotExposeUnsupportedSessionCommands
    Given an unchanged retained native client and a capable new native client view the same upgraded Core
    And either client may attach first with its original startup behavior
    When their session menus and command palettes are opened
    Then the new client can invoke Switch session from both entrypoints and select its current running session
    And the older client is not offered Switch session in either entrypoint
    And this exclusion remains after tab changes refresh the declarations
    And retained startup declarations cannot remove Switch session from the capable client's entrypoints
    And ordinary older-client terminal input, tab navigation and detach remain usable
    And both clients detach without stopping the Core or changing its original shell and background-job PID, start identity or PTY
