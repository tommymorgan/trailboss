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
