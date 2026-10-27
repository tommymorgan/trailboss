#[path = "support/console.rs"]
mod console;
#[path = "support/restart.rs"]
mod restart;
mod support;
use console::Console;
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use support::Core;
use trailboss::Client;

const SCENARIO: &str = "Restore persisted session topology after relaunch";
const GIVEN: &str =
    "Given a named session has persisted workspaces, tab layouts and pane working directories";
const WHEN: &str = "When the core stops and relaunches using that persistence";
const THEN: &str = "Then the restored session contains those workspaces, tabs and pane layouts with usable replacement PTYs in their restored working directories";

fn wait(mut predicate: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn snapshot(core: &Core) -> Value {
    core.request("session.snapshot", json!({}))["snapshot"].clone()
}
// Restarts remap runtime identifiers. Compare labels, membership, ordered leaf
// working directories and exact split geometry, not stale pane/process IDs.
fn topology(s: &Value) -> Value {
    json!(s["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            let tabs: Vec<_> = s["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["workspace_id"] == w["workspace_id"])
                .map(|t| {
                    let l = s["layouts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|l| l["tab_id"] == t["tab_id"])
                        .unwrap();
                    let leaves: Vec<_> = l["panes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| {
                            let pane = s["panes"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .find(|x| x["pane_id"] == p["pane_id"])
                                .unwrap();
                            json!([pane["cwd"], p["rect"]])
                        })
                        .collect();
                    let splits: Vec<_> = l["splits"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| json!([p["direction"], p["ratio"], p["rect"]]))
                        .collect();
                    json!([t["label"], leaves, splits])
                })
                .collect();
            json!([w["label"], tabs])
        })
        .collect::<Vec<_>>())
}
struct Session {
    core: Core,
    expected: Value,
    shells: Vec<u32>,
    persistence: PathBuf,
    stale_pid: u32,
}
fn shell_pid(core: &Core, pane: &str) -> u32 {
    core.request("pane.process_info", json!({"pane_id":pane}))["process_info"]["shell_pid"]
        .as_u64()
        .unwrap()
        .try_into()
        .unwrap()
}
fn given() -> Session {
    let core = Core::new();
    let a = core.home.join("alpha space");
    let b = core.home.join("βeta work");
    let c = core.home.join("nested ü cwd");
    for dir in [&a, &b, &c] {
        fs::create_dir_all(dir).unwrap();
    }
    let alpha = core.request(
        "workspace.create",
        json!({"label":"Restore Alpha","cwd":a,"focus":true}),
    );
    let pane = alpha["root_pane"]["pane_id"].as_str().unwrap();
    let split = core.request(
        "pane.split",
        json!({"target_pane_id":pane,"direction":"right","ratio":0.6,"cwd":b,"focus":false}),
    );
    let right = split["pane"]["pane_id"].as_str().unwrap();
    core.request(
        "pane.split",
        json!({"target_pane_id":right,"direction":"down","ratio":0.35,"cwd":c,"focus":false}),
    );
    core.request("tab.create", json!({"workspace_id":alpha["workspace"]["workspace_id"],"label":"Saved sibling","cwd":b,"focus":false}));
    core.request(
        "workspace.create",
        json!({"label":"Restore Beta","cwd":c,"focus":false}),
    );
    let s = snapshot(&core);
    assert_eq!(s["workspaces"].as_array().unwrap().len(), 3);
    assert_eq!(s["tabs"].as_array().unwrap().len(), 4);
    assert_eq!(s["panes"].as_array().unwrap().len(), 6);
    let shells = s["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| shell_pid(&core, p["pane_id"].as_str().unwrap()))
        .collect();
    // This process must die on Core shutdown, unlike Task 4.1's detach case.
    core.request("pane.send_text", json!({"pane_id":pane,"text":format!("sh -c 'echo $$ > \"{}/stale.pid\"; exec sleep 120'\n",core.home.display())}));
    wait(
        || core.home.join("stale.pid").exists(),
        "foreground command did not start",
    );
    let stale_pid = fs::read_to_string(core.home.join("stale.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let persistence = core
        .home
        .join(format!("trailboss/sessions/{}/session.json", core.name));
    Session {
        core,
        expected: topology(&s),
        shells,
        persistence,
        stale_pid,
    }
}
fn relaunch(session: &mut Session) {
    let first = session.core.child.id();
    let saved = session.core.restart();
    assert!(
        session.persistence.is_file(),
        "normal stop did not retain session artifact"
    );
    let artifact: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(artifact["workspaces"].as_array().unwrap().len(), 3);
    assert!(
        !PathBuf::from(format!("/proc/{}", session.stale_pid)).exists(),
        "stale foreground command survived Core restart"
    );
    println!(
        "Core {first}->{}; retained actual persistence {} ({} bytes); stale command {} ended",
        session.core.child.id(),
        session.persistence.display(),
        fs::metadata(&session.persistence).unwrap().len(),
        session.stale_pid
    );
}
fn restored(session: &Session) -> Value {
    let s = snapshot(&session.core);
    assert_eq!(
        topology(&s),
        session.expected,
        "restore changed saved topology/layout/cwds"
    );
    for p in s["panes"].as_array().unwrap() {
        let id = p["pane_id"].as_str().unwrap();
        let pid = shell_pid(&session.core, id);
        assert!(!session.shells.contains(&pid), "restored stale shell PID");
        assert_eq!(
            fs::read_link(format!("/proc/{pid}/cwd")).unwrap(),
            PathBuf::from(p["cwd"].as_str().unwrap())
        );
        println!("replacement {id}: shell {pid}, cwd {}", p["cwd"]);
    }
    s
}
fn command(core: &Core, pane: &str) -> String {
    // Real shell output is also captured without narrow-pane wrapping. The
    // short displayed token is split so input echo cannot satisfy observation.
    let report = core.home.join(format!("{pane}.cwd"));
    format!(
        "pwd > '{}'; printf 'restored%s\\n' '-{pane}'; pwd\n",
        report.display()
    )
}
fn output(pane: &Value) -> String {
    format!("restored-{}", pane["pane_id"].as_str().unwrap())
}
fn actual_cwd(core: &Core, pane: &Value) {
    let report = core
        .home
        .join(format!("{}.cwd", pane["pane_id"].as_str().unwrap()));
    wait(
        || report.exists(),
        "replacement PTY command did not execute",
    );
    assert_eq!(
        fs::read_to_string(report).unwrap().trim_end(),
        pane["cwd"].as_str().unwrap()
    );
}
fn stream_ready(client: &mut Client) {
    while client.primary.is_none() {
        client.receive().unwrap();
    }
}
#[test]
fn public_core_restart_restores_saved_layouts_cwds_and_replacement_ptys() {
    let mut session = given();
    let mut first = session.core.attach();
    stream_ready(&mut first);
    session.expected = topology(&snapshot(&session.core));
    let boot = first.boot_id.clone();
    drop(first);
    relaunch(&mut session);
    let mut client = session.core.attach();
    stream_ready(&mut client);
    assert_ne!(client.boot_id, boot);
    let s = restored(&session);
    for pane in s["panes"].as_array().unwrap() {
        let id = pane["pane_id"].as_str().unwrap();
        session.core.request("pane.focus", json!({"pane_id":id}));
        while client.focused_pane_id.as_deref() != Some(id) {
            client.receive().unwrap();
        }
        client.type_text(&command(&session.core, id)).unwrap();
        while !client
            .primary
            .as_ref()
            .is_some_and(|p| p.text().contains(&output(pane)))
        {
            client.receive().unwrap();
        }
        actual_cwd(&session.core, pane);
    }
}
#[test]
fn living_restore_persisted_session_topology_after_relaunch() {
    let steps: Vec<_> = include_str!("../features/session.feature")
        .split("Scenario: ")
        .find(|s| s.lines().next() == Some(SCENARIO))
        .expect("missing living scenario")
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(steps, [GIVEN, WHEN, THEN], "unbound restore living steps");
    let mut session = given();
    let mut first = Console::new(&session.core);
    first.wait(|rows| rows.iter().any(|row| row.contains("Saved sibling")));
    session.expected = topology(&snapshot(&session.core));
    drop(first);
    relaunch(&mut session);
    let mut client = Console::new(&session.core);
    client.wait(|rows| rows.iter().any(|row| row.contains("Saved sibling")));
    let s = restored(&session);
    for pane in s["panes"].as_array().unwrap() {
        let id = pane["pane_id"].as_str().unwrap();
        session.core.request("pane.focus", json!({"pane_id":id}));
        // Await the executable's new focused Surface before keyboard input.
        session.core.request(
            "pane.send_text",
            json!({"pane_id":id,"text":format!("printf 'focus%s\\n' '-{id}'\n")}),
        );
        client.wait(|rows| rows.iter().any(|r| r.contains(&format!("focus-{id}"))));
        client.click(40, 12);
        // Input enters the actual executable client, not a pane API shortcut.
        client.text(&command(&session.core, id));
        let expected = output(pane);
        client.wait(|rows| rows.iter().any(|r| r.contains(&expected)));
        actual_cwd(&session.core, pane);
    }
    println!(
        "actual restored client {} consumed every replacement pane's cwd output",
        client.child.process_id().unwrap()
    );
}
