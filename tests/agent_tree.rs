use trailboss::{sidebar, terminal, Client};
use trailboss_runtime::api_schema::Request;
use trailboss_runtime::protocol::widgets::Node;
mod support;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};
use support::{Core, HOMES};

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
    core.request(
        "plugin.link",
        json!({"path":format!("{}/plugins/agent-tree", env!("CARGO_MANIFEST_DIR")),"enabled":true}),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        client.receive().unwrap();
        if client.declarations.as_ref().is_some_and(|trees| {
            sidebar::rows(trees).any(|(owner, node)| {
                owner == "tommymorgan.agent-tree"
                    && matches!(node, Node::Row { context, .. } if context.tab_id.as_deref() == review["tab"]["tab_id"].as_str())
            })
        }) {
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
    let rows: Vec<_> = sidebar::rows(declarations).map(|(_, row)| row).collect();
    let review = rows.iter().position(|row| matches!(row, Node::Row { context, .. } if context.tab_id.as_deref() == Some(&fixture.selected_tab))).unwrap();
    let (plugin, action, context) = sidebar::action_at(declarations, review).unwrap();
    assert_eq!(action, "focus-tab");
    let mut input = fixture.client.input().unwrap();
    input
        .invoke(&fixture.client.boot_id, plugin, action, context, None)
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
            height: 32,
        },
        fixture.client.focused_tab_id.as_deref(),
    )
    .unwrap();
    let mut screen = ghostty_vt::Terminal::new(50, 32, 0).unwrap();
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
    for label in [
        "Build",
        "Review",
        "Activity03",
        "Activity04",
        "Activity05",
        "Activity15",
        "Beta",
    ] {
        assert!(
            text.iter().any(|row| row.contains(label)),
            "active tree entry disappeared: {label}"
        );
    }
}
#[test]
fn port_the_agent_tree_as_the_first_widget_tree_consumer() {
    assert_sidebar(&mut enable_first_consumer());
    assert_fifteen_active_tabs_survive_core_validation();
}
#[test]
fn living_agent_tree_feature() {
    let feature = include_str!("../features/agent-tree.feature");
    let plan = include_str!("../plans/2026-10-10-trailboss.md");
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
    let mut command = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_trailboss"));
    command.args(["--session", &fixture.core.name]);
    for key in HOMES {
        command.env(key, &fixture.core.home);
    }
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
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
        rows.iter()
            .any(|row| row.chars().take(25).collect::<String>().contains("Review"))
    });
    let y = text
        .iter()
        .position(|row| row.chars().take(25).collect::<String>().contains("Review"))
        .unwrap();
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
                .skip(25)
                .collect::<String>()
                .starts_with("selected-live")
        })
    });
    // Independent menu fragments arrive after the already-live sidebar producer.
    let alpha_y = text.iter().position(|row| row.contains("Alpha")).unwrap();
    writer
        .write_all(format!("\x1b[<35;24;{}M", alpha_y + 1).as_bytes())
        .unwrap();
    writer.flush().unwrap();
    wait(&mut screen, &|rows| rows[alpha_y].contains('⋯'));
    writer.write_all(b"\x1b[<35;80;20M").unwrap();
    writer.flush().unwrap();
    wait(&mut screen, &|rows| {
        rows.iter().all(|row| !row.contains('⋯'))
    });
    // Keyboard sidebar focus and a dismissed workspace menu must not move the selected tab.
    writer.write_all(b"\x02\t\t\t\x1b[21;2~").unwrap();
    writer.flush().unwrap();
    wait(&mut screen, &|rows| {
        rows.iter().any(|row| row.contains("New tab in “Alpha”"))
    });
    writer.write_all(b"\x1b").unwrap();
    writer.flush().unwrap();
    wait(&mut screen, &|rows| {
        rows.iter().all(|row| !row.contains("New tab in “Alpha”"))
    });
    writer.write_all(b"\x1b").unwrap();
    writer.flush().unwrap();
    wait(&mut screen, &|rows| {
        rows.iter().all(|row| !row.contains('⋯'))
    });
    assert!(
        fixture.core.request("session.snapshot", json!({}))["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["tab_id"] == fixture.selected_tab && tab["focused"] == true)
    );
    writer
        .write_all(b"printf 'menu-%s\\n' continuity\r")
        .unwrap();
    writer.flush().unwrap();
    wait(&mut screen, &|rows| {
        rows.iter().any(|row| {
            row.chars()
                .skip(25)
                .collect::<String>()
                .starts_with("menu-continuity")
        })
    });
    eprintln!("PTY SMOKE: selected tab remains focused and accepts shell input after keyboard menu dismissal");
    writer.write_all(b"\x02q").unwrap();
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
        .position(|row| row.trim_start().starts_with("Alpha"))
        .unwrap();
    let mut input = fixture.client.input().unwrap();
    for collapsed in [true, false] {
        let (plugin, action, context) =
            sidebar::action_at(fixture.client.declarations.as_ref().unwrap(), header).unwrap();
        input
            .invoke(&fixture.client.boot_id, plugin, action, context, None)
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
            rows.iter().any(|row| row.trim_start().starts_with("Beta")),
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
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    screen: ghostty_vt::Terminal,
}
impl LiveConsole {
    fn new_for_tab(core: &Core, workspace_id: &str, tab_id: &str) -> Self {
        // Stock fallback rows have the same labels but a different action owner.
        // Start the native client only after its actual drag source is published.
        let mut observer = core.attach();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if observer.declarations.as_ref().is_some_and(|trees| {
                sidebar::rows(trees).any(|(owner, node)| {
                    owner == "tommymorgan.agent-tree"
                        && matches!(node, Node::Row { action: Some(action), context, .. }
                            if action == "focus-tab"
                                && context.workspace_id.as_deref() == Some(workspace_id)
                                && context.tab_id.as_deref() == Some(tab_id)
                                && context.pane_id.is_none())
                })
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "agent-tree did not publish the native drag source"
            );
            observer.receive().unwrap();
        }
        drop(observer);
        Self::new(core)
    }

    fn new(core: &Core) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 25,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_trailboss"));
        command.args(["--session", &core.name]);
        for key in HOMES {
            command.env(key, &core.home);
        }
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            let name = key.to_string_lossy();
            name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
        }) {
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
            _master: pair.master,
            writer,
            rx,
            screen: ghostty_vt::Terminal::new(120, 25, 0).unwrap(),
        }
    }
    fn wait(
        &mut self,
        predicate: impl Fn(&[String], &ghostty_vt::Terminal) -> bool,
    ) -> Vec<String> {
        self.wait_for("live PTY tree", predicate)
    }
    fn wait_for(
        &mut self,
        stage: &str,
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
                        .take(30)
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
                "{stage}: client restarted/exited"
            );
            let bytes = self
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("{stage} did not update: {error}; {rows:?}"));
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
        let _ = self.writer.write_all(b"\x02q");
        let _ = self.writer.flush();
    }
}
#[test]
fn executable_pty_workspace_collapse_and_expand() {
    let fixture = enable_first_consumer_with_active_tabs(1);
    let mut console = LiveConsole::new(&fixture.core);
    let rows = console.wait(|rows, _| rows.iter().any(|row| row.contains("Activity03")));
    let header = rows
        .iter()
        .position(|row| row.trim_start().starts_with("Alpha"))
        .unwrap();
    console.click(header);
    let rows = console.wait(|rows, _| {
        rows[header].contains("3 hidden")
            && rows.iter().any(|row| row.contains("Idle"))
            && ["Build", "Review", "Activity03"]
                .iter()
                .all(|tab| !rows.iter().any(|row| row.contains(tab)))
    });
    for tab in ["Build", "Review", "Activity03"] {
        assert!(
            !rows.iter().any(|row| row.contains(tab)),
            "collapsed tab visible: {tab}"
        );
    }
    assert!(rows.iter().any(|row| row.contains("Idle")));
    console.click(header);
    let rows = console.wait(|rows, _| {
        ["Build", "Review", "Activity03"]
            .iter()
            .all(|tab| rows.iter().any(|row| row.contains(tab)))
            && !rows[header].contains("hidden")
    });
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
        extract(include_str!("../plans/2026-10-10-trailboss.md")),
        "living GWT/examples drift"
    );
}
fn producer_identity(core: &Core) -> (u32, String) {
    fn find(pid: u32) -> Option<u32> {
        let command = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        if String::from_utf8_lossy(&command).contains("agent_tree.ts") {
            return Some(pid);
        }
        let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
        children
            .split_whitespace()
            .filter_map(|pid| pid.parse().ok())
            .find_map(find)
    }
    let pid = find(core.child.id()).expect("live producer missing");
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let start = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned();
    (pid, start)
}
fn event_request(core: &Core, event: &str, method: &str, params: Value) -> Value {
    use std::{
        io::{BufRead, BufReader},
        os::unix::net::UnixStream,
    };
    let stream = UnixStream::connect(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    )
    .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let request: Request = serde_json::from_value(json!({"id":"event-observer", "method":"events.subscribe", "params":{"subscriptions":[{"type":event}]}})).unwrap();
    serde_json::to_writer(reader.get_mut(), &request).unwrap();
    reader.get_mut().write_all(b"\n").unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["result"]["type"],
        "subscription_started"
    );
    let result = core.request(method, params);
    line.clear();
    reader
        .read_line(&mut line)
        .unwrap_or_else(|error| panic!("{method} did not emit {event}: {error}"));
    let observed: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        observed["event"],
        event.replace('.', "_"),
        "wrong real core event: {observed}"
    );
    result
}
fn refresh_tree_from_session_events(executable: bool) {
    crossterm::style::force_color_output(true);
    let mut fixture = enable_first_consumer();
    let producer = producer_identity(&fixture.core);
    let boot = fixture.client.boot_id.clone();
    let mut console = executable.then(|| LiveConsole::new(&fixture.core));
    if let Some(console) = &mut console {
        console.wait(|rows, _| rows.iter().any(|row| row.contains("Review")));
    }
    let created = event_request(
        &fixture.core,
        "workspace.created",
        "workspace.create",
        json!({"label":"EventDelta", "cwd":fixture.core.home, "focus":false}),
    );
    let workspace = created["workspace"]["workspace_id"].clone();
    let observe =
        |fixture: &mut Fixture, console: &mut Option<LiveConsole>, label: &str, present: bool| {
            wait_sidebar(&mut fixture.client, |rows| {
                rows.iter().any(|row| row.contains(label)) == present
            });
            if let Some(console) = console {
                console.wait(|rows, _| rows.iter().any(|row| row.contains(label)) == present);
            }
            assert_eq!(
                producer_identity(&fixture.core),
                producer,
                "producer restarted"
            );
            assert_eq!(fixture.client.boot_id, boot, "client/core restarted");
        };
    observe(&mut fixture, &mut console, "EventDelta", true);
    event_request(
        &fixture.core,
        "workspace.renamed",
        "workspace.rename",
        json!({"workspace_id":workspace,"label":"EventRenamed"}),
    );
    observe(&mut fixture, &mut console, "EventRenamed", true);
    assert!(!rendered_sidebar(&fixture.client)
        .iter()
        .any(|row| row.contains("EventDelta")));
    let tab = event_request(
        &fixture.core,
        "tab.created",
        "tab.create",
        json!({"workspace_id":workspace,"label":"EventTab", "cwd":fixture.core.home,"focus":false}),
    );
    observe(&mut fixture, &mut console, "○ EventTab", true);
    let tab_id = tab["tab"]["tab_id"].clone();
    let pane_id = tab["root_pane"]["pane_id"].clone();
    event_request(
        &fixture.core,
        "tab.focused",
        "tab.focus",
        json!({"tab_id":tab_id}),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixture.client.focused_tab_id.as_deref() != tab_id.as_str() {
        assert!(Instant::now() < deadline);
        fixture.client.receive().unwrap();
    }
    let rows = rendered_sidebar(&fixture.client);
    let y = rows
        .iter()
        .position(|row| row.contains("EventTab"))
        .unwrap();
    let mut bytes = Vec::new();
    sidebar::draw(
        &mut bytes,
        fixture.client.declarations.as_ref().unwrap(),
        terminal::Rect {
            x: 0,
            y: 0,
            width: 34,
            height: 24,
        },
        fixture.client.focused_tab_id.as_deref(),
    )
    .unwrap();
    let mut focused_screen = ghostty_vt::Terminal::new(34, 24, 0).unwrap();
    focused_screen.write(&bytes);
    assert!(
        cell_style(&focused_screen, y, 4).0.bold,
        "selected tab not visibly focused"
    );
    if let Some(console) = &mut console {
        console.wait(|rows, screen| {
            rows.iter()
                .position(|row| row.contains("EventTab"))
                .is_some_and(|y| cell_style(screen, y, 4).0.bold)
        });
    }
    event_request(
        &fixture.core,
        "pane.agent_detected",
        "pane.report_agent",
        json!({"pane_id":pane_id,"source":"event-fixture","agent":"codex","state":"working"}),
    );
    observe(&mut fixture, &mut console, "● EventTab", true);
    // Changing agent ownership clears its assigned name and emits pane.updated
    // with the new status, in addition to the retained status/detection events.
    fixture.core.request(
        "agent.rename",
        json!({"target":pane_id,"name":"event-agent"}),
    );
    event_request(
        &fixture.core,
        "pane.updated",
        "pane.report_agent",
        json!({"pane_id":pane_id,"source":"event-fixture","agent":"claude","state":"blocked"}),
    );
    observe(&mut fixture, &mut console, "● EventTab", true);
    event_request(
        &fixture.core,
        "tab.closed",
        "tab.close",
        json!({"tab_id":tab_id}),
    );
    observe(&mut fixture, &mut console, "EventTab", false);
    event_request(
        &fixture.core,
        "workspace.closed",
        "workspace.close",
        json!({"workspace_id":workspace}),
    );
    observe(&mut fixture, &mut console, "EventRenamed", false);
    eprintln!("LIVE EVENTS: all eight outline examples changed rendered tree; same producer {producer:?}, same attached client");
}
#[test]
fn refresh_the_tree_from_all_eight_session_events() {
    refresh_tree_from_session_events(false);
}
#[test]
fn living_refresh_the_tree_from_all_eight_session_events() {
    assert_living_scenario("Scenario Outline: Refresh the tree from session events", 15);
    refresh_tree_from_session_events(false);
}
#[test]
fn executable_pty_refreshes_all_eight_session_events_without_restart() {
    refresh_tree_from_session_events(true);
}

#[test]
#[ignore = "requires an explicitly supplied pre-transfer legacy Core executable"]
fn executable_pty_current_store_starts_and_refreshes_against_legacy_core() {
    let legacy = std::path::PathBuf::from(
        std::env::var_os("TRAILBOSS_TEST_LEGACY_CORE")
            .expect("set TRAILBOSS_TEST_LEGACY_CORE to a pre-transfer Core executable"),
    );
    assert!(
        legacy.is_absolute(),
        "legacy executable must be an explicit absolute path"
    );
    let name = format!(
        "legacy-store-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let home = std::env::temp_dir().join(&name);
    std::fs::create_dir_all(&home).unwrap();
    let core = Core::launch_with_executable(home, name, legacy);
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let workspace = tools["workspace"]["workspace_id"].as_str().unwrap();
    let build = tools["tab"]["tab_id"].as_str().unwrap();
    core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
    let remaining = core.request(
        "tab.create",
        json!({
            "workspace_id":workspace,"label":"Remaining","cwd":core.home
        }),
    );
    let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
    let diagnostics = logs["root_pane"]["pane_id"].as_str().unwrap();
    core.request(
        "tab.rename",
        json!({"tab_id":logs["tab"]["tab_id"],"label":"Diagnostics"}),
    );
    core.request("pane.focus", json!({"pane_id":diagnostics}));
    core.request(
        "plugin.link",
        json!({
            "path":format!("{}/plugins/agent-tree",env!("CARGO_MANIFEST_DIR")),"enabled":true
        }),
    );
    let mut console = LiveConsole::new(&core);
    let native_pid = console.child.0.process_id();
    console.wait(|rows, _| {
        ["Tools", "Build", "Remaining", "Diagnostics"]
            .iter()
            .all(|label| rows.iter().any(|row| row.contains(label)))
    });
    // These legacy topology events must refresh the production store and live
    // declaration without reopening the native client or manually refreshing.
    core.request(
        "workspace.rename",
        json!({"workspace_id":workspace,"label":"UpdatedTools"}),
    );
    core.request("tab.rename", json!({"tab_id":build,"label":"RenamedBuild"}));
    core.request(
        "tab.move",
        json!({"tab_id":remaining["tab"]["tab_id"],"insert_index":0}),
    );
    console.wait(|rows, _| {
        let Some(header) = rows
            .iter()
            .position(|row| row.trim_start().starts_with("UpdatedTools"))
        else {
            return false;
        };
        rows[header + 1].contains("Remaining") && rows[header + 2].contains("RenamedBuild")
    });
    let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert_eq!(
        snapshot["focused_workspace_id"],
        logs["workspace"]["workspace_id"]
    );
    assert_eq!(snapshot["focused_tab_id"], logs["tab"]["tab_id"]);
    assert_eq!(snapshot["focused_pane_id"], diagnostics);
    let tabs = core.request("tab.list", json!({"workspace_id":workspace}));
    assert_eq!(tabs["tabs"][0]["tab_id"], remaining["tab"]["tab_id"]);
    assert_eq!(tabs["tabs"][1]["tab_id"], build);
    assert_eq!(tabs["tabs"][1]["label"], "RenamedBuild");
    assert_eq!(
        console.child.0.process_id(),
        native_pid,
        "native client restarted"
    );
    assert!(
        console.child.0.try_wait().unwrap().is_none(),
        "native client exited"
    );
}

#[test]
fn transferring_a_tab_keeps_its_running_popup_and_client_input() {
    use std::os::unix::net::UnixStream;
    use trailboss_runtime::protocol::{
        self,
        endpoint::{
            EndpointClientHello, BLOB_CODEC_V1, ENDPOINT_HELLO_KIND, INPUT_CODEC_V1,
            SNAPSHOT_CODEC_V1, SURFACE_CODEC_V1,
        },
        ClientMessage, ClientPaneInputEvent, ClientSurfaceSize, PaneSurfaceFrame, ServerMessage,
    };
    fn surface(
        stream: &mut UnixStream,
        stage: &str,
        ready: impl Fn(&PaneSurfaceFrame) -> bool,
    ) -> PaneSurfaceFrame {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "{stage}: surface did not arrive");
            let message: ServerMessage =
                protocol::read_message(stream, protocol::MAX_GRAPHICS_FRAME_SIZE)
                    .unwrap_or_else(|error| panic!("{stage}: {error}"));
            if let ServerMessage::PaneSurface(frame) = message {
                if ready(&frame) {
                    return frame;
                }
            }
        }
    }
    fn popup_text(frame: &PaneSurfaceFrame) -> String {
        frame
            .popup
            .as_ref()
            .map(|popup| {
                popup
                    .frame
                    .cells
                    .iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn process(pid: &str) -> (String, std::path::PathBuf) {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        (
            stat.rsplit_once(") ")
                .unwrap()
                .1
                .split_whitespace()
                .nth(19)
                .unwrap()
                .into(),
            std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        )
    }

    let core = Core::new();
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let source = tools["workspace"]["workspace_id"].as_str().unwrap();
    let build = tools["tab"]["tab_id"].as_str().unwrap();
    core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
    core.request(
        "tab.create",
        json!({"workspace_id":source,"label":"Remaining","cwd":core.home}),
    );
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    core.request("tab.focus", json!({"tab_id":build}));

    let plugin = core.home.join("popup-plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    let pid_file = core.home.join("popup.pid");
    let reply_file = core.home.join("popup.reply");
    std::fs::write(plugin.join("trailboss-plugin.toml"), format!(r#"
id = "example.transfer-popup"
name = "Transfer Popup"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux"]
[[panes]]
id = "live"
title = "Running Build Popup"
placement = "popup"
command = ["sh", "-c", '''popup_local=popup-state; printf '%s\n' "$$" > '{}'; printf 'POPUP_READY\n'; while IFS= read -r line; do printf '%s %s %s\n' "$$" "$popup_local" "$line" > '{}'; printf 'POPUP_REPLY-%s\n' "$line"; done''']
"#, pid_file.display(), reply_file.display())).unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));

    let mut client = UnixStream::connect(core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    )))
    .unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello = EndpointClientHello {
        generation: 1,
        cell_width_px: 0,
        cell_height_px: 0,
        surface_size: ClientSurfaceSize { cols: 80, rows: 24 },
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: true,
        surface_reuse: false,
        surface_delta: false,
        surface_scroll: false,
        snapshot_codecs: vec![SNAPSHOT_CODEC_V1.into()],
        surface_codecs: vec![SURFACE_CODEC_V1.into()],
        input_codecs: vec![INPUT_CODEC_V1.into()],
        blob_codecs: vec![BLOB_CODEC_V1.into()],
    };
    protocol::write_message(
        &mut client,
        &ClientMessage::EndpointControl {
            kind: ENDPOINT_HELLO_KIND.into(),
            data: serde_json::to_string(&hello).unwrap(),
        },
    )
    .unwrap();
    surface(&mut client, "Build before popup", |frame| {
        frame
            .panes
            .iter()
            .any(|pane| pane.pane_id == tools["root_pane"]["pane_id"])
    });
    core.request(
        "plugin.pane.open",
        json!({
            "plugin_id":"example.transfer-popup","entrypoint":"live","focus":true
        }),
    );
    let opened = surface(&mut client, "running popup before transfer", |frame| {
        popup_text(frame).contains("POPUP_READY")
    });
    let terminal = opened.popup.as_ref().unwrap().terminal_id.clone();
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_owned();
    let original_process = process(&pid);

    let moved = core.request(
        "tab.transfer",
        json!({
            "tab_id":build,"workspace_id":main["workspace"]["workspace_id"]
        }),
    );
    let new_pane = moved["transfer"]["panes"][0]["pane_id"].as_str().unwrap();
    let after = surface(&mut client, "Build after transfer", |frame| {
        frame.panes.iter().any(|pane| pane.pane_id == new_pane)
    });
    let popup = after
        .popup
        .as_ref()
        .expect("transferring Build closed its existing running popup");
    assert_eq!(popup.terminal_id, terminal, "popup runtime was replaced");
    assert_eq!(
        process(&pid),
        original_process,
        "popup process or PTY changed"
    );

    protocol::write_message(
        &mut client,
        &ClientMessage::ClientShellPopupInput {
            terminal_id: terminal,
            events: vec![ClientPaneInputEvent::TextCommit("after-transfer\n".into())],
        },
    )
    .unwrap();
    surface(
        &mut client,
        "same popup accepts input after transfer",
        |frame| popup_text(frame).contains("POPUP_REPLY-after-transfer"),
    );
    assert_eq!(
        std::fs::read_to_string(reply_file).unwrap(),
        format!("{pid} popup-state after-transfer\n")
    );
    assert_eq!(process(&pid), original_process);
    core.request("popup.close", json!({}));
}

#[test]
fn transferred_overlay_exit_restores_the_moved_tabs_original_zoom_and_focus() {
    let core = Core::new();
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let build = tools["tab"]["tab_id"].as_str().unwrap();
    let first = tools["root_pane"]["pane_id"].as_str().unwrap();
    core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
    let split = core.request(
        "pane.split",
        json!({
            "target_pane_id":first,"direction":"right","ratio":0.37,"cwd":core.home,"focus":true
        }),
    );
    let second = split["pane"]["pane_id"].as_str().unwrap();
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    core.request("pane.focus", json!({"pane_id":second}));
    core.request("pane.zoom", json!({"pane_id":second,"mode":"on"}));
    let original = core.request("pane.layout", json!({"pane_id":second}))["layout"].clone();
    assert_eq!(original["focused_pane_id"], second);
    assert_eq!(original["zoomed"], true);

    let plugin = core.home.join("overlay-plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    let ready = core.home.join("overlay.ready");
    std::fs::write(
        plugin.join("trailboss-plugin.toml"),
        format!(
            r#"
id = "example.transfer-overlay"
name = "Transfer Overlay"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux"]
[[panes]]
id = "live"
title = "Running Build Overlay"
placement = "overlay"
command = ["sh", "-c", '''printf ready > '{}'; IFS= read -r line; test "$line" = leave''']
"#,
            ready.display()
        ),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let opened = core.request(
        "plugin.pane.open",
        json!({
            "plugin_id":"example.transfer-overlay","entrypoint":"live","focus":true
        }),
    );
    let overlay = opened["plugin_pane"]["pane"]["pane_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "owned overlay command did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let before_exit = core.request("pane.layout", json!({"pane_id":overlay}))["layout"].clone();
    assert_eq!(before_exit["focused_pane_id"], overlay);
    assert_eq!(before_exit["panes"].as_array().unwrap().len(), 3);
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(overlay) {
        client.receive().unwrap();
    }

    let moved = core.request(
        "tab.transfer",
        json!({
            "tab_id":build,"workspace_id":main["workspace"]["workspace_id"]
        }),
    );
    assert_eq!(
        moved["transfer"]["closed_workspace_id"],
        tools["workspace"]["workspace_id"]
    );
    let mapped = |old: &str| {
        moved["transfer"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|pane| pane["previous_pane_id"] == old)
            .unwrap()["pane_id"]
            .as_str()
            .unwrap()
    };
    let moved_second = mapped(second);
    let moved_overlay = mapped(overlay);
    core.request(
        "pane.send_text",
        json!({"pane_id":moved_overlay,"text":"leave\n"}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let restored = loop {
        let layout = core.request("pane.layout", json!({"pane_id":moved_second}))["layout"].clone();
        if layout["panes"].as_array().unwrap().len() == 2 {
            break layout;
        }
        assert!(
            Instant::now() < deadline,
            "real overlay command did not exit"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        restored["zoomed"], true,
        "moved overlay exit lost Build's pre-existing zoom"
    );
    assert_eq!(
        restored["focused_pane_id"], moved_second,
        "moved overlay exit did not restore Build's original focused pane"
    );
    let before_splits = original["splits"].as_array().unwrap();
    let after_splits = restored["splits"].as_array().unwrap();
    assert_eq!(after_splits.len(), before_splits.len());
    for (before, after) in before_splits.iter().zip(after_splits) {
        for field in ["id", "direction", "ratio"] {
            assert_eq!(
                after[field], before[field],
                "restored split changed {field}"
            );
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.focused_pane_id.as_deref() != Some(moved_second) {
        assert!(
            Instant::now() < deadline,
            "attached client did not see restored Build focus"
        );
        client.receive().unwrap();
    }
    let untouched = core.request(
        "pane.layout",
        json!({"pane_id":main["root_pane"]["pane_id"]}),
    );
    assert_eq!(
        untouched["layout"]["zoomed"], false,
        "overlay exit changed Main's unrelated tab"
    );
}

#[test]
fn executable_pty_transfers_split_tab_to_each_cross_space_drop_slot() {
    use std::{
        io::{BufRead, BufReader},
        os::unix::net::UnixStream,
    };
    fn shell_state(core: &Core, pane: &str, index: usize, stage: &str) -> String {
        let initialize = if stage == "before" {
            format!("native_local=value-{index}; sleep 300 & native_job=$!; ")
        } else {
            String::new()
        };
        core.request("pane.send_text", json!({"pane_id":pane,"text":format!(
            "{initialize}kill -0 \"$native_job\" && printf '%s %s %s\\n' \"$$\" \"$native_job\" \"$native_local\" > native-{stage}-{index}\n"
        )}));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(state) =
                std::fs::read_to_string(core.home.join(format!("native-{stage}-{index}")))
            {
                if state.ends_with('\n') {
                    return state;
                }
            }
            assert!(
                Instant::now() < deadline,
                "native {stage} shell state missing"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn process(pid: &str) -> (String, std::path::PathBuf, std::path::PathBuf) {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        (
            stat.rsplit_once(") ")
                .unwrap()
                .1
                .split_whitespace()
                .nth(19)
                .unwrap()
                .to_owned(),
            std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
            std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
        )
    }
    for (target, expected) in [
        ("before", ["Build", "One", "Two"]),
        ("between", ["One", "Build", "Two"]),
        ("after", ["One", "Two", "Build"]),
        ("expanded-header", ["One", "Two", "Build"]),
        ("collapsed-header", ["One", "Two", "Build"]),
        ("walk-source-panes", ["One", "Two", "Build"]),
        ("walk-intervening-panes", ["One", "Two", "Build"]),
    ] {
        let core = Core::new();
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let build = tools["tab"]["tab_id"].as_str().unwrap();
        let first = tools["root_pane"]["pane_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
        let split = core.request(
            "pane.split",
            json!({"target_pane_id":first,"direction":"right","ratio":0.37,"cwd":core.home}),
        );
        let second = split["pane"]["pane_id"].as_str().unwrap();
        let row_walk = target.starts_with("walk-");
        if row_walk {
            for (pane, label) in [(first, "SourceLeft"), (second, "SourceRight")] {
                core.request("pane.rename", json!({"pane_id":pane,"label":label}));
            }
        } else {
            core.request(
                "tab.create",
                json!({"workspace_id":source,"label":"Remaining","cwd":core.home}),
            );
        }
        if target == "walk-intervening-panes" {
            let transit = core.request(
                "workspace.create",
                json!({
                    "label":"Transit","cwd":core.home
                }),
            );
            core.request(
                "tab.rename",
                json!({
                    "tab_id":transit["tab"]["tab_id"],"label":"Inspect"
                }),
            );
            let transit_split = core.request(
                "pane.split",
                json!({
                    "target_pane_id":transit["root_pane"]["pane_id"],
                    "direction":"right","cwd":core.home
                }),
            );
            for (pane, label) in [
                (&transit["root_pane"]["pane_id"], "TransitLeft"),
                (&transit_split["pane"]["pane_id"], "TransitRight"),
            ] {
                core.request("pane.rename", json!({"pane_id":pane,"label":label}));
            }
        }
        let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
        let destination = main["workspace"]["workspace_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":main["tab"]["tab_id"],"label":"One"}),
        );
        core.request(
            "tab.create",
            json!({"workspace_id":destination,"label":"Two","cwd":core.home}),
        );
        let panes = [first, second];
        let originals = panes
            .iter()
            .enumerate()
            .map(|(index, pane)| {
                let state = shell_state(&core, pane, index, "before");
                let ids = state
                    .split_whitespace()
                    .take(2)
                    .map(process)
                    .collect::<Vec<_>>();
                (
                    state,
                    ids,
                    core.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"]
                        .clone(),
                )
            })
            .collect::<Vec<_>>();
        if target == "collapsed-header" {
            core.request("tab.focus", json!({"tab_id":main["tab"]["tab_id"]}));
        }
        core.request("plugin.link", json!({"path":format!("{}/plugins/agent-tree",env!("CARGO_MANIFEST_DIR")),"enabled":true}));
        let mut console = LiveConsole::new_for_tab(&core, source, build);
        let mut rows = console.wait(|rows, _| {
            ["Build", "One", "Two"]
                .iter()
                .all(|label| rows.iter().any(|row| row.contains(label)))
                && if row_walk {
                    ["SourceLeft", "SourceRight"]
                        .iter()
                        .all(|label| rows.iter().any(|row| row.contains(label)))
                } else {
                    rows.iter().any(|row| row.contains("Remaining"))
                }
                && (target != "walk-intervening-panes"
                    || ["TransitLeft", "TransitRight"]
                        .iter()
                        .all(|label| rows.iter().any(|row| row.contains(label))))
        });
        let layout = core.request("pane.layout", json!({"pane_id":second}))["layout"].clone();
        let header = rows
            .iter()
            .position(|row| row.trim_start().starts_with("Main"))
            .unwrap();
        if target == "collapsed-header" {
            console.click(header);
            rows = console.wait(|rows, _| {
                rows[header].contains("hidden") && !rows.iter().any(|row| row.contains("One"))
            });
        }
        let source_row = rows.iter().position(|row| row.contains("Build")).unwrap();
        let target_row = match target {
            "before" => rows.iter().position(|row| row.contains("One")).unwrap(),
            "between" => rows.iter().position(|row| row.contains("Two")).unwrap(),
            "after" => rows.iter().position(|row| row.contains("Two")).unwrap() + 1,
            _ => header,
        };
        let before_source = core.request("tab.list", json!({"workspace_id":source}));
        let before_destination = core.request("tab.list", json!({"workspace_id":destination}));
        let socket = core
            .home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
        let mut events = BufReader::new(UnixStream::connect(socket).unwrap());
        events
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(events.get_mut(), &json!({"id":"native-transfer-events","method":"events.subscribe","params":{"subscriptions":[{"type":"tab.transferred"}]}})).unwrap();
        events.get_mut().write_all(b"\n").unwrap();
        let mut line = String::new();
        events.read_line(&mut line).unwrap();
        let subscribed: Value = serde_json::from_str(&line).unwrap();
        assert!(subscribed.get("error").is_none(), "{subscribed}");
        console
            .writer
            .write_all(format!("\x1b[<0;5;{}M", source_row + 1).as_bytes())
            .unwrap();
        if row_walk {
            for row in source_row + 1..=target_row {
                console
                    .writer
                    .write_all(format!("\x1b[<32;5;{}M", row + 1).as_bytes())
                    .unwrap();
            }
        } else {
            console
                .writer
                .write_all(format!("\x1b[<32;5;{}M", target_row + 1).as_bytes())
                .unwrap();
        }
        console.writer.flush().unwrap();
        console.wait_for(
            &format!("{target}: Main insertion after pointer motion"),
            |rows, _| {
                rows.iter().enumerate().any(|(index, row)| {
                    row.contains("────────────────────")
                        && if target.ends_with("header") || row_walk {
                            index >= header
                        } else {
                            index == target_row
                        }
                })
            },
        );
        assert_eq!(
            core.request("tab.list", json!({"workspace_id":source})),
            before_source,
            "{target}: source mutated before release"
        );
        assert_eq!(
            core.request("tab.list", json!({"workspace_id":destination})),
            before_destination,
            "{target}: destination mutated before release"
        );
        console
            .writer
            .write_all(format!("\x1b[<0;5;{}m", target_row + 1).as_bytes())
            .unwrap();
        console.writer.flush().unwrap();
        line.clear();
        events.read_line(&mut line).unwrap_or_else(|error| {
            let last: Vec<String> = console.screen.screen_text_rows().unwrap().iter()
                .map(|row| row.cells.iter().take(30)
                    .flat_map(|cell| cell.graphemes.iter())
                    .filter_map(|ch| char::from_u32(*ch)).collect())
                .collect();
            panic!("{target}: post-release tab.transferred read failed: {error}; initial sidebar={rows:?}; last rendered sidebar={last:?}");
        });
        let event: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(event["event"], "tab_transferred", "{target}: {event}");
        let tabs = core.request("tab.list", json!({"workspace_id":destination}));
        assert_eq!(
            tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tab| tab["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected,
            "{target}"
        );
        if row_walk {
            assert!(
                core.request("session.snapshot", json!({}))["snapshot"]["workspaces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|workspace| workspace["workspace_id"] != source)
            );
        } else {
            assert_eq!(
                core.request("tab.list", json!({"workspace_id":source}))["tabs"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
        for (index, pane) in panes.iter().enumerate() {
            let state = shell_state(&core, pane, index, "after");
            assert_eq!(state, originals[index].0, "{target}: shell state changed");
            for (pid, identity) in state.split_whitespace().take(2).zip(&originals[index].1) {
                assert_eq!(&process(pid), identity, "{target}: process/PTY changed");
            }
            let current = core.request("pane.get", json!({"pane_id":pane}));
            assert_eq!(current["pane"]["terminal_id"], originals[index].2);
            assert_eq!(current["pane"]["workspace_id"], destination);
            let after = core.request("pane.layout", json!({"pane_id":current["pane"]["pane_id"]}));
            let semantics = |layout: &Value| {
                layout["splits"].as_array().unwrap().iter()
                .map(|split| json!({"id":split["id"],"direction":split["direction"],"ratio":split["ratio"]}))
                .collect::<Vec<_>>()
            };
            assert_eq!(
                semantics(&after["layout"]),
                semantics(&layout),
                "{target}: split semantics changed"
            );
        }
        events
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        line.clear();
        assert!(
            events.read_line(&mut line).is_err(),
            "{target}: duplicate transfer event {line}"
        );
    }
}

#[test]
fn executable_pty_sidebar_transfer_preserves_viewed_and_background_panes() {
    fn native_input(
        console: &mut LiveConsole,
        core: &Core,
        case: &str,
        stage: &str,
        expected: &str,
    ) {
        let marker = format!("NATIVE-{stage}-{case}");
        console.writer.write_all(format!(
            "printf '%s %s\\n' \"$native_view\" \"$$\" > native-view-{stage}; printf '%s%s\\n' 'NATIVE-{stage}-' '{case}'\r"
        ).as_bytes()).unwrap();
        console.writer.flush().unwrap();
        console.wait(|_, screen| {
            screen.screen_text_rows().unwrap().iter().any(|row| {
                row.cells
                    .iter()
                    .skip(30)
                    .flat_map(|cell| cell.graphemes.iter())
                    .filter_map(|ch| char::from_u32(*ch))
                    .collect::<String>()
                    .contains(&marker)
            })
        });
        assert_eq!(
            std::fs::read_to_string(core.home.join(format!("native-view-{stage}")))
                .unwrap()
                .trim(),
            format!(
                "{expected} {}",
                std::fs::read_to_string(core.home.join(format!("native-ready-{expected}")))
                    .unwrap()
                    .trim()
            ),
            "{case}: native input reached a different pane"
        );
    }
    for case in ["viewed", "background"] {
        let core = Core::new();
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let build = tools["tab"]["tab_id"].as_str().unwrap();
        let first = tools["root_pane"]["pane_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
        let split = core.request(
            "pane.split",
            json!({"target_pane_id":first,"direction":"right","ratio":0.37,"cwd":core.home}),
        );
        let second = split["pane"]["pane_id"].as_str().unwrap();
        core.request(
            "tab.create",
            json!({"workspace_id":source,"label":"Remaining","cwd":core.home}),
        );
        let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
        let destination = main["workspace"]["workspace_id"].as_str().unwrap();
        let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
        let diagnostics = logs["root_pane"]["pane_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":logs["tab"]["tab_id"],"label":"Diagnostics"}),
        );
        for (pane, value) in [
            (first, "first"),
            (second, "second"),
            (diagnostics, "diagnostics"),
        ] {
            core.request(
                "pane.send_text",
                json!({"pane_id":pane,"text":format!(
                    "native_view={value}; printf '%s\\n' \"$$\" > native-ready-{value}\n"
                )}),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while ["first", "second", "diagnostics"].iter().any(|value| {
            !std::fs::read_to_string(core.home.join(format!("native-ready-{value}")))
                .is_ok_and(|text| text.ends_with('\n'))
        }) {
            assert!(
                Instant::now() < deadline,
                "view fixture shells did not initialize"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let (viewed, expected) = if case == "viewed" {
            (second, "second")
        } else {
            (diagnostics, "diagnostics")
        };
        core.request("pane.focus", json!({"pane_id":viewed}));
        let before = core.request("pane.get", json!({"pane_id":viewed}))["pane"].clone();
        core.request("plugin.link",json!({"path":format!("{}/plugins/agent-tree",env!("CARGO_MANIFEST_DIR")),"enabled":true}));
        let mut console = LiveConsole::new_for_tab(&core, source, build);
        let rows = console.wait(|rows, _| {
            ["Build", "Main", "Diagnostics"]
                .iter()
                .all(|label| rows.iter().any(|row| row.contains(label)))
        });
        native_input(&mut console, &core, case, "BEFORE", expected);
        let build_row = rows.iter().position(|row| row.contains("Build")).unwrap();
        let main_row = rows
            .iter()
            .position(|row| row.trim_start().starts_with("Main"))
            .unwrap();
        console
            .writer
            .write_all(
                format!("\x1b[<0;5;{}M\x1b[<32;5;{}M", build_row + 1, main_row + 1).as_bytes(),
            )
            .unwrap();
        console.writer.flush().unwrap();
        console.wait(|rows, _| rows[main_row].contains("────────────────────"));
        console
            .writer
            .write_all(format!("\x1b[<0;5;{}m", main_row + 1).as_bytes())
            .unwrap();
        console.writer.flush().unwrap();
        console.wait(|rows, _| {
            !rows[main_row].contains("────────────────────")
                && core.request("pane.get", json!({"pane_id":second}))["pane"]["workspace_id"]
                    == destination
        });
        native_input(&mut console, &core, case, "AFTER", expected);
        let current = core.request("pane.get", json!({"pane_id":viewed}))["pane"].clone();
        assert_eq!(
            current["terminal_id"], before["terminal_id"],
            "{case}: viewed terminal changed"
        );
        let expected_space = if case == "viewed" {
            destination
        } else {
            logs["workspace"]["workspace_id"].as_str().unwrap()
        };
        assert_eq!(current["workspace_id"], expected_space);
        let snapshot = core.request("session.snapshot", json!({}));
        assert_eq!(
            snapshot["snapshot"]["focused_workspace_id"], expected_space,
            "{case}: focused space changed"
        );
        assert_eq!(
            snapshot["snapshot"]["focused_pane_id"], current["pane_id"],
            "{case}: focused pane changed"
        );
        assert_eq!(
            snapshot["snapshot"]["focused_tab_id"], current["tab_id"],
            "{case}: focused tab changed"
        );
        assert_eq!(
            core.request("pane.layout", json!({"pane_id":viewed}))["layout"]["focused_pane_id"],
            current["pane_id"]
        );
    }
}

#[test]
fn executable_pty_transfer_uses_the_stationary_pointers_release_time_row() {
    let core = Core::new();
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let source = tools["workspace"]["workspace_id"].as_str().unwrap();
    let build = tools["tab"]["tab_id"].as_str().unwrap();
    core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
    core.request(
        "tab.create",
        json!({"workspace_id":source,"label":"Remaining","cwd":core.home}),
    );
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    let destination = main["workspace"]["workspace_id"].as_str().unwrap();
    core.request(
        "tab.rename",
        json!({"tab_id":main["tab"]["tab_id"],"label":"One"}),
    );
    core.request(
        "tab.create",
        json!({"workspace_id":destination,"label":"Two","cwd":core.home}),
    );
    core.request(
        "plugin.link",
        json!({"path":format!("{}/plugins/agent-tree",env!("CARGO_MANIFEST_DIR")),"enabled":true}),
    );
    let mut console = LiveConsole::new_for_tab(&core, source, build);
    let rows = console.wait(|rows, _| {
        ["Build", "One", "Two"]
            .iter()
            .all(|label| rows.iter().any(|row| row.contains(label)))
    });
    let build_row = rows.iter().position(|row| row.contains("Build")).unwrap();
    let pointer_row = rows.iter().position(|row| row.contains("One")).unwrap();
    assert!(rows[pointer_row + 1].contains("Two"));
    console
        .writer
        .write_all(
            format!(
                "\x1b[<0;5;{}M\x1b[<32;5;{}M",
                build_row + 1,
                pointer_row + 1
            )
            .as_bytes(),
        )
        .unwrap();
    console.writer.flush().unwrap();
    console.wait(|rows, _| rows[pointer_row].contains("────────────────────"));
    let zero = core.request(
        "tab.create",
        json!({"workspace_id":destination,"label":"Zero","cwd":core.home}),
    );
    core.request(
        "tab.move",
        json!({"tab_id":zero["tab"]["tab_id"],"insert_index":0}),
    );
    // No mouse motion follows the external update: the insertion line masks Zero
    // at the stationary pointer, while One and Two must shift down in the live UI.
    console.wait(|rows, _| {
        rows[pointer_row].contains("────────────────────")
            && rows[pointer_row + 1].contains("One")
            && rows[pointer_row + 2].contains("Two")
    });
    let labels = |workspace: &str| {
        core.request("tab.list", json!({"workspace_id":workspace}))["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["label"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(destination), ["Zero", "One", "Two"]);
    assert_eq!(labels(source), ["Build", "Remaining"]);
    console
        .writer
        .write_all(format!("\x1b[<0;5;{}m", pointer_row + 1).as_bytes())
        .unwrap();
    console.writer.flush().unwrap();
    let rows = console.wait(|rows, _| {
        let Some(header) = rows
            .iter()
            .position(|row| row.trim_start().starts_with("Main"))
        else {
            return false;
        };
        rows[header + 1].contains("Build")
            && rows[header + 2].contains("Zero")
            && rows[header + 3].contains("One")
            && rows[header + 4].contains("Two")
    });
    let header = rows
        .iter()
        .position(|row| row.trim_start().starts_with("Main"))
        .unwrap();
    assert!(rows[header + 1..header + 5]
        .iter()
        .all(|row| !row.trim_start().starts_with("────────────────────")));
    assert_eq!(labels(destination), ["Build", "Zero", "One", "Two"]);
    assert_eq!(labels(source), ["Remaining"]);
}

#[test]
fn executable_pty_sidebar_transfer_cancels_every_interrupted_capture() {
    use std::{io::BufRead, io::BufReader, os::unix::net::UnixStream, path::PathBuf};

    struct StopOwner(PathBuf);
    impl Drop for StopOwner {
        fn drop(&mut self) {
            let api = trailboss_runtime::api_client::ApiClient::for_target(
                trailboss_runtime::api_client::ConnectionTarget::SocketPath(self.0.clone()),
            );
            let request = serde_json::from_value::<trailboss_runtime::api_schema::Request>(
                json!({"id":"stop-handoff-owner","method":"server.stop","params":{}}),
            )
            .unwrap();
            let _ = api.request_value_with_timeout(&request, Duration::from_secs(2));
            let deadline = Instant::now() + Duration::from_secs(2);
            while self.0.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    fn topology(core: &Core) -> serde_json::Value {
        let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
        let select = |key: &str, fields: &[&str]| {
            snapshot[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|entity| {
                    fields
                        .iter()
                        .map(|field| ((*field).to_owned(), entity[*field].clone()))
                        .collect::<serde_json::Map<_, _>>()
                })
                .collect::<Vec<_>>()
        };
        json!({
            "workspace":snapshot["focused_workspace_id"],
            "tab":snapshot["focused_tab_id"],
            "pane":snapshot["focused_pane_id"],
            "workspaces":select("workspaces", &["workspace_id","number","active_tab_id"]),
            "tabs":select("tabs", &["tab_id","workspace_id","number","label"]),
            "panes":select("panes", &["pane_id","terminal_id","workspace_id","tab_id"]),
        })
    }
    for case in [
        "source-close",
        "destination-close",
        "boot",
        "declaration-replacement",
        "viewport",
        "modal",
        "outside",
        "pane-row",
        "escape",
    ] {
        let core = Core::new();
        let socket = core
            .home
            .join("trailboss/sessions")
            .join(&core.name)
            .join("trailboss.sock");
        // The imported owner is not Core.child; stop only this owned session.
        let _owner = (case == "boot").then(|| StopOwner(socket.clone()));
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let build = tools["tab"]["tab_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
        core.request(
            "tab.create",
            json!({"workspace_id":source,"label":"Remaining","cwd":core.home}),
        );
        let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
        let destination = main["workspace"]["workspace_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":main["tab"]["tab_id"],"label":"One"}),
        );
        let split = core.request(
            "pane.split",
            json!({
                "target_pane_id":main["root_pane"]["pane_id"],"direction":"right","cwd":core.home
            }),
        );
        for (pane, label) in [
            (&main["root_pane"]["pane_id"], "TargetLeft"),
            (&split["pane"]["pane_id"], "TargetRight"),
        ] {
            core.request("pane.rename", json!({"pane_id":pane,"label":label}));
        }
        let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
        let diagnostics = logs["root_pane"]["pane_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":logs["tab"]["tab_id"],"label":"Diagnostics"}),
        );
        core.request("pane.focus", json!({"pane_id":diagnostics}));
        core.request(
            "plugin.link",
            json!({
                "path":format!("{}/plugins/agent-tree",env!("CARGO_MANIFEST_DIR")),"enabled":true
            }),
        );
        let mut console = LiveConsole::new_for_tab(&core, source, build);
        let mut native_pid = console.child.0.process_id();
        let rows = console.wait(|rows, _| {
            ["Build", "TargetLeft", "TargetRight", "Diagnostics"]
                .iter()
                .all(|label| rows.iter().any(|row| row.contains(label)))
        });
        let source_row = rows.iter().position(|row| row.contains("Build")).unwrap();
        let target_row = rows
            .iter()
            .position(|row| row.trim_start().starts_with("Main"))
            .unwrap();
        let pane_row = rows
            .iter()
            .position(|row| row.contains("TargetRight"))
            .unwrap();
        console
            .writer
            .write_all(
                format!(
                    "\x1b[<0;5;{}M\x1b[<32;5;{}M",
                    source_row + 1,
                    target_row + 1
                )
                .as_bytes(),
            )
            .unwrap();
        console.writer.flush().unwrap();
        // A normal workspace header also contains dashes: require the insertion
        // line to replace its label, not merely find its ordinary separator.
        console.wait(|rows, _| {
            rows[target_row]
                .trim_start()
                .starts_with("────────────────────")
        });
        match case {
            "source-close" => {
                core.request("tab.close", json!({"tab_id":build}));
                console.wait(|rows, _| !rows.iter().any(|row| row.contains("Build")));
            }
            "destination-close" => {
                core.request("workspace.close", json!({"workspace_id":destination}));
                console.wait(|rows, _| {
                    !rows.iter().any(|row| row.contains("TargetRight"))
                        && !rows
                            .iter()
                            .any(|row| row.trim_start().starts_with("────────────────────"))
                });
            }
            "boot" => {
                let mut before = core.attach();
                while before.boot_id.is_empty() {
                    before.receive().unwrap();
                }
                let old_boot = before.boot_id.clone();
                drop(before);
                core.request(
                    "server.live_handoff",
                    json!({"import_exe":support::executable()}),
                );
                let mut after = core.attach();
                while after.boot_id.is_empty() {
                    after.receive().unwrap();
                }
                assert_ne!(
                    after.boot_id, old_boot,
                    "{case}: ownership did not change boot"
                );
                let deadline = Instant::now() + Duration::from_secs(8);
                while console.child.0.try_wait().unwrap().is_none() {
                    assert!(
                        Instant::now() < deadline,
                        "{case}: old native client retained its capture"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                // Handoff currently ends the native connection. Do not invent
                // reconnect behavior: the old captured gesture dies with it,
                // and releasing on a fresh client must not dispatch anything.
                console = LiveConsole::new_for_tab(&core, source, build);
                assert_ne!(
                    console.child.0.process_id(),
                    native_pid,
                    "{case}: client did not change"
                );
                native_pid = console.child.0.process_id();
                console.wait(|rows, _| {
                    ["Build", "TargetRight", "Diagnostics"]
                        .iter()
                        .all(|label| rows.iter().any(|row| row.contains(label)))
                        && !rows[target_row]
                            .trim_start()
                            .starts_with("────────────────────")
                });
            }
            "declaration-replacement" | "modal" => {
                let plugin = core.home.join("interrupt-capture");
                std::fs::create_dir_all(&plugin).unwrap();
                let tree = if case == "modal" {
                    // Resolve the unchanged agent-tree list zone in the same
                    // sidebar geometry; only the overlay captures input.
                    json!({"type":"row","children":[
                        {"type":"flex","weight":1,"child":{"type":"column","children":[
                            {"type":"flex","weight":1,"child":{"type":"zone","zone":"list"}},
                            {"type":"zone","zone":"footer"},
                            {"type":"modal","child":{"type":"column","children":[
                                {"type":"label","text":"INTERRUPT-CAPTURE","role":"label"},
                                {"type":"input"}
                            ]}}
                        ]}},
                        {"type":"flex","weight":3,"child":{"type":"surface"}}
                    ]})
                } else {
                    json!({"type":"column","children":[
                        {"type":"label","text":"INTERRUPT-CAPTURE","role":"label"},
                        {"type":"surface"}
                    ]})
                };
                let declaration = json!({"herdr":"widget_tree","version":1,"zones":[],
                    "composition":{"priority":100,"tree":tree}});
                std::fs::write(
                    plugin.join("producer.ts"),
                    format!("console.log(JSON.stringify({declaration})); await Bun.stdin.text();"),
                )
                .unwrap();
                std::fs::write(plugin.join("trailboss-plugin.toml"),
                    "id = \"fixture.interrupt-capture\"\nname = \"Capture interruption\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tree\"\nsurface = \"sidebar\"\ncommand = [\"bun\", \"producer.ts\"]\nstream = true\n"
                ).unwrap();
                core.request("plugin.link", json!({"path":plugin,"enabled":true}));
                console.wait(|_, screen| {
                    screen.screen_text_rows().unwrap().iter().any(|row| {
                        row.cells
                            .iter()
                            .flat_map(|cell| cell.graphemes.iter())
                            .filter_map(|ch| char::from_u32(*ch))
                            .collect::<String>()
                            .contains("INTERRUPT-CAPTURE")
                    })
                });
                if case == "modal" {
                    let mut observer = core.attach();
                    let deadline = Instant::now() + Duration::from_secs(8);
                    while !observer.declarations.as_ref().is_some_and(|declarations| {
                        declarations
                            .iter()
                            .any(|(owner, _)| owner == "fixture.interrupt-capture")
                    }) {
                        assert!(
                            Instant::now() < deadline,
                            "modal declaration was not published"
                        );
                        observer.receive().unwrap();
                    }
                    assert!(
                        sidebar::rows(observer.declarations.as_ref().unwrap()).any(
                            |(owner, node)| {
                                owner == "tommymorgan.agent-tree"
                                    && matches!(node,
                            Node::Row { action, context, .. }
                            if action.as_deref() == Some("focus-tab")
                                && context.workspace_id.as_deref() == Some(source)
                                && context.tab_id.as_deref() == Some(build)
                                && context.pane_id.is_none())
                            }
                        ),
                        "modal replaced the original captured sidebar identity"
                    );
                }
            }
            "viewport" => {
                console.screen.resize(110, 28, 0, 0).unwrap();
                console
                    ._master
                    .resize(portable_pty::PtySize {
                        rows: 28,
                        cols: 110,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .unwrap();
                console.wait(|rows, _| {
                    rows.len() == 28
                        && rows.iter().any(|row| row.contains("TargetRight"))
                        && !rows[target_row]
                            .trim_start()
                            .starts_with("────────────────────")
                });
            }
            "escape" => {
                console.writer.write_all(b"\x1b").unwrap();
                console.writer.flush().unwrap();
                console.wait(|rows, _| {
                    !rows[target_row]
                        .trim_start()
                        .starts_with("────────────────────")
                });
            }
            "outside" | "pane-row" => {}
            _ => unreachable!(),
        }
        assert_eq!(
            console.child.0.process_id(),
            native_pid,
            "{case}: native client replaced"
        );
        assert!(
            console.child.0.try_wait().unwrap().is_none(),
            "{case}: native client exited"
        );
        let before_release = topology(&core);
        assert_eq!(
            before_release["workspace"], logs["workspace"]["workspace_id"],
            "{case}"
        );
        assert_eq!(before_release["tab"], logs["tab"]["tab_id"], "{case}");
        assert_eq!(before_release["pane"], diagnostics, "{case}");
        if case != "source-close" {
            assert_eq!(
                core.request(
                    "pane.get",
                    json!({
                        "pane_id":tools["root_pane"]["pane_id"]
                    })
                )["pane"]["workspace_id"],
                source,
                "{case}: interruption itself moved Build"
            );
        }
        let mut events = BufReader::new(UnixStream::connect(&socket).unwrap());
        events
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        serde_json::to_writer(
            events.get_mut(),
            &json!({
                "id":"interrupted-release","method":"events.subscribe","params":{"subscriptions":[
                    {"type":"tab.transferred"},{"type":"tab.focused"},
                    {"type":"pane.focused"},{"type":"workspace.focused"}
                ]}
            }),
        )
        .unwrap();
        events.get_mut().write_all(b"\n").unwrap();
        let mut line = String::new();
        events.read_line(&mut line).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&line)
            .unwrap()
            .get("error")
            .is_none());
        let (x, row) = match case {
            "outside" => (60, target_row),
            "pane-row" => (5, pane_row),
            _ => (5, target_row),
        };
        console
            .writer
            .write_all(format!("\x1b[<0;{x};{}m", row + 1).as_bytes())
            .unwrap();
        console.writer.flush().unwrap();
        line.clear();
        let error = events.read_line(&mut line).expect_err(&format!(
            "{case}: interrupted release emitted a transfer or click: {line}"
        ));
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "{case}: event stream closed instead of remaining quiet: {error}"
        );
        assert_eq!(
            topology(&core),
            before_release,
            "{case}: release changed topology or view"
        );
        assert_eq!(
            console.child.0.process_id(),
            native_pid,
            "{case}: native client replaced"
        );
        assert!(
            console.child.0.try_wait().unwrap().is_none(),
            "{case}: native client exited"
        );
    }
}

#[test]
#[ignore = "requires an explicitly supplied pre-transfer legacy Core executable"]
fn executable_pty_unsupported_sidebar_transfer_reports_notice_without_reordering() {
    use std::{
        io::{BufRead, BufReader},
        os::unix::net::UnixStream,
    };
    let legacy = std::path::PathBuf::from(
        std::env::var_os("TRAILBOSS_TEST_LEGACY_CORE")
            .expect("set TRAILBOSS_TEST_LEGACY_CORE to a pre-transfer Core executable"),
    );
    assert!(
        legacy.is_absolute(),
        "legacy executable must be an explicit absolute path"
    );
    let name = format!(
        "old-drag-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let home = std::env::temp_dir().join(&name);
    std::fs::create_dir_all(&home).unwrap();
    let core = Core::launch_with_executable(home, name, legacy);
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let source = tools["workspace"]["workspace_id"].as_str().unwrap();
    core.request(
        "tab.rename",
        json!({"tab_id":tools["tab"]["tab_id"],"label":"One"}),
    );
    let build = core.request(
        "tab.create",
        json!({"workspace_id":source,"label":"Build","cwd":core.home}),
    );
    let tab = build["tab"]["tab_id"].as_str().unwrap();
    core.request(
        "tab.create",
        json!({"workspace_id":source,"label":"Two","cwd":core.home}),
    );
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    let destination = main["workspace"]["workspace_id"].as_str().unwrap();
    core.request(
        "tab.rename",
        json!({"tab_id":main["tab"]["tab_id"],"label":"Target"}),
    );
    let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
    let diagnostics = logs["root_pane"]["pane_id"].as_str().unwrap();
    core.request(
        "tab.rename",
        json!({"tab_id":logs["tab"]["tab_id"],"label":"Diagnostics"}),
    );
    core.request("pane.focus", json!({"pane_id":diagnostics}));
    core.request(
        "plugin.link",
        json!({
            "path":format!("{}/plugins/agent-tree",env!("CARGO_MANIFEST_DIR")),"enabled":true
        }),
    );
    let mut console = LiveConsole::new_for_tab(&core, source, tab);
    let native_pid = console.child.0.process_id();
    let rows = console.wait_for("T16 actual agent-tree sidebar startup", |rows, _| {
        ["One", "Build", "Two", "Target", "Diagnostics"]
            .iter()
            .all(|label| rows.iter().any(|row| row.contains(label)))
    });
    let source_row = rows.iter().position(|row| row.contains("Build")).unwrap();
    let target_row = rows.iter().position(|row| row.contains("Target")).unwrap();
    let before_source = core.request("tab.list", json!({"workspace_id":source}));
    let before_destination = core.request("tab.list", json!({"workspace_id":destination}));
    let before_build =
        core.request("pane.get", json!({"pane_id":build["root_pane"]["pane_id"]}))["pane"].clone();
    let socket = core
        .home
        .join("trailboss/sessions")
        .join(&core.name)
        .join("trailboss.sock");
    let mut events = BufReader::new(UnixStream::connect(socket).unwrap());
    events
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(250)))
        .unwrap();
    serde_json::to_writer(
        events.get_mut(),
        &json!({
            "id":"unsupported-native-events","method":"events.subscribe","params":{"subscriptions":[
                {"type":"tab.moved"},{"type":"pane.moved"},{"type":"tab.focused"},
                {"type":"pane.focused"},{"type":"workspace.focused"}
            ]}
        }),
    )
    .unwrap();
    events.get_mut().write_all(b"\n").unwrap();
    let mut line = String::new();
    events.read_line(&mut line).unwrap();
    assert!(serde_json::from_str::<Value>(&line)
        .unwrap()
        .get("error")
        .is_none());
    console
        .writer
        .write_all(
            format!(
                "\x1b[<0;5;{}M\x1b[<32;5;{}M",
                source_row + 1,
                target_row + 1
            )
            .as_bytes(),
        )
        .unwrap();
    console.writer.flush().unwrap();
    console.wait_for(
        "T16 cross-space insertion indicator before release",
        |rows, _| {
            rows[target_row]
                .trim_start()
                .starts_with("────────────────────")
        },
    );
    assert_eq!(
        core.request("tab.list", json!({"workspace_id":source})),
        before_source
    );
    assert_eq!(
        core.request("tab.list", json!({"workspace_id":destination})),
        before_destination
    );
    console
        .writer
        .write_all(format!("\x1b[<0;5;{}m", target_row + 1).as_bytes())
        .unwrap();
    console.writer.flush().unwrap();
    console.wait_for(
        "T16 visible unsupported tab.transfer endpoint notice after release",
        |_, screen| {
            let text: String = screen
                .screen_text_rows()
                .unwrap()
                .iter()
                .flat_map(|row| row.cells.iter())
                .flat_map(|cell| cell.graphemes.iter())
                .filter_map(|ch| char::from_u32(*ch))
                .collect();
            text.contains("Action unavailable") && text.contains("tab.transfer")
        },
    );
    line.clear();
    let quiet = events
        .read_line(&mut line)
        .expect_err("unsupported drop emitted movement or focus");
    assert!(matches!(
        quiet.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    assert_eq!(
        core.request("tab.list", json!({"workspace_id":source})),
        before_source
    );
    assert_eq!(
        core.request("tab.list", json!({"workspace_id":destination})),
        before_destination
    );
    assert_eq!(
        core.request("pane.get", json!({"pane_id":build["root_pane"]["pane_id"]}))["pane"],
        before_build
    );
    let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert_eq!(
        snapshot["focused_workspace_id"],
        logs["workspace"]["workspace_id"]
    );
    assert_eq!(snapshot["focused_tab_id"], logs["tab"]["tab_id"]);
    assert_eq!(snapshot["focused_pane_id"], diagnostics);
    assert_eq!(console.child.0.process_id(), native_pid);
    assert!(console.child.0.try_wait().unwrap().is_none());
}

#[test]
fn executable_pty_preserves_existing_sidebar_and_splitter_gestures() {
    use std::{io::BufRead, io::BufReader, os::unix::net::UnixStream};

    fn mouse(console: &mut LiveConsole, button: u8, x: usize, y: usize, end: char) {
        console
            .writer
            .write_all(format!("\x1b[<{button};{};{}{end}", x + 1, y + 1).as_bytes())
            .unwrap();
        console.writer.flush().unwrap();
    }
    fn full_rows(screen: &ghostty_vt::Terminal) -> Vec<String> {
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
    fn marker_at(screen: &ghostty_vt::Terminal, column: usize) -> bool {
        let marker = b"NATIVE-SPLITTER";
        screen.screen_text_rows().unwrap().iter().any(|row| {
            row.cells
                .iter()
                .skip(column)
                .flat_map(|cell| cell.graphemes.iter().copied())
                .take(marker.len())
                .eq(marker.iter().copied().map(u32::from))
        })
    }
    fn tab_ids(core: &Core, workspace: &str) -> Vec<Value> {
        core.request("tab.list", json!({"workspace_id":workspace}))["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["tab_id"].clone())
            .collect()
    }

    for gesture in ["tab-reorder", "space-reorder", "click", "menu", "splitter"] {
        let core = Core::new();
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let build = tools["tab"]["tab_id"].as_str().unwrap();
        let first = tools["root_pane"]["pane_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
        let split = core.request(
            "pane.split",
            json!({
                "target_pane_id":first,"direction":"right","ratio":0.37,"cwd":core.home
            }),
        );
        let second = split["pane"]["pane_id"].as_str().unwrap();
        let remaining = core.request(
            "tab.create",
            json!({
                "workspace_id":source,"label":"Remaining","cwd":core.home
            }),
        );
        let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
        let destination = main["workspace"]["workspace_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":main["tab"]["tab_id"],"label":"Target"}),
        );
        if gesture != "click" {
            core.request("pane.focus", json!({"pane_id":second}));
        }
        core.request(
            "plugin.link",
            json!({
                "path":format!("{}/plugins/agent-tree",env!("CARGO_MANIFEST_DIR")),"enabled":true
            }),
        );
        // Complete both sidebar producers before any single-shot native gesture.
        {
            let mut observer = core.attach();
            let menu_name = format!("sidebar.tab-menu:{build}");
            let ui = trailboss::core::Core::default();
            let menu_ready = |trees: &sidebar::Declarations| {
                ui.fragment(trees, &menu_name).is_some_and(|(owner, tree)| {
                    owner == "tommymorgan.sidebar-menus"
                        && matches!(tree, Node::Column { children } if !children.is_empty())
                })
            };
            core.request(
                "plugin.link",
                json!({
                    "path":format!("{}/plugins/sidebar-menus",env!("CARGO_MANIFEST_DIR")),
                    "enabled":true
                }),
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if observer.declarations.as_ref().is_some_and(|trees| {
                    menu_ready(trees) && sidebar::rows(trees).any(|(owner, node)| {
                        owner == "tommymorgan.agent-tree"
                            && matches!(node, Node::Row { action: Some(action), menu: Some(menu), context, .. }
                                if action == "focus-tab"
                                    && context.workspace_id.as_deref() == Some(source)
                                    && context.tab_id.as_deref() == Some(build)
                                    && context.pane_id.is_none()
                                    && matches!(menu.as_ref(), Node::Use { name } if name == &menu_name))
                    })
                }) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "T17 Build named menu/context not published"
                );
                observer.receive().unwrap();
            }
            drop(observer);
        }
        let mut console = LiveConsole::new_for_tab(&core, source, build);
        let native_pid = console.child.0.process_id();
        let rows = console.wait_for("T17 real agent-tree startup", |rows, _| {
            ["Tools", "Build", "Remaining", "Main", "Target"]
                .iter()
                .all(|label| rows.iter().any(|row| row.contains(label)))
        });
        let row = |label: &str| rows.iter().position(|row| row.contains(label)).unwrap();
        let before_source = tab_ids(&core, source);
        let before_destination = tab_ids(&core, destination);
        let mut expected_workspace_order: Vec<Value> = core.request("session.snapshot", json!({}))
            ["snapshot"]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|workspace| workspace["workspace_id"].clone())
            .collect();
        let before_layout =
            core.request("pane.layout", json!({"pane_id":second}))["layout"].clone();
        let before_terminal =
            core.request("pane.get", json!({"pane_id":second}))["pane"]["terminal_id"].clone();
        let mut events = BufReader::new(
            UnixStream::connect(
                core.home
                    .join("trailboss/sessions")
                    .join(&core.name)
                    .join("trailboss.sock"),
            )
            .unwrap(),
        );
        events
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        serde_json::to_writer(
            events.get_mut(),
            &json!({
                "id":"existing-gesture-transfers","method":"events.subscribe",
                "params":{"subscriptions":[{"type":"tab.transferred"}]}
            }),
        )
        .unwrap();
        events.get_mut().write_all(b"\n").unwrap();
        let mut line = String::new();
        events.read_line(&mut line).unwrap();
        assert!(serde_json::from_str::<Value>(&line)
            .unwrap()
            .get("error")
            .is_none());

        match gesture {
            "tab-reorder" => {
                mouse(&mut console, 0, 4, row("Remaining"), 'M');
                mouse(&mut console, 32, 4, row("Build"), 'M');
                mouse(&mut console, 0, 4, row("Build"), 'm');
                console.wait_for("T17 same-space tab order", |rows, _| {
                    rows.iter().position(|row| row.contains("Remaining"))
                        < rows.iter().position(|row| row.contains("Build"))
                });
                assert_eq!(
                    tab_ids(&core, source),
                    vec![
                        remaining["tab"]["tab_id"].clone(),
                        tools["tab"]["tab_id"].clone(),
                    ]
                );
            }
            "space-reorder" => {
                mouse(&mut console, 0, 4, row("Main"), 'M');
                mouse(&mut console, 32, 4, row("Tools"), 'M');
                mouse(&mut console, 0, 4, row("Tools"), 'm');
                console.wait_for("T17 space header order", |rows, _| {
                    rows.iter().position(|row| row.contains("Main"))
                        < rows.iter().position(|row| row.contains("Tools"))
                });
                let previous = expected_workspace_order
                    .iter()
                    .position(|workspace| workspace == destination)
                    .unwrap();
                let moved = expected_workspace_order.remove(previous);
                let insertion = expected_workspace_order
                    .iter()
                    .position(|workspace| workspace == source)
                    .unwrap();
                expected_workspace_order.insert(insertion, moved);
            }
            "click" => {
                console.click(row("Build"));
                console.wait_for("T17 Build click selection", |rows, _| {
                    rows.iter()
                        .any(|row| row.starts_with('▏') && row.contains("Build"))
                });
                let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
                assert_eq!(snapshot["focused_workspace_id"], source);
                assert_eq!(snapshot["focused_tab_id"], build);
            }
            "menu" => {
                mouse(&mut console, 35, 4, row("Build"), 'M');
                let hovered = console.wait_for("T17 Build menu affordance", |rows, _| {
                    rows[row("Build")].contains('⋯')
                });
                let x = hovered[row("Build")]
                    .chars()
                    .position(|ch| ch == '⋯')
                    .unwrap();
                mouse(&mut console, 0, x, row("Build"), 'M');
                mouse(&mut console, 0, x, row("Build"), 'm');
                console.wait_for("T17 Build context menu", |_, screen| {
                    full_rows(screen)
                        .iter()
                        .any(|row| row.contains("Rename “Build”"))
                });
                console.writer.write_all(b"\x1b").unwrap();
                console.writer.flush().unwrap();
                console.wait_for("T17 menu dismissal", |_, screen| {
                    full_rows(screen)
                        .iter()
                        .all(|row| !row.contains("Rename “Build”"))
                });
            }
            "splitter" => {
                core.request(
                    "pane.send_text",
                    json!({"pane_id":second,
                    "text":"printf '\\033[2J\\033[HNATIVE-SPLITTER\\n'\n"}),
                );
                console.wait_for("T17 original terminal origin", |_, screen| {
                    marker_at(screen, 30)
                });
                mouse(&mut console, 0, 30, 15, 'M');
                mouse(&mut console, 32, 42, 15, 'M');
                mouse(&mut console, 0, 42, 15, 'm');
                console.wait_for("T17 resized terminal origin", |_, screen| {
                    marker_at(screen, 42)
                });
            }
            _ => unreachable!(),
        }
        let actual_workspace_order: Vec<Value> = core.request("session.snapshot", json!({}))
            ["snapshot"]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|workspace| workspace["workspace_id"].clone())
            .collect();
        assert_eq!(
            actual_workspace_order, expected_workspace_order,
            "{gesture}"
        );
        if gesture != "tab-reorder" {
            assert_eq!(tab_ids(&core, source), before_source, "{gesture}");
        }
        assert_eq!(tab_ids(&core, destination), before_destination, "{gesture}");
        let after_layout = core.request("pane.layout", json!({"pane_id":second}))["layout"].clone();
        let split_semantics = |layout: &Value| {
            layout["splits"].as_array().unwrap().iter()
            .map(|split| json!({"id":split["id"],"direction":split["direction"],"ratio":split["ratio"]}))
            .collect::<Vec<_>>()
        };
        assert_eq!(
            split_semantics(&after_layout),
            split_semantics(&before_layout),
            "{gesture}"
        );
        let pane = core.request("pane.get", json!({"pane_id":second}));
        assert_eq!(pane["pane"]["terminal_id"], before_terminal, "{gesture}");
        assert_eq!(pane["pane"]["workspace_id"], source, "{gesture}");
        assert_eq!(console.child.0.process_id(), native_pid, "{gesture}");
        assert!(console.child.0.try_wait().unwrap().is_none(), "{gesture}");
        line.clear();
        let quiet = events
            .read_line(&mut line)
            .expect_err("existing gesture issued a transfer");
        assert!(
            matches!(
                quiet.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "{gesture}: {quiet}"
        );
    }
}
