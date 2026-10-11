Feature: Public declarations determine application composition

  Scenario Outline: Stock UI uses the public widget declaration path
    Given the stock UI plugin is enabled in the <state> state
    When the client displays the application using the plugin’s public widget declarations
    Then all visible UI composition, including placements of pane Surfaces, sidebar, tab strip and menus when present, is determined solely by those public declarations, with no privileged stock-UI rendering path

    Examples:
      | state |
      | no pane yet |
      | live session |
      | new-tab interaction open |
      | menu open |

  Scenario Outline: Compose each supported widget node
    Given a producer declares a <node> inside a nested widget tree
    When the client applies that declaration
    Then the client displays the <observable> using its renderer under core-defined semantics

    Examples:
      | node | observable |
      | row | children arranged horizontally |
      | column | children arranged vertically |
      | label | declared text |
      | leader | elastic leader between neighboring labels |
      | spacer | declared empty spacing |
      | list | declared ordered entries |

  Scenario: Pin styled declarations into zones
    Given a plugin declares label, muted, leader and status-role segments with named tones in the list, footer and footer-right zones
    When the client projects those declarations into the terminal
    Then each declaration stays in its client-renderer-defined zone geometry and the client palette styles its roles and tones without plugin-supplied pixels
