mod support;

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
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

#[test]
fn cli_transfer_uses_each_destination_insertion_slot() {
    for (index, expected) in [
        (0, ["Build", "One", "Two"]),
        (1, ["One", "Build", "Two"]),
        (2, ["One", "Two", "Build"]),
    ] {
        let core = Core::new();
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let tab = tools["tab"]["tab_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":tab,"label":"Build"}));
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

        let output = cli(
            &core,
            &[
                "tab",
                "move",
                tab,
                "--workspace",
                destination,
                "--index",
                &index.to_string(),
            ],
        );
        assert!(
            output.status.success(),
            "insertion slot {index} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["result"]["transfer"]["index"], index);
        let tabs = core.request("tab.list", json!({"workspace_id":destination}));
        let actual = tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["label"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "destination insertion slot {index}");
        let remaining = core.request("tab.list", json!({"workspace_id":source}));
        assert_eq!(remaining["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(remaining["tabs"][0]["label"], "Remaining");
    }
}

#[test]
fn plugin_transfer_publishes_completed_topology_to_events_and_live_store() {
    let core = Core::new();
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let source = tools["workspace"]["workspace_id"].as_str().unwrap();
    let tab = tools["tab"]["tab_id"].as_str().unwrap();
    let first = tools["root_pane"]["pane_id"].as_str().unwrap();
    core.request("tab.rename", json!({"tab_id":tab,"label":"Build"}));
    let split = core.request(
        "pane.split",
        json!({"target_pane_id":first,"direction":"right","cwd":core.home}),
    );
    let second = split["pane"]["pane_id"].as_str().unwrap();
    for (index, pane) in [first, second].iter().enumerate() {
        core.request(
            "pane.rename",
            json!({"pane_id":pane,"label":format!("Build pane {index}")}),
        );
    }
    core.request(
        "tab.create",
        json!({"workspace_id":source,"label":"Remaining","cwd":core.home}),
    );
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    let destination = main["workspace"]["workspace_id"].as_str().unwrap();
    let before = core.request("session.snapshot", json!({}));

    let stream = UnixStream::connect(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    )
    .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut events = BufReader::new(stream);
    let subscription = json!({"id":"transfer-events","method":"events.subscribe","params":{"subscriptions":[
        {"type":"tab.transferred"},{"type":"workspace.updated"},{"type":"pane.moved"},
        {"type":"tab.created"},{"type":"tab.closed"},{"type":"pane.created"},{"type":"pane.closed"},{"type":"pane.exited"}
    ]}});
    serde_json::to_writer(events.get_mut(), &subscription).unwrap();
    events.get_mut().write_all(b"\n").unwrap();
    let mut line = String::new();
    events.read_line(&mut line).unwrap();
    let started: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        started["result"]["type"], "subscription_started",
        "{started}"
    );

    let plugin = core.home.join("transfer-plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    let receipt_path = plugin.join("receipt.json");
    let command = json!([
        "bun",
        format!(
            "{}/tests/support/tab_transfer_plugin.ts",
            env!("CARGO_MANIFEST_DIR")
        ),
        tab,
        destination,
        receipt_path
    ]);
    std::fs::write(plugin.join("trailboss-plugin.toml"),format!(
        "id = \"fixture.tab-transfer\"\nname = \"Transfer contract\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"transfer\"\ntitle = \"Transfer\"\ncommand = {command}\n"
    )).unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    core.request(
        "plugin.action.invoke",
        json!({"plugin_id":"fixture.tab-transfer","action_id":"transfer"}),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let receipt: Value = loop {
        if let Ok(bytes) = std::fs::read(&receipt_path) {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                break value;
            }
        }
        assert!(
            Instant::now() < deadline,
            "plugin did not finish its subscribed transfer"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(receipt.get("error").is_none(), "{receipt}");
    let transfer = &receipt["response"]["transfer"];
    assert_eq!(transfer["previous_tab_id"], tab);
    assert_eq!(transfer["previous_workspace_id"], source);
    assert_eq!(transfer["tab"]["workspace_id"], destination);
    let mappings = transfer["panes"].as_array().unwrap();
    assert_eq!(mappings.len(), 2);
    let rows_before = receipt["before"].as_array().unwrap();
    let rows_after = receipt["after"].as_array().unwrap();
    for old in [first, second] {
        assert!(rows_before
            .iter()
            .any(|row| row["paneId"] == old && row["workspaceId"] == source));
        assert!(!rows_after.iter().any(|row| row["paneId"] == old));
        let mapping = mappings
            .iter()
            .find(|mapping| mapping["previous_pane_id"] == old)
            .unwrap();
        assert!(rows_after
            .iter()
            .any(|row| row["paneId"] == mapping["pane_id"] && row["workspaceId"] == destination));
        assert!(receipt["tree"]["children"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["context"]["pane_id"] == mapping["pane_id"]
                && row["context"]["workspace_id"] == destination));
    }
    assert!(receipt["events"].as_u64().unwrap() > 0);
    let mut observed = Vec::new();
    while observed
        .iter()
        .filter(|event: &&Value| event["event"] == "pane_moved")
        .count()
        < 2
        || !observed
            .iter()
            .any(|event: &Value| event["event"] == "tab_transferred")
        || observed
            .iter()
            .filter(|event: &&Value| event["event"] == "workspace_updated")
            .count()
            < 2
    {
        line.clear();
        events.read_line(&mut line).unwrap();
        observed.push(serde_json::from_str::<Value>(&line).unwrap());
    }
    assert_eq!(
        observed
            .iter()
            .filter(|event| event["event"] == "tab_transferred")
            .count(),
        1
    );
    assert_eq!(
        observed
            .iter()
            .find(|event| event["event"] == "tab_transferred")
            .unwrap()["data"]["transfer"],
        *transfer
    );
    assert!(
        observed.iter().all(|event| matches!(
            event["event"].as_str(),
            Some("tab_transferred" | "workspace_updated" | "pane_moved")
        )),
        "{observed:?}"
    );
    for mapping in mappings {
        let moved = observed
            .iter()
            .find(|event| {
                event["event"] == "pane_moved"
                    && event["data"]["previous_pane_id"] == mapping["previous_pane_id"]
            })
            .unwrap();
        assert_eq!(moved["data"]["pane"]["pane_id"], mapping["pane_id"]);
        assert_eq!(moved["data"]["pane"]["workspace_id"], destination);
    }
    let after = core.request("session.snapshot", json!({}));
    let mut original_terminals = before["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pane| pane["terminal_id"].clone())
        .collect::<Vec<_>>();
    let mut current_terminals = after["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pane| pane["terminal_id"].clone())
        .collect::<Vec<_>>();
    original_terminals.sort_by_key(Value::to_string);
    current_terminals.sort_by_key(Value::to_string);
    assert_eq!(
        current_terminals, original_terminals,
        "plugin transfer recreated or closed a terminal"
    );
}

#[test]
fn background_transfer_preserves_each_current_view_and_unaffected_tab_focus() {
    for viewed_space in ["Tools", "Main", "Logs"] {
        let core = Core::new();
        let tools = core.request(
            "workspace.create",
            json!({"label":"Tools","cwd":core.home,"focus":true}),
        );
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let build = tools["tab"]["tab_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":build,"label":"Build"}));
        let tools_diagnostics = core.request(
            "tab.create",
            json!({"workspace_id":source,"label":"Diagnostics","cwd":core.home}),
        );
        let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
        let destination = main["workspace"]["workspace_id"].as_str().unwrap();
        let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
        let mut diagnostics = Vec::new();
        for (space, created) in [
            ("Tools", &tools_diagnostics),
            ("Main", &main),
            ("Logs", &logs),
        ] {
            let tab = created["tab"]["tab_id"].as_str().unwrap();
            let root = created["root_pane"]["pane_id"].as_str().unwrap();
            core.request("tab.rename", json!({"tab_id":tab,"label":"Diagnostics"}));
            let split = core.request(
                "pane.split",
                json!({"target_pane_id":root,"direction":"right","cwd":core.home,"focus":true}),
            );
            let second = split["pane"]["pane_id"].as_str().unwrap().to_owned();
            core.request("pane.focus", json!({"pane_id":second}));
            diagnostics.push((space, tab.to_owned(), second));
        }
        // Tools' active tab is deliberately Build when another space is viewed.
        if viewed_space != "Tools" {
            core.request("tab.focus", json!({"tab_id":build}));
        }
        let (_, viewed_tab, viewed_pane) = diagnostics
            .iter()
            .find(|(space, _, _)| *space == viewed_space)
            .unwrap();
        core.request("pane.focus", json!({"pane_id":viewed_pane}));
        let remembered = diagnostics
            .iter()
            .map(|(_, _, pane)| {
                core.request("pane.layout", json!({"pane_id":pane}))["layout"]["focused_pane_id"]
                    .clone()
            })
            .collect::<Vec<_>>();
        let before = core.request("session.snapshot", json!({}));
        let remembered_tabs = before["snapshot"]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|workspace| workspace["workspace_id"] != source)
            .map(|workspace| {
                (
                    workspace["workspace_id"].clone(),
                    workspace["active_tab_id"].clone(),
                )
            })
            .collect::<Vec<_>>();
        let mut client = core.attach();
        while client.focused_pane_id.as_deref() != Some(viewed_pane.as_str()) {
            client.receive().unwrap();
        }

        core.request(
            "tab.transfer",
            json!({"tab_id":build,"workspace_id":destination}),
        );
        // A fresh output frame prevents an unchanged, stale Client value from passing.
        core.request("pane.send_text",json!({"pane_id":viewed_pane,"text":format!("printf '%s%s\\n' 'BG-DIAGNOSTICS-' '{viewed_space}'\n")}));
        let marker = format!("BG-DIAGNOSTICS-{viewed_space}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while client
            .primary
            .as_ref()
            .is_none_or(|surface| !surface.text().contains(&marker))
        {
            assert!(
                Instant::now() < deadline,
                "Diagnostics stopped displaying in {viewed_space}"
            );
            client.receive().unwrap();
        }
        assert_eq!(
            client.focused_tab_id.as_deref(),
            Some(viewed_tab.as_str()),
            "background move changed {viewed_space} client tab"
        );
        assert_eq!(
            client.focused_pane_id.as_deref(),
            Some(viewed_pane.as_str()),
            "background move changed {viewed_space} client pane"
        );
        let snapshot = core.request("session.snapshot", json!({}));
        assert_eq!(
            snapshot["snapshot"]["panes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|pane| pane["focused"] == true)
                .unwrap()["pane_id"],
            *viewed_pane
        );
        for (workspace_id, active_tab) in remembered_tabs {
            let workspace = snapshot["snapshot"]["workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .find(|workspace| workspace["workspace_id"] == workspace_id)
                .unwrap();
            assert_eq!(
                workspace["active_tab_id"], active_tab,
                "remembered active tab changed while viewing {viewed_space}"
            );
        }
        for ((space, _, pane), expected) in diagnostics.iter().zip(remembered) {
            assert_eq!(
                core.request("pane.layout", json!({"pane_id":pane}))["layout"]["focused_pane_id"],
                expected,
                "remembered focus changed in {space}, viewing {viewed_space}"
            );
        }
    }
}

#[test]
fn final_tab_transfer_removes_only_empty_source_before_or_after_destination() {
    for tools_first in [true, false] {
        let core = Core::new();
        let mut created = Vec::new();
        for label in if tools_first {
            ["Tools", "Main"]
        } else {
            ["Main", "Tools"]
        } {
            created.push(core.request("workspace.create", json!({"label":label,"cwd":core.home})));
        }
        let tools = &created[usize::from(!tools_first)];
        let main = &created[usize::from(tools_first)];
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let destination = main["workspace"]["workspace_id"].as_str().unwrap();
        let tab = tools["tab"]["tab_id"].as_str().unwrap();
        let first = tools["root_pane"]["pane_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":tab,"label":"Build"}));
        let split = core.request(
            "pane.split",
            json!({"target_pane_id":first,"direction":"right","cwd":core.home}),
        );
        for (index, pane) in [first, split["pane"]["pane_id"].as_str().unwrap()]
            .iter()
            .enumerate()
        {
            core.request(
                "pane.rename",
                json!({"pane_id":pane,"label":format!("Build pane {index}")}),
            );
        }
        let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
        let diagnostics = logs["root_pane"]["pane_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":logs["tab"]["tab_id"],"label":"Diagnostics"}),
        );
        core.request("pane.focus", json!({"pane_id":diagnostics}));
        let before = core.request("session.snapshot", json!({}));
        let mut originals = Vec::new();
        for (index, pane) in before["snapshot"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            core.request("pane.send_text",json!({"pane_id":pane["pane_id"],"text":format!("transfer_local=source-last-{index}; printf '%s %s\\n' \"$$\" \"$transfer_local\" > last-before-{index}\n")}));
            let state = wait_for_file(&core.home.join(format!("last-before-{index}")));
            originals.push((
                pane["terminal_id"].clone(),
                state.clone(),
                process_identity(state.split_whitespace().next().unwrap()),
            ));
        }
        let mut client = core.attach();
        while client.focused_pane_id.as_deref() != Some(diagnostics) {
            client.receive().unwrap();
        }
        // Reuse the registered plugin's production store and shared command seam.
        let plugin = core.home.join("transfer-plugin");
        std::fs::create_dir_all(&plugin).unwrap();
        let receipt_path = plugin.join("receipt.json");
        let command = json!([
            "bun",
            format!(
                "{}/tests/support/tab_transfer_plugin.ts",
                env!("CARGO_MANIFEST_DIR")
            ),
            tab,
            destination,
            receipt_path
        ]);
        std::fs::write(plugin.join("trailboss-plugin.toml"),format!(
            "id = \"fixture.tab-transfer\"\nname = \"Transfer contract\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"transfer\"\ntitle = \"Transfer\"\ncommand = {command}\n"
        )).unwrap();
        core.request("plugin.link", json!({"path":plugin,"enabled":true}));
        core.request(
            "plugin.action.invoke",
            json!({"plugin_id":"fixture.tab-transfer","action_id":"transfer"}),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let receipt: Value = loop {
            if let Ok(bytes) = std::fs::read(&receipt_path) {
                if let Ok(value) = serde_json::from_slice(&bytes) {
                    break value;
                }
            }
            assert!(
                Instant::now() < deadline,
                "source-last store transfer did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(receipt.get("error").is_none(), "{receipt}");
        let transfer = &receipt["response"]["transfer"];
        assert_eq!(transfer["closed_workspace_id"], source);
        assert_eq!(
            transfer["tab"]["workspace_id"], destination,
            "transfer changed destination after source removal"
        );
        let after = core.request("session.snapshot", json!({}));
        let workspaces = after["snapshot"]["workspaces"].as_array().unwrap();
        assert_eq!(
            workspaces.len(),
            before["snapshot"]["workspaces"].as_array().unwrap().len() - 1
        );
        assert!(!workspaces
            .iter()
            .any(|workspace| workspace["workspace_id"] == source));
        assert!(workspaces
            .iter()
            .any(|workspace| workspace["workspace_id"] == destination
                && workspace["label"] == "Main"));
        assert!(workspaces
            .iter()
            .any(|workspace| workspace["workspace_id"] == logs["workspace"]["workspace_id"]));
        let rows = receipt["after"].as_array().unwrap();
        assert!(!rows.iter().any(|row| row["workspaceId"] == source));
        assert!(rows
            .iter()
            .any(|row| row["kind"] == "workspace" && row["workspaceId"] == destination));
        assert!(rows.iter().any(|row| row["kind"] == "workspace"
            && row["workspaceId"] == logs["workspace"]["workspace_id"]));
        let panes = after["snapshot"]["panes"].as_array().unwrap();
        assert_eq!(
            panes.len(),
            originals.len(),
            "source removal launched a replacement shell"
        );
        for (index, (terminal, state, identity)) in originals.iter().enumerate() {
            let pane = panes
                .iter()
                .find(|pane| pane["terminal_id"] == *terminal)
                .expect("source removal replaced an original terminal");
            core.request("pane.send_text",json!({"pane_id":pane["pane_id"],"text":format!("printf '%s %s\\n' \"$$\" \"$transfer_local\" > last-after-{index}\n")}));
            let current = wait_for_file(&core.home.join(format!("last-after-{index}")));
            assert_eq!(&current, state, "shell-local state or PID changed");
            assert_eq!(
                &process_identity(current.split_whitespace().next().unwrap()),
                identity
            );
        }
        core.request(
            "pane.send_text",
            json!({"pane_id":diagnostics,"text":"printf '%s%s\\n' 'LAST-' 'DIAGNOSTICS'\n"}),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while client
            .primary
            .as_ref()
            .is_none_or(|surface| !surface.text().contains("LAST-DIAGNOSTICS"))
        {
            assert!(
                Instant::now() < deadline,
                "Logs stopped displaying after final-tab movement"
            );
            client.receive().unwrap();
        }
        assert_eq!(client.focused_pane_id.as_deref(), Some(diagnostics));
        assert_eq!(
            client.focused_tab_id,
            logs["tab"]["tab_id"].as_str().map(str::to_owned)
        );
        assert_eq!(after["snapshot"]["focused_pane_id"], diagnostics);
    }
}
