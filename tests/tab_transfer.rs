#[path = "support/restart.rs"]
mod restart;
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

// The imported owner is not Core.child. Stop whichever owner currently serves
// this isolated session, including on a failed post-handoff assertion.
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
fn positional_transfer_locators_preserve_each_attached_clients_view() {
    for locator in ["w2:1", "t_2_1"] {
        let core = Core::new();
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let first = tools["root_pane"]["pane_id"].as_str().unwrap();
        let split = core.request(
            "pane.split",
            json!({"target_pane_id":first,"direction":"right","cwd":core.home}),
        );
        let viewed = split["pane"]["pane_id"].as_str().unwrap();
        core.request("pane.send_text", json!({"pane_id":viewed,"text":"view_local=original-pane; printf 'ready\\n' > positional-ready\n"}));
        wait_for_file(&core.home.join("positional-ready"));
        core.request("tab.create", json!({"workspace_id":tools["workspace"]["workspace_id"],"label":"Remaining","cwd":core.home}));
        let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
        let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
        core.request("pane.focus", json!({"pane_id":viewed}));
        let mut followers = [core.attach(), core.attach()];
        for client in &mut followers {
            while client.focused_pane_id.as_deref() != Some(viewed) {
                client.receive().unwrap();
            }
        }
        let mut background = core.attach();
        while background.boot_id.is_empty() {
            background.receive().unwrap();
        }
        background
            .input()
            .unwrap()
            .request(
                &background.boot_id,
                "tab.focus",
                json!({"tab_id":logs["tab"]["tab_id"]}),
            )
            .unwrap();
        let logs_pane = logs["root_pane"]["pane_id"].as_str().unwrap();
        while background.focused_pane_id.as_deref() != Some(logs_pane) {
            background.receive().unwrap();
        }
        let moved = cli(
            &core,
            &[
                "tab",
                "move",
                locator,
                "--workspace",
                main["workspace"]["workspace_id"].as_str().unwrap(),
            ],
        );
        assert!(
            moved.status.success(),
            "{locator}: {}",
            String::from_utf8_lossy(&moved.stderr)
        );
        let moved: Value = serde_json::from_slice(&moved.stdout).unwrap();
        let transfer = &moved["result"]["transfer"];
        let moved_tab = transfer["tab"]["tab_id"].as_str().unwrap();
        let moved_pane = transfer["panes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|mapping| mapping["previous_pane_id"] == viewed)
            .unwrap()["pane_id"]
            .as_str()
            .unwrap();
        for (index, client) in followers.iter_mut().enumerate() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while client.focused_pane_id.as_deref() != Some(moved_pane) {
                assert!(
                    Instant::now() < deadline,
                    "{locator}: follower {index} lost its viewed pane"
                );
                client
                    .receive()
                    .expect("positional transfer did not move the attached client view");
            }
            assert_eq!(client.focused_tab_id.as_deref(), Some(moved_tab));
            let marker = format!("POSITIONAL-FOLLOW-{index}");
            let mut input = client.input().unwrap();
            input.focus_pane_target(client.focused_pane_id.as_deref().unwrap());
            input.type_text(&format!("printf '%s\\n' \"$view_local\" > positional-{index}.state; printf '%s%s\\n' 'POSITIONAL-FOLLOW-' '{index}'\n")).unwrap();
            assert_eq!(
                wait_for_file(&core.home.join(format!("positional-{index}.state"))).trim(),
                "original-pane"
            );
            while client
                .primary
                .as_ref()
                .is_none_or(|surface| !surface.text().contains(&marker))
            {
                assert!(
                    Instant::now() < deadline,
                    "{locator}: follower {index} stopped rendering"
                );
                client.receive().unwrap();
            }
            assert_eq!(client.focused_pane_id.as_deref(), Some(moved_pane));
        }
        let mut input = background.input().unwrap();
        input.focus_pane_target(background.focused_pane_id.as_deref().unwrap());
        input
            .type_text("printf '%s%s\\n' 'POSITIONAL-' 'BACKGROUND'\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while background
            .primary
            .as_ref()
            .is_none_or(|surface| !surface.text().contains("POSITIONAL-BACKGROUND"))
        {
            assert!(
                Instant::now() < deadline,
                "{locator}: background stopped rendering"
            );
            background.receive().unwrap();
        }
        assert_eq!(background.focused_pane_id.as_deref(), Some(logs_pane));
        assert_eq!(
            background.focused_tab_id.as_deref(),
            logs["tab"]["tab_id"].as_str()
        );
        assert_eq!(
            core.request(
                "tab.list",
                json!({"workspace_id":tools["workspace"]["workspace_id"]})
            )["tabs"][0]["label"],
            "Remaining"
        );
    }
}

#[test]
fn client_endpoint_transfer_follows_every_viewer_without_moving_background_clients() {
    let core = Core::new();
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let first = tools["root_pane"]["pane_id"].as_str().unwrap();
    let split = core.request(
        "pane.split",
        json!({"target_pane_id":first,"direction":"right","cwd":core.home}),
    );
    let viewed = split["pane"]["pane_id"].as_str().unwrap();
    core.request("pane.send_text", json!({"pane_id":viewed,"text":"endpoint_local=original-pane; printf 'ready\\n' > endpoint-ready\n"}));
    wait_for_file(&core.home.join("endpoint-ready"));
    core.request("tab.create", json!({"workspace_id":tools["workspace"]["workspace_id"],"label":"Remaining","cwd":core.home}));
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    let destination = main["workspace"]["workspace_id"].as_str().unwrap();
    let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
    core.request("pane.focus", json!({"pane_id":viewed}));
    let mut followers = [core.attach(), core.attach()];
    for client in &mut followers {
        while client.focused_pane_id.as_deref() != Some(viewed) {
            client.receive().unwrap();
        }
    }
    let mut background = core.attach();
    while background.boot_id.is_empty() {
        background.receive().unwrap();
    }
    background
        .input()
        .unwrap()
        .request(
            &background.boot_id,
            "tab.focus",
            json!({"tab_id":logs["tab"]["tab_id"]}),
        )
        .unwrap();
    let logs_pane = logs["root_pane"]["pane_id"].as_str().unwrap();
    while background.focused_pane_id.as_deref() != Some(logs_pane) {
        background.receive().unwrap();
    }
    followers[0]
        .input()
        .unwrap()
        .request(
            &followers[0].boot_id,
            "tab.transfer",
            json!({"tab_id":tools["tab"]["tab_id"],"workspace_id":destination}),
        )
        .unwrap();
    while followers[0].workspace_id.as_deref() != Some(destination) {
        followers[0].receive().unwrap();
    }
    let moved = core.request("pane.get", json!({"pane_id":viewed}));
    let moved_pane = moved["pane"]["pane_id"].as_str().unwrap();
    let moved_tab = moved["pane"]["tab_id"].as_str().unwrap();
    for (index, client) in followers.iter_mut().enumerate() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.focused_pane_id.as_deref() != Some(moved_pane) {
            assert!(
                Instant::now() < deadline,
                "endpoint follower {index} lost its viewed pane"
            );
            client
                .receive()
                .expect("client endpoint transfer did not move every attached viewer");
        }
        assert_eq!(client.focused_tab_id.as_deref(), Some(moved_tab));
        let marker = format!("ENDPOINT-FOLLOW-{index}");
        let mut input = client.input().unwrap();
        input.focus_pane_target(client.focused_pane_id.as_deref().unwrap());
        input.type_text(&format!("printf '%s\\n' \"$endpoint_local\" > endpoint-{index}.state; printf '%s%s\\n' 'ENDPOINT-FOLLOW-' '{index}'\n")).unwrap();
        assert_eq!(
            wait_for_file(&core.home.join(format!("endpoint-{index}.state"))).trim(),
            "original-pane"
        );
        while client
            .primary
            .as_ref()
            .is_none_or(|surface| !surface.text().contains(&marker))
        {
            assert!(
                Instant::now() < deadline,
                "endpoint follower {index} stopped rendering"
            );
            client.receive().unwrap();
        }
        assert_eq!(client.focused_pane_id.as_deref(), Some(moved_pane));
    }
    let mut input = background.input().unwrap();
    input.focus_pane_target(background.focused_pane_id.as_deref().unwrap());
    input
        .type_text("printf '%s%s\\n' 'ENDPOINT-' 'BACKGROUND'\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while background
        .primary
        .as_ref()
        .is_none_or(|surface| !surface.text().contains("ENDPOINT-BACKGROUND"))
    {
        assert!(
            Instant::now() < deadline,
            "endpoint background stopped rendering"
        );
        background.receive().unwrap();
    }
    assert_eq!(background.focused_pane_id.as_deref(), Some(logs_pane));
    assert_eq!(
        background.focused_tab_id.as_deref(),
        logs["tab"]["tab_id"].as_str()
    );
    assert_eq!(
        core.request(
            "tab.list",
            json!({"workspace_id":tools["workspace"]["workspace_id"]})
        )["tabs"][0]["label"],
        "Remaining"
    );
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

#[test]
fn invalid_transfers_preserve_topology_view_pane_targeting_and_processes() {
    for invalid in [
        "source",
        "destination",
        "w_999999",
        "999999",
        "index",
        "negative",
        "old-tab",
    ] {
        let core = Core::new();
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let source = tools["workspace"]["workspace_id"].as_str().unwrap();
        let tab = tools["tab"]["tab_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":tab,"label":"Build"}));
        core.request("pane.split",json!({"target_pane_id":tools["root_pane"]["pane_id"],"direction":"right","cwd":core.home}));
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
        let mut params = json!({"tab_id":tab,"workspace_id":destination});
        let expected_code = match invalid {
            "source" => {
                let doomed = core.request(
                    "tab.create",
                    json!({"workspace_id":source,"label":"Closed","cwd":core.home}),
                );
                core.request("tab.close", json!({"tab_id":doomed["tab"]["tab_id"]}));
                params["tab_id"] = doomed["tab"]["tab_id"].clone();
                "tab_not_found"
            }
            "destination" => {
                let doomed = core.request(
                    "workspace.create",
                    json!({"label":"Closed","cwd":core.home}),
                );
                core.request(
                    "workspace.close",
                    json!({"workspace_id":doomed["workspace"]["workspace_id"]}),
                );
                params["workspace_id"] = doomed["workspace"]["workspace_id"].clone();
                "workspace_not_found"
            }
            "w_999999" | "999999" => {
                params["workspace_id"] = json!(invalid);
                "workspace_not_found"
            }
            "index" => {
                params["insert_index"] = json!(3);
                "tab_transfer_failed"
            }
            "negative" => {
                params["insert_index"] = json!(-1);
                "invalid_request"
            }
            "old-tab" => {
                core.request("tab.transfer", params.clone());
                "tab_not_found"
            }
            _ => unreachable!(),
        };
        let invalid_token = match invalid {
            "source" | "old-tab" => params["tab_id"].as_str().unwrap().to_owned(),
            "destination" | "w_999999" | "999999" => {
                params["workspace_id"].as_str().unwrap().to_owned()
            }
            "index" | "negative" => params["insert_index"].to_string(),
            _ => unreachable!(),
        };
        let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
        let diagnostics = logs["root_pane"]["pane_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":logs["tab"]["tab_id"],"label":"Diagnostics"}),
        );
        core.request("pane.focus", json!({"pane_id":diagnostics}));
        let semantic_topology = |snapshot: &Value| {
            json!({
                "focused_pane_id":snapshot["focused_pane_id"],
                "workspaces":snapshot["workspaces"].as_array().unwrap().iter().map(|workspace|
                    json!([workspace["workspace_id"],workspace["label"],workspace["active_tab_id"],workspace["tab_count"]])
                ).collect::<Vec<_>>(),
                "tabs":snapshot["tabs"].as_array().unwrap().iter().map(|tab|
                    json!([tab["tab_id"],tab["workspace_id"],tab["number"],tab["label"],tab["focused"],tab["pane_count"]])
                ).collect::<Vec<_>>(),
                "panes":snapshot["panes"].as_array().unwrap().iter().map(|pane|
                    json!([pane["pane_id"],pane["terminal_id"],pane["workspace_id"],pane["tab_id"],pane["focused"],pane["label"]])
                ).collect::<Vec<_>>()
            })
        };
        let mut client = core.attach();
        while client.focused_pane_id.as_deref() != Some(diagnostics) {
            client.receive().unwrap();
        }
        let before = core.request("session.snapshot", json!({}));
        let mut originals = Vec::new();
        for (index, pane) in before["snapshot"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let pane_id = pane["pane_id"].as_str().unwrap();
            core.request("pane.send_text",json!({"pane_id":pane_id,"text":format!("transfer_local=invalid-{index}; printf '%s %s\\n' \"$$\" \"$transfer_local\" > invalid-before-{index}\n")}));
            let state = wait_for_file(&core.home.join(format!("invalid-before-{index}")));
            let identity = process_identity(state.split_whitespace().next().unwrap());
            let layout = core.request("pane.layout", json!({"pane_id":pane_id}))["layout"].clone();
            originals.push((pane_id.to_owned(), state, identity, layout));
        }
        // Send the malformed negative index over the actual wire, not typed serde.
        let mut stream = BufReader::new(
            UnixStream::connect(
                core.home
                    .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
            )
            .unwrap(),
        );
        stream
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let request_id = format!("invalid-transfer-{invalid}");
        serde_json::to_writer(
            stream.get_mut(),
            &json!({"id":request_id,"method":"tab.transfer","params":params}),
        )
        .unwrap();
        stream.get_mut().write_all(b"\n").unwrap();
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        let rejected: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            rejected["id"], request_id,
            "uncorrelated {invalid} rejection: {rejected}"
        );
        assert_eq!(
            rejected["error"]["code"], expected_code,
            "{invalid}: {rejected}"
        );
        assert!(
            rejected["error"]["message"]
                .as_str()
                .unwrap()
                .contains(&invalid_token),
            "{invalid} rejection did not identify {invalid_token}: {rejected}"
        );
        assert!(rejected.get("result").is_none(), "{invalid}: {rejected}");
        let after = core.request("session.snapshot", json!({}));
        assert_eq!(
            semantic_topology(&after["snapshot"]),
            semantic_topology(&before["snapshot"]),
            "{invalid} partially changed topology or view"
        );
        for (index, (pane, state, identity, layout)) in originals.iter().enumerate() {
            assert_eq!(
                &core.request("pane.layout", json!({"pane_id":pane}))["layout"],
                layout,
                "{invalid} changed pane layout/target"
            );
            core.request("pane.send_text",json!({"pane_id":pane,"text":format!("printf '%s %s\\n' \"$$\" \"$transfer_local\" > invalid-after-{index}\n")}));
            let current = wait_for_file(&core.home.join(format!("invalid-after-{index}")));
            assert_eq!(
                &current, state,
                "{invalid} changed pane targeting, shell locals or PID"
            );
            assert_eq!(
                &process_identity(current.split_whitespace().next().unwrap()),
                identity
            );
        }
        core.request("pane.send_text",json!({"pane_id":diagnostics,"text":format!("printf '%s%s\\n' 'INVALID-DIAGNOSTICS-' '{invalid}'\n")}));
        let marker = format!("INVALID-DIAGNOSTICS-{invalid}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while client
            .primary
            .as_ref()
            .is_none_or(|surface| !surface.text().contains(&marker))
        {
            assert!(
                Instant::now() < deadline,
                "{invalid} changed displayed Diagnostics"
            );
            client.receive().unwrap();
        }
        assert_eq!(client.focused_pane_id.as_deref(), Some(diagnostics));
        assert_eq!(
            client.focused_tab_id,
            logs["tab"]["tab_id"].as_str().map(str::to_owned)
        );
    }
}

#[test]
fn same_space_transfers_reorder_with_stable_ids_and_silent_noops() {
    for (slot, order, changed) in [
        (0, ["Build", "One", "Two"], true),
        (1, ["One", "Build", "Two"], false),
        (2, ["One", "Build", "Two"], false),
        (3, ["One", "Two", "Build"], true),
    ] {
        let core = Core::new();
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let workspace = tools["workspace"]["workspace_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":tools["tab"]["tab_id"],"label":"One"}),
        );
        let build = core.request(
            "tab.create",
            json!({"workspace_id":workspace,"label":"Build","cwd":core.home}),
        );
        let tab = build["tab"]["tab_id"].as_str().unwrap();
        core.request("pane.split",json!({"target_pane_id":build["root_pane"]["pane_id"],"direction":"right","cwd":core.home}));
        core.request(
            "tab.create",
            json!({"workspace_id":workspace,"label":"Two","cwd":core.home}),
        );
        let logs = core.request("workspace.create", json!({"label":"Logs","cwd":core.home}));
        let diagnostics = logs["root_pane"]["pane_id"].as_str().unwrap();
        core.request(
            "tab.rename",
            json!({"tab_id":logs["tab"]["tab_id"],"label":"Diagnostics"}),
        );
        core.request("pane.focus", json!({"pane_id":diagnostics}));
        let mut client = core.attach();
        while client.focused_pane_id.as_deref() != Some(diagnostics) {
            client.receive().unwrap();
        }
        let before = core.request("session.snapshot", json!({}));
        let identities = |snapshot: &Value| {
            let mut tabs = snapshot["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tab| tab["tab_id"].to_string())
                .collect::<Vec<_>>();
            let mut panes = snapshot["panes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|pane| {
                    json!([
                        pane["pane_id"],
                        pane["terminal_id"],
                        pane["tab_id"],
                        pane["workspace_id"]
                    ])
                    .to_string()
                })
                .collect::<Vec<_>>();
            tabs.sort();
            panes.sort();
            (tabs, panes)
        };
        let mut subscriber = BufReader::new(
            UnixStream::connect(
                core.home
                    .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
            )
            .unwrap(),
        );
        subscriber
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(subscriber.get_mut(),&json!({
            "id":"same-space-events","method":"events.subscribe","params":{"subscriptions":[
                {"type":"tab.moved"},{"type":"tab.transferred"},{"type":"workspace.updated"},{"type":"workspace.closed"},
                {"type":"pane.moved"},{"type":"tab.created"},{"type":"tab.closed"},{"type":"pane.created"},{"type":"pane.closed"}
            ]}
        })).unwrap();
        subscriber.get_mut().write_all(b"\n").unwrap();
        let mut line = String::new();
        subscriber.read_line(&mut line).unwrap();
        let started: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(started["result"]["type"], "subscription_started");
        let moved = core.request(
            "tab.transfer",
            json!({"tab_id":tab,"workspace_id":workspace,"insert_index":slot}),
        );
        assert_eq!(moved["transfer"]["changed"], changed);
        assert_eq!(moved["transfer"]["tab"]["tab_id"], tab);
        assert_eq!(moved["transfer"]["closed_workspace_id"], Value::Null);
        for mapping in moved["transfer"]["panes"].as_array().unwrap() {
            assert_eq!(mapping["previous_pane_id"], mapping["pane_id"]);
        }
        let after = core.request("session.snapshot", json!({}));
        assert_eq!(
            identities(&after["snapshot"]),
            identities(&before["snapshot"]),
            "slot {slot} reallocated public identities or terminals"
        );
        let tabs = core.request("tab.list", json!({"workspace_id":workspace}));
        assert_eq!(
            tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tab| tab["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            order,
            "slot {slot}"
        );
        assert_eq!(after["snapshot"]["focused_pane_id"], diagnostics);
        core.request("pane.send_text",json!({"pane_id":diagnostics,"text":format!("printf '%s%s\\n' 'SAME-SPACE-DIAGNOSTICS-' '{slot}'\n")}));
        let marker = format!("SAME-SPACE-DIAGNOSTICS-{slot}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while client
            .primary
            .as_ref()
            .is_none_or(|surface| !surface.text().contains(&marker))
        {
            assert!(
                Instant::now() < deadline,
                "slot {slot} changed displayed Diagnostics"
            );
            client.receive().unwrap();
        }
        assert_eq!(client.focused_pane_id.as_deref(), Some(diagnostics));
        assert_eq!(
            client.focused_tab_id,
            logs["tab"]["tab_id"].as_str().map(str::to_owned)
        );
        subscriber
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut notifications = Vec::new();
        loop {
            line.clear();
            match subscriber.read_line(&mut line) {
                Ok(0) => panic!("topology subscription closed for slot {slot}"),
                Ok(_) => notifications.push(serde_json::from_str::<Value>(&line).unwrap()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break
                }
                Err(error) => panic!("topology subscription failed for slot {slot}: {error}"),
            }
        }
        if changed {
            assert_eq!(notifications.len(), 1, "slot {slot}: {notifications:?}");
            assert_eq!(notifications[0]["event"], "tab_moved");
        } else {
            assert!(
                notifications.is_empty(),
                "no-op slot {slot} emitted topology notifications: {notifications:?}"
            );
        }
    }
}

#[test]
fn inherited_pane_context_survives_transfer_and_live_handoff_until_close() {
    let core = Core::new();
    let _stop_owner = StopOwner(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    );
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let first = tools["root_pane"]["pane_id"].as_str().unwrap().to_owned();
    let second = core.request(
        "pane.split",
        json!({"target_pane_id":first,"direction":"right","cwd":core.home}),
    )["pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = [first, second];
    let mut current = original.to_vec();
    let mut terminals = original
        .iter()
        .map(|pane| {
            core.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone()
        })
        .collect::<Vec<_>>();
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    let mut processes = Vec::new();
    let mut shell_states = Vec::new();
    for stage in ["before", "moved", "handoff"] {
        if stage == "moved" {
            let transfer = core.request("tab.transfer",json!({"tab_id":tools["tab"]["tab_id"],"workspace_id":main["workspace"]["workspace_id"]}));
            current = original
                .iter()
                .map(|pane| {
                    transfer["transfer"]["panes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|mapping| mapping["previous_pane_id"] == *pane)
                        .unwrap()["pane_id"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect();
        } else if stage == "handoff" {
            core.request(
                "server.live_handoff",
                json!({"import_exe":support::executable()}),
            );
            // Existing handoff allocates fresh logical TerminalIds around the
            // imported FDs. Aliases must target those current runtimes, while
            // PID, PTY and shell context must survive unchanged.
            terminals = current
                .iter()
                .map(|pane| {
                    core.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone()
                })
                .collect();
        }
        for (index, pane) in current.iter().enumerate() {
            let initialize = if stage == "before" {
                format!("transfer_local=local-{index}; ")
            } else {
                String::new()
            };
            core.request("pane.send_text",json!({"pane_id":pane,"text":format!(
                "{initialize}printf '%s %s %s\\n' \"$$\" \"$transfer_local\" \"$HERDR_PANE_ID\" > {stage}-{index}.state; '{}' pane current --current > {stage}-{index}.json 2> {stage}-{index}.err; printf '%s\\n' \"$?\" > {stage}-{index}.done\n",
                support::executable().display()
            )}));
            let status = wait_for_file(&core.home.join(format!("{stage}-{index}.done")));
            assert_eq!(
                status.trim(),
                "0",
                "inherited CLI context failed after {stage}: {}",
                std::fs::read_to_string(core.home.join(format!("{stage}-{index}.err"))).unwrap()
            );
            let state = wait_for_file(&core.home.join(format!("{stage}-{index}.state")));
            let fields = state.split_whitespace().collect::<Vec<_>>();
            assert_eq!(fields[1], format!("local-{index}"));
            assert_eq!(
                fields[2], original[index],
                "transfer must not rewrite inherited pane context"
            );
            if stage == "before" {
                processes.push(process_identity(fields[0]));
                shell_states.push(state.clone());
            } else {
                assert_eq!(
                    state, shell_states[index],
                    "shell PID/local/context changed after {stage}"
                );
                assert_eq!(
                    process_identity(fields[0]),
                    processes[index],
                    "process/PTY changed after {stage}"
                );
            }
            let inherited: Value = serde_json::from_str(
                &std::fs::read_to_string(core.home.join(format!("{stage}-{index}.json"))).unwrap(),
            )
            .unwrap();
            assert_eq!(
                inherited["result"]["pane"]["pane_id"], *pane,
                "inherited context retargeted after {stage}"
            );
            assert_eq!(inherited["result"]["pane"]["terminal_id"], terminals[index]);
            let aliased = core.request("pane.get", json!({"pane_id":original[index]}));
            assert_eq!(aliased["pane"]["pane_id"], *pane);
            assert_eq!(aliased["pane"]["terminal_id"], terminals[index]);
        }
    }
    core.request("pane.close", json!({"pane_id":original[0]}));
    let replacement = core.request(
        "pane.split",
        json!({"target_pane_id":current[1],"direction":"down","cwd":core.home}),
    );
    assert_ne!(replacement["pane"]["terminal_id"], terminals[0]);
    let mut wire = BufReader::new(
        UnixStream::connect(
            core.home
                .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
        )
        .unwrap(),
    );
    wire.get_mut()
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    serde_json::to_writer(
        wire.get_mut(),
        &json!({"id":"closed-transfer-alias","method":"pane.get","params":{"pane_id":original[0]}}),
    )
    .unwrap();
    wire.get_mut().write_all(b"\n").unwrap();
    let mut line = String::new();
    wire.read_line(&mut line).unwrap();
    let closed: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(closed["id"], "closed-transfer-alias");
    assert_eq!(closed["error"]["code"], "pane_not_found");
    assert!(
        closed.get("result").is_none(),
        "closed alias retargeted a replacement pane"
    );
    assert_eq!(
        core.request("pane.get", json!({"pane_id":original[1]}))["pane"]["terminal_id"],
        terminals[1]
    );
}

#[test]
fn handoff_reserves_removed_workspace_ids_carried_by_live_pane_aliases() {
    let core = Core::new();
    let _stop_owner = StopOwner(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    );
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let inherited = tools["root_pane"]["pane_id"].as_str().unwrap();
    core.request("pane.send_text",json!({"pane_id":inherited,"text":"alias_local=transferred; printf '%s %s %s\\n' \"$$\" \"$alias_local\" \"$HERDR_PANE_ID\" > alias-before.state\n"}));
    let before = wait_for_file(&core.home.join("alias-before.state"));
    let before_pid = before.split_whitespace().next().unwrap();
    let identity = process_identity(before_pid);
    let moved = cli(
        &core,
        &[
            "tab",
            "move",
            tools["tab"]["tab_id"].as_str().unwrap(),
            "--workspace",
            main["workspace"]["workspace_id"].as_str().unwrap(),
        ],
    );
    assert!(
        moved.status.success(),
        "CLI transfer failed: {}",
        String::from_utf8_lossy(&moved.stderr)
    );
    let moved: Value = serde_json::from_slice(&moved.stdout).unwrap();
    assert_eq!(
        moved["result"]["transfer"]["closed_workspace_id"],
        tools["workspace"]["workspace_id"]
    );
    let current = moved["result"]["transfer"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap();
    core.request(
        "server.live_handoff",
        json!({"import_exe":support::executable()}),
    );
    let fresh = core.request("workspace.create", json!({"label":"Fresh","cwd":core.home}));
    let fresh_pane = fresh["root_pane"]["pane_id"].as_str().unwrap();
    for (pane, stage) in [(inherited, "old"), (fresh_pane, "fresh")] {
        core.request("pane.send_text",json!({"pane_id":pane,"text":format!(
            "printf '%s %s %s\\n' \"$$\" \"${{alias_local:-fresh}}\" \"$HERDR_PANE_ID\" > alias-{stage}.state; '{}' pane current --current > alias-{stage}.json 2> alias-{stage}.err; printf '%s\\n' \"$?\" > alias-{stage}.done\n",
            support::executable().display()
        )}));
        assert_eq!(
            wait_for_file(&core.home.join(format!("alias-{stage}.done"))).trim(),
            "0",
            "inherited CLI targeting failed for {stage}: {}",
            std::fs::read_to_string(core.home.join(format!("alias-{stage}.err"))).unwrap()
        );
    }
    assert_eq!(wait_for_file(&core.home.join("alias-old.state")), before);
    assert_eq!(process_identity(before_pid), identity);
    let old_context: Value =
        serde_json::from_str(&std::fs::read_to_string(core.home.join("alias-old.json")).unwrap())
            .unwrap();
    assert_eq!(old_context["result"]["pane"]["pane_id"], current);
    let fresh_state = wait_for_file(&core.home.join("alias-fresh.state"));
    let fields = fresh_state.split_whitespace().collect::<Vec<_>>();
    assert_eq!(
        fields[1], "fresh",
        "new workspace input reached the transferred shell through a reused alias"
    );
    assert_ne!(fields[0], before_pid, "new root reused the old shell");
    assert_eq!(fields[2], fresh_pane);
    let fresh_context: Value =
        serde_json::from_str(&std::fs::read_to_string(core.home.join("alias-fresh.json")).unwrap())
            .unwrap();
    assert_eq!(fresh_context["result"]["pane"]["pane_id"], fresh_pane);
    assert_eq!(
        fresh_context["result"]["pane"]["terminal_id"],
        fresh["root_pane"]["terminal_id"]
    );
    assert_ne!(
        fresh["workspace"]["workspace_id"], tools["workspace"]["workspace_id"],
        "new workspace reused a public identity retained by a live alias"
    );
}

#[test]
fn explicit_closes_remove_transferred_aliases_before_the_next_live_handoff() {
    for close_method in ["pane.close", "tab.close", "workspace.close"] {
        let core = Core::new();
        let socket = core
            .home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
        let _stop_owner = StopOwner(socket.clone());
        let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
        let original = tools["root_pane"]["pane_id"].as_str().unwrap();
        let split = core.request(
            "pane.split",
            json!({"target_pane_id":original,"direction":"right","cwd":core.home}),
        );
        let second = split["pane"]["pane_id"].as_str().unwrap();
        let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
        let survivor = core.request(
            "workspace.create",
            json!({"label":"Survivor","cwd":core.home}),
        );
        let survivor_pane = survivor["root_pane"]["pane_id"].as_str().unwrap();
        core.request("pane.send_text", json!({"pane_id":survivor_pane,"text":"close_local=surviving; printf '%s %s\\n' \"$$\" \"$close_local\" > close-before.state\n"}));
        let before = wait_for_file(&core.home.join("close-before.state"));
        let pid = before.split_whitespace().next().unwrap();
        let identity = process_identity(pid);
        let moved = cli(
            &core,
            &[
                "tab",
                "move",
                tools["tab"]["tab_id"].as_str().unwrap(),
                "--workspace",
                main["workspace"]["workspace_id"].as_str().unwrap(),
            ],
        );
        assert!(
            moved.status.success(),
            "{close_method}: {}",
            String::from_utf8_lossy(&moved.stderr)
        );
        let moved: Value = serde_json::from_slice(&moved.stdout).unwrap();
        core.request(
            "server.live_handoff",
            json!({"import_exe":support::executable()}),
        );
        let target = match close_method {
            "pane.close" => json!({"pane_id":original}),
            "tab.close" => json!({"tab_id":moved["result"]["transfer"]["tab"]["tab_id"]}),
            "workspace.close" => json!({"workspace_id":main["workspace"]["workspace_id"]}),
            _ => unreachable!(),
        };
        core.request(close_method, target);
        let removed = if close_method == "pane.close" {
            vec![original]
        } else {
            vec![original, second]
        };
        for alias in removed {
            let mut wire = BufReader::new(UnixStream::connect(&socket).unwrap());
            wire.get_mut()
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            serde_json::to_writer(
                wire.get_mut(),
                &json!({"id":"closed-alias","method":"pane.get","params":{"pane_id":alias}}),
            )
            .unwrap();
            wire.get_mut().write_all(b"\n").unwrap();
            let mut line = String::new();
            wire.read_line(&mut line).unwrap();
            let closed: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(closed["id"], "closed-alias");
            assert_eq!(
                closed["error"]["code"], "pane_not_found",
                "{close_method}: {closed}"
            );
            assert!(closed.get("result").is_none());
        }
        // A failed lookup alone does not prove stale entries were removed:
        // the next exporter must still accept every retained live alias.
        core.request(
            "server.live_handoff",
            json!({"import_exe":support::executable()}),
        );
        core.request("pane.send_text", json!({"pane_id":survivor_pane,"text":"printf '%s %s\\n' \"$$\" \"$close_local\" > close-after.state\n"}));
        assert_eq!(
            wait_for_file(&core.home.join("close-after.state")),
            before,
            "{close_method} changed the surviving shell"
        );
        assert_eq!(
            process_identity(pid),
            identity,
            "{close_method} replaced the surviving process/PTY"
        );
        if close_method == "pane.close" {
            core.request("pane.send_text", json!({"pane_id":second,"text":"printf 'retained-alias\\n' > close-retained.state\n"}));
            assert_eq!(
                wait_for_file(&core.home.join("close-retained.state")),
                "retained-alias\n"
            );
        }
    }
}

#[test]
fn ordinary_restart_restores_transferred_tab_placement_layout_labels_and_focus() {
    let mut core = Core::new();
    let directories = [core.home.join("build left"), core.home.join("build right")];
    for directory in &directories {
        std::fs::create_dir_all(directory).unwrap();
    }
    let tools = core.request(
        "workspace.create",
        json!({"label":"Tools","cwd":directories[0]}),
    );
    let first = tools["root_pane"]["pane_id"].as_str().unwrap();
    core.request(
        "tab.rename",
        json!({"tab_id":tools["tab"]["tab_id"],"label":"Build"}),
    );
    let split = core.request("pane.split",json!({"target_pane_id":first,"direction":"right","ratio":0.37,"cwd":directories[1],"focus":true}));
    let second = split["pane"]["pane_id"].as_str().unwrap();
    for (pane, label) in [(first, "Compiler"), (second, "Tests")] {
        core.request("pane.rename", json!({"pane_id":pane,"label":label}));
    }
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    let workspace = main["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    core.request(
        "tab.rename",
        json!({"tab_id":main["tab"]["tab_id"],"label":"One"}),
    );
    core.request(
        "tab.create",
        json!({"workspace_id":workspace,"label":"Two","cwd":core.home}),
    );
    let moved = cli(
        &core,
        &[
            "tab",
            "move",
            tools["tab"]["tab_id"].as_str().unwrap(),
            "--workspace",
            &workspace,
            "--index",
            "1",
        ],
    );
    assert!(
        moved.status.success(),
        "CLI move failed: {}",
        String::from_utf8_lossy(&moved.stderr)
    );
    let transfer: Value = serde_json::from_slice(&moved.stdout).unwrap();
    let panes = [first, second].map(|old| {
        transfer["result"]["transfer"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|mapping| mapping["previous_pane_id"] == old)
            .unwrap()["pane_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    core.request("pane.focus", json!({"pane_id":panes[1]}));
    core.request("pane.zoom", json!({"pane_id":panes[1],"mode":"on"}));
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(panes[1].as_str()) {
        client.receive().unwrap();
    }
    let boot = client.boot_id.clone();
    let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
    let expected_layout =
        core.request("pane.layout", json!({"pane_id":panes[1]}))["layout"].clone();
    let pane_placement = |core: &Core| {
        panes
            .iter()
            .map(|pane| {
                let info = core.request("pane.get", json!({"pane_id":pane}))["pane"].clone();
                json!([
                    info["pane_id"],
                    info["workspace_id"],
                    info["tab_id"],
                    info["label"],
                    info["cwd"]
                ])
            })
            .collect::<Vec<_>>()
    };
    let expected_panes = pane_placement(&core);
    let original_shells = panes
        .iter()
        .map(|pane| {
            core.request("pane.process_info", json!({"pane_id":pane}))["process_info"]["shell_pid"]
                .as_u64()
                .unwrap()
        })
        .collect::<Vec<_>>();
    drop(client);
    let saved = core.restart();
    let persisted: Value = serde_json::from_slice(&saved).unwrap();
    assert!(
        !persisted["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|space| space["label"] == "Tools"),
        "saved session retained removed Tools"
    );
    let mut restored_client = core.attach();
    while restored_client.focused_pane_id.as_deref() != Some(panes[1].as_str()) {
        restored_client.receive().unwrap();
    }
    assert_ne!(
        restored_client.boot_id, boot,
        "ordinary restart retained the old Core"
    );
    let restored = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert_eq!(restored["focused_pane_id"], panes[1]);
    let spaces = |snapshot: &Value| {
        snapshot["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|space| {
                json!([
                    space["workspace_id"],
                    space["label"],
                    space["focused"],
                    space["active_tab_id"],
                    space["tab_count"],
                    space["pane_count"]
                ])
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        spaces(&restored),
        spaces(&before),
        "saved space order or remembered selection changed"
    );
    let tabs = core.request("tab.list", json!({"workspace_id":workspace}));
    assert_eq!(
        tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["One", "Build", "Two"]
    );
    assert!(tabs["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tab| tab["label"] == "Build")
        .unwrap()["focused"]
        .as_bool()
        .unwrap());
    assert!(
        !restored["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|space| space["label"] == "Tools"),
        "restore recreated removed Tools"
    );
    assert_eq!(
        pane_placement(&core),
        expected_panes,
        "restored moved pane labels/CWDs/membership changed"
    );
    assert_eq!(
        core.request("pane.layout", json!({"pane_id":panes[1]}))["layout"],
        expected_layout,
        "restored split geometry/zoom/focus changed"
    );
    for (index, pane) in panes.iter().enumerate() {
        let pid = core.request("pane.process_info", json!({"pane_id":pane}))["process_info"]
            ["shell_pid"]
            .as_u64()
            .unwrap();
        assert!(
            !original_shells.contains(&pid),
            "cold restore claimed old shell process survival"
        );
        assert_eq!(
            std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap(),
            directories[index]
        );
        core.request("pane.send_text",json!({"pane_id":pane,"text":format!("pwd > '{}/cold-cwd-{index}'; printf '%s%s\\n' 'RESTORED-BUILD-' '{index}'\n",core.home.display())}));
        assert_eq!(
            wait_for_file(&core.home.join(format!("cold-cwd-{index}"))).trim(),
            directories[index].to_str().unwrap()
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while restored_client
        .primary
        .as_ref()
        .is_none_or(|surface| !surface.text().contains("RESTORED-BUILD-1"))
    {
        assert!(
            Instant::now() < deadline,
            "restored Client did not display saved Tests pane"
        );
        restored_client.receive().unwrap();
    }
    assert_eq!(
        restored_client.focused_pane_id.as_deref(),
        Some(panes[1].as_str())
    );
}

#[test]
#[ignore = "requires an explicitly supplied alias-blind legacy Core executable"]
fn alias_blind_importer_is_rejected_before_public_socket_or_owner_changes() {
    use std::os::unix::fs::MetadataExt;

    let legacy = PathBuf::from(
        std::env::var_os("TRAILBOSS_TEST_LEGACY_CORE")
            .expect("set TRAILBOSS_TEST_LEGACY_CORE to an alias-blind legacy Core executable"),
    );
    assert!(
        legacy.is_absolute(),
        "legacy executable must be an explicit absolute path"
    );
    let mut core = Core::new();
    let _stop_owner = StopOwner(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    );
    let probe_home = core.home.join("legacy-probe");
    std::fs::create_dir_all(&probe_home).unwrap();
    let probe =
        Core::launch_with_executable(probe_home, format!("{}-legacy", core.name), legacy.clone());
    let source_ping = core.request("ping", json!({}));
    let legacy_ping = probe.request("ping", json!({}));
    assert_eq!(
        source_ping["version"], legacy_ping["version"],
        "fixture must advertise the same version"
    );
    assert_eq!(
        source_ping["protocol"], legacy_ping["protocol"],
        "fixture must advertise the same protocol"
    );
    drop(probe);
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let inherited = tools["root_pane"]["pane_id"].as_str().unwrap().to_owned();
    let terminal = tools["root_pane"]["terminal_id"].clone();
    core.request("pane.send_text", json!({"pane_id":inherited,"text":"rollback_local=retained; printf '%s %s %s\\n' \"$$\" \"$rollback_local\" \"$HERDR_PANE_ID\" > rollback-before.state\n"}));
    let initial_state = wait_for_file(&core.home.join("rollback-before.state"));
    let shell = initial_state.split_whitespace().next().unwrap();
    let identity = process_identity(shell);
    let main = core.request("workspace.create", json!({"label":"Main","cwd":core.home}));
    core.request("pane.focus", json!({"pane_id":inherited}));
    let moved = cli(
        &core,
        &[
            "tab",
            "move",
            tools["tab"]["tab_id"].as_str().unwrap(),
            "--workspace",
            main["workspace"]["workspace_id"].as_str().unwrap(),
        ],
    );
    assert!(
        moved.status.success(),
        "transfer failed: {}",
        String::from_utf8_lossy(&moved.stderr)
    );
    let transfer: Value = serde_json::from_slice(&moved.stdout).unwrap();
    let current = transfer["result"]["transfer"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        core.request("pane.get", json!({"pane_id":inherited}))["pane"]["terminal_id"],
        terminal
    );
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(current) {
        client.receive().unwrap();
    }
    let boot = client.boot_id.clone();
    let socket = core
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
    let client_socket = core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    ));
    let socket_inodes = [
        std::fs::metadata(&socket).unwrap().ino(),
        std::fs::metadata(&client_socket).unwrap().ino(),
    ];
    let topology = core.request("session.snapshot", json!({}))["snapshot"].clone();
    let source_pid = core.child.id();
    let mut wire = BufReader::new(UnixStream::connect(&socket).unwrap());
    wire.get_mut()
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    serde_json::to_writer(wire.get_mut(), &json!({"id":"alias-blind-import","method":"server.live_handoff","params":{"import_exe":legacy}})).unwrap();
    wire.get_mut().write_all(b"\n").unwrap();
    let mut line = String::new();
    wire.read_line(&mut line).unwrap();
    let rejected: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(rejected["id"], "alias-blind-import");
    assert_eq!(
        rejected["error"]["code"], "handoff_failed",
        "alias-blind import was not rejected: {rejected}"
    );
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("did not confirm restored public pane aliases"),
        "rejected for the wrong reason: {rejected}"
    );
    assert!(
        rejected.get("result").is_none(),
        "rejected import reported success"
    );
    assert_eq!(core.child.id(), source_pid);
    assert!(
        core.child.try_wait().unwrap().is_none(),
        "source Core exited after precommit rejection"
    );
    assert_eq!(
        [
            std::fs::metadata(&socket).unwrap().ino(),
            std::fs::metadata(&client_socket).unwrap().ino()
        ],
        socket_inodes,
        "precommit rejection replaced public sockets"
    );
    // Handoff intentionally disconnects clients before preparing the importer.
    // Reattach to prove rollback still serves the original Core boot.
    drop(client);
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(current) {
        client.receive().unwrap();
    }
    assert_eq!(client.boot_id, boot);
    let restored = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert_eq!(restored["focused_pane_id"], topology["focused_pane_id"]);
    for key in ["workspaces", "tabs"] {
        assert_eq!(restored[key], topology[key], "rollback changed {key}");
    }
    assert_eq!(
        core.request("pane.get", json!({"pane_id":inherited}))["pane"]["terminal_id"],
        terminal
    );
    assert_eq!(process_identity(shell), identity);
    core.request("pane.send_text",json!({"pane_id":inherited,"text":format!(
        "printf '%s %s %s\\n' \"$$\" \"$rollback_local\" \"$HERDR_PANE_ID\" > rollback-after.state; '{}' pane current --current > rollback-current.json 2> rollback-current.err; printf '%s\\n' \"$?\" > rollback-current.done; printf '%s%s\\n' 'ROLLBACK-' 'READER-RESUMED'\n",
        support::executable().display()
    )}));
    assert_eq!(
        wait_for_file(&core.home.join("rollback-after.state")),
        initial_state
    );
    assert_eq!(
        wait_for_file(&core.home.join("rollback-current.done")).trim(),
        "0",
        "inherited CLI failed after rollback: {}",
        std::fs::read_to_string(core.home.join("rollback-current.err")).unwrap()
    );
    let inherited_result: Value =
        serde_json::from_str(&wait_for_file(&core.home.join("rollback-current.json"))).unwrap();
    assert_eq!(inherited_result["result"]["pane"]["pane_id"], current);
    let deadline = Instant::now() + Duration::from_secs(5);
    while client
        .primary
        .as_ref()
        .is_none_or(|surface| !surface.text().contains("ROLLBACK-READER-RESUMED"))
    {
        assert!(
            Instant::now() < deadline,
            "source terminal reader did not resume after rollback"
        );
        client.receive().unwrap();
    }
    assert_eq!(client.boot_id, boot);
    assert_eq!(client.focused_pane_id.as_deref(), Some(current));
}

#[test]
#[ignore = "requires an explicitly supplied alias-free legacy Core executable"]
fn alias_free_legacy_source_hands_off_through_plain_restored_handshake() {
    let legacy = PathBuf::from(
        std::env::var_os("TRAILBOSS_TEST_LEGACY_CORE")
            .expect("set TRAILBOSS_TEST_LEGACY_CORE to an alias-free legacy Core executable"),
    );
    assert!(
        legacy.is_absolute(),
        "legacy executable must be an explicit absolute path"
    );
    let name = format!(
        "legacy-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let home = std::env::temp_dir().join(&name);
    std::fs::create_dir_all(&home).unwrap();
    let mut core = Core::launch_with_executable(home, name, legacy);
    let socket = core
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
    let _stop_owner = StopOwner(socket.clone());
    // No tab transfer occurs: this actual legacy source exports its original,
    // alias-free runtime manifest and accepts only the plain restored receipt.
    let tools = core.request("workspace.create", json!({"label":"Tools","cwd":core.home}));
    let pane = tools["root_pane"]["pane_id"].as_str().unwrap().to_owned();
    core.request("pane.focus", json!({"pane_id":pane}));
    core.request("pane.send_text", json!({"pane_id":pane,"text":"legacy_local=retained; printf '%s %s %s\\n' \"$$\" \"$legacy_local\" \"$HERDR_PANE_ID\" > legacy-before.state\n"}));
    let initial_state = wait_for_file(&core.home.join("legacy-before.state"));
    let shell = initial_state.split_whitespace().next().unwrap();
    let identity = process_identity(shell);
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(pane.as_str()) {
        client.receive().unwrap();
    }
    let old_boot = client.boot_id.clone();
    let mut wire = BufReader::new(UnixStream::connect(&socket).unwrap());
    wire.get_mut()
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    serde_json::to_writer(wire.get_mut(), &json!({"id":"alias-free-legacy-import","method":"server.live_handoff","params":{"import_exe":support::executable()}})).unwrap();
    wire.get_mut().write_all(b"\n").unwrap();
    let mut line = String::new();
    wire.read_line(&mut line).unwrap();
    let imported: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(imported["id"], "alias-free-legacy-import");
    assert!(
        imported.get("error").is_none() && imported.get("result").is_some(),
        "alias-free legacy handoff failed: {imported}"
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = core.child.try_wait().unwrap() {
            assert!(
                status.success(),
                "legacy source exited unsuccessfully: {status}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "legacy source did not relinquish ownership"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(client);
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(pane.as_str()) {
        client.receive().unwrap();
    }
    assert_ne!(
        client.boot_id, old_boot,
        "client reattached to the legacy boot"
    );
    let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert_eq!(snapshot["focused_pane_id"], pane);
    let current = core.request("pane.get", json!({"pane_id":pane}))["pane"].clone();
    let terminal = current["terminal_id"].clone();
    assert!(
        terminal.as_str().is_some_and(|id| !id.is_empty()),
        "imported pane has no current terminal: {current}"
    );
    assert_eq!(process_identity(shell), identity);
    core.request("pane.send_text",json!({"pane_id":pane,"text":format!(
        "printf '%s %s %s\\n' \"$$\" \"$legacy_local\" \"$HERDR_PANE_ID\" > legacy-after.state; '{}' pane current --current > legacy-current.json 2> legacy-current.err; printf '%s\\n' \"$?\" > legacy-current.done; printf '%s%s\\n' 'LEGACY-' 'IMPORT-USABLE'\n",
        support::executable().display()
    )}));
    assert_eq!(
        wait_for_file(&core.home.join("legacy-after.state")),
        initial_state,
        "legacy shell PID/local/inherited context changed"
    );
    assert_eq!(
        wait_for_file(&core.home.join("legacy-current.done")).trim(),
        "0",
        "inherited CLI failed after legacy import: {}",
        std::fs::read_to_string(core.home.join("legacy-current.err")).unwrap()
    );
    let inherited: Value =
        serde_json::from_str(&wait_for_file(&core.home.join("legacy-current.json"))).unwrap();
    assert_eq!(inherited["result"]["pane"]["pane_id"], pane);
    assert_eq!(inherited["result"]["pane"]["terminal_id"], terminal);
    assert_eq!(process_identity(shell), identity);
    let deadline = Instant::now() + Duration::from_secs(5);
    while client
        .primary
        .as_ref()
        .is_none_or(|surface| !surface.text().contains("LEGACY-IMPORT-USABLE"))
    {
        assert!(
            Instant::now() < deadline,
            "imported terminal reader is not usable"
        );
        client.receive().unwrap();
    }
    assert_eq!(client.focused_pane_id.as_deref(), Some(pane.as_str()));
}
