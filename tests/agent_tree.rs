use herdr_fork::{sidebar, terminal, Client};
use herdr_runtime::protocol::widgets::Node;
use herdr_runtime::{
    api_client::{ApiClient, ConnectionTarget},
    api_schema::Request,
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Core {
    child: Child,
    home: PathBuf,
    name: String,
    api: ApiClient,
}
impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
const HOMES: &[&str] = &[
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
];
impl Core {
    fn new() -> Self {
        let name = format!(
            "at-{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let home = std::env::temp_dir().join(&name);
        std::fs::create_dir_all(&home).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-fork"));
        command.args(["server", "--session", &name]);
        for key in HOMES {
            command.env(key, &home);
        }
        for (key, _) in
            std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("HERDR_"))
        {
            command.env_remove(key);
        }
        command
            .env("SHELL", "/bin/sh")
            .env("HERDR_STARTUP_CWD", &home)
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let child = command.spawn().unwrap();
        let socket = home.join(format!("herdr-fork/sessions/{name}/herdr.sock"));
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(socket.clone()));
        let mut core = Self {
            child,
            home,
            name,
            api,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            assert!(
                Instant::now() < deadline && core.child.try_wait().unwrap().is_none(),
                "isolated core startup failed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        core
    }
    fn request(&self, method: &str, params: Value) -> Value {
        let request: Request = serde_json::from_value(
            json!({"id":"agent-tree-fixture", "method":method, "params":params}),
        )
        .unwrap();
        let reply = self
            .api
            .request_value_with_timeout(&request, Duration::from_secs(5))
            .unwrap();
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }
    fn attach(&self) -> Client {
        Client::attach(
            &self.home.join(format!(
                "herdr-fork/sessions/{}/herdr-client.sock",
                self.name
            )),
            80,
            24,
        )
        .unwrap()
    }
}
struct Fixture {
    core: Core,
    client: Client,
    selected_tab: String,
}
fn enable_first_consumer() -> Fixture {
    enable_first_consumer_with_active_tabs(0)
}
fn enable_first_consumer_with_active_tabs(additional: usize) -> Fixture {
    let core = Core::new();
    let plugins = core.request("plugin.list", json!({}));
    assert_eq!(
        plugins["plugins"].as_array().unwrap().len(),
        0,
        "a plugin preceded agent-tree"
    );
    let alpha = core.request(
        "workspace.create",
        json!({"label":"Alpha", "cwd":core.home, "focus":true}),
    );
    let workspace = alpha["workspace"]["workspace_id"].as_str().unwrap();
    let build_tab = alpha["tab"]["tab_id"].as_str().unwrap();
    core.request("tab.rename", json!({"tab_id":build_tab,"label":"Build"}));
    core.request("pane.report_agent", json!({"pane_id":alpha["root_pane"]["pane_id"],"source":"agent-tree-fixture","agent":"codex","state":"working"}));
    let review = core.request(
        "tab.create",
        json!({"workspace_id":workspace,"label":"Review", "cwd":core.home, "focus":false}),
    );
    core.request("pane.report_agent", json!({"pane_id":review["root_pane"]["pane_id"],"source":"agent-tree-fixture","agent":"claude","state":"blocked"}));
    for index in 0..additional {
        let tab = core.request("tab.create", json!({"workspace_id":workspace,"label":format!("Activity{:02}", index + 3), "cwd":core.home,"focus":false}));
        let state = ["working", "blocked"][index % 2];
        core.request("pane.report_agent", json!({"pane_id":tab["root_pane"]["pane_id"],"source":"agent-tree-fixture","agent":"codex","state":state}));
    }
    let beta = core.request(
        "workspace.create",
        json!({"label":"Beta", "cwd":core.home, "focus":false}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":beta["tab"]["tab_id"],"label":"Idle"}),
    );
    let mut client = core.attach();
    while client.primary.is_none() || client.focused_pane_id.is_none() {
        client.receive().unwrap();
    }
    assert!(client.declarations.as_ref().is_none_or(Vec::is_empty));
    core.request(
        "plugin.link",
        json!({"path":format!("{}/plugins/agent-tree", env!("CARGO_MANIFEST_DIR")),"enabled":true}),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        client.receive().unwrap();
        if client
            .declarations
            .as_ref()
            .is_some_and(|trees| !trees.is_empty())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "agent-tree did not declare widgets"
        );
    }
    Fixture {
        core,
        client,
        selected_tab: review["tab"]["tab_id"].as_str().unwrap().to_owned(),
    }
}
fn assert_sidebar(fixture: &mut Fixture) {
    crossterm::style::force_color_output(true);
    let declarations = fixture.client.declarations.as_ref().unwrap();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].0, "tommymorgan.agent-tree");
    let rows: Vec<_> = sidebar::rows(declarations).map(|(_, row)| row).collect();
    assert!(
        matches!(rows[0], Node::Row { children, .. } if matches!(&children[0], Node::Label { text, role } if text.ends_with(" · herdr") && role == "muted"))
    );
    let review = rows.iter().position(|row| matches!(row, Node::Row { context, .. } if context.tab_id.as_deref() == Some(&fixture.selected_tab))).unwrap();
    for width in [20, 34] {
        let mut bytes = Vec::new();
        sidebar::draw(
            &mut bytes,
            declarations,
            terminal::Rect {
                x: 0,
                y: 0,
                width,
                height: 24,
            },
            fixture.client.focused_tab_id.as_deref(),
        )
        .unwrap();
        let mut screen = ghostty_vt::Terminal::new(width, 24, 0).unwrap();
        screen.write(&bytes);
        let text: Vec<String> = screen
            .screen_text_rows()
            .unwrap()
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .flat_map(|c| c.graphemes.iter())
                    .filter_map(|c| char::from_u32(*c))
                    .collect()
            })
            .collect();
        let header = text.iter().find(|row| row.starts_with("Alpha")).unwrap();
        assert!(
            header.contains('·') && header.trim_end().ends_with("● ◉"),
            "elastic header: {header}"
        );
        assert!(text.iter().any(|row| row.starts_with("  ● Build")));
        assert!(text.iter().any(|row| row.starts_with("  ◉ Review")));
        assert!(text
            .iter()
            .any(|row| row.starts_with("Beta") && row.trim_end().ends_with('○')));
        assert!(text.iter().any(|row| row.starts_with("  ○ Idle")));
        assert!(
            bytes
                .windows(b"\x1b[38;5;9m".len())
                .any(|part| part == b"\x1b[38;5;9m"),
            "blocked role not rendered red"
        );
        assert!(
            bytes
                .windows(b"\x1b[38;5;11m".len())
                .any(|part| part == b"\x1b[38;5;11m"),
            "working role not rendered yellow"
        );
    }
    let (plugin, action, context) = sidebar::action_at(declarations, review).unwrap();
    assert_eq!(action, "focus-tab");
    let mut input = fixture.client.input().unwrap();
    input
        .invoke(&fixture.client.boot_id, plugin, action, context)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.client.focused_tab_id.as_deref() != Some(&fixture.selected_tab) {
        fixture.client.receive().unwrap();
        assert!(
            Instant::now() < deadline,
            "forwarded focus-tab did not focus its target"
        );
    }
}
fn assert_fifteen_active_tabs_survive_core_validation() {
    let fixture = enable_first_consumer_with_active_tabs(13);
    let declarations = fixture.client.declarations.as_ref().unwrap();
    let mut bytes = Vec::new();
    sidebar::draw(
        &mut bytes,
        declarations,
        terminal::Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 24,
        },
        fixture.client.focused_tab_id.as_deref(),
    )
    .unwrap();
    let mut screen = ghostty_vt::Terminal::new(50, 24, 0).unwrap();
    screen.write(&bytes);
    let text: Vec<String> = screen
        .screen_text_rows()
        .unwrap()
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .flat_map(|cell| cell.graphemes.iter())
                .filter_map(|ch| char::from_u32(*ch))
                .collect()
        })
        .collect();
    let header = text
        .iter()
        .find(|row| row.starts_with("Alpha"))
        .expect("15 active tabs erased the sidebar");
    assert!(header.contains('·'));
    assert!(
        header.trim_end().ends_with("● ◉ ● ◉ ● ◉ ● ◉ ● ◉ ● ◉ ● ◉ ●"),
        "retained active glyphs were lost: {header}"
    );
    assert!(text.iter().any(|row| row.starts_with("  ● Build")));
    assert!(text.iter().any(|row| row.starts_with("  ◉ Review")));
    for (label, glyph) in [
        ("Activity03", "●"),
        ("Activity04", "◉"),
        ("Activity05", "●"),
        ("Activity15", "●"),
    ] {
        assert!(
            text.iter()
                .any(|row| row.starts_with(&format!("  {glyph} {label}"))),
            "missing active tab {label}"
        );
    }
    assert!(text.iter().any(|row| row.starts_with("Beta")));
}
#[test]
fn port_the_agent_tree_as_the_first_widget_tree_consumer() {
    assert_sidebar(&mut enable_first_consumer());
    assert_fifteen_active_tabs_survive_core_validation();
}
#[test]
fn living_agent_tree_feature() {
    let feature = include_str!("../features/agent-tree.feature");
    let plan = include_str!("../plans/2026-10-10-herdr-fork.md");
    let scenario = "Scenario: Port the agent tree as the first widget-tree consumer";
    assert_eq!(
        feature.matches("Scenario:").count(),
        2,
        "unimplemented living scenarios"
    );
    let from_feature: Vec<_> = feature
        .lines()
        .skip_while(|line| line.trim() != scenario)
        .take(4)
        .map(str::trim)
        .collect();
    let from_plan: Vec<_> = plan
        .lines()
        .skip_while(|line| line.trim() != scenario)
        .take(4)
        .map(str::trim)
        .collect();
    assert_eq!(
        from_feature, from_plan,
        "living title/GWT drifted from Task2.1"
    );
    assert_sidebar(&mut enable_first_consumer());
    assert_fifteen_active_tabs_survive_core_validation();
}

struct Console(Box<dyn portable_pty::Child + Send + Sync>);
impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn executable_pty_sidebar_click_focuses_selected_tab_and_renders_its_live_surface() {
    let fixture = enable_first_consumer();
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: 25,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_herdr-fork"));
    command.args(["--session", &fixture.core.name]);
    for key in HOMES {
        command.env(key, &fixture.core.home);
    }
    for (key, _) in
        std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("HERDR_"))
    {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    command.env("SHELL", "/bin/sh");
    command.env_remove("NO_COLOR");
    let mut console = Console(pair.slave.spawn_command(command).unwrap());
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reading = std::thread::spawn(move || {
        let mut bytes = [0; 16384];
        while let Ok(n) = reader.read(&mut bytes) {
            if n == 0 || tx.send(bytes[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut screen = ghostty_vt::Terminal::new(100, 25, 0).unwrap();
    let wait = |screen: &mut ghostty_vt::Terminal, predicate: &dyn Fn(&[String]) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text: Vec<String> = screen
                .screen_text_rows()
                .unwrap()
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .flat_map(|c| c.graphemes.iter())
                        .filter_map(|c| char::from_u32(*c))
                        .collect()
                })
                .collect();
            if predicate(&text) {
                return text;
            }
            let bytes = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|e| panic!("PTY rendered observation timed out: {e}; {text:?}"));
            screen.write(&bytes);
        }
    };
    let text = wait(&mut screen, &|rows| {
        rows.iter().any(|row| row.starts_with("  ◉ Review"))
    });
    let y = text
        .iter()
        .position(|row| row.starts_with("  ◉ Review"))
        .unwrap();
    assert!(text
        .iter()
        .any(|row| row.starts_with("Alpha") && row.chars().take(20).any(|ch| ch == '·')));
    writer
        .write_all(format!("\x1b[<0;5;{}M\x1b[<0;5;{}m", y + 1, y + 1).as_bytes())
        .unwrap();
    writer.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = fixture.core.request("session.snapshot", json!({}));
        if snapshot["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["tab_id"] == fixture.selected_tab && tab["focused"] == true)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "real sidebar click never focused Review"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cell_style(&screen, y, 4).0.bold {
        let bytes = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("selected tab was not rendered focused");
        screen.write(&bytes);
    }
    // Type immediately after visible focus, without waiting for a PTY Surface.
    writer.write_all(b"printf 'selected-%s\\n' live\r").unwrap();
    writer.flush().unwrap();
    wait(&mut screen, &|rows| {
        rows.iter().any(|row| {
            row.chars()
                .skip(20)
                .collect::<String>()
                .starts_with("selected-live")
        })
    });
    assert!(
        cell_style(&screen, y, 4).0.bold,
        "focused row not highlighted"
    );
    eprintln!("PTY SMOKE: agent-tree sidebar, elastic leaders, Review click, focused live Surface and shell input observed");
    writer.write_all(b"\x11").unwrap();
    writer.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while console.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "client detach hung");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(console);
    drop(writer);
    drop(pair.master);
    drop(rx);
    reading.join().unwrap();
}
fn cell_style(
    screen: &ghostty_vt::Terminal,
    y: usize,
    x: u16,
) -> (ghostty_vt::CellStyle, Option<ghostty_vt::RgbColor>) {
    let mut state = ghostty_vt::RenderState::new().unwrap();
    state.update(screen).unwrap();
    let mut storage = ghostty_vt::RowIterator::new().unwrap();
    let mut rows = state.populate_row_iterator(&mut storage).unwrap();
    for _ in 0..=y {
        assert!(rows.next());
    }
    let mut cell_storage = ghostty_vt::RowCells::new().unwrap();
    let mut cells = rows.populate_cells(&mut cell_storage).unwrap();
    cells.select(x).unwrap();
    (cells.style().unwrap(), cells.fg_color().unwrap())
}

fn rendered_sidebar(client: &Client) -> Vec<String> {
    let mut bytes = Vec::new();
    sidebar::draw(
        &mut bytes,
        client.declarations.as_ref().unwrap(),
        terminal::Rect {
            x: 0,
            y: 0,
            width: 34,
            height: 24,
        },
        client.focused_tab_id.as_deref(),
    )
    .unwrap();
    let mut screen = ghostty_vt::Terminal::new(34, 24, 0).unwrap();
    screen.write(&bytes);
    screen
        .screen_text_rows()
        .unwrap()
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .flat_map(|cell| cell.graphemes.iter())
                .filter_map(|ch| char::from_u32(*ch))
                .collect()
        })
        .collect()
}
fn wait_sidebar(client: &mut Client, predicate: impl Fn(&[String]) -> bool) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let rows = rendered_sidebar(client);
        if predicate(&rows) {
            return rows;
        }
        assert!(Instant::now() < deadline, "tree did not update: {rows:?}");
        client.receive().unwrap();
    }
}
fn collapse_and_expand_workspace() {
    let mut fixture = enable_first_consumer_with_active_tabs(1);
    let before = wait_sidebar(&mut fixture.client, |rows| {
        rows.iter().any(|row| row.contains("Activity03"))
    });
    let header = before
        .iter()
        .position(|row| row.starts_with("Alpha"))
        .unwrap();
    let mut input = fixture.client.input().unwrap();
    for collapsed in [true, false] {
        let (plugin, action, context) =
            sidebar::action_at(fixture.client.declarations.as_ref().unwrap(), header).unwrap();
        input
            .invoke(&fixture.client.boot_id, plugin, action, context)
            .unwrap();
        let rows = wait_sidebar(&mut fixture.client, |rows| {
            rows[header].contains("3 hidden") == collapsed
        });
        for tab in ["Build", "Review", "Activity03"] {
            assert_eq!(
                rows.iter().any(|row| row.contains(tab)),
                !collapsed,
                "{tab}: {rows:?}"
            );
        }
        assert!(
            rows.iter().any(|row| row.starts_with("Beta")),
            "unrelated workspace disappeared"
        );
        assert!(
            rows.iter().any(|row| row.contains("Idle")),
            "unrelated tabs collapsed"
        );
    }
}
#[test]
fn expand_and_collapse_workspace_declarations() {
    collapse_and_expand_workspace();
}
#[test]
fn living_expand_and_collapse_workspace_declarations() {
    assert_living_scenario("Scenario: Expand and collapse workspace declarations", 4);
    collapse_and_expand_workspace();
}
struct LiveConsole {
    child: Console,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    screen: ghostty_vt::Terminal,
}
impl LiveConsole {
    fn new(core: &Core) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 25,
                cols: 120,
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
        command.env("TERM", "xterm-256color");
        command.env("SHELL", "/bin/sh");
        command.env_remove("NO_COLOR");
        let child = Console(pair.slave.spawn_command(command).unwrap());
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = [0; 16384];
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 || tx.send(bytes[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            master: pair.master,
            writer,
            rx,
            screen: ghostty_vt::Terminal::new(120, 25, 0).unwrap(),
        }
    }
    fn wait(
        &mut self,
        predicate: impl Fn(&[String], &ghostty_vt::Terminal) -> bool,
    ) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let rows: Vec<String> = self
                .screen
                .screen_text_rows()
                .unwrap()
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .take(24)
                        .flat_map(|cell| cell.graphemes.iter())
                        .filter_map(|ch| char::from_u32(*ch))
                        .collect()
                })
                .collect();
            if predicate(&rows, &self.screen) {
                return rows;
            }
            assert!(
                self.child.0.try_wait().unwrap().is_none(),
                "client restarted/exited"
            );
            let bytes = self
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("live PTY tree did not update: {error}; {rows:?}"));
            self.screen.write(&bytes);
        }
    }
    fn click(&mut self, row: usize) {
        self.writer
            .write_all(format!("\x1b[<0;5;{}M\x1b[<0;5;{}m", row + 1, row + 1).as_bytes())
            .unwrap();
        self.writer.flush().unwrap();
    }
}
impl Drop for LiveConsole {
    fn drop(&mut self) {
        let _ = self.writer.write_all(b"\x11");
        let _ = self.writer.flush();
        let _ = self.master.get_size();
    }
}
#[test]
fn executable_pty_workspace_collapse_and_expand() {
    let fixture = enable_first_consumer_with_active_tabs(1);
    let mut console = LiveConsole::new(&fixture.core);
    let rows = console.wait(|rows, _| rows.iter().any(|row| row.contains("Activity03")));
    let header = rows
        .iter()
        .position(|row| row.starts_with("Alpha"))
        .unwrap();
    console.click(header);
    let rows = console.wait(|rows, _| rows[header].contains("3 hidden"));
    for tab in ["Build", "Review", "Activity03"] {
        assert!(
            !rows.iter().any(|row| row.contains(tab)),
            "collapsed tab visible: {tab}"
        );
    }
    assert!(rows.iter().any(|row| row.contains("Idle")));
    console.click(header);
    let rows = console.wait(|rows, _| rows.iter().any(|row| row.contains("Activity03")));
    for tab in ["Build", "Review", "Activity03"] {
        assert!(rows.iter().any(|row| row.contains(tab)));
    }
    assert!(!rows[header].contains("hidden"));
    eprintln!("PTY SMOKE: three Alpha tabs disappear/reappear via actual header clicks; 3 hidden; Beta remains visible");
}
fn assert_living_scenario(title: &str, lines: usize) {
    let extract = |source: &str| -> Vec<String> {
        source
            .lines()
            .skip_while(|line| line.trim() != title)
            .take(lines)
            .map(|line| line.trim().to_owned())
            .collect()
    };
    let feature = extract(include_str!("../features/agent-tree.feature"));
    assert_eq!(feature.len(), lines, "living scenario missing");
    assert_eq!(
        feature,
        extract(include_str!("../plans/2026-10-10-herdr-fork.md")),
        "living GWT/examples drift"
    );
}
