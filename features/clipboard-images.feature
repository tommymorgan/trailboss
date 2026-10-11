Feature: Clipboard images

Scenario Outline: Paste clipboard images into the focused pane
  Given I am attached in <mode> mode and an image is on the client clipboard
  When I paste into the focused pane
  Then the client intercepts the image, the session core stages it in /tmp/herdr-clipboard-images-UID/ on the server host where UID is the server user’s UID, and the staged path is injected only into that focused pane

  Examples:
    | mode |
    | local |
    | remote from achy |
