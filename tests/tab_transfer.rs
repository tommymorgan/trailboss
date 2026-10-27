mod support;

use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
use support::Core;

fn cli(core: &Core, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(support::executable());
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let key = key.to_string_lossy();
        key.starts_with("HERDR_") || key.starts_with("TRAILBOSS_")
    }) {
        command.env_remove(key);
    }
    for key in support::HOMES {
        command.env(key, &core.home);
    }
    command
        .args(args)
        .args(["--session", &core.name])
        .current_dir(&core.home)
        .output()
        .unwrap()
}

fn wait_for_file(path: &std::path::Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if text.ends_with('\n') {
                return text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "shell did not write {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn process_identity(pid: &str) -> (String, String, PathBuf, PathBuf) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[4], "0", "process lost its controlling PTY");
    (
        fields[19].to_owned(),
        fields[4].to_owned(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}

#[test]
fn cli_transfers_complete_split_tab_without_restarting_terminals() {
    let core = Core::new();
    let tools = core.request(
        "workspace.create",
        json!({"label":"Tools","cwd":core.home,"focus":true}),
    );
    let source = tools["workspace"]["workspace_id"].as_str().unwrap();
    let tab = tools["tab"]["tab_id"].as_str().unwrap();
    let first = tools["root_pane"]["pane_id"].as_str().unwrap();
    core.request("tab.rename", json!({"tab_id":tab,"label":"Build"}));
    core.request(
        "tab.create",
        json!({"workspace_id":source,"label":"Remaining","cwd":core.home}),
    );
    let split = core.request("pane.split", json!({"target_pane_id":first,"direction":"right","ratio":0.37,"cwd":core.home,"focus":true}));
    let second = split["pane"]["pane_id"].as_str().unwrap();
    let panes = [first, second];
    let mut originals = Vec::new();
    for (index, pane) in panes.iter().enumerate() {
        core.request(
            "pane.rename",
            json!({"pane_id":pane,"label":format!("Build pane {index}")}),
        );
        core.request("pane.send_text", json!({"pane_id":pane,"text":format!("transfer_local=value-{index}; sleep 300 & transfer_job=$!; printf '%s %s %s\\n' \"$$\" \"$transfer_job\" \"$transfer_local\" > before-{index}\n")}));
        let state = wait_for_file(&core.home.join(format!("before-{index}")));
        let fields: Vec<_> = state.split_whitespace().collect();
        assert_eq!(fields[2], format!("value-{index}"));
        originals.push((
            state.clone(),
            process_identity(fields[0]),
            process_identity(fields[1]),
        ));
    }
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(second) {
        client.receive().unwrap();
    }
    core.request("pane.zoom", json!({"pane_id":second,"mode":"on"}));
    let before_layout = core.request("pane.layout", json!({"pane_id":second}))["layout"].clone();
    let before_panes: Vec<_> = panes
        .iter()
        .map(|pane| core.request("pane.get", json!({"pane_id":pane}))["pane"].clone())
        .collect();
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

    let output = cli(&core, &["tab", "move", tab, "--workspace", destination]);
    assert!(
        output.status.success(),
        "CLI transfer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["result"]["type"], "tab_transfer");
    let transfer = &response["result"]["transfer"];
    assert_eq!(transfer["changed"], true);
    assert_eq!(transfer["previous_tab_id"], tab);
    let moved_tab = transfer["tab"]["tab_id"].as_str().unwrap();
    let tabs = core.request("tab.list", json!({"workspace_id":destination}));
    assert_eq!(
        tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["One", "Two", "Build"]
    );
    assert_eq!(tabs["tabs"][2]["tab_id"], moved_tab);
    let source_tabs = core.request("tab.list", json!({"workspace_id":source}));
    assert_eq!(source_tabs["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(source_tabs["tabs"][0]["label"], "Remaining");
    let snapshot = core.request("session.snapshot", json!({}));
    let moved: Vec<_> = snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pane| pane["tab_id"] == moved_tab)
        .collect();
    assert_eq!(moved.len(), 2);
    let moved_second = moved
        .iter()
        .find(|pane| pane["terminal_id"] == before_panes[1]["terminal_id"])
        .unwrap()["pane_id"]
        .as_str()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.focused_tab_id.as_deref() != Some(moved_tab) {
        assert!(
            Instant::now() < deadline,
            "attached client did not follow the existing tab"
        );
        client.receive().unwrap();
    }
    assert_eq!(client.focused_pane_id.as_deref(), Some(moved_second));
    for (index, old) in before_panes.iter().enumerate() {
        let pane = moved
            .iter()
            .find(|pane| pane["terminal_id"] == old["terminal_id"])
            .expect("original terminal missing");
        assert_eq!(pane["label"], old["label"]);
        assert_eq!(pane["workspace_id"], destination);
        let new_id = pane["pane_id"].as_str().unwrap();
        let layout = core.request("pane.layout", json!({"pane_id":new_id}))["layout"].clone();
        assert_eq!(layout["zoomed"], true);
        assert_eq!(layout["splits"], before_layout["splits"]);
        assert_eq!(
            layout["panes"].as_array().unwrap().len(),
            before_layout["panes"].as_array().unwrap().len()
        );
        if index == 1 {
            assert_eq!(layout["focused_pane_id"], new_id);
        }
        core.request("pane.send_text", json!({"pane_id":new_id,"text":format!("kill -0 \"$transfer_job\" && printf '%s %s %s\\n' \"$$\" \"$transfer_job\" \"$transfer_local\" > after-{index}\n")}));
        let state = wait_for_file(&core.home.join(format!("after-{index}")));
        assert_eq!(
            state, originals[index].0,
            "shell-local or background-job state changed"
        );
        let fields: Vec<_> = state.split_whitespace().collect();
        assert_eq!(process_identity(fields[0]), originals[index].1);
        assert_eq!(process_identity(fields[1]), originals[index].2);
    }
}
