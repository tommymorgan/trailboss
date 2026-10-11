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
