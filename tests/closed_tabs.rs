#![cfg(unix)]
mod support;

use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};
use support::Core;

fn request(core: &Core, method: &str, params: Value) -> Value {
    let socket = core
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
    let mut connection = UnixStream::connect(socket).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    serde_json::to_writer(
        &mut connection,
        &json!({"id":"closed-tab-regression","method":method,"params":params}),
    )
    .unwrap();
    connection.write_all(b"\n").unwrap();
    let mut response = String::new();
    BufReader::new(connection).read_line(&mut response).unwrap();
    serde_json::from_str(&response).unwrap()
}

fn result(core: &Core, method: &str, params: Value) -> Value {
    let response = request(core, method, params);
    assert!(response.get("error").is_none(), "{method}: {response}");
    response["result"].clone()
}

fn wait_until(mut predicate: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn layout_context(core: &Core, node: &Value) -> Value {
    if node["type"] == "pane" {
        let pane = core.request("pane.get", json!({"pane_id":node["pane_id"]}));
        json!({"cwd":pane["pane"]["cwd"],"label":pane["pane"]["label"]})
    } else {
        json!({
            "direction":node["direction"],"ratio":node["ratio"],
            "first":layout_context(core,&node["first"]),
            "second":layout_context(core,&node["second"]),
        })
    }
}

#[test]
fn closed_tab_recreates_split_labels_directories_and_usable_replacement_shells() {
    let core = Core::new();
    let _client = core.attach();
    let left = core.home.join("left work");
    let right = core.home.join("right work");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    let created = core.request(
        "workspace.create",
        json!({"label":"Recovery space","cwd":left,"focus":true}),
    );
    let workspace_id = created["workspace"]["workspace_id"].clone();
    let pane = created["root_pane"]["pane_id"].clone();
    let tab_id = created["root_pane"]["tab_id"].clone();
    core.request("tab.rename", json!({"tab_id":tab_id,"label":"Editor"}));
    core.request("pane.rename", json!({"pane_id":pane,"label":"Source"}));
    let split = core.request(
        "pane.split",
        json!({"target_pane_id":pane,"direction":"right","ratio":0.6,"cwd":right,"focus":false}),
    );
    let second = split["pane"]["pane_id"].clone();
    core.request("pane.rename", json!({"pane_id":second,"label":"Checks"}));
    core.request(
        "tab.create",
        json!({"workspace_id":workspace_id,"label":"Survivor","cwd":core.home,"focus":false}),
    );
    let original = core.request("layout.export", json!({"tab_id":tab_id}));
    let expected = layout_context(&core, &original["layout"]["root"]);
    let old_pids: Vec<_> = [&pane, &second]
        .into_iter()
        .map(|id| {
            core.request("pane.process_info", json!({"pane_id":id}))["process_info"]["shell_pid"]
                .as_u64()
                .unwrap()
        })
        .collect();
    core.request("tab.close", json!({"tab_id":tab_id}));
    wait_until(
        || {
            old_pids
                .iter()
                .all(|pid| !Path::new(&format!("/proc/{pid}/exe")).exists())
        },
        "closing the tab did not end its original shells",
    );
    let history = result(&core, "tab.closed.list", json!({}));
    let entries = history["closed_tabs"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["label"], "Editor");
    assert_eq!(entries[0]["workspace_id"], workspace_id);
    assert_eq!(entries[0]["pane_count"], 2);
    let restored = result(
        &core,
        "tab.closed.restore",
        json!({"closed_tab_id":entries[0]["closed_tab_id"]}),
    );
    assert_eq!(restored["workspace_id"], workspace_id);
    assert_ne!(restored["tab_id"], tab_id);
    let layout = core.request("layout.export", json!({"tab_id":restored["tab_id"]}));
    assert_eq!(layout_context(&core, &layout["layout"]["root"]), expected);
    let snapshot = core.request("session.snapshot", json!({}));
    let panes: Vec<_> = snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pane| pane["tab_id"] == restored["tab_id"])
        .collect();
    assert_eq!(panes.len(), 2);
    for (index, pane) in panes.iter().enumerate() {
        let pid = core.request("pane.process_info", json!({"pane_id":pane["pane_id"]}))
            ["process_info"]["shell_pid"]
            .as_u64()
            .unwrap();
        assert!(!old_pids.contains(&pid));
        let output = core.home.join(format!("restored-cwd-{index}"));
        core.request(
            "pane.send_text",
            json!({"pane_id":pane["pane_id"],"text":format!("pwd > '{}'\n",output.display())}),
        );
        wait_until(
            || output.exists(),
            "replacement shell did not execute terminal input",
        );
        assert_eq!(
            fs::read_to_string(output).unwrap().trim_end(),
            pane["cwd"].as_str().unwrap()
        );
    }
    assert_eq!(
        result(&core, "tab.closed.list", json!({}))["closed_tabs"],
        json!([])
    );
}

fn restart(core: Core) -> Core {
    restart_with_snapshot_edit(core, |_| {})
}

fn restart_with_snapshot_edit(mut core: Core, edit: impl FnOnce(&mut Value)) -> Core {
    core.request("server.stop", json!({}));
    assert!(core.child.wait().unwrap().success());
    let home = core.home.clone();
    let name = core.name.clone();
    let snapshot_path = home.join(format!("trailboss/sessions/{name}/session.json"));
    let mut snapshot = serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
    edit(&mut snapshot);
    fs::write(snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    // Keep the fixture's cleanup from removing the state being restarted.
    let saved_home = home.with_extension("restart");
    fs::rename(&home, &saved_home).unwrap();
    drop(core);
    fs::rename(saved_home, &home).unwrap();
    Core::launch(home, name)
}

#[test]
fn history_survives_restart_with_no_open_spaces_and_recreates_the_original_space() {
    let core = Core::new();
    let _client = core.attach();
    let created = core.request(
        "workspace.create",
        json!({"label":"Closed project","cwd":core.home,"focus":true}),
    );
    let tab_id = created["root_pane"]["tab_id"].clone();
    core.request(
        "tab.rename",
        json!({"tab_id":tab_id,"label":"Saved editor"}),
    );
    drop(_client);
    let tabs = core.request("tab.list", json!({}));
    for tab in tabs["tabs"].as_array().unwrap() {
        core.request("tab.close", json!({"tab_id":tab["tab_id"]}));
    }
    assert_eq!(
        core.request("workspace.list", json!({}))["workspaces"],
        json!([])
    );
    let before = result(&core, "tab.closed.list", json!({}));
    let saved = before["closed_tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["label"] == "Saved editor")
        .unwrap();
    let saved_id = saved["closed_tab_id"].clone();
    let original_workspace = saved["workspace_id"].clone();
    let core = restart(core);
    let _client = core.attach();
    assert_eq!(result(&core, "tab.closed.list", json!({})), before);
    let open_tabs = core.request("tab.list", json!({}));
    assert!(open_tabs["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tab| tab["label"] != "Saved editor"));
    let restored = result(
        &core,
        "tab.closed.restore",
        json!({"closed_tab_id":saved_id}),
    );
    assert_ne!(restored["workspace_id"], original_workspace);
    let workspace = core.request(
        "workspace.get",
        json!({"workspace_id":restored["workspace_id"]}),
    );
    assert_eq!(workspace["workspace"]["label"], "Closed project");
    let tab = core.request("tab.get", json!({"tab_id":restored["tab_id"]}));
    assert_eq!(tab["tab"]["label"], "Saved editor");
    let panes = core.request(
        "pane.list",
        json!({"workspace_id":restored["workspace_id"]}),
    );
    assert_eq!(panes["panes"][0]["cwd"], core.home.to_str().unwrap());
    let output = core.home.join("recreated-space-shell");
    core.request(
        "pane.send_text",
        json!({
            "pane_id":panes["panes"][0]["pane_id"],
            "text":format!("printf 'restored' > '{}'\n",output.display()),
        }),
    );
    wait_until(
        || output.exists(),
        "recreated space did not contain a usable shell",
    );
    assert_eq!(fs::read_to_string(output).unwrap(), "restored");
}

#[test]
fn history_keeps_the_latest_fifty_closures_in_order_and_rejects_evicted_entries() {
    let core = Core::new();
    let other = Core::new();
    let other_tab = other.request("tab.list", json!({}))["tabs"][0]["tab_id"].clone();
    other.request(
        "tab.rename",
        json!({"tab_id":other_tab,"label":"Other session"}),
    );
    other.request("tab.close", json!({"tab_id":other_tab}));
    let other_history = result(&other, "tab.closed.list", json!({}));
    assert_eq!(other_history["closed_tabs"][0]["label"], "Other session");
    let workspace_id =
        core.request("workspace.list", json!({}))["workspaces"][0]["workspace_id"].clone();
    let mut evicted_id = Value::Null;
    for index in 0..51 {
        let created = core.request(
            "tab.create",
            json!({
                "workspace_id":workspace_id,"cwd":core.home,"label":format!("Closed {index:02}"),
            }),
        );
        core.request("tab.close", json!({"tab_id":created["tab"]["tab_id"]}));
        if index == 0 {
            evicted_id = result(&core, "tab.closed.list", json!({}))["closed_tabs"][0]
                ["closed_tab_id"]
                .clone();
        }
    }
    let core = restart(core);
    let history = result(&core, "tab.closed.list", json!({}));
    let entries = history["closed_tabs"].as_array().unwrap();
    assert_eq!(entries.len(), 50);
    let names: Vec<_> = entries
        .iter()
        .map(|entry| entry["label"].as_str().unwrap())
        .collect();
    let expected: Vec<_> = (1..=50)
        .rev()
        .map(|index| format!("Closed {index:02}"))
        .collect();
    assert_eq!(names, expected);
    assert_eq!(result(&other, "tab.closed.list", json!({})), other_history);
    let foreign = request(
        &core,
        "tab.closed.restore",
        json!({
            "closed_tab_id":other_history["closed_tabs"][0]["closed_tab_id"],
        }),
    );
    assert_eq!(foreign["error"]["code"], "closed_tab_restore_failed");
    assert_eq!(result(&other, "tab.closed.list", json!({})), other_history);
    assert!(entries
        .windows(2)
        .all(|pair| pair[0]["closed_at"].as_u64() >= pair[1]["closed_at"].as_u64()));
    let tabs_before = core.request("tab.list", json!({}));
    let error = request(
        &core,
        "tab.closed.restore",
        json!({"closed_tab_id":evicted_id}),
    );
    assert_eq!(error["error"]["code"], "closed_tab_restore_failed");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no longer in history"));
    assert_eq!(core.request("tab.list", json!({})), tabs_before);
    assert_eq!(result(&core, "tab.closed.list", json!({})), history);
}

#[test]
fn actual_tab_removals_are_recorded_once_without_recording_moves_or_failed_layouts() {
    for mode in [
        "workspace",
        "pane",
        "pane-last-space",
        "exit",
        "exit-last-space",
        "layout",
    ] {
        let core = Core::new();
        let created = core.request(
            "workspace.create",
            json!({"label":"Close paths","cwd":core.home}),
        );
        let workspace_id = created["workspace"]["workspace_id"].clone();
        let tab_id = created["root_pane"]["tab_id"].clone();
        let pane_id = created["root_pane"]["pane_id"].clone();
        let name = format!("Archived {mode}");
        core.request("tab.rename", json!({"tab_id":tab_id,"label":name}));
        if !mode.ends_with("last-space") {
            core.request(
                "tab.create",
                json!({"workspace_id":workspace_id,"cwd":core.home,"label":"Second tab"}),
            );
        }
        let mut expected = vec![name.clone()];
        match mode {
            "workspace" => {
                core.request("workspace.close", json!({"workspace_id":workspace_id}));
                expected.push("Second tab".into());
            }
            "pane" | "pane-last-space" => {
                core.request("pane.close", json!({"pane_id":pane_id}));
            }
            "exit" | "exit-last-space" => {
                core.request("pane.send_text", json!({"pane_id":pane_id,"text":"exit\n"}));
                wait_until(
                    || {
                        core.request("tab.list", json!({}))["tabs"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|tab| tab["tab_id"] != tab_id)
                    },
                    "exiting the final shell did not remove its tab",
                );
            }
            "layout" => {
                core.request("layout.apply", json!({
                    "tab_id":tab_id,"tab_label":"Replacement","root":{"type":"pane","cwd":core.home},
                }));
            }
            _ => unreachable!(),
        }
        let history = result(&core, "tab.closed.list", json!({}));
        let entries = history["closed_tabs"].as_array().unwrap();
        let mut actual: Vec<_> = entries
            .iter()
            .map(|entry| entry["label"].as_str().unwrap().to_string())
            .collect();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected, "close path {mode}");
        let mut destination = None;
        for entry in entries {
            let restored = result(
                &core,
                "tab.closed.restore",
                json!({"closed_tab_id":entry["closed_tab_id"]}),
            );
            let tab = core.request("tab.get", json!({"tab_id":restored["tab_id"]}));
            assert_eq!(tab["tab"]["label"], entry["label"], "restore path {mode}");
            if let Some(workspace) = &destination {
                assert_eq!(&restored["workspace_id"], workspace);
            }
            destination = Some(restored["workspace_id"].clone());
        }
        assert_eq!(
            result(&core, "tab.closed.list", json!({}))["closed_tabs"],
            json!([])
        );
    }

    let core = Core::new();
    let source = core.request(
        "workspace.create",
        json!({"label":"Move source","cwd":core.home}),
    );
    let target = core.request(
        "workspace.create",
        json!({"label":"Move target","cwd":core.home}),
    );
    let transferred = core.request("tab.transfer", json!({
        "tab_id":source["root_pane"]["tab_id"],"workspace_id":target["workspace"]["workspace_id"],
    }));
    assert_eq!(transferred["transfer"]["changed"], true);
    let tabs_before = core.request("tab.list", json!({}));
    let failed = request(
        &core,
        "layout.apply",
        json!({
            "workspace_id":target["workspace"]["workspace_id"],
            "root":{"type":"split","direction":"right","ratio":0.5,
                "first":{"type":"pane","cwd":core.home},
                "second":{"type":"pane","cwd":core.home,"command":["/nonexistent/trailboss-closed-tab-fixture"]}},
        }),
    );
    assert_eq!(failed["error"]["code"], "layout_apply_failed");
    assert_eq!(core.request("tab.list", json!({})), tabs_before);
    assert_eq!(
        result(&core, "tab.closed.list", json!({}))["closed_tabs"],
        json!([])
    );
    core.request(
        "tab.close",
        json!({"tab_id":transferred["transfer"]["tab"]["tab_id"]}),
    );
    let before_handoff = result(&core, "tab.closed.list", json!({}));
    core.request(
        "server.live_handoff",
        json!({"import_exe":support::executable()}),
    );
    wait_until(
        || {
            core.request("ping", json!({}))["protocol"]
                .as_u64()
                .is_some()
        },
        "handoff replacement did not answer",
    );
    assert_eq!(result(&core, "tab.closed.list", json!({})), before_handoff);
}

#[test]
fn an_unavailable_directory_retains_history_and_live_tabs_until_retry_succeeds() {
    let core = Core::new();
    let directory = core.home.join("saved-directory");
    fs::create_dir(&directory).unwrap();
    let workspace = core.request(
        "workspace.create",
        json!({"label":"Directory recovery","cwd":core.home}),
    );
    let workspace_id = workspace["workspace"]["workspace_id"].clone();
    let created = core.request(
        "tab.create",
        json!({
            "workspace_id":workspace_id,"cwd":directory,"label":"Needs directory",
        }),
    );
    let tab_id = created["tab"]["tab_id"].clone();
    core.request(
        "pane.split",
        json!({
            "target_pane_id":created["root_pane"]["pane_id"],"direction":"right","cwd":core.home,
        }),
    );
    core.request("tab.close", json!({"tab_id":tab_id}));
    fs::remove_dir(&directory).unwrap();
    let history = result(&core, "tab.closed.list", json!({}));
    let saved_id = history["closed_tabs"][0]["closed_tab_id"].clone();
    let tabs_before = core.request("tab.list", json!({}));
    let live_pane = workspace["root_pane"]["pane_id"].clone();
    let live_pid = core.request("pane.process_info", json!({"pane_id":live_pane}))["process_info"]
        ["shell_pid"]
        .clone();
    let failure = request(
        &core,
        "tab.closed.restore",
        json!({"closed_tab_id":saved_id}),
    );
    assert_eq!(failure["error"]["code"], "closed_tab_restore_failed");
    assert!(failure["error"]["message"]
        .as_str()
        .unwrap()
        .contains(directory.to_str().unwrap()));
    assert_eq!(result(&core, "tab.closed.list", json!({})), history);
    assert_eq!(core.request("tab.list", json!({})), tabs_before);
    assert_eq!(
        core.request("pane.process_info", json!({"pane_id":live_pane}))["process_info"]
            ["shell_pid"],
        live_pid
    );
    fs::create_dir(&directory).unwrap();
    let restored = result(
        &core,
        "tab.closed.restore",
        json!({"closed_tab_id":saved_id}),
    );
    assert_eq!(restored["workspace_id"], workspace_id);
    let panes = core.request("pane.list", json!({"workspace_id":workspace_id}));
    let restored_panes: Vec<_> = panes["panes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pane| pane["tab_id"] == restored["tab_id"])
        .collect();
    assert_eq!(restored_panes.len(), 2);
    assert!(restored_panes
        .iter()
        .any(|pane| pane["cwd"] == directory.to_str().unwrap()));
    assert_eq!(
        result(&core, "tab.closed.list", json!({}))["closed_tabs"],
        json!([])
    );
}

#[test]
#[ignore = "requires installed OMP; run explicitly for isolated native-conversation verification"]
fn an_unavailable_agent_transcript_retains_history_and_live_tabs() {
    let core = Core::new();
    let transcript = core.home.join("owned-synthetic-session.jsonl");
    fs::write(&transcript, concat!(
        "{\"type\":\"session\",\"version\":3,\"id\":\"019a4444-0000-7000-8000-000000000001\",",
        "\"timestamp\":\"2026-11-03T00:00:00.000Z\",\"cwd\":\"/tmp\",\"title\":\"Owned recovery fixture\"}\n",
    )).unwrap();
    let workspace = core.request("workspace.create", json!({"cwd":core.home}));
    let tab = core.request("tab.create", json!({
        "workspace_id":workspace["workspace"]["workspace_id"],"cwd":core.home,"label":"Saved conversation",
    }));
    core.request("pane.send_text", json!({
        "pane_id":tab["root_pane"]["pane_id"],
        "text":format!("omp --no-extensions --no-skills --no-context-files --resume='{}'\n", transcript.display()),
    }));
    wait_until(
        || {
            request(
                &core,
                "agent.get",
                json!({"target":tab["root_pane"]["pane_id"]}),
            )
            .get("result")
            .is_some()
        },
        "owned OMP fixture did not start",
    );
    core.request("pane.report_agent", json!({
        "pane_id":tab["root_pane"]["pane_id"],"source":"herdr:omp","agent":"omp","state":"idle","seq":0,
    }));
    core.request(
        "pane.report_agent_session",
        json!({
            "pane_id":tab["root_pane"]["pane_id"],"source":"herdr:omp","agent":"omp","seq":1,
            "session_start_source":"resume","agent_session_path":transcript,
        }),
    );
    assert_eq!(
        core.request("agent.get", json!({"target":tab["root_pane"]["pane_id"]}))["agent"]
            ["agent_session"]["value"],
        transcript.to_str().unwrap(),
    );
    core.request("tab.close", json!({"tab_id":tab["tab"]["tab_id"]}));
    let original_transcript = fs::read(&transcript).unwrap();
    fs::remove_file(&transcript).unwrap();
    let history = result(&core, "tab.closed.list", json!({}));
    let before = core.request("tab.list", json!({}));
    let failure = request(
        &core,
        "tab.closed.restore",
        json!({
            "closed_tab_id":history["closed_tabs"][0]["closed_tab_id"],
        }),
    );
    assert_eq!(
        failure["error"]["code"], "closed_tab_restore_failed",
        "{failure}"
    );
    assert!(failure["error"]["message"]
        .as_str()
        .unwrap()
        .contains(transcript.to_str().unwrap()));
    assert_eq!(result(&core, "tab.closed.list", json!({})), history);
    assert_eq!(core.request("tab.list", json!({})), before);
    fs::write(&transcript, original_transcript).unwrap();
    let restored = result(
        &core,
        "tab.closed.restore",
        json!({
            "closed_tab_id":history["closed_tabs"][0]["closed_tab_id"],
        }),
    );
    assert_eq!(
        restored["workspace_id"],
        workspace["workspace"]["workspace_id"]
    );
    assert_eq!(
        result(&core, "tab.closed.list", json!({}))["closed_tabs"],
        json!([])
    );
}

const OWNED_CONVERSATION_TEXT: &str = "Closed-tab proof 000000000002.";

fn wait_for_owned_omp_context(core: &Core, pane: &Value) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let screen = core.request(
            "pane.read",
            json!({"pane_id":pane,"source":"visible","format":"text"}),
        );
        if screen["read"]["text"]
            .as_str()
            .unwrap()
            .contains(OWNED_CONVERSATION_TEXT)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "owned OMP did not render its saved conversation: {screen}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn launch_owned_omp(core: &Core, pane: &Value, transcript: &Path) {
    core.request("pane.send_text", json!({
        "pane_id":pane,
        "text":format!("omp --no-extensions --no-skills --no-context-files --resume='{}'\n", transcript.display()),
    }));
    wait_for_owned_omp_context(core, pane);
    wait_until(
        || {
            request(core, "agent.get", json!({"target":pane}))
                .get("result")
                .is_some()
        },
        "owned OMP process was not detected",
    );
    core.request(
        "pane.report_agent",
        json!({
            "pane_id":pane,"source":"herdr:omp","agent":"omp","state":"idle","seq":1,
            "agent_session_path":transcript,
        }),
    );
    core.request(
        "pane.report_agent_session",
        json!({
            "pane_id":pane,"source":"herdr:omp","agent":"omp","seq":2,
            "session_start_source":"resume","agent_session_path":transcript,
        }),
    );
    assert_eq!(
        core.request("agent.get", json!({"target":pane}))["agent"]["agent_session"]["value"],
        transcript.to_str().unwrap()
    );
}

#[test]
#[ignore = "requires installed OMP; run explicitly for isolated native-conversation verification"]
fn exact_conversation_restore_blocks_live_and_pending_writers_without_replaying_jobs() {
    let core = Core::new();
    let id = "019a4444-0000-7000-8000-000000000002";
    let transcript = core.home.join("owned-exact-conversation.jsonl");
    let header = json!({"type":"session","version":3,"id":id,"timestamp":"2026-11-03T00:00:00.000Z","cwd":core.home,"title":"Owned exact conversation"});
    let message = json!({
        "type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2026-11-03T00:00:01.000Z",
        "message":{"role":"assistant","api":"anthropic-messages","provider":"anthropic","model":"claude-sonnet-4-5",
            "content":[{"type":"text","text":OWNED_CONVERSATION_TEXT}],
            "usage":{"input":100,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":120,
                "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"stop","timestamp":1762128001000_i64},
    });
    fs::write(&transcript, format!("{header}\n{message}\n")).unwrap();
    let marker = core.home.join("ordinary-job-launches");
    let workspace = core.request("workspace.create", json!({"cwd":core.home}));
    let layout = core.request("layout.apply", json!({
        "workspace_id":workspace["workspace"]["workspace_id"],"tab_label":"Conversation and job",
        "root":{"type":"split","direction":"right","ratio":0.5,
            "first":{"type":"pane","cwd":core.home,"label":"Conversation"},
            "second":{"type":"pane","cwd":core.home,"label":"Ordinary job",
                "command":["/bin/sh","-c",format!("printf 'launched\\n' >> '{}'; sleep 300", marker.display())]}},
    }));
    let pane = layout["layout"]["root"]["first"]["pane_id"].clone();
    launch_owned_omp(&core, &pane, &transcript);
    wait_until(|| marker.exists(), "owned ordinary job did not start");
    let original_processes = core.request("pane.process_info", json!({"pane_id":pane}));
    let original_pids: Vec<_> = original_processes["process_info"]["foreground_processes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|process| process["pid"].as_u64().unwrap())
        .collect();
    assert!(!original_pids.is_empty());
    core.request("tab.close", json!({"tab_id":layout["layout"]["tab_id"]}));
    wait_until(
        || {
            original_pids
                .iter()
                .all(|pid| !Path::new(&format!("/proc/{pid}/exe")).exists())
        },
        "closing the tab left its original writer running",
    );
    let history = result(&core, "tab.closed.list", json!({}));
    let entry = history["closed_tabs"][0]["closed_tab_id"].clone();
    let copy = core.request("tab.create", json!({
        "workspace_id":workspace["workspace"]["workspace_id"],"cwd":core.home,"label":"Existing writer",
    }));
    let alias = core.home.join("same-conversation-alias.jsonl");
    std::os::unix::fs::symlink(&transcript, &alias).unwrap();
    launch_owned_omp(&core, &copy["root_pane"]["pane_id"], &alias);
    let live_tabs = core.request("tab.list", json!({}));
    let rejected = request(&core, "tab.closed.restore", json!({"closed_tab_id":entry}));
    assert_eq!(
        rejected["error"]["code"], "closed_tab_restore_failed",
        "{rejected}"
    );
    assert!(rejected["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already"));
    assert_eq!(result(&core, "tab.closed.list", json!({})), history);
    assert_eq!(core.request("tab.list", json!({})), live_tabs);
    let core = restart(core);
    let pending = core.request("agent.get", json!({"target":copy["root_pane"]["pane_id"]}));
    assert_eq!(
        pending["agent"]["agent_session"]["value"],
        alias.to_str().unwrap()
    );
    // No attached client's geometry: this saved conversation has no runtime yet.
    assert_eq!(
        request(
            &core,
            "pane.process_info",
            json!({
                "pane_id":copy["root_pane"]["pane_id"],
            })
        )["error"]["code"],
        "pane_not_found"
    );
    let pending_tabs = core.request("tab.list", json!({}));
    let rejected = request(&core, "tab.closed.restore", json!({"closed_tab_id":entry}));
    assert_eq!(
        rejected["error"]["code"], "closed_tab_restore_failed",
        "{rejected}"
    );
    assert_eq!(result(&core, "tab.closed.list", json!({})), history);
    assert_eq!(core.request("tab.list", json!({})), pending_tabs);
    core.request("tab.close", json!({"tab_id":copy["tab"]["tab_id"]}));
    let before_failure = result(&core, "tab.closed.list", json!({}));
    let saved_copy = before_failure["closed_tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|closed| closed["closed_tab_id"] != entry)
        .unwrap()["closed_tab_id"]
        .clone();
    let tabs_before_failure = core.request("tab.list", json!({}));
    for (configuration, expected_error) in [
        ("[session]\nresume_agents_on_restore = false\n", "disabled"),
        (
            "[terminal]\ndefault_shell = '/nonexistent/closed-tab-owned-shell'\n",
            "Could not start the saved shell",
        ),
    ] {
        fs::write(core.home.join("config.toml"), configuration).unwrap();
        core.request("server.reload_config", json!({}));
        let failed = request(
            &core,
            "tab.closed.restore",
            json!({"closed_tab_id":saved_copy}),
        );
        assert_eq!(
            failed["error"]["code"], "closed_tab_restore_failed",
            "{failed}"
        );
        assert!(
            failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains(expected_error),
            "{failed}"
        );
        assert_eq!(result(&core, "tab.closed.list", json!({})), before_failure);
        assert_eq!(core.request("tab.list", json!({})), tabs_before_failure);
    }
    fs::write(
        core.home.join("config.toml"),
        "[terminal]\ndefault_shell = '/bin/sh'\n",
    )
    .unwrap();
    core.request("server.reload_config", json!({}));
    let restored = result(&core, "tab.closed.restore", json!({"closed_tab_id":entry}));
    let panes = core.request(
        "pane.list",
        json!({"workspace_id":restored["workspace_id"]}),
    );
    let conversation = panes["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["tab_id"] == restored["tab_id"] && pane["label"] == "Conversation")
        .unwrap();
    wait_for_owned_omp_context(&core, &conversation["pane_id"]);
    let job = panes["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["tab_id"] == restored["tab_id"] && pane["label"] == "Ordinary job")
        .unwrap();
    let usable = core.home.join("replacement-job-shell");
    core.request(
        "pane.send_text",
        json!({"pane_id":job["pane_id"],"text":format!("printf ready > '{}'\n", usable.display())}),
    );
    wait_until(
        || usable.exists(),
        "ordinary job pane was not restored as a fresh usable shell",
    );
    assert_eq!(fs::read_to_string(&marker).unwrap(), "launched\n");
    let before_repeat = core.request("tab.list", json!({}));
    let stale = request(&core, "tab.closed.restore", json!({"closed_tab_id":entry}));
    assert_eq!(stale["error"]["code"], "closed_tab_restore_failed");
    assert_eq!(core.request("tab.list", json!({})), before_repeat);
    let remaining = result(&core, "tab.closed.list", json!({}));
    assert_eq!(remaining["closed_tabs"].as_array().unwrap().len(), 1);
    let duplicate = request(
        &core,
        "tab.closed.restore",
        json!({
            "closed_tab_id":remaining["closed_tabs"][0]["closed_tab_id"],
        }),
    );
    assert_eq!(duplicate["error"]["code"], "closed_tab_restore_failed");
}

#[test]
fn older_snapshots_start_empty_and_handoff_preserves_history_live_processes_and_reserved_ids() {
    let core = Core::new();
    let client = core.attach();
    let live = core.request("pane.list", json!({}))["panes"][0].clone();
    core.request(
        "tab.rename",
        json!({"tab_id":live["tab_id"],"label":"Legacy editor"}),
    );
    drop(client);
    let core = restart_with_snapshot_edit(core, |snapshot| {
        snapshot.as_object_mut().unwrap().remove("closed_tabs");
    });
    let _client = core.attach();
    assert_eq!(
        result(&core, "tab.closed.list", json!({}))["closed_tabs"],
        json!([])
    );
    let live = core.request("pane.list", json!({}))["panes"][0].clone();
    assert_eq!(
        core.request("tab.get", json!({"tab_id":live["tab_id"]}))["tab"]["label"],
        "Legacy editor",
    );
    let before_output = core.home.join("older-snapshot-shell");
    core.request("pane.send_text", json!({
        "pane_id":live["pane_id"],"text":format!("printf legacy > '{}'\n",before_output.display()),
    }));
    wait_until(
        || before_output.exists(),
        "older snapshot did not restore a usable terminal",
    );
    assert_eq!(fs::read_to_string(before_output).unwrap(), "legacy");
    let live_pid = core.request("pane.process_info", json!({"pane_id":live["pane_id"]}))
        ["process_info"]["shell_pid"]
        .clone();
    let closed = core.request(
        "workspace.create",
        json!({"label":"Archived project","cwd":core.home}),
    );
    let closed_workspace = closed["workspace"]["workspace_id"].clone();
    core.request(
        "tab.rename",
        json!({"tab_id":closed["root_pane"]["tab_id"],"label":"Archived editor"}),
    );
    core.request("tab.close", json!({"tab_id":closed["root_pane"]["tab_id"]}));
    let history = result(&core, "tab.closed.list", json!({}));
    assert_eq!(history["closed_tabs"][0]["label"], "Archived editor");
    let live_tabs = core.request("tab.list", json!({}));
    core.request(
        "server.live_handoff",
        json!({"import_exe":support::executable()}),
    );
    assert_eq!(result(&core, "tab.closed.list", json!({})), history);
    assert_eq!(core.request("tab.list", json!({})), live_tabs);
    assert_eq!(
        core.request("pane.process_info", json!({"pane_id":live["pane_id"]}))["process_info"]
            ["shell_pid"],
        live_pid,
    );
    let after_output = core.home.join("handoff-shell");
    core.request("pane.send_text", json!({
        "pane_id":live["pane_id"],"text":format!("printf handoff > '{}'\n",after_output.display()),
    }));
    wait_until(
        || after_output.exists(),
        "handoff did not preserve a usable live terminal",
    );
    assert_eq!(fs::read_to_string(after_output).unwrap(), "handoff");
    let unrelated = core.request(
        "workspace.create",
        json!({"label":"Unrelated project","cwd":core.home}),
    );
    assert_ne!(unrelated["workspace"]["workspace_id"], closed_workspace);
    let restored = result(
        &core,
        "tab.closed.restore",
        json!({
            "closed_tab_id":history["closed_tabs"][0]["closed_tab_id"],
        }),
    );
    assert_ne!(
        restored["workspace_id"],
        unrelated["workspace"]["workspace_id"]
    );
    assert_eq!(
        core.request(
            "workspace.get",
            json!({"workspace_id":restored["workspace_id"]})
        )["workspace"]["label"],
        "Archived project",
    );
}

#[test]
#[ignore = "requires installed OMP; run explicitly for isolated native-conversation verification"]
fn a_closed_tab_cannot_start_two_writers_for_the_same_saved_conversation() {
    let core = Core::new();
    let transcript = core.home.join("owned-duplicate-conversation.jsonl");
    fs::write(&transcript, concat!(
        "{\"type\":\"session\",\"version\":3,\"id\":\"019a4444-0000-7000-8000-000000000003\",",
        "\"timestamp\":\"2026-11-03T00:00:00.000Z\",\"cwd\":\"/tmp\",\"title\":\"Owned duplicate fixture\"}\n",
    )).unwrap();
    let created = core.request(
        "workspace.create",
        json!({"label":"Duplicate conversations","cwd":core.home}),
    );
    let first = created["root_pane"]["pane_id"].clone();
    let second = core.request(
        "pane.split",
        json!({
            "target_pane_id":first,"direction":"right","cwd":core.home,
        }),
    )["pane"]["pane_id"]
        .clone();
    for pane in [&first, &second] {
        core.request("pane.send_text", json!({
            "pane_id":pane,"text":format!("omp --no-extensions --no-skills --no-context-files --resume='{}'\n",transcript.display()),
        }));
        wait_until(
            || {
                request(&core, "agent.get", json!({"target":pane}))
                    .get("result")
                    .is_some()
            },
            "owned duplicate fixture did not start its agent",
        );
        core.request(
            "pane.report_agent",
            json!({
                "pane_id":pane,"source":"herdr:omp","agent":"omp","state":"idle","seq":1,
                "agent_session_path":transcript,
            }),
        );
        core.request(
            "pane.report_agent_session",
            json!({
                "pane_id":pane,"source":"herdr:omp","agent":"omp","seq":2,
                "session_start_source":"resume","agent_session_path":transcript,
            }),
        );
        assert_eq!(
            core.request("agent.get", json!({"target":pane}))["agent"]["agent_session"]["value"],
            transcript.to_str().unwrap()
        );
    }
    core.request(
        "tab.close",
        json!({"tab_id":created["root_pane"]["tab_id"]}),
    );
    let history = result(&core, "tab.closed.list", json!({}));
    let tabs = core.request("tab.list", json!({}));
    let rejected = request(
        &core,
        "tab.closed.restore",
        json!({
            "closed_tab_id":history["closed_tabs"][0]["closed_tab_id"],
        }),
    );
    assert_eq!(rejected["error"]["code"], "closed_tab_restore_failed");
    assert!(rejected["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already"));
    assert_eq!(result(&core, "tab.closed.list", json!({})), history);
    assert_eq!(core.request("tab.list", json!({})), tabs);
}
