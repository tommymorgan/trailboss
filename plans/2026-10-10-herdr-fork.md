# Feature: Herdr-fork personal daily driver

**Created**: 2026-10-10
**Goal**: Run Tommy’s terminal sessions with plugin-declared UI, remote attach and clipboard images within the accepted repaint budget.
> Source PRD: docs/design-source.md (Kizn accepted design revision 6; no separate formal PRD)
> PRD coverage: plans/herdr-fork.prd-coverage.md
> Context: [Glossary](../CONTEXT.md), [context map](../CONTEXT-MAP.md), [session API ADR](../docs/adr/0001-reuse-session-api.md), [declarations ADR](../docs/adr/0002-widget-tree-declarations.md), [runtime ADR](../docs/adr/0003-rust-vendored-runtime.md), [scope ADR](../docs/adr/0004-spike-scope-and-exit.md), [renderer-neutral contracts ADR](../docs/adr/0005-renderer-neutral-client-contracts.md), [validation](../docs/spike-validation.md), [source inventory](../docs/planning-source-inventory.md)

## Slice 1: Local session compatibility

**Tracer bullet**: A Rust client exchanges input and terminal output with a vendored PTY runtime.
**Touches**: runtime crate, Rust client, session API compatibility
**Off-limits**: UI/plugin protocol and remote attach

### Task 1.1: Use a local PTY through the Rust client
<!-- estimate: ~30000 reviewable lines | feature-files: features/session.feature | demoable: yes | confidence: low | OVER SOFT CAP: scenario-atomic; use slice-split after implementation -->

<!-- DONE -->
# Living: features/session.feature::Use a local PTY through the Rust client
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Use a local PTY through the Rust client
  Given a named local session is started by the vendored core with one shell pane
  When I attach the Rust client with the default workspace focused and type "printf spike-ready" into its primary working pane
  Then the client’s primary focused working Surface is the live PTY-backed pane, it shows "spike-ready" from that PTY, and it accepts further shell input while plugin UI remains in reserved Zones

### Task 1.2: Existing session automation works without protocol changes
<!-- estimate: ~350 reviewable lines | feature-files: features/session-api.feature | demoable: yes -->

<!-- DONE -->
# Living: features/session-api.feature::Existing session automation works without protocol changes
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Existing session automation works without protocol changes
  Given the new core is built with the fork’s server-side session API reused verbatim and the fork’s unmodified session API client and compatibility fixtures are available
  When the client exercises the reference session methods, response schemas and event subscriptions against the new core
  Then each result and event satisfies the original session API contract without rewriting the client or translating the protocol

## Slice 2: Agent-tree acceptance consumer

**Tracer bullet**: Agent-tree is the first real consumer of the new widget-tree API, using its retained reactive store and stdin transport.
**Touches**: shared widget protocol, first renderer, agent-tree producer, retained store and actions
**Off-limits**: stock-UI consumers until agent-tree has emitted the first real declaration stream

### Task 2.1: Port the agent tree as the first widget-tree consumer
<!-- estimate: ~1350 reviewable lines | feature-files: features/agent-tree.feature | demoable: yes | confidence: low | OVER SOFT CAP: scenario-atomic; use slice-split after implementation -->

<!-- DONE -->
# Living: features/agent-tree.feature::Port the agent tree as the first widget-tree consumer
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Port the agent tree as the first widget-tree consumer
  Given no plugin has yet consumed the new widget-tree API, and the fork’s agent-tree producer and TanStack DB store are ported without replacing the store or stdin-envelope transport
  When I enable agent-tree as the first real plugin for a session with named workspaces and tabs
  Then the first real widget-declaration stream is emitted by agent-tree, its sidebar shows those workspaces and tabs with the fork’s status glyphs, roles and elastic leaders, and its focus-tab action focuses the selected tab

### Task 2.2: Expand and collapse workspace declarations
<!-- estimate: ~500 reviewable lines | feature-files: features/agent-tree.feature | demoable: yes -->

<!-- DONE -->
# Living: features/agent-tree.feature::Expand and collapse workspace declarations
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Expand and collapse workspace declarations
  Given a workspace in the agent tree contains three tabs
  When I collapse the workspace and expand it again
  Then its tabs disappear and reappear under that workspace and the collapsed header shows the hidden-tab count

<!-- DONE -->
# Living: features/agent-tree.feature::Refresh the tree from session events
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: Refresh the tree from session events
  Given the agent-tree producer has subscribed through the carried-over TanStack DB store and the tree shows the current session
  When the session emits <event> with the corresponding updated topology or status
  Then the visible widget tree reflects <change> without restarting the producer or client

  Examples:
    | event | change |
    | workspace.created | new workspace |
    | workspace.renamed | changed workspace name |
    | workspace.closed | removed workspace |
    | tab.created | new tab |
    | tab.closed | removed tab |
    | tab.focused | selected tab |
    | pane.agent_detected | detected agent |
    | pane.updated | changed agent status |

## Slice 3: Public declarations and stock UI

**Tracer bullet**: A stock producer declares sidebar and tabs through the same widget API as a third-party plugin.
**Touches**: widget protocol, renderer, producer supervision, stock UI, actions
**Off-limits**: session API contracts and plugin cell-grid rendering

### Task 3.1: Stock UI uses the public widget declaration path
<!-- estimate: ~300 reviewable lines | feature-files: features/declarations.feature | demoable: yes | confidence: low -->

<!-- DONE -->
# Living: features/declarations.feature::Stock UI uses the public widget declaration path
# Action: creates
# Status: DONE
# Living updated: YES
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

### Task 3.2: Compose each supported widget node
<!-- estimate: ~400 reviewable lines | feature-files: features/declarations.feature | demoable: yes -->

<!-- DONE -->
# Living: features/declarations.feature::Compose each supported widget node
# Action: creates
# Status: DONE
# Living updated: YES
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

### Task 3.3: Pin styled declarations into zones
<!-- estimate: ~440 reviewable lines | feature-files: features/declarations.feature | demoable: yes -->

<!-- DONE -->
# Living: features/declarations.feature::Pin styled declarations into zones
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Pin styled declarations into zones
  Given a plugin declares label, muted, leader and status-role segments with named tones in the list, footer and footer-right zones
  When the client projects those declarations into the terminal
  Then each declaration stays in its client-renderer-defined zone geometry and the client palette styles its roles and tones without plugin-supplied pixels

<!-- DONE -->
# Living: features/declarations.feature::Hot-apply a new widget declaration
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Hot-apply a new widget declaration
  Given the running client displays a producer’s widget tree
  When the producer emits new widget declarations by replacing its widget tree with changed text and nesting
  Then the visible UI changes to the replacement tree without rebuilding or restarting the client

### Task 3.4: Dispatch plugin actions through stdin envelopes
<!-- estimate: ~480 reviewable lines | feature-files: features/plugin-actions.feature | demoable: yes -->

<!-- DONE -->
# Living: features/plugin-actions.feature::Dispatch plugin actions through stdin envelopes
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Dispatch plugin actions through stdin envelopes
  Given a long-lived producer declares a focus-tab action for an unfocused tab
  When I activate that declaration
  Then the producer receives the correlated action and tab context on stdin and the requested tab becomes focused after its successful action result

<!-- DONE -->
# Living: features/plugin-actions.feature::Plugins place core-defined actions
# Action: creates
# Status: DONE
# Living updated: YES
Scenario Outline: Plugins place core-defined actions
  Given a plugin declares its own label and placement for the client-local <action> in the reserved core-action namespace
  When I activate the action, change its plugin label and placement, and activate it again while the plugin attempts to supply its own behavior
  Then only the core-defined <behavior> occurs, both before and after the change, and the plugin cannot initially define or later redefine those semantics

  Examples:
    | action | behavior |
    | new | new-tab interaction |
    | menu | menu interaction |

### Task 3.5: Consume shared UI and pane state independently of the terminal renderer
<!-- estimate: ~350 reviewable lines | feature-files: features/client-contracts.feature | demoable: yes -->

<!-- DONE -->
# Living: features/client-contracts.feature::Consume shared UI and pane state independently of the terminal renderer
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Consume shared UI and pane state independently of the terminal renderer
  Given the shared Core owns the live session state and action semantics, each client owns its renderer, and the Core publishes widget declarations and pane Surface state through the shared client contract
  When an independent protocol consumer decodes that stream using the documented contract without the terminal renderer
  Then it obtains the declared structure, action identities, semantic styling and pane terminal state, the declaration payload itself contains no ratatui types, terminal escape sequences or fixed terminal-cell geometry, and Core-owned state and action semantics remain independent of each client-owned renderer

## Slice 4: Session continuity

**Tracer bullet**: Disconnect and attach again without losing a live PTY; restore persisted topology on relaunch.
**Touches**: vendored persistence/restore and client attach
**Off-limits**: persistence format redesign and unrelated runtime refactors

### Task 4.1: Disconnect without losing the running session
<!-- estimate: ~240 reviewable lines | feature-files: features/session.feature | demoable: yes -->

<!-- DONE -->
# Living: features/session.feature::Disconnect without losing the running session
# Action: creates
# Status: DONE
# Living updated: YES
Scenario: Disconnect without losing the running session
  Given a named session has workspaces, tabs and panes including a running shell command
  When I disconnect the client and attach a new client to that session
  Then the workspaces, tabs and live panes remain and the command continues in the same PTY

### Task 4.2: Restore persisted session topology after relaunch
<!-- estimate: ~350 reviewable lines | feature-files: features/session.feature | demoable: yes -->

<!-- TODO -->
# Living: features/session.feature::Restore persisted session topology after relaunch
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Restore persisted session topology after relaunch
  Given a named session has persisted workspaces, tab layouts and pane working directories
  When the core stops and relaunches using that persistence
  Then the restored session contains those workspaces, tabs and pane layouts with usable replacement PTYs in their restored working directories

## Slice 5: Remote session and clipboard images

**Tracer bullet**: Attach from achy to one SSH session and paste an image into its focused pane.
**Touches**: remote transport, clipboard intercept, image staging
**Off-limits**: multi-endpoint aggregation and new security/authentication mechanisms

### Task 5.1: Attach from achy to one remote session
<!-- estimate: ~450 reviewable lines | feature-files: features/remote-attach.feature | demoable: yes | confidence: low -->

<!-- TODO -->
# Living: features/remote-attach.feature::Attach from achy to one remote session
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Attach from achy to one remote session
  Given the remote core owns a named session and achy has SSH access to it
  When I run the client with --remote on achy and it attaches over SSH to that session
  Then the client attaches to exactly that session and displays its live Surfaces while shell input executes on the remote core

### Task 5.2: Paste clipboard images into the focused pane
<!-- estimate: ~450 reviewable lines | feature-files: features/clipboard-images.feature | demoable: yes -->

<!-- TODO -->
# Living: features/clipboard-images.feature::Paste clipboard images into the focused pane
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Paste clipboard images into the focused pane
  Given I am attached in <mode> mode and an image is on the client clipboard
  When I paste into the focused pane
  Then the client intercepts the image, the session core stages it in /tmp/herdr-clipboard-images-UID/ on the server host where UID is the server user’s UID, and the staged path is injected only into that focused pane

  Examples:
    | mode |
    | local |
    | remote from achy |

## Slice 6: Latency and daily-driver exit

**Tracer bullet**: Prove the remote and local 100ms gates before switching daily drivers.
**Touches**: measurement harness and exit evidence report
**Off-limits**: synthetic-only acceptance and automatic fallback migration

### Task 6.1: Meet the local and remote click-to-repaint gate
<!-- estimate: ~400 reviewable lines | feature-files: features/latency.feature | demoable: yes -->

<!-- TODO -->
# Living: features/latency.feature::Meet the local and remote click-to-repaint gate
# Action: creates
# Status: TODO
# Living updated: NO
Scenario Outline: Meet the local and remote click-to-repaint gate
  Given 15 populated panes are running in <mode> mode on the recorded acceptance setup
  When click-to-repaint-15-panes measures repeated workspace, tab and pane focus clicks through the completed visible repaint
  Then every measured click-to-repaint sample is at most 100ms including producer and transport delays and the displayed Surface and focus state match the requested selection

  Examples:
    | mode |
    | local |
    | remote from achy |

### Task 6.2: Accept the spike only with complete daily-driver evidence
<!-- estimate: ~340 reviewable lines | feature-files: features/spike-exit.feature | demoable: yes -->

<!-- TODO -->
# Living: features/spike-exit.feature::Accept the spike only with complete daily-driver evidence
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Accept the spike only with complete daily-driver evidence
  Given Tommy’s actual session has been exercised end-to-end with stock UI fully plugin-declared, the ported agent tree, attach from achy and clipboard-image paste
  When the exit report evaluates that evidence and the passing local and remote 15-pane latency results within the 2–3 week spike
  Then the report marks the spike passed only when every exit requirement has demonstrated results

<!-- TODO -->
# Living: features/spike-exit.feature::Keep the fork when the spike fails its exit test
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Keep the fork when the spike fails its exit test
  Given at least one required daily-driver exit result is failing or absent
  When the exit report evaluates the spike
  Then the report marks the spike failed and directs work back to plugin development on the existing herdr fork rather than switching daily drivers

