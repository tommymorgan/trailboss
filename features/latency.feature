Feature: Click-to-repaint latency

  Scenario Outline: Meet the local and remote click-to-repaint gate
    Given 15 populated panes are running in <mode> mode on the recorded acceptance setup
    When click-to-repaint-15-panes measures repeated workspace, tab and pane focus clicks through the completed visible repaint
    Then every measured click-to-repaint sample is at most 100ms including producer and transport delays and the displayed Surface and focus state match the requested selection

    Examples:
      | mode |
      | local |
      | remote from achy |
