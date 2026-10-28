use super::*;

use crate::protocol::endpoint::{EndpointPluginUi, EndpointPluginUiItem, EndpointPluginUiSegment};

fn plugin_rows() -> Vec<EndpointPluginUiItem> {
    vec![
        EndpointPluginUiItem {
            command_inputs: Vec::new(),
            declaration: None,
            plugin_id: "example.ui".into(),
            ui_id: "review".into(),
            title: "Review focus".into(),
            action: Some("focus".into()),
            tab_id: None,
            workspace_id: None,
            pane_id: None,
            segments: Vec::new(),
        },
        EndpointPluginUiItem {
            command_inputs: Vec::new(),
            declaration: None,
            plugin_id: "example.ui".into(),
            ui_id: "zoom".into(),
            title: "Zoom label".into(),
            action: None,
            tab_id: None,
            workspace_id: None,
            pane_id: None,
            segments: Vec::new(),
        },
    ]
}

fn state_with_plugin_rows() -> ClientShellState {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: plugin_rows(),
        },
    );
    state
}

fn state_with_many_plugin_rows(count: usize, focused_tab: Option<&str>) -> ClientShellState {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    let rows = (0..count)
        .map(|index| EndpointPluginUiItem {
            command_inputs: Vec::new(),
            declaration: None,
            plugin_id: "example.ui".into(),
            ui_id: "tree".into(),
            title: format!("row-{index:02}"),
            action: None,
            tab_id: (index % 3 == 0).then(|| format!("tab-{index}")),
            workspace_id: None,
            pane_id: None,
            segments: Vec::new(),
        })
        .collect();
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    if let Some(tab) = focused_tab {
        let mut view = snapshot();
        view.focused_tab_id = Some(tab.into());
        state.set_snapshot(Box::new(view));
    }
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: rows,
        },
    );
    state
}

#[test]
fn plugin_tree_overflow_scrolls_and_clamps() {
    let mut state = state_with_many_plugin_rows(40, None);
    state.compose(106, 20).expect("expanded sidebar");
    let visible = state.hits.plugin_ui.len();
    assert!(visible < 40, "overflow is sliced to the visible area");
    assert!(state.hits.workspace_max_scroll > 0, "scroll range exposed");
    assert!(
        !state.hits.workspace_scrollbar.is_empty(),
        "scrollbar drawn"
    );
    assert_eq!(state.hits.plugin_ui[0].rect.y, frame_sidebar_top(&state));

    // Wheel down the whole range: the last rows become visible and the
    // offset clamps instead of running past the end.
    state.workspace_scroll = usize::MAX;
    state.compose(106, 20).expect("expanded sidebar");
    let last = state.hits.plugin_ui.last().expect("rows remain").rect.y;
    let body_bottom = state.hits.workspace_body.bottom();
    assert!(
        last < body_bottom,
        "clamped scroll keeps rows inside the body"
    );
    assert_eq!(
        state.hits.plugin_ui.len(),
        visible,
        "full-height window at the bottom of the list"
    );
}

#[test]
fn plugin_tree_reveals_the_focused_tab_row() {
    let mut state = state_with_many_plugin_rows(40, Some("tab-39"));
    state.reveal_focused_workspace = true;
    state.compose(106, 20).expect("expanded sidebar");
    let focused_row = state
        .hits
        .plugin_ui
        .iter()
        .find(|hit| hit.tab_id.as_deref() == Some("tab-39"))
        .expect("focused tab row is visible after reveal");
    let body = state.hits.workspace_body;
    assert!(focused_row.rect.y >= body.y && focused_row.rect.y < body.bottom());
}

fn frame_sidebar_top(state: &ClientShellState) -> u16 {
    state.hits.workspace_body.y.saturating_add(1)
}

#[test]
fn expanded_sidebar_renders_plugin_rows_with_hits() {
    let mut state = state_with_plugin_rows();
    let frame = state.compose(106, 20).expect("expanded sidebar");
    assert_eq!(state.hits.plugin_ui.len(), 2);
    let review = state.hits.plugin_ui[0].rect;
    assert!(review.width > 0 && review.height == 1);
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let (x, y) = cell_symbol_position(&frame, review, "Review");
    assert_eq!(buffer.cell((x, y)).expect("title cell").symbol(), "R");
}

#[test]
fn clicking_plugin_row_invokes_declared_action() {
    let mut state = state_with_plugin_rows();
    state.compose(106, 20).expect("expanded sidebar");
    let review = state.hits.plugin_ui[0].rect;
    let press = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: review.x + 1,
        row: review.y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    })]);
    assert!(matches!(
        &press.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::PluginActionInvoke(params)
                    if params.action_id == "focus"
                        && params.plugin_id.as_deref() == Some("example.ui")
            )
    ));
}

#[test]
fn plugin_row_without_action_is_not_clickable() {
    let mut state = state_with_plugin_rows();
    state.compose(106, 20).expect("expanded sidebar");
    let zoom = state.hits.plugin_ui[1].rect;
    let press = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: zoom.x + 1,
        row: zoom.y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    })]);
    assert!(!press.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(request.method, crate::api::schema::Method::PluginActionInvoke(_))
    )));
}

#[test]
fn clicking_dynamic_row_sends_row_context() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: vec![EndpointPluginUiItem {
                command_inputs: Vec::new(),
                declaration: None,
                plugin_id: "tree.plugin".into(),
                ui_id: "tree".into(),
                title: "    ● build".into(),
                action: Some("focus-tab".into()),
                tab_id: Some("tab-9".into()),
                workspace_id: Some("ws-2".into()),
                pane_id: None,
                segments: Vec::new(),
            }],
        },
    );
    state.compose(106, 20).expect("expanded sidebar");
    let row = state.hits.plugin_ui[0].rect;
    let press = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: row.x + 1,
        row: row.y,
        modifiers: crossterm::event::KeyModifiers::empty(),
    })]);
    assert!(matches!(
        &press.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::PluginActionInvoke(params)
                    if params.action_id == "focus-tab"
                        && params.context.as_ref().and_then(|c| c.tab_id.as_deref()) == Some("tab-9")
                        && params.context.as_ref().and_then(|c| c.workspace_id.as_deref())
                            == Some("ws-2")
                        && params.context.as_ref().and_then(|c| c.invocation_source.as_deref())
                            == Some("plugin_ui")
            )
    ));
}

#[test]
fn focused_tab_row_carries_active_row_background() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let row =
        |ui_id: &str, tab_id: Option<&str>, workspace_id: Option<&str>| EndpointPluginUiItem {
            command_inputs: Vec::new(),
            declaration: None,
            plugin_id: "tree.plugin".into(),
            ui_id: ui_id.into(),
            title: "row".into(),
            action: Some("focus-tab".into()),
            tab_id: tab_id.map(Into::into),
            workspace_id: workspace_id.map(Into::into),
            pane_id: None,
            segments: Vec::new(),
        };
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: vec![
                row("header", None, Some("ws_1")),
                row("focused", Some("tab_1"), Some("ws_1")),
                row("other", Some("tab_2"), Some("ws_1")),
            ],
        },
    );
    let frame = state.compose(106, 20).expect("expanded sidebar");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let expected = state.config.palette.active_row_bg;
    let bg_of = |index: usize| {
        let rect = state.hits.plugin_ui[index].rect;
        buffer[(rect.x, rect.y)].bg
    };
    assert_eq!(bg_of(1), expected, "focused tab row highlighted");
    assert_ne!(
        bg_of(0),
        expected,
        "header not highlighted while its focused tab is visible"
    );
    assert_ne!(bg_of(2), expected, "unfocused tab row not highlighted");
}

#[test]
fn workspace_header_carries_focus_background_when_focused_tab_row_is_absent() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: vec![EndpointPluginUiItem {
                command_inputs: Vec::new(),
                declaration: None,
                plugin_id: "tree.plugin".into(),
                ui_id: "header".into(),
                title: "ws_1        ···· ○ 1 hidden".into(),
                action: Some("open-workspace".into()),
                tab_id: None,
                workspace_id: Some("ws_1".into()),
                pane_id: None,
                segments: Vec::new(),
            }],
        },
    );
    let frame = state.compose(106, 20).expect("expanded sidebar");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let rect = state.hits.plugin_ui[0].rect;
    assert_eq!(
        buffer[(rect.x, rect.y)].bg,
        state.config.palette.active_row_bg
    );
}

#[test]
fn segments_render_with_role_colors_and_unknown_roles_fall_back() {
    use crate::protocol::endpoint::EndpointPluginUiSegment;
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    // (palette fields are read inline below: Color is Copy, Palette is not)
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let segment = |text: &str, role: &str| EndpointPluginUiSegment {
        text: text.into(),
        role: role.into(),
    };
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: vec![
                EndpointPluginUiItem {
                    command_inputs: Vec::new(),
                    declaration: None,
                    plugin_id: "tree.plugin".into(),
                    ui_id: "cluster".into(),
                    title: "infilla ···· ● ◉".into(),
                    action: Some("open-workspace".into()),
                    tab_id: None,
                    workspace_id: Some("ws_9".into()),
                    pane_id: None,
                    segments: vec![
                        segment("infilla ", "label"),
                        segment("···· ", "rule"),
                        segment("●", "status-working"),
                        segment(" ", "label"),
                        segment("◉", "status-blocked"),
                        segment(" 2 hidden", "muted"),
                    ],
                },
                EndpointPluginUiItem {
                    command_inputs: Vec::new(),
                    declaration: None,
                    plugin_id: "tree.plugin".into(),
                    ui_id: "unknown-role".into(),
                    title: "fallback".into(),
                    action: Some("focus-tab".into()),
                    tab_id: None,
                    workspace_id: Some("ws_9".into()),
                    pane_id: None,
                    segments: vec![segment("plain", "status-exotic")],
                },
            ],
        },
    );
    let frame = state.compose(106, 20).expect("expanded sidebar");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");

    let cluster = state.hits.plugin_ui[0].rect;
    let base_accent = state.config.palette.accent;
    // Walk the cluster row and verify each run's foreground color.
    let cells: Vec<(char, ratatui::style::Color)> = (cluster.x..cluster.x + 20)
        .map(|x| {
            let cell = &buffer[(x, cluster.y)];
            (cell.symbol().chars().next().unwrap_or(' '), cell.fg)
        })
        .collect();
    let color_of = |needle: char| {
        cells
            .iter()
            .find(|(symbol, _)| *symbol == needle)
            .map(|(_, color)| *color)
            .unwrap_or(ratatui::style::Color::Reset)
    };
    assert_eq!(
        color_of('●'),
        state.config.palette.yellow,
        "working glyph colored"
    );
    assert_eq!(
        color_of('◉'),
        state.config.palette.red,
        "blocked glyph colored"
    );
    assert_eq!(
        color_of('·'),
        state.config.palette.overlay1,
        "rule run visible against the background"
    );
    assert_eq!(color_of('n'), base_accent, "label run keeps base style");

    let unknown = state.hits.plugin_ui[1].rect;
    let cell = &buffer[(unknown.x + 1, unknown.y)];
    assert_eq!(cell.fg, base_accent);
}

#[test]
fn bg_segment_role_paints_the_row_background() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: vec![EndpointPluginUiItem {
                command_inputs: Vec::new(),
                declaration: None,
                plugin_id: "lime.plugin".into(),
                ui_id: "tree".into(),
                title: " lime sidebar".into(),
                action: None,
                tab_id: None,
                workspace_id: None,
                pane_id: None,
                segments: vec![
                    EndpointPluginUiSegment {
                        text: String::new(),
                        role: "bg-lime".into(),
                    },
                    EndpointPluginUiSegment {
                        text: " lime sidebar".into(),
                        role: "muted".into(),
                    },
                ],
            }],
        },
    );
    let frame = state.compose(106, 20).expect("expanded sidebar");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let rect = state.hits.plugin_ui[0].rect;
    for x in rect.x..rect.right() {
        assert_eq!(
            buffer[(x, rect.y)].bg,
            ratatui::style::Color::Rgb(50, 205, 50),
            "whole row painted lime at column {x}"
        );
    }
    // The declared tone also claims the whole column body: every row from the
    // divider down to the row above the footer is lime, empty rows included.
    let body = state.hits.workspace_body;
    for y in body.y..body.bottom() {
        assert_eq!(
            buffer[(body.x, y)].bg,
            ratatui::style::Color::Rgb(50, 205, 50),
            "column body painted lime at row {y}"
        );
    }
}

#[test]
fn footer_role_rows_pin_to_the_footer_line_and_replace_the_default_footer() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: vec![
                EndpointPluginUiItem {
                    command_inputs: Vec::new(),
                    declaration: None,
                    plugin_id: "tree.plugin".into(),
                    ui_id: "tree".into(),
                    title: "header".into(),
                    action: None,
                    tab_id: None,
                    workspace_id: None,
                    pane_id: None,
                    segments: Vec::new(),
                },
                EndpointPluginUiItem {
                    command_inputs: Vec::new(),
                    declaration: None,
                    plugin_id: "tree.plugin".into(),
                    ui_id: "tree".into(),
                    title: " new".into(),
                    action: Some("host:new".into()),
                    tab_id: None,
                    workspace_id: None,
                    pane_id: None,
                    segments: vec![EndpointPluginUiSegment {
                        text: " new".into(),
                        role: "footer".into(),
                    }],
                },
            ],
        },
    );
    state.compose(106, 20).expect("expanded sidebar");
    // The footer row left the scroll list...
    assert_eq!(state.hits.plugin_ui.len(), 2, "list row and footer row");
    let body = state.hits.workspace_body;
    let footer_hit = &state.hits.plugin_ui[1];
    assert!(
        footer_hit.rect.y >= body.bottom(),
        "footer-row pinned to the footer line, below the body"
    );
    assert_eq!(footer_hit.action.as_deref(), Some("host:new"));
    // ...and the host's default new-workspace affordance is suppressed.
    assert!(
        state.hits.new_workspace.is_empty(),
        "default footer suppressed while the plugin owns it"
    );
}

#[test]
fn plugin_tree_replaces_native_workspace_rows() {
    let mut state = state_with_plugin_rows();
    state.compose(106, 20).expect("expanded sidebar");
    assert!(
        state.hits.workspaces.is_empty(),
        "native workspace rows suppressed while the plugin tree owns the sidebar"
    );
    assert_eq!(state.hits.plugin_ui.len(), 2);
    assert!(
        state.hits.workspace_scrollbar.is_empty(),
        "native list scrollbar hidden"
    );
}

#[test]
fn native_workspace_rows_return_when_plugin_ui_is_empty() {
    let mut state = state_with_plugin_rows();
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 2,
            ui: Vec::new(),
        },
    );
    state.compose(106, 20).expect("expanded sidebar");
    assert!(
        !state.hits.workspaces.is_empty(),
        "native rows render as the fallback when the plugin projects no rows"
    );
    assert!(state.hits.plugin_ui.is_empty());
}

#[test]
fn leader_rows_fill_with_dots_and_right_align_the_cluster() {
    use crate::protocol::endpoint::EndpointPluginUiSegment;
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let segment = |text: &str, role: &str| EndpointPluginUiSegment {
        text: text.into(),
        role: role.into(),
    };
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 1,
            ui: vec![EndpointPluginUiItem {
                command_inputs: Vec::new(),
                declaration: None,
                plugin_id: "tree.plugin".into(),
                ui_id: "tree".into(),
                title: "kizn ·· ● 2 hidden".into(),
                action: Some("open-workspace".into()),
                tab_id: None,
                workspace_id: Some("ws_9".into()),
                pane_id: None,
                segments: vec![
                    segment("kizn", "label"),
                    segment(" ", "leader"),
                    segment("● ", "status-working"),
                    segment("2 hidden", "muted"),
                ],
            }],
        },
    );
    let frame = state.compose(106, 20).expect("expanded sidebar");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let rect = state.hits.plugin_ui[0].rect;
    let row: String = (rect.x..rect.right())
        .map(|x| buffer[(x, rect.y)].symbol().to_string())
        .collect();
    let trimmed = row.trim();
    assert!(
        trimmed.starts_with("kizn"),
        "label hugs the left edge: {row:?}"
    );
    assert!(
        trimmed.ends_with("2 hidden"),
        "trailing run right-aligns at the edge: {row:?}"
    );
    let dots = row.matches('·').count();
    assert!(dots >= 4, "leader fills the middle with dots: {row:?}");
}

#[test]
fn focused_plugin_row_renders_bold() {
    let mut state = state_with_plugin_rows();
    state.set_endpoint_plugin_ui(
        &ClientEndpointId::Local,
        EndpointPluginUi {
            boot_id: "boot-1".into(),
            revision: 2,
            ui: vec![EndpointPluginUiItem {
                command_inputs: Vec::new(),
                declaration: None,
                plugin_id: "example.ui".into(),
                ui_id: "review".into(),
                title: "Review focus".into(),
                action: Some("focus".into()),
                tab_id: Some("tab_1".into()),
                workspace_id: Some("ws_1".into()),
                pane_id: None,
                segments: Vec::new(),
            }],
        },
    );
    let frame = state.compose(106, 20).expect("expanded sidebar");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let rect = state.hits.plugin_ui[0].rect;
    let cell = &buffer[(rect.x + 1, rect.y)];
    assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
}

#[test]
fn plugin_tree_owns_the_sidebar_in_multi_endpoint_sessions() {
    use crate::client::endpoint::{
        ClientEndpointId, ClientEndpointStatus, ProfileId, SavedSshEndpoint,
    };

    let mut state = state_with_plugin_rows();
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.workspaces[0].label = "remote-workspace".into();
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let frame = state.compose(106, 20).expect("expanded sidebar");
    assert_eq!(
        state.hits.plugin_ui.len(),
        2,
        "plugin tree renders while a remote endpoint is attached"
    );
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let mut sidebar = String::new();
    for y in 0..buffer.area.height {
        for x in 0..30u16 {
            sidebar.push_str(buffer[(x, y)].symbol());
        }
    }
    assert!(
        !sidebar.contains("remote-workspace"),
        "the plugin-owned tree replaces the aggregate endpoint sidebar"
    );
}
