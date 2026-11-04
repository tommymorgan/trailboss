#![cfg(unix)]

mod support;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use support::Core;

const SCROLLBACK: &str = "CLOSED_HISTORY_SCROLLBACK_survivor";
const VISIBLE: &str = "CLOSED_HISTORY_VISIBLE_survivor";

fn wait(mut predicate: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read(core: &Core, pane: &str, source: &str) -> String {
    core.request(
        "pane.read",
        json!({"pane_id":pane,"source":source,"lines":200}),
    )["read"]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn populate(core: &Core) -> String {
    fs::write(
        core.home.join("config.toml"),
        "[experimental]\npane_history = true\n",
    )
    .unwrap();
    core.request("server.reload_config", json!({}));
    let created = core.request(
        "workspace.create",
        json!({"label":"History survivor","cwd":core.home,"focus":true}),
    );
    let pane = created["root_pane"]["pane_id"].as_str().unwrap().to_owned();
    // Split each marker across printf arguments: shell input echo cannot satisfy
    // either assertion. Enough output pushes the first marker off the viewport.
    core.request("pane.send_text", json!({"pane_id":pane,"text":
        "printf 'CLOSED_HISTORY_SCROLLBACK_%s\\n' survivor; i=0; while [ \"$i\" -lt 80 ]; do printf 'history filler %s\\n' \"$i\"; i=$((i+1)); done; printf 'CLOSED_HISTORY_VISIBLE_%s\\n' survivor\n"
    }));
    wait(
        || read(core, &pane, "visible").contains(VISIBLE),
        "real survivor PTY did not produce visible marker",
    );
    assert!(read(core, &pane, "recent").contains(SCROLLBACK));
    assert!(!read(core, &pane, "visible").contains(SCROLLBACK));
    pane
}

fn session_path(core: &Core) -> PathBuf {
    core.home
        .join(format!("trailboss/sessions/{}/session.json", core.name))
}

fn stop(core: &mut Core) {
    core.request("server.stop", json!({}));
    wait(
        || core.child.try_wait().unwrap().is_some(),
        "normal Core shutdown did not finish",
    );
    assert!(core.child.wait().unwrap().success());
}

fn relaunch(mut core: Core) -> Core {
    let replacement = Core::launch(core.home.clone(), core.name.clone());
    assert_ne!(replacement.child.id(), core.child.id());
    // Transfer ownership without the stopped fixture removing persisted state.
    core.home = PathBuf::new();
    replacement
}

fn assert_survivor_history(core: &Core) {
    let _client = core.attach();
    let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
    let workspace = snapshot["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|workspace| workspace["label"] == "History survivor")
        .expect("survivor workspace was not restored");
    let tab = snapshot["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tab| tab["workspace_id"] == workspace["workspace_id"])
        .unwrap();
    let pane = snapshot["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["tab_id"] == tab["tab_id"])
        .unwrap()["pane_id"]
        .as_str()
        .unwrap();
    wait(
        || read(core, pane, "visible").contains(VISIBLE),
        "restart discarded the survivor pane's visible content",
    );
    assert!(
        read(core, pane, "recent").contains(SCROLLBACK),
        "restart discarded the survivor pane's off-screen scrollback"
    );
    assert!(!read(core, pane, "visible").contains(SCROLLBACK));
}

#[test]
fn survivor_pane_visible_content_and_scrollback_survive_restart_with_closed_tabs() {
    let mut core = Core::new();
    let client = core.attach();
    populate(&core);
    let disposable = core.request("tab.create", json!({
        "workspace_id":core.request("session.snapshot", json!({}))["snapshot"]["focused_workspace_id"],
        "label":"Synthetic closed sibling","cwd":core.home,"focus":false
    }));
    core.request("tab.close", json!({"tab_id":disposable["tab"]["tab_id"]}));
    let closed = core.request("tab.closed.list", json!({}))["closed_tabs"].clone();
    assert_eq!(closed.as_array().unwrap().len(), 1);
    drop(client);
    stop(&mut core);
    let saved: Value = serde_json::from_slice(&fs::read(session_path(&core)).unwrap()).unwrap();
    assert_eq!(saved["closed_tabs"].as_array().unwrap().len(), 1);
    let core = relaunch(core);
    assert_eq!(
        core.request("tab.closed.list", json!({}))["closed_tabs"],
        closed
    );
    assert_survivor_history(&core);
}

// Exact top-level SessionSnapshot representation at pre-feature revision
// 7c5fe57ff43fc7296aeddfd6a4ed11c421dd6932 (persist/snapshot.rs:15-29).
// Nested workspace/tab/pane fields were unchanged by Recently closed tabs;
// preserve their actual captured JSON rather than manufacturing history bytes.
// Deliberately do not use today's SessionSnapshot serializer or fingerprint:
// doing so on both sides would hide the upgrade regression.
#[derive(Deserialize, Serialize)]
struct PreClosedTabsSnapshot {
    version: u32,
    workspaces: Vec<Value>,
    active: Option<usize>,
    selected: usize,
    sidebar_width: Option<u16>,
    sidebar_section_split: Option<f32>,
    collapsed_space_keys: Vec<String>,
}

fn pre_feature_pair(core: &Core) {
    let path = session_path(core);
    let mut legacy: PreClosedTabsSnapshot =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // The base fingerprint sorts this set before canonical JSON hashing
    // (persist/snapshot.rs:404-413 at the pinned revision).
    legacy.collapsed_space_keys.sort_unstable();
    let value = serde_json::to_value(&legacy).unwrap();
    let canonical = serde_json::to_vec(&value).unwrap();
    let mut hash = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    hash.stdin.take().unwrap().write_all(&canonical).unwrap();
    let digest = hash.wait_with_output().unwrap();
    assert!(digest.status.success());
    let fingerprint = String::from_utf8(digest.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let history_path = path.with_file_name("session-history.json");
    let mut history: Value = serde_json::from_slice(&fs::read(&history_path).unwrap()).unwrap();
    history["layout_fingerprint"] = json!(fingerprint);
    fs::write(path, canonical).unwrap();
    fs::write(history_path, serde_json::to_vec(&history).unwrap()).unwrap();
}

#[test]
fn pre_closed_tabs_snapshot_preserves_survivor_visible_content_and_scrollback_on_upgrade() {
    let mut core = Core::new();
    let client = core.attach();
    populate(&core);
    assert_eq!(
        core.request("tab.closed.list", json!({}))["closed_tabs"],
        json!([])
    );
    drop(client);
    stop(&mut core);
    pre_feature_pair(&core);
    let core = relaunch(core);
    assert_survivor_history(&core);
}
