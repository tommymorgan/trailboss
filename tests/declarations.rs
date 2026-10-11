#[path = "support/styled_cases.rs"]
mod styled_cases;
#[path = "support/widget_cases.rs"]
mod widget_cases;
use herdr_fork::{
    core::Core,
    terminal::{self, Rect},
    Cell, Surface,
};
use herdr_runtime::protocol::widgets::{Context, Declaration};
use serde_json::{json, Value};
use std::process::Command;
fn at(row: &str, column: usize) -> String {
    row.chars().skip(column).collect()
}
fn stock_declaration(live: bool) -> Declaration {
    let entities = if live {
        json!([
            {"id":"workspace:w1","kind":"workspace","workspaceId":"w1","tabId":null,"paneId":null,"label":"Alpha","number":1,"focused":true,"agentStatus":"working","agent":null,"cwd":null},
            {"id":"tab:t1","kind":"tab","workspaceId":"w1","tabId":"t1","paneId":null,"label":"Build","number":1,"focused":true,"agentStatus":"working","agent":null,"cwd":null},
            {"id":"pane:p1","kind":"pane","workspaceId":"w1","tabId":"t1","paneId":"p1","label":"shell","number":0,"focused":true,"agentStatus":"working","agent":null,"cwd":null}
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
    for (key, _) in
        std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("HERDR_"))
    {
        command.env_remove(key);
    }
    command.env("HERDR_CONFIG_PATH", home.join("config.toml"));
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
        }],
        cursor: None,
        focused: true,
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
    terminal::draw_frame(
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
fn public_declarations_are_the_only_visible_composition_in_all_four_states() {
    for (state, action, pane_y) in [
        ("no pane yet", None, None),
        ("live session", None, Some(1)),
        ("new-tab interaction open", Some("host:new"), Some(5)),
        ("menu open", Some("host:menu"), Some(4)),
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
            core.invoke(action, &Context::default());
        }
        let original = screen(&core, Some(declaration.clone()));
        assert!(
            original[0].starts_with(" herdr · stock"),
            "{state}: sidebar header"
        );
        assert!(
            original[22].starts_with(" + New tab "),
            "{state}: new-tab affordance"
        );
        assert!(
            original[23].starts_with(" Menu "),
            "{state}: menu affordance"
        );
        if live {
            assert!(
                at(&original[0], 20).starts_with(" Build "),
                "{state}: tab-strip placement"
            );
            assert!(
                original[1].starts_with("Alpha"),
                "{state}: sidebar placement"
            );
        } else {
            assert!(
                at(&original[1], 20).starts_with(" No pane yet"),
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
        if state == "new-tab interaction open" {
            assert!(at(&original[1], 20).starts_with(" New tab "));
            assert!(at(&original[2], 20).starts_with(" Name: "));
            assert!(at(&original[3], 20).starts_with(" Create "));
            assert!(at(&original[4], 20).starts_with(" Cancel "));
            let _ = core.text("Typed name");
            assert!(at(&screen(&core, Some(declaration.clone()))[2], 20)
                .starts_with(" Name: Typed name"));
        }
        if state == "menu open" {
            assert!(at(&original[1], 20).starts_with(" Session menu "));
            assert!(at(&original[2], 20).starts_with(" New tab "));
            assert!(at(&original[3], 20).starts_with(" Close menu "));
        }
        // An independently-authored arrangement must move every visible group, including the Surface.
        let mut changed: Value = serde_json::to_value(&declaration).unwrap();
        changed["composition"]["tree"]["children"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let moved: Declaration = serde_json::from_value(changed).unwrap();
        let swapped = screen(&core, Some(moved));
        assert!(
            at(&swapped[0], 60).starts_with(" herdr · stock"),
            "{state}: declaration did not move sidebar"
        );
        if let Some(y) = pane_y {
            assert!(
                swapped[y].starts_with('X'),
                "{state}: declaration did not move Surface"
            );
        } else {
            assert!(swapped[1].starts_with(" No pane yet"));
        }
        if live {
            assert!(swapped[0].starts_with(" Build "));
        }
        if state == "menu open" {
            assert!(swapped[1].starts_with(" Session menu "));
        }
        if state == "new-tab interaction open" {
            assert!(swapped[1].starts_with(" New tab "));
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
    terminal::draw_frame(&mut bytes, None, &declarations, &core, viewport).unwrap();
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
            terminal::action_at(&core, &declarations, viewport, 0, y as u16)
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
        terminal::draw_frame(
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
