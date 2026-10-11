mod support;
#[path = "support/widget_cases.rs"]
mod widget_cases;
use serde_json::json;
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};
use support::{Core, HOMES};

struct Console {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    screen: ghostty_vt::Terminal,
}
impl Console {
    fn new(core: &Core) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_herdr-fork"));
        command.args(["--session", &core.name]);
        for key in HOMES {
            command.env(key, &core.home);
        }
        for (key, _) in
            std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("HERDR_"))
        {
            command.env_remove(key);
        }
        command.env("HERDR_CONFIG_PATH", core.home.join("config.toml"));
        command.env("TERM", "xterm-256color");
        command.env("SHELL", "/bin/sh");
        command.env_remove("NO_COLOR");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0; 16384];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || tx.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            _master: pair.master,
            writer,
            rx,
            screen: ghostty_vt::Terminal::new(80, 24, 0).unwrap(),
        }
    }
    fn rows(&self) -> Vec<String> {
        self.screen
            .screen_text_rows()
            .unwrap()
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        cell.graphemes
                            .first()
                            .and_then(|cp| char::from_u32(*cp))
                            .unwrap_or(' ')
                    })
                    .collect()
            })
            .collect()
    }
    fn wait(&mut self, predicate: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let rows = self.rows();
            if predicate(&rows) {
                match self.rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(bytes) => {
                        self.screen.write(&bytes);
                        continue;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return rows,
                    Err(error) => panic!("executable output closed: {error}"),
                }
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "executable exited: {rows:?}"
            );
            let bytes = self
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!("executable composition observation: {error}; {rows:?}")
                });
            self.screen.write(&bytes);
        }
    }
    fn click(&mut self, x: usize, y: usize) {
        self.writer
            .write_all(
                format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes(),
            )
            .unwrap();
        self.writer.flush().unwrap();
    }
    fn text(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.writer.write_all(b"\x11");
        let _ = self.writer.flush();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().is_ok_and(|status| status.is_none())
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn at(row: &str, column: usize) -> String {
    row.chars().skip(column).collect()
}
#[test]
fn living_stock_ui_uses_the_public_widget_declaration_path() {
    let feature = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/features/declarations.feature"
    ))
    .unwrap();
    let lines: Vec<_> = feature.lines().map(str::trim).collect();
    let outline = lines
        .iter()
        .position(|line| {
            *line == "Scenario Outline: Stock UI uses the public widget declaration path"
        })
        .unwrap();
    assert_eq!(&lines[outline+1..outline+4], [
        "Given the stock UI plugin is enabled in the <state> state",
        "When the client displays the application using the plugin’s public widget declarations",
        "Then all visible UI composition, including placements of pane Surfaces, sidebar, tab strip and menus when present, is determined solely by those public declarations, with no privileged stock-UI rendering path",
    ]);
    let states: Vec<_> = lines[outline + 4..]
        .iter()
        .take_while(|line| !line.starts_with("Scenario"))
        .filter_map(|line| {
            line.strip_prefix('|')
                .and_then(|line| line.strip_suffix('|'))
        })
        .map(str::trim)
        .filter(|state| *state != "state")
        .collect();
    assert_eq!(
        states,
        [
            "no pane yet",
            "live session",
            "new-tab interaction open",
            "menu open"
        ]
    );
    assert_shared_list_producer_routing();
    for state in states {
        let core = Core::new();
        core.request(
            "plugin.link",
            json!({"path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/stock-ui"),"enabled":true}),
        );
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
        // Consume the real neutral declaration channel as well as the outer terminal.
        let mut client = core.attach();
        let deadline = Instant::now() + Duration::from_secs(8);
        while client.declarations.as_ref().is_none_or(|declarations| {
            !declarations
                .iter()
                .any(|(_, declaration)| declaration.composition.is_some())
        }) {
            client.receive().unwrap();
            assert!(
                Instant::now() < deadline,
                "stock producer never declared composition"
            );
        }
        if state == "no pane yet" {
            let snapshot = core.request("session.snapshot", json!({}));
            for workspace in snapshot["snapshot"]["workspaces"].as_array().unwrap() {
                core.request(
                    "workspace.close",
                    json!({"workspace_id":workspace["workspace_id"]}),
                );
            }
        }
        let mut console = Console::new(&core);
        let initial = console.wait(|rows| {
            rows[0].starts_with(" herdr · stock")
                && if state == "no pane yet" {
                    at(&rows[1], 20).starts_with(" No pane yet")
                } else {
                    at(&rows[0], 20).starts_with(" Build ") && rows[1].starts_with("Alpha")
                }
        });
        assert!(initial[22].starts_with(" + New tab "));
        assert!(initial[23].starts_with(" Menu "));
        if state == "no pane yet" {
            assert!(
                core.request("session.snapshot", json!({}))["snapshot"]["panes"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            console.click(4, 22);
            console.wait(|rows| at(&rows[2], 20).starts_with(" New tab "));
            console.text("First tab\r");
            console.wait(|rows| {
                rows[1].starts_with("First tab") && at(&rows[0], 20).starts_with(" 1 ")
            });
            let snapshot = core.request("session.snapshot", json!({}));
            assert!(
                !snapshot["snapshot"]["panes"].as_array().unwrap().is_empty(),
                "Core create did not create a real PTY"
            );
        } else {
            if state == "new-tab interaction open" {
                console.click(4, 22);
                console.wait(|rows| {
                    at(&rows[1], 20).starts_with(" New tab ")
                        && at(&rows[3], 20).starts_with(" Create ")
                        && at(&rows[4], 20).starts_with(" Cancel ")
                });
                console.text("Draft name");
                console.wait(|rows| at(&rows[2], 20).starts_with(" Name: Draft name"));
                console.text("\x1b");
                console.wait(|rows| !at(&rows[1], 20).starts_with(" New tab "));
            }
            if state == "menu open" {
                console.click(4, 23);
                console.wait(|rows| {
                    at(&rows[1], 20).starts_with(" Session menu ")
                        && at(&rows[2], 20).starts_with(" New tab ")
                        && at(&rows[3], 20).starts_with(" Close menu ")
                });
                console.click(22, 3);
                console.wait(|rows| !at(&rows[1], 20).starts_with(" Session menu "));
            }
            console.text("printf 'surface-%s\\n' live\r");
            console.wait(|rows| {
                rows.iter()
                    .skip(1)
                    .any(|row| at(row, 20).starts_with("surface-live"))
            });
        }
        eprintln!("OUTER PTY: {state}; public sidebar/tab strip/interaction composition and real Core inputs observed");
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
        "/plugins/agent-tree/herdr-plugin.toml"
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
        std::fs::write(path.join("herdr-plugin.toml"), manifest).unwrap();
        core.request("plugin.link", json!({"path":path,"enabled":true}));
    }
    // Both are the real retained agent-tree producer, with separately supervised identities/state.
    let mut console = Console::new(&core);
    let expanded = |rows: &[String]| {
        rows[2].starts_with("Alpha")
            && rows[6].starts_with("Alpha")
            && rows[3].contains("Build")
            && rows[4].contains("Review")
            && rows[7].contains("Build")
            && rows[8].contains("Review")
    };
    let rows = console.wait(expanded);
    assert!(rows[1].contains("· herdr") && rows[5].contains("· herdr"));
    console.click(1, 2);
    console.wait(|rows| {
        rows[2].contains("2 hidden")
            && rows[4].starts_with("Alpha")
            && !rows[4].contains("hidden")
            && rows[5].contains("Build")
            && rows[6].contains("Review")
    });
    console.click(1, 2);
    console.wait(expanded);
    console.click(1, 6);
    console.wait(|rows| {
        rows[2].starts_with("Alpha")
            && !rows[2].contains("hidden")
            && rows[3].contains("Build")
            && rows[4].contains("Review")
            && rows[6].contains("2 hidden")
    });
    console.click(1, 6);
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
        std::fs::write(plugin.join("herdr-plugin.toml"), "id = \"fixture.nested\"\nname = \"Nested widgets\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tree\"\nsurface = \"sidebar\"\ncommand = [\"bun\", \"producer.ts\"]\nstream = true\n").unwrap();
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
