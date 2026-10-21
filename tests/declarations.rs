#[path = "support/styled_cases.rs"]
mod styled_cases;
#[path = "support/widget_cases.rs"]
mod widget_cases;
use serde_json::{json, Value};
use std::process::Command;
use trailboss::{
    core::Core,
    terminal::{self, Rect},
    Cell, Surface,
};
use trailboss_runtime::protocol::widgets::{Context, Declaration};
#[test]
fn sidebar_menu_exports_validate_against_their_actual_manifest_actions() {
    let output = Command::new("bun")
        .args(["-e", "import {declareMenus} from './plugins/sidebar-menus/menus.ts'; import {snapshotRows} from './packages/trailboss-store/adapter.ts'; const manifest=Bun.TOML.parse(await Bun.file('./plugins/sidebar-menus/trailboss-plugin.toml').text()); const entities=snapshotRows({type:'session_snapshot',snapshot:{workspaces:[{workspace_id:'w',label:'Space',number:1,focused:true,agent_status:'idle'}],tabs:[{workspace_id:'w',tab_id:'t',label:'Tab',number:1,focused:true,agent_status:'idle'}],panes:[]}}); console.log(JSON.stringify({declaration:declareMenus(entities),actions:manifest.actions.map(action=>action.id)}));"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let declaration: Declaration = serde_json::from_value(value["declaration"].clone()).unwrap();
    let actions = value["actions"].as_array().unwrap();
    assert!(declaration.validate(|id| actions.iter().any(|action| action.as_str() == Some(id))));
    assert!(
        !declaration.validate(|_| false),
        "close-space must be manifest-owned"
    );
}

fn at(row: &str, column: usize) -> String {
    row.chars().skip(column).collect()
}
fn stock_declaration(live: bool) -> Declaration {
    let entities = if live {
        json!([
            {"id":"workspace:w1","kind":"workspace","workspaceId":"w1","tabId":null,"paneId":null,"label":"Alpha","number":1,"position":0,"focused":true,"agentStatus":"working","agent":null,"cwd":null},
            {"id":"tab:t1","kind":"tab","workspaceId":"w1","tabId":"t1","paneId":null,"label":"Build","number":1,"position":0,"focused":true,"agentStatus":"working","agent":null,"cwd":null},
            {"id":"pane:p1","kind":"pane","workspaceId":"w1","tabId":"t1","paneId":"p1","label":"shell","number":0,"position":0,"focused":true,"agentStatus":"working","agent":null,"cwd":null}
        ])
    } else {
        json!([])
    };
    let home = std::env::temp_dir().join(format!(
        "stock-projection-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let mut command = Command::new("bun");
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        command.env(key, &home);
    }
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
        command.env_remove(key);
    }
    command.env("TRAILBOSS_CONFIG_PATH", home.join("config.toml"));
    let output = command.args(["-e", &format!("import {{declareStock}} from './plugins/agent-tree/stock.ts'; console.log(JSON.stringify(declareStock({entities})))")]).current_dir(env!("CARGO_MANIFEST_DIR")).output().unwrap();
    std::fs::remove_dir_all(home).unwrap();
    assert!(
        output.status.success(),
        "producer: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn surface() -> Surface {
    Surface {
        pane_id: "p1".into(),
        width: 1,
        height: 1,
        cells: vec![Cell {
            symbol: "X".into(),
            foreground: 0,
            background: 0,
            modifiers: 0,
            hyperlink: None,
        }],
        hyperlinks: Vec::new(),
        graphics: Default::default(),
        graphics_scope: String::new(),
        cursor: None,
        focused: true,
        mouse_reporting: false,
    }
}
fn screen(core: &Core, declaration: Option<Declaration>) -> Vec<String> {
    screen_width(core, declaration, 80)
}
fn screen_width(core: &Core, declaration: Option<Declaration>, width: u16) -> Vec<String> {
    let declarations = declaration
        .into_iter()
        .map(|d| ("arbitrary.public-plugin".into(), d))
        .collect();
    let mut bytes = Vec::new();
    terminal::FrameRenderer::default()
        .draw_frame(
            &mut bytes,
            Some(&surface()),
            &declarations,
            core,
            Rect::viewport(width, 24),
        )
        .unwrap();
    let mut screen = ghostty_vt::Terminal::new(width, 24, 0).unwrap();
    screen.write(&bytes);
    screen
        .screen_text_rows()
        .unwrap()
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|c| {
                    c.graphemes
                        .first()
                        .and_then(|c| char::from_u32(*c))
                        .unwrap_or(' ')
                })
                .collect()
        })
        .collect()
}
#[test]
fn scoped_menu_commands_never_substitute_the_selected_workspace_or_tab() {
    use trailboss::core::Effect;
    let mut core = Core::default();
    core.update_session(Some("selected".into()), Some("selected-tab".into()), true);
    let context = Context {
        workspace_id: Some("other".into()),
        tab_id: Some("other-tab".into()),
        ..Default::default()
    };
    for (action, expected_method, expected_params) in [
        (
            "host:new-tab",
            "tab.create",
            json!({"workspace_id":"other","label":"New tab","focus":true}),
        ),
        ("host:close-tab", "tab.close", json!({"tab_id":"other-tab"})),
    ] {
        let Effect::Request { method, params } = core.invoke(action, &context, None) else {
            panic!("missing scoped request")
        };
        assert_eq!(method, expected_method);
        assert_eq!(params, expected_params);
        assert!(matches!(
            core.invoke(action, &Context::default(), None),
            Effect::Consumed
        ));
    }
    assert!(matches!(
        core.invoke("host:detach", &context, None),
        Effect::Consumed
    ));
    assert!(core.detach_requested);
}

#[test]
fn rename_keeps_its_nonselected_target_when_focus_changes_and_empty_submit_stays_open() {
    use trailboss::core::Effect;
    use trailboss_runtime::protocol::widgets::Condition;
    for (action, condition, method, key) in [
        (
            "host:rename-space",
            Condition::RenameSpace,
            "workspace.rename",
            "workspace_id",
        ),
        (
            "host:rename-tab",
            Condition::RenameTab,
            "tab.rename",
            "tab_id",
        ),
    ] {
        let mut core = Core::default();
        core.update_session(Some("selected".into()), Some("selected-tab".into()), true);
        core.invoke(
            action,
            &Context {
                workspace_id: Some("other".into()),
                tab_id: Some("other".into()),
                ..Default::default()
            },
            None,
        );
        assert!(matches!(core.text("\r"), Some(Effect::Consumed)));
        assert!(core.visible(condition));
        core.update_session(Some("changed".into()), Some("changed-tab".into()), true);
        core.text("Renamed 🦀");
        let Some(Effect::Request {
            method: actual_method,
            params,
        }) = core.text("\r")
        else {
            panic!("rename did not submit")
        };
        assert_eq!(actual_method, method);
        assert_eq!(params, json!({key:"other","label":"Renamed 🦀"}));
        assert!(!core.visible(condition));
    }
}

#[test]
fn edits_consume_unicode_keys_and_safe_paste_until_escape_returns_input_to_the_pane() {
    use trailboss::core::Effect;
    use trailboss_runtime::protocol::widgets::Condition;
    for action in ["host:new", "host:rename-space", "host:rename-tab"] {
        let mut core = Core::default();
        core.update_session(Some("w1".into()), Some("t1".into()), true);
        core.invoke(
            action,
            &Context {
                workspace_id: Some("w1".into()),
                tab_id: Some("t1".into()),
                ..Default::default()
            },
            None,
        );
        assert!(matches!(core.text("é🦀"), Some(Effect::Consumed)));
        assert!(matches!(core.text("\x7f"), Some(Effect::Consumed)));
        assert_eq!(core.input_text, "é");
        for control in ["\r", "\x1b", "\x7f", "bad\ntext"] {
            assert!(matches!(core.paste(control), Some(Effect::Consumed)));
            assert_eq!(core.input_text, "é");
        }
        core.paste(&"界".repeat(100));
        assert_eq!(core.input_text.chars().count(), 80);
        assert!(matches!(core.text("\x1b"), Some(Effect::Consumed)));
        assert!(core.input_text.is_empty());
        assert!(!core.visible(Condition::NewTab));
        assert!(!core.visible(Condition::RenameSpace));
        assert!(!core.visible(Condition::RenameTab));
        assert!(core.text("pty input").is_none());
    }
}

#[test]
fn new_tab_edit_keeps_its_original_workspace_after_focus_changes() {
    use trailboss::core::Effect;
    let mut core = Core::default();
    core.update_session(Some("original".into()), None, true);
    core.invoke("host:new", &Context::default(), None);
    core.update_session(Some("changed".into()), None, true);
    let Some(Effect::Request { method, params }) = core.text("\r") else {
        panic!("new tab did not submit")
    };
    assert_eq!(method, "tab.create");
    assert_eq!(
        params,
        json!({"workspace_id":"original","label":"New tab","focus":true})
    );
}

#[test]
fn reserved_actions_keep_stock_requests_controls_and_plugin_validation() {
    use trailboss::core::Effect;
    let context = Context {
        workspace_id: Some("captured-space".into()),
        tab_id: Some("captured-tab".into()),
        ..Default::default()
    };
    for (action, method, params) in [
        (
            "host:new-space",
            "workspace.create",
            json!({"label":"New space","focus":true}),
        ),
        (
            "host:new-tab",
            "tab.create",
            json!({"workspace_id":"captured-space","label":"New tab","focus":true}),
        ),
        (
            "host:close-tab",
            "tab.close",
            json!({"tab_id":"captured-tab"}),
        ),
        ("host:focus", "tab.focus", json!({"tab_id":"captured-tab"})),
        ("host:restart-idle-agents", "agent.restart_idle", json!({})),
    ] {
        let mut core = Core::default();
        core.update_session(Some("other-space".into()), Some("other-tab".into()), true);
        let Effect::Request {
            method: actual,
            params: actual_params,
        } = core.invoke(action, &context, None)
        else {
            panic!("{action} did not issue its stock request")
        };
        assert_eq!((actual, actual_params), (method, params), "{action}");
        assert!(core.text("unrelated PTY input").is_none(), "{action}");
    }
    for workspace in [None, Some("native-space")] {
        let mut core = Core::default();
        core.update_session(workspace.map(str::to_owned), None, true);
        core.invoke("host:new", &context, None);
        core.update_session(Some("later-space".into()), None, true);
        assert!(matches!(core.text("Entered"), Some(Effect::Consumed)));
        let Some(Effect::Request { method, params }) = core.text("\r") else {
            panic!("name interaction did not submit")
        };
        let expected = match workspace {
            Some(id) => (
                "tab.create",
                json!({"workspace_id":id,"label":"Entered","focus":true}),
            ),
            None => ("workspace.create", json!({"label":"Entered","focus":true})),
        };
        assert_eq!((method, params), expected);
        assert!(!core.editing());
    }
    for action in [
        "host:new-space",
        "host:new",
        "host:new-tab",
        "host:rename-space",
        "host:rename-tab",
        "host:close-tab",
        "host:focus",
        "host:restart-idle-agents",
        "host:detach",
        "host:create",
        "host:cancel",
        "plugin-action",
        "host:unknown",
        "host:Focus",
        "focus",
    ] {
        let declaration: Declaration = serde_json::from_value(json!({
            "herdr":"widget_tree","version":1,"zones":[],
            "fragments":[{"name":"reserved-test","priority":0,"tree":{"type":"row","children":[],
                "action":action}}]
        }))
        .unwrap();
        let reserved =
            action.starts_with("host:") && !matches!(action, "host:unknown" | "host:Focus");
        assert_eq!(declaration.validate(|_| false), reserved, "{action}");
        assert_eq!(
            declaration.validate(|_| true),
            !action.starts_with("host:") || reserved,
            "{action}"
        );
    }
    let mut core = Core::default();
    core.update_session(None, None, true);
    assert!(matches!(
        core.invoke("plugin-action", &context, None),
        Effect::Plugin
    ));
    assert!(matches!(
        core.invoke("host:unknown", &context, None),
        Effect::Plugin
    ));
    core.invoke("host:new", &context, None);
    core.text("Cancelled");
    core.invoke("host:cancel", &context, None);
    assert!(!core.editing());
    assert!(core.input_text.is_empty());
    assert!(core.text("PTY input").is_none());
    assert!(matches!(
        core.invoke("host:create", &context, None),
        Effect::Consumed
    ));
}

#[test]
fn public_declarations_are_the_only_visible_composition_in_all_four_states() {
    for (state, action, pane_y) in [
        ("no pane yet", None, None),
        ("live session", None, Some(0)),
        ("new-tab interaction open", Some("host:new"), Some(0)),
        (
            "rename interaction open",
            Some("host:rename-space"),
            Some(0),
        ),
    ] {
        let live = state != "no pane yet";
        let declaration = stock_declaration(live);
        assert!(
            declaration.validate(|_| false),
            "public producer rejected: {state}"
        );
        let mut core = Core::default();
        core.update_session(live.then(|| "w1".into()), live.then(|| "t1".into()), live);
        if let Some(action) = action {
            core.invoke(
                action,
                &Context {
                    workspace_id: Some("w1".into()),
                    ..Default::default()
                },
                None,
            );
        }
        let original = screen(&core, Some(declaration.clone()));
        if !live {
            assert!(
                at(&original[0], 20).starts_with(" No pane yet"),
                "no-pane screen: {original:?}"
            );
        }
        if let Some(y) = pane_y {
            assert_eq!(
                &at(&original[y], 20).chars().next().unwrap().to_string(),
                "X",
                "{state}: Surface placement"
            );
        } else {
            assert!(!original.iter().any(|row| row.contains('X')));
        }
        // An independently-authored arrangement must move every visible group, including the Surface.
        let mut changed: Value = serde_json::to_value(&declaration).unwrap();
        changed["composition"]["tree"]["children"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let moved: Declaration = serde_json::from_value(changed).unwrap();
        let swapped = screen(&core, Some(moved));
        if let Some(y) = pane_y {
            assert!(
                swapped[y].starts_with('X'),
                "{state}: declaration did not move Surface"
            );
        } else {
            assert!(swapped[0].starts_with(" No pane yet"));
        }
        let mut without_root = declaration;
        without_root.composition = None;
        for rows in [screen(&core, None), screen(&core, Some(without_root))] {
            assert!(
                rows.iter().all(|row| row.trim().is_empty()),
                "{state}: undeclared privileged UI: {rows:?}"
            );
        }
    }
}

#[test]
fn shared_list_producers_keep_visible_rows_and_action_identity() {
    let declaration = |label: &str, action: &str, root: bool| -> Declaration {
        serde_json::from_value(json!({"herdr":"widget_tree","version":1,
            "zones":[{"zone":"list","tree":{"type":"column","children":[
                {"type":"row","children":[{"type":"label","text":label,"role":"label"}],"action":action,"context":{"tab_id":label}}
            ]}}], "composition":root.then(||json!({"priority":10,"tree":{"type":"zone","zone":"list"}}))
        })).unwrap()
    };
    let declarations = vec![
        (
            "first-producer".into(),
            declaration("First row", "first-action", true),
        ),
        (
            "second-producer".into(),
            declaration("Second row", "second-action", false),
        ),
    ];
    assert!(declarations.iter().all(|(_, d)| d.validate(|_| true)));
    let core = Core::default();
    let viewport = Rect::viewport(80, 24);
    let mut bytes = Vec::new();
    terminal::FrameRenderer::default()
        .draw_frame(&mut bytes, None, &declarations, &core, viewport)
        .unwrap();
    let mut terminal = ghostty_vt::Terminal::new(80, 24, 0).unwrap();
    terminal.write(&bytes);
    let rows = terminal.screen_text_rows().unwrap();
    for (y, plugin, action, label) in [
        (0, "first-producer", "first-action", "First row"),
        (1, "second-producer", "second-action", "Second row"),
    ] {
        let text: String = rows[y]
            .cells
            .iter()
            .flat_map(|cell| cell.graphemes.iter())
            .filter_map(|cp| char::from_u32(*cp))
            .collect();
        assert!(
            text.starts_with(label),
            "shared Zone erased {label}: {text}"
        );
        let (actual_plugin, actual_action, context) =
            terminal::action_at(&core, &declarations, viewport, 0, 0, y as u16)
                .expect("declared row must be actionable");
        assert_eq!(
            (actual_plugin, actual_action, context.tab_id.as_deref()),
            (plugin, action, Some(label))
        );
    }
}

fn assert_nested_widget(node: &str) {
    assert!(widget_cases::NODES.contains(&node));
    let declaration: Declaration = serde_json::from_value(widget_cases::declaration(node))
        .unwrap_or_else(|error| panic!("{node}: {error}"));
    assert!(
        declaration.validate(|_| false),
        "{node}: rejected declaration"
    );
    for width in [1, 2, 4, 13, 80] {
        widget_cases::assert_rows(
            node,
            &screen_width(&Core::default(), Some(declaration.clone()), width),
            usize::from(width),
        );
    }
}
macro_rules! widget_test {
    ($name:ident, $node:literal) => {
        #[test]
        fn $name() {
            assert_nested_widget($node);
        }
    };
}
widget_test!(nested_widget_row, "row");
widget_test!(nested_widget_column, "column");
widget_test!(nested_widget_label, "label");
widget_test!(nested_widget_leader, "leader");
widget_test!(nested_widget_spacer, "spacer");
widget_test!(nested_widget_list, "list");

#[test]
fn styled_declarations_stay_in_zones_with_client_palette() {
    // Palette decoding must not depend on the runner's NO_COLOR policy.
    crossterm::style::force_color_output(true);
    let declaration: Declaration =
        serde_json::from_value(styled_cases::declaration()).expect("styled public declaration");
    assert!(declaration.validate(|_| false));
    let declarations = vec![
        ("stock".into(), stock_declaration(false)),
        ("styles".into(), declaration),
    ];
    for (width, height, focused) in [
        (80, 24, false),
        (52, 11, false),
        (80, 24, true),
        (52, 11, true),
    ] {
        let mut core = Core::default();
        core.update_session(None, focused.then(|| "padding-focus".into()), false);
        let mut bytes = Vec::new();
        terminal::FrameRenderer::default()
            .draw_frame(
                &mut bytes,
                None,
                &declarations,
                &core,
                Rect::viewport(width, height),
            )
            .unwrap();
        let mut screen = ghostty_vt::Terminal::new(width, height, 0).unwrap();
        screen.write(&bytes);
        styled_cases::assert_screen(&screen, width, height, focused);
    }
    let mut short = styled_cases::declaration();
    short["zones"][2]["tree"] = json!({"type":"label","text":"R","role":"muted"});
    let short: Declaration = serde_json::from_value(short).unwrap();
    let mut root = stock_declaration(false);
    root.zones = short.zones;
    let rows = screen(&Core::default(), Some(root));
    assert_eq!(
        rows[23].chars().take(20).collect::<String>(),
        format!("{}R", " ".repeat(19))
    );
    for field in ["x", "width", "pixels"] {
        let mut invalid = styled_cases::declaration();
        invalid["zones"][0]["tree"][field] = json!(12);
        assert!(
            serde_json::from_value::<Declaration>(invalid).is_err(),
            "accepted plugin geometry: {field}"
        );
    }
    let mut duplicate = styled_cases::declaration();
    duplicate["zones"][2]["zone"] = json!("list");
    assert!(!serde_json::from_value::<Declaration>(duplicate)
        .unwrap()
        .validate(|_| false));
}
