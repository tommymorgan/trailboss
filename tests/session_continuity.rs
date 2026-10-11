#[path = "support/console.rs"]
mod console;
mod support;
use console::Console;
use herdr_fork::Client;
use serde_json::{json, Value};
use std::{
    fs,
    time::{Duration, Instant},
};
use support::Core;

const SCENARIO: &str = "Disconnect without losing the running session";
const GIVEN: &str =
    "Given a named session has workspaces, tabs and panes including a running shell command";
const WHEN: &str = "When I disconnect the client and attach a new client to that session";
const THEN: &str =
    "Then the workspaces, tabs and live panes remain and the command continues in the same PTY";

struct Session {
    core: Core,
    topology: Value,
    pane: String,
    tab: String,
    other_tab: String,
}
fn topology(core: &Core) -> Value {
    let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
    json!({
        "workspaces": snapshot["workspaces"].as_array().unwrap().iter().map(|w| json!([w["workspace_id"], w["label"]])).collect::<Vec<_>>(),
        "tabs": snapshot["tabs"].as_array().unwrap().iter().map(|t| json!([t["tab_id"], t["workspace_id"], t["label"]])).collect::<Vec<_>>(),
        "panes": snapshot["panes"].as_array().unwrap().iter().map(|p| json!([p["pane_id"], p["tab_id"]])).collect::<Vec<_>>()
    })
}
fn given() -> Session {
    let core = Core::new();
    let alpha = core.request(
        "workspace.create",
        json!({"label":"Continuity Alpha","cwd":core.home,"focus":true}),
    );
    let tab = alpha["tab"]["tab_id"].as_str().unwrap().to_owned();
    let pane = alpha["root_pane"]["pane_id"].as_str().unwrap().to_owned();
    let other = core.request("tab.create", json!({"workspace_id":alpha["workspace"]["workspace_id"],"label":"Unfocused sibling","cwd":core.home,"focus":false}));
    core.request(
        "workspace.create",
        json!({"label":"Continuity Beta","cwd":core.home,"focus":false}),
    );
    // A foreground command owns the original slave PTY and advances independently of clients.
    fs::write(core.home.join("continuity.sh"), "echo $$ > command.pid\ni=0\nwhile :; do i=$((i+1)); printf '%s\\n' \"$i\" > ticks.tmp; mv ticks.tmp ticks; printf 'continuity-tick-%s\\n' \"$i\"; sleep 0.1; done\n").unwrap();
    let topology = topology(&core);
    assert_eq!(topology["workspaces"].as_array().unwrap().len(), 3);
    assert_eq!(topology["tabs"].as_array().unwrap().len(), 4);
    assert_eq!(topology["panes"].as_array().unwrap().len(), 4);
    Session {
        core,
        topology,
        pane,
        tab,
        other_tab: other["tab"]["tab_id"].as_str().unwrap().to_owned(),
    }
}
fn wait_until(mut predicate: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn tick(session: &Session) -> u64 {
    fs::read_to_string(session.core.home.join("ticks"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
fn identity(session: &Session) -> (u32, String, String, std::path::PathBuf) {
    let pid: u32 = fs::read_to_string(session.core.home.join("command.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let stat =
        fs::read_to_string(format!("/proc/{pid}/stat")).expect("original command process died");
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    let tty = fs::read_link(format!("/proc/{pid}/fd/0")).unwrap();
    assert!(
        tty.to_string_lossy().starts_with("/dev/pts/"),
        "command has no slave PTY"
    );
    assert_ne!(fields[4], "0", "command has no controlling terminal");
    (pid, fields[19].to_owned(), fields[4].to_owned(), tty)
}
fn start(session: &Session, mut type_text: impl FnMut(&str)) {
    type_text("exec /bin/sh ./continuity.sh\r");
    wait_until(|| tick(session) >= 2, "running command did not start");
}
fn detached(
    session: &mut Session,
    original: &(u32, String, String, std::path::PathBuf),
    before: u64,
) -> u64 {
    wait_until(
        || tick(session) >= before + 3,
        "command made no progress while disconnected",
    );
    assert_eq!(
        &identity(session),
        original,
        "detach replaced the command or its PTY"
    );
    assert!(
        session.core.child.try_wait().unwrap().is_none(),
        "Core exited on detach"
    );
    assert_eq!(
        topology(&session.core),
        session.topology,
        "detach destroyed topology"
    );
    tick(session)
}
fn receive_tick(client: &mut Client, session: &Session, after: u64) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if client.primary.as_ref().is_some_and(|s| {
            s.pane_id == session.pane
                && s.text().lines().any(|l| {
                    l.trim()
                        .strip_prefix("continuity-tick-")
                        .and_then(|n| n.parse::<u64>().ok())
                        .is_some_and(|n| n > after)
                })
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "new client saw no live post-detach command output"
        );
        client.receive().unwrap();
    }
    assert_eq!(client.focused_tab_id.as_deref(), Some(session.tab.as_str()));
}
#[test]
fn public_attach_stream_preserves_topology_process_and_pty_across_disconnect() {
    let mut session = given();
    let mut first = session.core.attach();
    while first.primary.is_none() {
        first.receive().unwrap();
    }
    let boot = first.boot_id.clone();
    start(&session, |s| first.type_text(s).unwrap());
    let original = identity(&session);
    let before = tick(&session);
    drop(first);
    let detached_tick = detached(&mut session, &original, before);
    // Reattach when a different tab is focused: stale saved client focus must not win.
    session
        .core
        .request("tab.focus", json!({"tab_id":session.other_tab}));
    let mut second = session.core.attach();
    while second.focused_tab_id.as_deref() != Some(session.other_tab.as_str()) {
        second.receive().unwrap();
    }
    assert_ne!(
        second.focused_pane_id.as_deref(),
        Some(session.pane.as_str())
    );
    session
        .core
        .request("tab.focus", json!({"tab_id":session.tab}));
    receive_tick(&mut second, &session, detached_tick);
    assert_eq!(second.boot_id, boot, "Core restarted during reconnect");
    assert_eq!(identity(&session), original);
    assert_eq!(topology(&session.core), session.topology);
    println!(
        "same command/PTY {original:?}; disconnected ticks {before}->{detached_tick}; Core PID {}",
        session.core.child.id()
    );
}
#[test]
fn living_disconnect_without_losing_the_running_session() {
    let steps: Vec<_> = include_str!("../features/session.feature")
        .split("Scenario: ")
        .find(|s| s.lines().next() == Some(SCENARIO))
        .expect("missing living scenario")
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        steps,
        [GIVEN, WHEN, THEN],
        "unbound continuity living steps"
    );
    let mut session = given();
    let core_pid = session.core.child.id();
    let mut first = Console::new(&session.core);
    first.wait(|rows| rows.iter().any(|r| r.contains("$")));
    let first_pid = first.child.process_id().unwrap();
    first.click(40, 12);
    start(&session, |s| first.text(s));
    first.wait(|rows| {
        rows.iter()
            .any(|r| r.trim().starts_with("continuity-tick-"))
    });
    let original = identity(&session);
    let before = tick(&session);
    first.text("\x11");
    wait_until(
        || first.child.try_wait().unwrap().is_some(),
        "normal client detach hung",
    );
    drop(first);
    let detached_tick = detached(&mut session, &original, before);
    let mut second = Console::new(&session.core);
    assert_ne!(
        second.child.process_id().unwrap(),
        first_pid,
        "reattach reused the first client"
    );
    second.wait(|rows| {
        rows.iter().any(|r| {
            r.trim()
                .strip_prefix("continuity-tick-")
                .and_then(|n| n.parse::<u64>().ok())
                .is_some_and(|n| n > detached_tick)
        })
    });
    assert_eq!(identity(&session), original);
    assert_eq!(session.core.child.id(), core_pid);
    assert_eq!(topology(&session.core), session.topology);
    // The other retained panes must still accept input, not merely retain metadata.
    let snapshot = session.core.request("session.snapshot", json!({}));
    for pane in snapshot["snapshot"]["panes"].as_array().unwrap() {
        if pane["pane_id"] == session.pane {
            continue;
        }
        let id = pane["pane_id"].as_str().unwrap();
        session.core.request(
            "pane.send_text",
            json!({"pane_id":id,"text":format!("printf 'usable-%s\\n' '{id}'\n")}),
        );
        session.core.request("pane.focus", json!({"pane_id":id}));
        second.wait(|rows| rows.iter().any(|r| r.contains(&format!("usable-{id}"))));
    }
    assert_eq!(identity(&session), original);
    println!("two actual clients {first_pid}->{}; Core PID {core_pid}; same command/PTY {original:?}; disconnected ticks {before}->{detached_tick}", second.child.process_id().unwrap());
}
