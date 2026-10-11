#[path = "support/replacement.rs"]
mod replacement;
#[path = "support/styled_cases.rs"]
mod styled_cases;
mod support;
#[path = "support/widget_cases.rs"]
mod widget_cases;
use serde_json::json;
use std::time::{Duration, Instant};
use support::Core;

#[path = "support/console.rs"]
mod console;
use console::Console;
fn at(row: &str, column: usize) -> String {
    row.chars().skip(column).collect()
}
fn open_native_menu(console: &mut Console, row: usize) {
    console.text(&format!("\x1b[<35;20;{}M", row + 1));
    let rows = console.wait(|rows| rows[row].contains('⋯'));
    let x = rows[row].chars().position(|ch| ch == '⋯').unwrap();
    assert_eq!(rows[row].chars().nth(x + 1), Some(' '));
    console.click(x, row);
}
fn choose_native_menu(console: &mut Console, label: &str) {
    let rows = console.wait(|rows| rows.iter().any(|row| row.contains(label)));
    let y = rows.iter().position(|row| row.contains(label)).unwrap();
    let x = rows[y][..rows[y].find(label).unwrap()].chars().count();
    console.click(x, y);
}
#[test]
fn living_stock_ui_uses_the_public_widget_declaration_path() {
    assert_shared_list_producer_routing();
    for state in [
        "no pane yet",
        "live session",
        "rename interaction open",
        "menu open",
    ] {
        let core = Core::new();
        let mut console = Console::new(&core);
        console.wait(|rows| rows.iter().any(|row| row.starts_with('▏')));
        let snapshot = core.request("session.snapshot", json!({}));
        for workspace in snapshot["snapshot"]["workspaces"].as_array().unwrap() {
            core.request(
                "workspace.close",
                json!({"workspace_id":workspace["workspace_id"]}),
            );
        }
        if state != "no pane yet" {
            let workspace = core.request(
                "workspace.create",
                json!({"label":"Alpha", "cwd":core.home,"focus":true}),
            );
            core.request(
                "tab.rename",
                json!({"tab_id":workspace["tab"]["tab_id"],"label":"Build"}),
            );
        }

        let initial = console.wait(|rows| {
            if state == "no pane yet" {
                at(&rows[0], 20).starts_with(" No pane yet")
            } else {
                rows.iter()
                    .any(|row| row.chars().take(20).collect::<String>().contains("Build"))
            }
        });
        // The executable installs all public producers, including independent menu fragments.
        let mut client = core.attach();
        let deadline = Instant::now() + Duration::from_secs(8);
        while client.declarations.as_ref().is_none_or(|declarations| {
            !declarations
                .iter()
                .any(|(_, declaration)| declaration.composition.is_some())
                || !declarations
                    .iter()
                    .any(|(id, _)| id == "tommymorgan.sidebar-menus")
        }) {
            client.receive().unwrap();
            assert!(
                Instant::now() < deadline,
                "public composition/menu declarations unavailable"
            );
        }
        drop(client);
        if state == "no pane yet" {
            assert!(
                core.request("session.snapshot", json!({}))["snapshot"]["panes"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            open_native_menu(&mut console, 23);
            choose_native_menu(&mut console, "New space");
            console.wait(|rows| {
                rows.iter().any(|row| row.starts_with('▏'))
                    && rows.iter().any(|row| at(row, 20).starts_with("sh-"))
            });
            assert!(
                !core.request("session.snapshot", json!({}))["snapshot"]["panes"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        } else {
            let workspace_row = initial
                .iter()
                .position(|row| row.chars().take(20).collect::<String>().contains("Alpha"))
                .unwrap();
            if state == "rename interaction open" {
                open_native_menu(&mut console, workspace_row);
                choose_native_menu(&mut console, "Rename “Alpha”");
                console.text("Draft");
                let rows = console.wait(|rows| rows.iter().any(|row| row.contains("Draft▏")));
                assert!(rows.iter().all(|row| !at(row, 20).contains("Draft")));
                console.text("\x1b");
                console.wait(|rows| rows.iter().all(|row| !row.contains("Draft▏")));
            }
            if state == "menu open" {
                open_native_menu(&mut console, workspace_row);
                console.wait(|rows| rows.iter().any(|row| row.contains("New tab in “Alpha”")));
                console.text("\x1b");
                console.wait(|rows| rows.iter().all(|row| !row.contains("New tab in “Alpha”")));
                console.text("\x1b[<35;70;20M\x1b");
                console.wait(|rows| rows.iter().all(|row| !row.contains('⋯')));
            }
        }
        console.text("printf 'surface-%s\\n' live\r");
        console.wait(|rows| {
            rows.iter()
                .any(|row| at(row, 20).starts_with("surface-live"))
        });
    }
}

fn assert_shared_list_producer_routing() {
    let core = Core::new();
    let snapshot = core.request("session.snapshot", json!({}));
    for workspace in snapshot["snapshot"]["workspaces"].as_array().unwrap() {
        core.request(
            "workspace.close",
            json!({"workspace_id":workspace["workspace_id"]}),
        );
    }
    let workspace = core.request(
        "workspace.create",
        json!({"label":"Alpha","cwd":core.home,"focus":true}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":workspace["tab"]["tab_id"],"label":"Build"}),
    );
    core.request("tab.create",json!({"workspace_id":workspace["workspace"]["workspace_id"],"label":"Review","cwd":core.home,"focus":false}));
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/plugins/agent-tree/trailboss-plugin.toml"
    ))
    .unwrap();
    for id in ["first-list", "second-list"] {
        let path = core.home.join(id);
        std::fs::create_dir_all(&path).unwrap();
        let manifest = source
            .replace("tommymorgan.agent-tree", &format!("fixture.{id}"))
            .replace(
                r#""agent_tree.ts""#,
                &format!(
                    "{:?}",
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/plugins/agent-tree/agent_tree.ts"
                    )
                ),
            );
        std::fs::write(path.join("trailboss-plugin.toml"), manifest).unwrap();
        core.request("plugin.link", json!({"path":path,"enabled":true}));
    }
    // Both are the real retained agent-tree producer, with separately supervised identities/state.
    let mut console = Console::new(&core);
    let expanded = |rows: &[String]| {
        ["Build", "Review"].iter().all(|label| {
            rows.iter()
                .filter(|row| row.chars().take(20).collect::<String>().contains(label))
                .count()
                == 2
        }) && !rows.iter().any(|row| row.contains("hidden"))
    };
    let headers = |rows: &[String]| {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| {
                row.trim_start().starts_with("Alpha")
                    || row.chars().take(20).collect::<String>().contains("hidden")
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    };
    let rows = console.wait(expanded);
    let first = headers(&rows)[0];
    console.click(1, first);
    console.wait(|rows| {
        let indices = headers(rows);
        indices.len() == 2
            && rows[indices[0]].contains("2 hidden")
            && !rows[indices[1]].contains("hidden")
            && ["Build", "Review"].iter().all(|label| {
                rows.iter()
                    .filter(|row| row.chars().take(20).collect::<String>().contains(label))
                    .count()
                    == 1
            })
    });
    console.click(1, first);
    let rows = console.wait(expanded);
    let second = headers(&rows)[1];
    console.click(1, second);
    console.wait(|rows| {
        let indices = headers(rows);
        indices.len() == 2
            && !rows[indices[0]].contains("hidden")
            && rows[indices[1]].contains("2 hidden")
            && ["Build", "Review"].iter().all(|label| {
                rows.iter()
                    .filter(|row| row.chars().take(20).collect::<String>().contains(label))
                    .count()
                    == 1
            })
    });
    console.click(1, second);
    console.wait(expanded);
    eprintln!("OUTER PTY: both real list producers have separate visible rows; first/second clicks collapse only their own producer and restore independently");
}

fn assert_living_widget(node: &str) {
    let feature = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/features/declarations.feature"
    ))
    .unwrap();
    let lines: Vec<_> = feature.lines().map(str::trim).collect();
    let start = lines
        .iter()
        .position(|line| *line == "Scenario Outline: Compose each supported widget node")
        .unwrap();
    assert_eq!(&lines[start+1..start+4], [
        "Given a producer declares a <node> inside a nested widget tree",
        "When the client applies that declaration",
        "Then the client displays the <observable> using its renderer under core-defined semantics",
    ]);
    let examples: Vec<_> = lines[start + 4..]
        .iter()
        .take_while(|line| !line.starts_with("Scenario"))
        .filter_map(|line| line.strip_prefix('|'))
        .filter_map(|line| line.split('|').next())
        .map(str::trim)
        .filter(|node| *node != "node")
        .collect();
    assert_eq!(examples, widget_cases::NODES);
    assert!(examples.contains(&node));
    {
        let core = Core::new();
        let plugin = core.home.join("nested-producer");
        std::fs::create_dir_all(&plugin).unwrap();
        let schema = concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/agent-tree/widgets.ts");
        std::fs::write(plugin.join("producer.ts"), format!("import {{ WidgetDeclaration }} from {schema:?}; console.log(JSON.stringify(WidgetDeclaration.parse({}))); await Bun.stdin.text();", widget_cases::declaration(node))).unwrap();
        std::fs::write(plugin.join("trailboss-plugin.toml"), "id = \"fixture.nested\"\nname = \"Nested widgets\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tree\"\nsurface = \"sidebar\"\ncommand = [\"bun\", \"producer.ts\"]\nstream = true\n").unwrap();
        core.request("plugin.link", json!({"path":plugin,"enabled":true}));
        let mut console = Console::new(&core);
        let rows = console.wait(|rows| rows.iter().any(|row| row.starts_with("END")));
        widget_cases::assert_rows(node, &rows, 80);
        eprintln!(
            "nested {node}: {:?}",
            &rows[..widget_cases::expected(node, 80).len()]
        );
    }
}
macro_rules! living_widget_test {
    ($name:ident, $node:literal) => {
        #[test]
        fn $name() {
            assert_living_widget($node);
        }
    };
}
living_widget_test!(living_nested_widget_row, "row");
living_widget_test!(living_nested_widget_column, "column");
living_widget_test!(living_nested_widget_label, "label");
living_widget_test!(living_nested_widget_leader, "leader");
living_widget_test!(living_nested_widget_spacer, "spacer");
living_widget_test!(living_nested_widget_list, "list");

#[test]
fn living_pin_styled_declarations_into_zones() {
    let feature = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/features/declarations.feature"
    ))
    .unwrap();
    assert!(feature.contains("  Scenario: Pin styled declarations into zones\n    Given a plugin declares label, muted, leader and status-role segments with named tones in the list, footer and footer-right zones\n    When the client projects those declarations into the terminal\n    Then each declaration stays in its client-renderer-defined zone geometry and the client palette styles its roles and tones without plugin-supplied pixels"));
    let core = Core::new();
    core.request(
        "plugin.link",
        json!({"path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/stock-ui"),"enabled":true}),
    );
    let workspace = core.request(
        "workspace.create",
        json!({"label":"Padding focus","cwd":core.home,"focus":true}),
    );
    let mut declaration = styled_cases::declaration();
    declaration["zones"][0]["tree"]["entries"][1]["context"]["tab_id"] =
        workspace["tab"]["tab_id"].clone();
    let plugin = core.home.join("styled-producer");
    std::fs::create_dir_all(&plugin).unwrap();
    let schema = concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/agent-tree/widgets.ts");
    std::fs::write(plugin.join("producer.ts"), format!("import {{ WidgetDeclaration }} from {schema:?}; console.log(JSON.stringify(WidgetDeclaration.parse({declaration}))); await Bun.stdin.text();")).unwrap();
    std::fs::write(plugin.join("trailboss-plugin.toml"), "id = \"fixture.styled\"\nname = \"Styled zones\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tree\"\nsurface = \"sidebar\"\ncommand = [\"bun\", \"producer.ts\"]\nstream = true\n").unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let mut console = Console::new(&core);
    console.wait(|rows| {
        ["Lm", "Fm", "Rm"]
            .iter()
            .all(|prefix| rows.iter().any(|row| row.starts_with(prefix)))
    });
    styled_cases::assert_screen(&console.screen, 80, 24, true);
}

#[test]
fn living_hot_apply_a_new_widget_declaration() {
    let feature = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/features/declarations.feature"
    ))
    .unwrap();
    assert!(feature.contains("  Scenario: Hot-apply a new widget declaration\n    Given the running client displays a producer’s widget tree\n    When the producer emits new widget declarations by replacing its widget tree with changed text and nesting\n    Then the visible UI changes to the replacement tree without rebuilding or restarting the client"));
    let core = Core::new();
    let plugin = replacement::enable(&core);
    let mut console = Console::new(&core);
    let rows = console.wait(|rows| rows[0].starts_with("OLDTREE"));
    assert_eq!(rows[0].trim_end(), "OLDTREE");
    assert!(rows[1..].iter().all(|row| row.trim().is_empty()));
    let process = console.child.process_id();
    assert!(process.is_some());
    let socket = core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    ));
    use std::os::unix::fs::MetadataExt;
    let inode = std::fs::metadata(&socket).unwrap().ino();
    replacement::replace(&plugin);
    let rows = console.wait(|rows| rows[0].starts_with("NEW") && rows[2].starts_with("ONLY"));
    replacement::assert_replacement(&rows);
    replacement::assert_same_producer_and_socket(&core, &plugin);
    assert_eq!(console.child.process_id(), process);
    assert!(console.child.try_wait().unwrap().is_none());
    assert_eq!(std::fs::metadata(socket).unwrap().ino(), inode);
}

#[test]
fn create_space_click_preserves_existing_tabs_and_focuses_a_new_shell() {
    let core = Core::new();
    let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
    core.request(
        "plugin.link",
        json!({"path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/stock-ui"),"enabled":true}),
    );
    let mut console = Console::new(&core);
    console.wait(|rows| rows.iter().any(|row| row.starts_with('▏')));
    open_native_menu(&mut console, 23);
    choose_native_menu(&mut console, "New space");
    let deadline = Instant::now() + Duration::from_secs(3);
    let after = loop {
        let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
        if snapshot["workspaces"].as_array().unwrap().len()
            == before["workspaces"].as_array().unwrap().len() + 1
        {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "create-space click did not create a workspace: {snapshot}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    for workspace in before["workspaces"].as_array().unwrap() {
        assert!(
            after["workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|new| new["workspace_id"] == workspace["workspace_id"]),
            "existing workspace was removed"
        );
    }
    for pane in before["panes"].as_array().unwrap() {
        assert!(
            after["panes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|new| new["pane_id"] == pane["pane_id"]),
            "existing pane was removed"
        );
    }
    let mut observer = core.attach();
    while observer.focused_pane_id.is_none() {
        observer.receive().unwrap();
    }
    assert!(
        !before["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pane| pane["pane_id"].as_str() == observer.focused_pane_id.as_deref()),
        "new space did not receive focus"
    );
    observer
        .type_text("printf 'new-space-%s\\n' live\r")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        observer.receive().unwrap();
        if observer.primary.as_ref().is_some_and(|surface| {
            surface
                .cells
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
                .contains("new-space-live")
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "new space did not run its shell");
    }
}

#[test]
fn attaching_replaces_stale_bundled_producers_and_keeps_menu_actions_usable() {
    let core = Core::new();
    for (directory, plugin_id, tree) in [
        (
            "stock-ui",
            "tommymorgan.stock-ui",
            json!({"zones":[],"composition":{"priority":10,"tree":{"type":"label","text":"OLD STOCK","role":"label"}}}),
        ),
        (
            "agent-tree",
            "tommymorgan.agent-tree",
            json!({"zones":[{"zone":"list","tree":{"type":"label","text":"OLD TREE","role":"label"}}]}),
        ),
        (
            "sidebar-menus",
            "tommymorgan.sidebar-menus",
            json!({"zones":[],"fragments":[{"name":"sidebar.session-menu","priority":0,"tree":{"type":"column","children":[{"type":"row","action":"host:detach","children":[{"type":"label","text":"OLD MENU","role":"label"}]}]}}]}),
        ),
    ] {
        let root = core.home.join(directory);
        std::fs::create_dir_all(&root).unwrap();
        let mut declaration = tree;
        declaration["herdr"] = json!("widget_tree");
        declaration["version"] = json!(1);
        std::fs::write(
            root.join("producer.ts"),
            format!("console.log(JSON.stringify({declaration})); await Bun.stdin.text();"),
        )
        .unwrap();
        std::fs::write(root.join("trailboss-plugin.toml"), format!("id = {plugin_id:?}\nname = \"Old bundle\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tree\"\nsurface = \"sidebar\"\ncommand = [\"bun\", \"producer.ts\"]\nstream = true\n")).unwrap();
        core.request("plugin.link", json!({"path":root,"enabled":true}));
    }
    let mut console = Console::new(&core);
    let rows = console.wait(|rows| rows.iter().any(|row| row.contains("~ ─")));
    let section = rows.iter().position(|row| row.contains("~ ─")).unwrap();
    // The stock fallback can paint the same header before the interactive producer arrives.
    let mut observer = core.attach();
    let deadline = Instant::now() + Duration::from_secs(8);
    while observer.declarations.as_ref().is_none_or(|declarations| {
        !trailboss::sidebar::action_at(declarations, section).is_some_and(|(plugin, action, _)| {
            plugin == "tommymorgan.agent-tree" && action == "open-workspace"
        })
    }) {
        observer.receive().unwrap();
        assert!(
            Instant::now() < deadline,
            "interactive sidebar did not arrive"
        );
    }
    drop(observer);
    // A pane round-trip drains prior declaration frames on this client before the click.
    console.text("printf 'menu-ready-%s\\n' live\r");
    console.wait(|rows| rows.iter().any(|row| row.contains("menu-ready-live")));
    console.click(3, section);
    let rows = console.wait(|rows| rows.iter().any(|row| row.contains("1 hidden")));
    assert!(rows
        .iter()
        .all(|row| !row.contains("OLD TREE") && !row.contains("OLD STOCK")));
    open_native_menu(&mut console, 23);
    choose_native_menu(&mut console, "New space");
    console.wait(|rows| rows.iter().any(|row| row.contains("New space ─")));
    assert_eq!(
        core.request("session.snapshot", json!({}))["snapshot"]["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
