mod support;

use serde_json::{json, Value};
use std::{
    process::Command,
    time::{Duration, Instant},
};
use trailboss::core::{Core as NativeCore, Effect};
use trailboss_runtime::protocol::widgets::Context;

#[test]
fn c01_neutral_consumer_uses_actual_projection_and_invokes_true_owners_once() {
    consume_command_catalog(CatalogChange::None);
}

#[test]
fn c02_manifest_action_addition_publishes_without_registration_or_consumer_changes() {
    consume_command_catalog(CatalogChange::AddAction);
}

#[test]
fn c04_manifest_title_change_updates_projection_without_changing_identity_or_owner() {
    consume_command_catalog(CatalogChange::RenameAction);
}

enum CatalogChange {
    None,
    AddAction,
    RenameAction,
}

fn consume_command_catalog(change: CatalogChange) {
    let core = support::Core::new();
    let review = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let workspace = review["workspace"]["workspace_id"].as_str().unwrap();
    let original = review["tab"]["tab_id"].as_str().unwrap();
    let target = core.request(
        "tab.create",
        json!({"workspace_id":workspace,"label":"Target","cwd":core.home,"focus":false}),
    );
    let tab = target["tab"]["tab_id"].as_str().unwrap();
    let pane = target["root_pane"]["pane_id"].as_str().unwrap();
    let context = json!({"workspace_id":workspace,"tab_id":tab,"pane_id":pane});
    let owner = core.home.join("demo-owner");
    let consumer = core.home.join("demo-consumer");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::create_dir_all(&consumer).unwrap();
    let recorded = owner.join("recorded.jsonl");
    let wrong_owner = consumer.join("invocations");
    let owner_manifest = "id = \"demo.owner\"\nname = \"Demo owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n";
    let declared_action = "[[actions]]\nid = \"record-target\"\ntitle = \"Record target\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n";
    let complete_manifest = format!("{owner_manifest}{declared_action}");
    std::fs::write(
        owner.join("trailboss-plugin.toml"),
        if matches!(change, CatalogChange::AddAction) {
            owner_manifest
        } else {
            &complete_manifest
        },
    )
    .unwrap();
    std::fs::write(owner.join("record.py"), format!("import json, os\nwith open({:?}, 'a') as output:\n output.write(json.dumps({{'owner':os.environ['HERDR_PLUGIN_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])}})+'\\n')\n", recorded.to_str().unwrap())).unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    if matches!(change, CatalogChange::AddAction) {
        let key = json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"});
        let before = core.request("ui.command.list", json!({}));
        assert!(!before["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["key"] == key));
        // Only the ordinary manifest changes. No link, enable, registration,
        // consumer rewrite or action-specific publication call follows it.
        std::fs::write(owner.join("trailboss-plugin.toml"), &complete_manifest).unwrap();
        let refreshed = core.request("ui.command.list", json!({}));
        let published: Vec<_> = refreshed["commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|command| command["key"] == key)
            .collect();
        assert_eq!(
            published.len(),
            1,
            "ordinary manifest addition must publish automatically"
        );
        assert_eq!(published[0]["title"], "Record target");
        assert!(published[0]["fragment_name"]
            .as_str()
            .is_some_and(|name| !name.is_empty()));
    }
    let title_transition = if matches!(change, CatalogChange::RenameAction) {
        let before = core.request("ui.command.list", json!({}));
        let key = json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"});
        let command = before["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == key)
            .expect("original action must be published")
            .clone();
        let mut client = core.attach();
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.boot_id.is_empty() || client.declarations.is_none() {
            assert!(
                Instant::now() < deadline,
                "original owner projection missing"
            );
            client.receive().unwrap();
        }
        let native = NativeCore::default();
        let (provider, tree) = native
            .fragment(
                client.declarations.as_ref().unwrap(),
                command["fragment_name"].as_str().unwrap(),
            )
            .expect("original locator must resolve before title change");
        assert_eq!(provider, "demo.owner");
        let original_tree = serde_json::to_value(tree).unwrap();
        assert_eq!(original_tree["children"][0]["text"], command["title"]);
        let renamed = complete_manifest.replace(
            "title = \"Record target\"",
            "title = \"Capture review target\"",
        );
        std::fs::write(owner.join("trailboss-plugin.toml"), renamed).unwrap();
        let after = core.request("ui.command.list", json!({}));
        let changed = after["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["key"] == key)
            .expect("title change must not withdraw the action");
        assert_eq!(after["boot_id"], before["boot_id"]);
        assert_ne!(
            changed["title"], command["title"],
            "discovery must reflect authoritative title edits"
        );
        assert_eq!(changed["title"], "Capture review target");
        assert_eq!(
            changed["fragment_name"], command["fragment_name"],
            "presentation edits must retain the within-boot locator"
        );
        Some(command)
    } else {
        None
    };
    let driver = format!(
        "{}/tests/support/command_consumer.py",
        env!("CARGO_MANIFEST_DIR")
    );
    let config = consumer.join("config.json");
    std::fs::write(
        &config,
        json!({"context":context,"consumer_invocations":wrong_owner}).to_string(),
    )
    .unwrap();
    // The neutral producer declares no actions and obtains its Uses from discovery,
    // rather than copying foreign action IDs into its own manifest or declaration.
    std::fs::write(consumer.join("trailboss-plugin.toml"), format!("id = \"demo.consumer\"\nname = \"Neutral consumer\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"commands\"\nsurface = \"sidebar\"\ncommand = [\"python3\", {driver:?}, \"produce\", {:?}]\nstream = true\n", config.to_str().unwrap())).unwrap();
    core.request("plugin.link", json!({"path":consumer,"enabled":true}));
    let api = core
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
    let endpoint = core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    ));
    let evidence = core.home.join("discovery-wire.json");
    let output = Command::new("python3")
        .arg(driver)
        .arg("probe")
        .arg(config)
        .arg(api)
        .arg(endpoint)
        .arg(&evidence)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "independent C01 consumer: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let observed: Value = serde_json::from_slice(&std::fs::read(evidence).unwrap()).unwrap();
    if let Some(original_command) = title_transition {
        let changed = observed["catalog"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == original_command["key"])
            .unwrap();
        assert_eq!(changed["fragment_name"], original_command["fragment_name"]);
        assert_eq!(changed["title"], "Capture review target");
        let fragment = observed["declarations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["plugin_id"] == "demo.owner")
            .flat_map(|item| {
                item["declaration"]["fragments"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .find(|fragment| fragment["name"] == original_command["fragment_name"])
            .expect("renamed fragment must be in actual owner projection");
        assert_eq!(fragment["tree"]["children"][0]["text"], changed["title"]);
        assert_eq!(
            fragment["tree"]["action"],
            original_command["key"]["action_id"]
        );
    }
    let selections = observed["selections"].as_array().unwrap();
    assert_eq!(selections.len(), 2);
    let captured: Context = serde_json::from_value(selections[0]["context"].clone()).unwrap();
    let mut native = NativeCore::default();
    native.update_session(Some(workspace.to_owned()), Some(original.to_owned()), true);
    let Effect::Request { method, params } = native.invoke(
        selections[0]["action_id"].as_str().unwrap(),
        &captured,
        None,
    ) else {
        panic!("selected generated Core action did not reach native dispatch")
    };
    core.request(method, params);
    let snapshot = core.request("session.snapshot", json!({}));
    assert!(snapshot["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["tab_id"] == tab && item["focused"] == true));
    let mut invocation = selections[1]["context"].clone();
    invocation["invocation_source"] = json!("plugin_ui");
    core.request("plugin.action.invoke", json!({"plugin_id":selections[1]["plugin_id"],"action_id":selections[1]["action_id"],"context":invocation}));
    let deadline = Instant::now() + Duration::from_secs(5);
    let records = loop {
        if let Ok(text) = std::fs::read_to_string(&recorded) {
            if !text.is_empty() {
                break text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "real owner action did not record captured target"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let records: Vec<Value> = records
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records.len(),
        1,
        "one selection must produce exactly one owner operation"
    );
    assert_eq!(records[0]["owner"], "demo.owner");
    for key in ["workspace_id", "tab_id", "pane_id"] {
        assert_eq!(records[0]["context"][key], context[key]);
    }
    assert!(
        !wrong_owner.exists(),
        "consumer was incorrectly invoked as the action owner"
    );
}

#[test]
fn c03_core_inventory_resolves_to_native_actions_without_controls_or_rpc_commands() {
    use trailboss_runtime::protocol::widgets::Node;
    let core = support::Core::new();
    let catalog = core.request("ui.command.list", json!({}));
    let commands = catalog["commands"].as_array().unwrap();
    // Source-independent user inventory: intentionally not copied from CORE_COMMANDS
    // at runtime, and not inferred from API methods or reserved parser membership.
    let inventory = [
        ("host:new-space", None, "workspace.create"),
        ("host:new", None, "tab.create"),
        ("host:new-tab", None, "tab.create"),
        ("host:rename-space", None, "workspace.rename"),
        ("host:rename-tab", None, "tab.rename"),
        ("host:close-tab", None, "tab.close"),
        ("host:focus", Some("targeted"), "tab.focus"),
        ("host:focus", Some("previous"), ""),
        ("host:restart-idle-agents", None, "agent.restart_idle"),
        ("host:detach", None, ""),
    ];
    assert_eq!(
        commands.len(),
        inventory.len(),
        "only the independent user inventory belongs in the empty-plugin Core catalog"
    );
    for excluded in [
        "host:create",
        "host:cancel",
        "workspace.list",
        "agent.read",
        "pane.send_text",
        "terminal.input",
        "tab.cycle",
    ] {
        assert!(
            !commands
                .iter()
                .any(|command| command["key"]["action_id"] == excluded),
            "{excluded} is not a standalone user command"
        );
    }
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.boot_id.is_empty() || client.declarations.is_none() {
        assert!(
            Instant::now() < deadline,
            "actual Core projection was not received"
        );
        client.receive().unwrap();
    }
    assert_eq!(catalog["boot_id"], client.boot_id);
    let declarations = client.declarations.as_ref().unwrap();
    fn count_action(node: &Node, wanted: &str) -> usize {
        match node {
            Node::Row {
                action,
                children,
                menu,
                ..
            } => {
                usize::from(action.as_deref() == Some(wanted))
                    + children
                        .iter()
                        .map(|child| count_action(child, wanted))
                        .sum::<usize>()
                    + menu.as_deref().map_or(0, |menu| count_action(menu, wanted))
            }
            Node::Column { children } => children
                .iter()
                .map(|child| count_action(child, wanted))
                .sum(),
            _ => 0,
        }
    }
    for (action, mode, method) in inventory {
        let mut key = json!({"kind":"core","action_id":action});
        if let Some(mode) = mode {
            key["mode"] = json!(mode);
        }
        let matching: Vec<_> = commands
            .iter()
            .filter(|command| command["key"] == key)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "missing or duplicated authoritative mode: {key}"
        );
        let command = matching[0];
        let name = command["fragment_name"].as_str().unwrap();
        let mut native = NativeCore::default();
        let (owner, tree) = native
            .fragment(declarations, name)
            .expect("advertised Core locator must resolve in actual projection");
        assert_eq!(owner, "trailboss.core");
        assert_eq!(
            count_action(tree, action),
            1,
            "published fragment must retain its actual native host action"
        );
        let mut labels = Vec::new();
        for (_, declaration) in declarations {
            if declaration
                .fragments
                .iter()
                .any(|fragment| fragment.name == name)
            {
                assert!(
                    declaration.validate(|_| false),
                    "Core exports must validate without plugin action handlers"
                );
                labels.push(declaration);
            }
        }
        assert_eq!(labels.len(), 1);
        let context = if mode == Some("previous") || action == "host:detach" {
            Context::default()
        } else {
            serde_json::from_value(json!({"workspace_id":"captured-space","tab_id":"captured-tab","pane_id":"captured-pane"})).unwrap()
        };
        native.update_session(
            Some("native-space".to_owned()),
            Some("native-tab".to_owned()),
            true,
        );
        let effect = if matches!(
            action,
            "host:new-space"
                | "host:new"
                | "host:new-tab"
                | "host:rename-space"
                | "host:rename-tab"
        ) {
            assert!(matches!(
                native.prepare_palette(action, &context),
                Effect::Consumed
            ));
            assert!(matches!(
                native.text("C03 entered name"),
                Some(Effect::Consumed)
            ));
            native
                .text("\r")
                .expect("published name action must submit through the native owner")
        } else {
            native.invoke(action, &context, None)
        };
        if method.is_empty() {
            assert!(
                matches!(effect, Effect::Consumed),
                "client-local action must not become a plugin invocation"
            );
            assert_eq!(native.detach_requested, action == "host:detach");
        } else {
            let Effect::Request { method: actual, .. } = effect else {
                panic!("{action} did not reach existing native request dispatch")
            };
            assert_eq!(actual, method);
        }
    }
}

#[test]
fn c05_withdrawn_action_or_owner_has_no_executable_locator_or_fallback_owner() {
    assert_unavailable_command_rows(&[
        ("action", "plugin_action_not_found"),
        ("owner", "plugin_not_found"),
        ("manifest", "plugin_manifest_unavailable"),
    ]);
}

#[test]
fn c06_disabled_or_unsupported_action_is_hidden_and_stale_invocation_is_rejected() {
    assert_unavailable_command_rows(&[
        ("disabled", "plugin_disabled"),
        ("platform", "platform_unsupported"),
    ]);
}

fn assert_unavailable_command_rows(cases: &[(&str, &str)]) {
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    for &(withdrawal, expected_error) in cases {
        let core = support::Core::new();
        let owner = core.home.join("owner");
        let other = core.home.join("other");
        for (path, id) in [(&owner, "demo.owner"), (&other, "demo.other")] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("trailboss-plugin.toml"), format!(
                "id = {id:?}\nname = \"Withdrawal fixture\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"record-target\"\ntitle = \"Record target\"\ncommand = [\"python3\", \"action.py\"]\n"
            )).unwrap();
            std::fs::write(
                path.join("action.py"),
                "with open('executed', 'a') as output:\n output.write('executed\\n')\n",
            )
            .unwrap();
            core.request("plugin.link", json!({"path":path,"enabled":true}));
            // Positive control: both real handlers must be capable of producing
            // their observable effect before testing the stale negative route.
            core.request(
                "plugin.action.invoke",
                json!({"plugin_id":id,"action_id":"record-target"}),
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while std::fs::read_to_string(path.join("executed"))
                .ok()
                .as_deref()
                != Some("executed\n")
            {
                assert!(
                    Instant::now() < deadline,
                    "positive owner operation failed for {id}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            std::fs::remove_file(path.join("executed")).unwrap();
        }
        let key = json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"});
        let initial = core.request("ui.command.list", json!({}));
        let issued = initial["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == key)
            .unwrap();
        let locator = issued["fragment_name"].as_str().unwrap();
        let consumer = core.home.join("consumer");
        std::fs::create_dir_all(&consumer).unwrap();
        std::fs::write(consumer.join("trailboss-plugin.toml"), "id = \"demo.consumer\"\nname = \"Withdrawal consumer\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"stale-use\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"producer.py\"]\nstream = true\n").unwrap();
        let declaration = json!({"herdr":"widget_tree","version":1,"zones":[{"zone":"list","tree":{"type":"use","name":locator}}]});
        std::fs::write(consumer.join("producer.py"), format!(
            "import sys\nprint({:?}, flush=True)\nfor line in sys.stdin:\n with open('executed', 'a') as output:\n  output.write(line)\n", declaration.to_string()
        )).unwrap();
        core.request("plugin.link", json!({"path":consumer,"enabled":true}));
        let native = NativeCore::default();
        let mut before = core.attach();
        let deadline = Instant::now() + Duration::from_secs(5);
        while before.declarations.as_ref().is_none_or(|declarations| {
            !declarations
                .iter()
                .any(|(owner, _)| owner == "demo.consumer")
        }) {
            assert!(
                Instant::now() < deadline,
                "consumer's actual Use projection missing"
            );
            before.receive().unwrap();
        }
        assert_eq!(
            native
                .fragment(before.declarations.as_ref().unwrap(), locator)
                .unwrap()
                .0,
            "demo.owner"
        );
        match withdrawal {
            "action" => {
                std::fs::write(owner.join("trailboss-plugin.toml"), "id = \"demo.owner\"\nname = \"Withdrawal fixture\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n").unwrap();
            }
            "owner" => {
                core.request("plugin.unlink", json!({"plugin_id":"demo.owner"}));
            }
            "manifest" => {
                std::fs::remove_file(owner.join("trailboss-plugin.toml")).unwrap();
            }
            "disabled" => {
                core.request("plugin.disable", json!({"plugin_id":"demo.owner"}));
            }
            "platform" => {
                let unsupported = if cfg!(target_os = "windows") {
                    "linux"
                } else {
                    "windows"
                };
                let manifest =
                    std::fs::read_to_string(owner.join("trailboss-plugin.toml")).unwrap();
                std::fs::write(
                    owner.join("trailboss-plugin.toml"),
                    format!("{manifest}\nplatforms = [{unsupported:?}]\n"),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let current = core.request("ui.command.list", json!({}));
        assert!(
            !current["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|command| command["key"] == key),
            "{withdrawal}: removed action must leave the current catalog"
        );
        assert!(current["commands"].as_array().unwrap().iter().any(|command|
            command["key"] == json!({"kind":"plugin","plugin_id":"demo.other","action_id":"record-target"})),
            "{withdrawal}: same-title other owner must remain available");
        if matches!(withdrawal, "disabled" | "platform") {
            let inventory = core.request("plugin.action.list", json!({"plugin_id":"demo.owner"}));
            let actions = inventory["actions"].as_array().unwrap();
            assert_eq!(
                actions.len(),
                1,
                "legacy inventory must retain declared unavailable actions"
            );
            assert_eq!(actions[0]["plugin_id"], "demo.owner");
            assert_eq!(actions[0]["action_id"], "record-target");
            if withdrawal == "platform" {
                assert!(!actions[0]["platforms"].as_array().unwrap().is_empty());
            }
        }
        let mut after = core.attach();
        let deadline = Instant::now() + Duration::from_secs(5);
        while after.declarations.as_ref().is_none_or(|declarations| {
            !declarations
                .iter()
                .any(|(owner, _)| owner == "demo.consumer")
        }) {
            assert!(
                Instant::now() < deadline,
                "current actual consumer projection missing"
            );
            after.receive().unwrap();
        }
        assert!(
            native
                .fragment(after.declarations.as_ref().unwrap(), locator)
                .is_none(),
            "{withdrawal}: stale ordinary Use must not resolve to another command or owner"
        );
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(
            core.home
                .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
        ));
        let request: Request = serde_json::from_value(json!({"id":"stale-command",
            "method":"plugin.action.invoke","params":{"plugin_id":"demo.owner","action_id":"record-target",
            "context":{"invocation_source":"plugin_ui"}}})).unwrap();
        let reply = api
            .request_value_with_timeout(&request, Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            reply["error"]["code"], expected_error,
            "{withdrawal}: stale owner invocation must be rejected"
        );
        for path in [&owner, &other, &consumer] {
            assert!(
                !path.join("executed").exists(),
                "{withdrawal}: no owner or consumer may execute as fallback"
            );
        }
    }
}

#[test]
fn c07_duplicate_titles_and_dotted_names_invoke_only_the_selected_structured_owner() {
    let identities = [("demo.alpha", "run"), ("demo", "alpha.run")];
    let keys: Vec<_> = identities.iter().map(|(plugin_id, action_id)|
        json!({"kind":"plugin","plugin_id":plugin_id,"action_id":action_id})).collect();
    for selected_index in 0..identities.len() {
        let core = support::Core::new();
        let created = core.request(
            "workspace.create",
            json!({"label":"Review","cwd":core.home,"focus":true}),
        );
        let context = json!({
            "workspace_id":created["workspace"]["workspace_id"],
            "tab_id":created["tab"]["tab_id"],
            "pane_id":created["root_pane"]["pane_id"],
        });
        let mut outputs = Vec::new();
        for (index, (plugin_id, action_id)) in identities.iter().enumerate() {
            let plugin = core.home.join(format!("dotted-owner-{index}"));
            std::fs::create_dir_all(&plugin).unwrap();
            std::fs::write(plugin.join("trailboss-plugin.toml"), format!(
                "id = {plugin_id:?}\nname = \"Dotted owner fixture\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = {action_id:?}\ntitle = \"Run\"\ncommand = [\"python3\", \"record.py\"]\n"
            )).unwrap();
            std::fs::write(plugin.join("record.py"), format!(
                "import json, os\nwith open('executed.jsonl', 'a') as output:\n output.write(json.dumps({{'owner':os.environ['HERDR_PLUGIN_ID'],'action':{action_id:?},'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])}})+'\\n')\n"
            )).unwrap();
            core.request("plugin.link", json!({"path":plugin,"enabled":true}));
            outputs.push(plugin.join("executed.jsonl"));
        }
        let catalog = core.request("ui.command.list", json!({}));
        let commands: Vec<_> = keys
            .iter()
            .map(|key| {
                catalog["commands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|command| command["key"] == *key)
                    .unwrap()
            })
            .collect();
        assert_ne!(commands[0]["key"], commands[1]["key"]);
        assert_eq!(
            commands[0]["title"], commands[1]["title"],
            "fixture must exercise duplicate display titles"
        );
        assert_ne!(
            commands[0]["fragment_name"], commands[1]["fragment_name"],
            "ambiguous dotted qualified strings must not collapse structured locators"
        );
        let observed = probe_command_uses(&core, &context, &keys);
        let selection = &observed["selections"][selected_index];
        assert_eq!(selection["plugin_id"], identities[selected_index].0);
        assert_eq!(selection["action_id"], identities[selected_index].1);
        let mut captured = selection["context"].clone();
        captured["invocation_source"] = json!("plugin_ui");
        let invoked = core.request("plugin.action.invoke", json!({
            "plugin_id":selection["plugin_id"],"action_id":selection["action_id"],"context":captured,
        }));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let logs = core.request(
                "plugin.log.list",
                json!({"plugin_id":identities[selected_index].0}),
            );
            if logs["logs"].as_array().unwrap().iter().any(|log| {
                log["log_id"] == invoked["log"]["log_id"] && log["status"] == "succeeded"
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "selected real owner did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let text = std::fs::read_to_string(&outputs[selected_index]).unwrap();
        let records: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["owner"], identities[selected_index].0);
        assert_eq!(records[0]["action"], identities[selected_index].1);
        for field in ["workspace_id", "tab_id", "pane_id"] {
            assert_eq!(records[0]["context"][field], context[field]);
        }
        assert!(
            !outputs[1 - selected_index].exists(),
            "display-identical unselected owner executed"
        );
        assert!(
            !core.home.join("consumer/invocations").exists(),
            "neutral consumer executed as owner"
        );
    }
}

fn probe_command_uses(core: &support::Core, context: &Value, keys: &[Value]) -> Value {
    let consumer = core.home.join("consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    let config = consumer.join("config.json");
    std::fs::write(
        &config,
        json!({"keys":keys,"context":context,
        "consumer_invocations":consumer.join("invocations")})
        .to_string(),
    )
    .unwrap();
    let driver = format!(
        "{}/tests/support/command_consumer.py",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::write(consumer.join("trailboss-plugin.toml"), format!(
        "id = \"demo.consumer\"\nname = \"Neutral identity consumer\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"commands\"\nsurface = \"sidebar\"\ncommand = [\"python3\", {driver:?}, \"produce\", {:?}]\nstream = true\n",
        config.to_str().unwrap()
    )).unwrap();
    core.request("plugin.link", json!({"path":consumer,"enabled":true}));
    let api = core
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
    let endpoint = core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    ));
    let evidence = core.home.join("identity-wire.json");
    let output = Command::new("python3")
        .arg(driver)
        .arg("probe")
        .arg(config)
        .arg(api)
        .arg(endpoint)
        .arg(&evidence)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "independent identity consumer: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&std::fs::read(evidence).unwrap()).unwrap()
}

#[test]
fn c08_captured_use_targets_survive_focus_change_into_real_owner_invocation() {
    let core = support::Core::new();
    let review = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let captured = json!({
        "workspace_id":review["workspace"]["workspace_id"],
        "tab_id":review["tab"]["tab_id"],
        "pane_id":review["root_pane"]["pane_id"],
    });
    let plugin = core.home.join("capture-owner");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("trailboss-plugin.toml"), "id = \"demo.owner\"\nname = \"Captured target owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"record-target\"\ntitle = \"Record target\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n").unwrap();
    std::fs::write(plugin.join("record.py"), "import json, os\nwith open('captured.jsonl', 'a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let observed = probe_command_uses(
        &core,
        &captured,
        &[json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"})],
    );
    let selection = &observed["selections"][0];
    // The ordinary Use has already resolved under the captured resource wrapper.
    // Move all three current-resource identities before the selected action runs.
    let other = core.request(
        "workspace.create",
        json!({"label":"Other","cwd":core.home,"focus":true}),
    );
    let focused = core.request("session.snapshot", json!({}));
    assert!(focused["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tab| tab["tab_id"] == other["tab"]["tab_id"] && tab["focused"] == true));
    for (field, other_value) in [
        ("workspace_id", &other["workspace"]["workspace_id"]),
        ("tab_id", &other["tab"]["tab_id"]),
        ("pane_id", &other["root_pane"]["pane_id"]),
    ] {
        assert_ne!(
            captured[field], *other_value,
            "fixture must change actual current {field}"
        );
    }
    let mut context = selection["context"].clone();
    context["invocation_source"] = json!("plugin_ui");
    let invoked = core.request(
        "plugin.action.invoke",
        json!({
            "plugin_id":selection["plugin_id"],"action_id":selection["action_id"],"context":context,
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let logs = core.request("plugin.log.list", json!({"plugin_id":"demo.owner"}));
        if logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|log| log["log_id"] == invoked["log"]["log_id"] && log["status"] == "succeeded")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "captured-context owner action did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let text = std::fs::read_to_string(plugin.join("captured.jsonl")).unwrap();
    let records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["owner"], "demo.owner");
    for field in ["workspace_id", "tab_id", "pane_id"] {
        assert_eq!(
            records[0]["context"][field], captured[field],
            "actual owner must retain the opening scope's captured {field}, not later focus"
        );
    }
    assert!(!core.home.join("consumer/invocations").exists());
}

#[test]
fn c09_removed_explicit_rename_target_does_not_redirect_to_later_focus() {
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    let core = support::Core::new();
    let review = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let target = &review["tab"]["tab_id"];
    core.request("tab.rename", json!({"tab_id":target,"label":"Target"}));
    let captured = json!({
        "workspace_id":review["workspace"]["workspace_id"],
        "tab_id":target,"pane_id":review["root_pane"]["pane_id"],
    });
    let observed = probe_command_uses(
        &core,
        &captured,
        &[json!({"kind":"core","action_id":"host:rename-tab"})],
    );
    let selection = &observed["selections"][0];
    let fragment = observed["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| {
            item["declaration"]["fragments"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .find(|fragment| fragment["name"] == selection["fragment_name"])
        .unwrap();
    assert!(
        fragment["tree"]["menu"]["children"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["type"] == "input"),
        "published rename must retain its native name step"
    );
    let context: Context = serde_json::from_value(selection["context"].clone()).unwrap();
    let mut native = NativeCore::default();
    native.update_session(context.workspace_id.clone(), context.tab_id.clone(), true);
    assert!(matches!(
        native.prepare_palette(selection["action_id"].as_str().unwrap(), &context),
        Effect::Consumed
    ));
    let other = core.request(
        "workspace.create",
        json!({"label":"Other","cwd":core.home,"focus":true}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":other["tab"]["tab_id"],"label":"Other"}),
    );
    core.request("tab.close", json!({"tab_id":target}));
    let before = core.request("tab.get", json!({"tab_id":other["tab"]["tab_id"]}));
    native.update_session(
        other["workspace"]["workspace_id"]
            .as_str()
            .map(str::to_owned),
        other["tab"]["tab_id"].as_str().map(str::to_owned),
        true,
    );
    assert!(matches!(
        native.text("MUST NOT RENAME OTHER"),
        Some(Effect::Consumed)
    ));
    let Some(Effect::Request { method, params }) = native.text("\r") else {
        panic!("captured native rename interaction did not submit")
    };
    let request: Request = serde_json::from_value(
        json!({"id":"removed-explicit-target","method":method,"params":params}),
    )
    .unwrap();
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    ));
    let reply = api
        .request_value_with_timeout(&request, Duration::from_secs(5))
        .unwrap();
    let after = core.request("tab.get", json!({"tab_id":other["tab"]["tab_id"]}));
    assert_eq!(
        after["tab"]["label"], before["tab"]["label"],
        "removed explicit rename target must not be replaced by the currently focused tab"
    );
    assert_eq!(reply["error"]["code"], "tab_not_found");
    let snapshot = core.request("session.snapshot", json!({}));
    assert!(!snapshot["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tab| tab["tab_id"] == *target));
    assert!(snapshot["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tab| tab["tab_id"] == other["tab"]["tab_id"] && tab["focused"] == true));
}

#[test]
fn c10_published_action_uses_one_existing_stream_or_spawn_without_failure_retry() {
    for (stream_count, fail_forward) in [(0, false), (1, false), (2, false), (1, true)] {
        let core = support::Core::new();
        let created = core.request(
            "workspace.create",
            json!({"label":"Review","cwd":core.home,"focus":true}),
        );
        let captured = json!({
            "workspace_id":created["workspace"]["workspace_id"],
            "tab_id":created["tab"]["tab_id"],"pane_id":created["root_pane"]["pane_id"],
        });
        let plugin = core.home.join("dispatch-owner");
        std::fs::create_dir_all(&plugin).unwrap();
        let mut manifest = "id = \"demo.owner\"\nname = \"Dispatch owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"record-target\"\ntitle = \"Record target\"\ncommand = [\"python3\", \"spawn.py\"]\n".to_owned();
        for index in 0..stream_count {
            manifest.push_str(&format!(
                "\n[[ui]]\nid = \"stream-{index}\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"stream.py\", \"{index}\", \"{}\"]\nstream = true\nstream_actions = true\n",
                if fail_forward { "closed" } else { "ready" }
            ));
        }
        std::fs::write(plugin.join("trailboss-plugin.toml"), manifest).unwrap();
        std::fs::write(plugin.join("spawn.py"), "import json, os\nwith open('spawn.jsonl', 'a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
        std::fs::write(plugin.join("stream.py"), r#"import json, os, sys, time
index, mode = sys.argv[1:]
if mode == 'closed':
    os.close(0)
print(json.dumps({'herdr':'widget_tree','version':1,'zones':[],'fragments':[
    {'name':'fixture.dispatch-stream.'+index,'priority':0,'tree':{'type':'label','role':'label','text':'Ready stream '+index}}
]}), flush=True)
if mode == 'closed':
    while True:
        time.sleep(0.01)
for line in sys.stdin:
    envelope = json.loads(line)
    with open('stream-'+index+'.jsonl', 'a') as output:
        output.write(json.dumps(envelope)+'\n')
    print(json.dumps({'herdr':'action_result','id':envelope['id'],'ok':True}), flush=True)
"#).unwrap();
        core.request("plugin.link", json!({"path":plugin,"enabled":true}));
        if stream_count > 0 {
            let mut client = core.attach();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let ready = client.declarations.as_ref().is_some_and(|declarations| {
                    (0..stream_count).all(|index| {
                        declarations.iter().any(|(owner, declaration)| {
                            owner == "demo.owner"
                                && declaration.fragments.iter().any(|fragment| {
                                    fragment.name == format!("fixture.dispatch-stream.{index}")
                                })
                        })
                    })
                });
                if ready {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "real eligible owner streams did not publish readiness"
                );
                client.receive().unwrap();
            }
        }
        let observed = probe_command_uses(
            &core,
            &captured,
            &[json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"})],
        );
        let selection = &observed["selections"][0];
        let mut context = selection["context"].clone();
        context["invocation_source"] = json!("plugin_ui");
        let invoked = core.request("plugin.action.invoke", json!({
            "plugin_id":selection["plugin_id"],"action_id":selection["action_id"],"context":context,
        }));
        let deadline = Instant::now() + Duration::from_secs(5);
        let finished = loop {
            let logs = core.request("plugin.log.list", json!({"plugin_id":"demo.owner"}));
            let matching: Vec<_> = logs["logs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|log| log["log_id"] == invoked["log"]["log_id"])
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "invocation must have one correlated existing owner log"
            );
            if matching[0]["status"] != "running" {
                break matching[0].clone();
            }
            assert!(
                Instant::now() < deadline,
                "existing owner completion path did not settle"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let spawn: Vec<Value> = std::fs::read_to_string(plugin.join("spawn.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let forwarded: Vec<Value> = (0..stream_count)
            .flat_map(|index| {
                std::fs::read_to_string(plugin.join(format!("stream-{index}.jsonl")))
                    .unwrap_or_default()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect::<Vec<Value>>()
            })
            .collect();
        if fail_forward {
            assert_eq!(
                finished["status"], "failed",
                "broken eligible stream must surface failure"
            );
            assert!(
                finished["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("forward write failed")),
                "failure must come from the real closed producer input: {finished}"
            );
            assert!(
                spawn.is_empty(),
                "forwarding failure must not retry via an action process"
            );
            assert!(
                forwarded.is_empty(),
                "closed producer input cannot execute the operation"
            );
        } else {
            assert_eq!(
                finished["status"], "succeeded",
                "owner operation failed: {finished}"
            );
            if stream_count == 0 {
                assert_eq!(
                    spawn.len(),
                    1,
                    "no eligible owner stream must spawn exactly one action process"
                );
                assert!(forwarded.is_empty());
                assert_eq!(spawn[0]["owner"], "demo.owner");
                for field in ["workspace_id", "tab_id", "pane_id"] {
                    assert_eq!(spawn[0]["context"][field], captured[field]);
                }
            } else {
                assert!(
                    spawn.is_empty(),
                    "eligible stream must not also spawn an action process"
                );
                assert_eq!(
                    forwarded.len(),
                    1,
                    "one selection must reach exactly one eligible owner stream"
                );
                assert_eq!(forwarded[0]["id"], invoked["log"]["log_id"]);
                assert_eq!(forwarded[0]["action_id"], "record-target");
                for field in ["workspace_id", "tab_id", "pane_id"] {
                    assert_eq!(forwarded[0]["context"][field], captured[field]);
                }
            }
        }
        assert!(!core.home.join("consumer/invocations").exists());
    }
}

#[test]
fn c11_context_hints_do_not_probe_or_override_private_owner_availability() {
    let core = support::Core::new();
    let created = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let captured = json!({
        "workspace_id":created["workspace"]["workspace_id"],
        "tab_id":created["tab"]["tab_id"],"pane_id":created["root_pane"]["pane_id"],
    });
    let plugin = core.home.join("private-owner");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("trailboss-plugin.toml"), "id = \"demo.owner\"\nname = \"Private availability owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"record-target\"\ntitle = \"Record target\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"private.py\"]\n").unwrap();
    std::fs::write(
        plugin.join("private.py"),
        r#"import json, os, pathlib, sys
with open('attempts.jsonl', 'a') as output:
    output.write(json.dumps(json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON']))+'\n')
if not pathlib.Path('owner-unlocked').exists():
    print('private owner state is locked', file=sys.stderr)
    sys.exit(23)
with open('executed', 'a') as output:
    output.write('private operation performed\n')
"#,
    )
    .unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let key = json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"});
    let catalog = core.request("ui.command.list", json!({}));
    let command = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["key"] == key)
        .unwrap();
    assert_eq!(
        command["contexts"],
        json!(["pane"]),
        "legacy context hints must survive discovery unchanged"
    );
    assert!(
        !plugin.join("attempts.jsonl").exists(),
        "discovery must not probe the owner action"
    );
    let observed = probe_command_uses(&core, &captured, &[key]);
    assert!(
        !plugin.join("attempts.jsonl").exists(),
        "publication/Use resolution must not execute an availability probe"
    );
    let selection = &observed["selections"][0];
    let mut context = selection["context"].clone();
    context["invocation_source"] = json!("plugin_ui");
    let invoked = core.request(
        "plugin.action.invoke",
        json!({
            "plugin_id":selection["plugin_id"],"action_id":selection["action_id"],"context":context,
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let finished = loop {
        let logs = core.request("plugin.log.list", json!({"plugin_id":"demo.owner"}));
        let log = logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|log| log["log_id"] == invoked["log"]["log_id"])
            .unwrap();
        if log["status"] != "running" {
            break log.clone();
        }
        assert!(
            Instant::now() < deadline,
            "private owner failure was not surfaced"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        finished["status"], "failed",
        "catalog membership cannot turn private failure into success"
    );
    assert_eq!(finished["exit_code"], 23);
    assert!(finished["stderr"]
        .as_str()
        .is_some_and(|stderr| stderr.contains("private owner state is locked")));
    let attempts = std::fs::read_to_string(plugin.join("attempts.jsonl")).unwrap();
    let attempts: Vec<Value> = attempts
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        attempts.len(),
        1,
        "only selection may invoke the actual owner"
    );
    for field in ["workspace_id", "tab_id", "pane_id"] {
        assert_eq!(attempts[0][field], captured[field]);
    }
    assert!(
        !plugin.join("executed").exists(),
        "private rejection must prevent the operation"
    );
    assert!(!core.home.join("consumer/invocations").exists());
}

#[test]
fn c12_ordinary_producer_cannot_claim_issued_command_namespace_or_redirect_owner() {
    let core = support::Core::new();
    let created = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let captured = json!({"workspace_id":created["workspace"]["workspace_id"],
        "tab_id":created["tab"]["tab_id"],"pane_id":created["root_pane"]["pane_id"]});
    let owner = core.home.join("namespace-owner");
    let attacker = core.home.join("namespace-attacker");
    for (path, id, action) in [
        (&owner, "demo.owner", "record-target"),
        (&attacker, "demo.other", "hijack"),
    ] {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(
            path.join("action.py"),
            "with open('executed', 'a') as output:\n output.write('executed\\n')\n",
        )
        .unwrap();
        let ui = if id == "demo.owner" {
            "[[ui]]\nid = \"ordinary\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"ordinary.py\"]\n"
        } else {
            ""
        };
        std::fs::write(path.join("trailboss-plugin.toml"), format!(
            "id = {id:?}\nname = \"Namespace fixture\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = {action:?}\ntitle = \"Record target\"\ncommand = [\"python3\", \"action.py\"]\n{ui}"
        )).unwrap();
    }
    let ordinary = |action: &str, priority| {
        json!({"herdr":"widget_tree","version":1,"zones":[],
        "fragments":[{"name":"fixture.ordinary.override","priority":priority,
            "tree":{"type":"row","action":action,"children":[{"type":"label","role":"label","text":"Ordinary override"}]}}]})
    };
    std::fs::write(
        owner.join("ordinary.py"),
        format!("print({:?})\n", ordinary("record-target", 1).to_string()),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let key = json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"});
    let catalog = core.request("ui.command.list", json!({}));
    let issued = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["key"] == key)
        .unwrap();
    let locator = issued["fragment_name"].as_str().unwrap();
    std::fs::write(
        attacker.join("ordinary.py"),
        format!("print({:?})\n", ordinary("hijack", 1000000).to_string()),
    )
    .unwrap();
    let collision = json!({"herdr":"widget_tree","version":1,"zones":[],
        "fragments":[{"name":locator,"priority":2000000,
            "tree":{"type":"row","action":"hijack","children":[{"type":"label","role":"label","text":"Hijacked command"}]}}]});
    std::fs::write(
        attacker.join("collision.py"),
        format!("print({:?})\n", collision.to_string()),
    )
    .unwrap();
    let mut manifest = std::fs::read_to_string(attacker.join("trailboss-plugin.toml")).unwrap();
    manifest.push_str("[[ui]]\nid = \"ordinary\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"ordinary.py\"]\n[[ui]]\nid = \"collision\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"collision.py\"]\n");
    std::fs::write(attacker.join("trailboss-plugin.toml"), manifest).unwrap();
    core.request("plugin.link", json!({"path":attacker,"enabled":true}));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let logs = core.request("plugin.log.list", json!({"plugin_id":"demo.other"}));
        let finished = ["ordinary.py", "collision.py"].iter().all(|script| {
            logs["logs"].as_array().unwrap().iter().any(|log| {
                log["status"] == "succeeded"
                    && log["command"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|arg| arg.as_str() == Some(*script))
            })
        });
        if finished {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "real ordinary/colliding producer completion missing: {logs}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.boot_id.is_empty() || client.declarations.is_none() {
        assert!(
            Instant::now() < deadline,
            "actual projection missing after producer completion"
        );
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    let native = NativeCore::default();
    assert_eq!(
        native
            .fragment(declarations, "fixture.ordinary.override")
            .unwrap()
            .0,
        "demo.other",
        "unrelated ordinary overrides must retain public priority rules"
    );
    assert!(!declarations.iter().any(|(plugin, declaration)| plugin == "demo.other" &&
        declaration.fragments.iter().any(|fragment| fragment.name == locator)),
        "ordinary producer's claim of an issued command locator must be rejected by declaration validation");
    assert_eq!(
        native.fragment(declarations, locator).unwrap().0,
        "demo.owner"
    );
    let observed = probe_command_uses(&core, &captured, &[key]);
    let selection = &observed["selections"][0];
    let mut context = selection["context"].clone();
    context["invocation_source"] = json!("plugin_ui");
    let invoked = core.request(
        "plugin.action.invoke",
        json!({"plugin_id":selection["plugin_id"],
        "action_id":selection["action_id"],"context":context}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let logs = core.request("plugin.log.list", json!({"plugin_id":"demo.owner"}));
        if logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|log| log["log_id"] == invoked["log"]["log_id"] && log["status"] == "succeeded")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "actual selected owner did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(owner.join("executed")).unwrap(),
        "executed\n"
    );
    assert!(
        !attacker.join("executed").exists(),
        "colliding producer must not steal invocation"
    );
    assert!(!core.home.join("consumer/invocations").exists());
}

#[test]
fn c13_all_129_owner_actions_have_bounded_exports_and_action_129_executes() {
    use std::collections::BTreeSet;
    let core = support::Core::new();
    let created = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let captured = json!({"workspace_id":created["workspace"]["workspace_id"],
        "tab_id":created["tab"]["tab_id"],"pane_id":created["root_pane"]["pane_id"]});
    let plugin = core.home.join("large-owner");
    std::fs::create_dir_all(&plugin).unwrap();
    let mut manifest = "id = \"demo.owner\"\nname = \"Large command owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n".to_owned();
    for index in 1..=129 {
        manifest.push_str(&format!(
            "\n[[actions]]\nid = \"action-{index}\"\ntitle = \"Operation {index}\"\ncommand = [\"python3\", \"record.py\", \"action-{index}\"]\n"
        ));
    }
    std::fs::write(plugin.join("trailboss-plugin.toml"), manifest).unwrap();
    std::fs::write(plugin.join("record.py"), "import json, os, sys\nwith open('executed.jsonl', 'a') as output:\n output.write(json.dumps({'action':sys.argv[1],'owner':os.environ['HERDR_PLUGIN_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let catalog = core.request("ui.command.list", json!({}));
    let commands: Vec<_> = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|command| {
            command["key"]["kind"] == "plugin" && command["key"]["plugin_id"] == "demo.owner"
        })
        .collect();
    assert_eq!(
        commands.len(),
        129,
        "the owner must not be truncated to one declaration"
    );
    let names: BTreeSet<_> = commands
        .iter()
        .map(|command| command["fragment_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names.len(),
        129,
        "every structured action needs its own locator"
    );
    assert!(names.iter().all(|name| name.chars().count() <= 200));
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.boot_id.is_empty() || client.declarations.is_none() {
        assert!(
            Instant::now() < deadline,
            "actual large-owner projection missing"
        );
        client.receive().unwrap();
    }
    let declarations: Vec<_> = client
        .declarations
        .as_ref()
        .unwrap()
        .iter()
        .filter(|(owner, _)| owner == "demo.owner")
        .map(|(_, declaration)| declaration)
        .collect();
    let exported: BTreeSet<_> = declarations
        .iter()
        .flat_map(|declaration| {
            declaration
                .fragments
                .iter()
                .map(|fragment| fragment.name.as_str())
        })
        .collect();
    assert_eq!(
        exported, names,
        "actual projection must export the complete advertised owner inventory"
    );
    for declaration in declarations {
        assert!(declaration.fragments.len() <= 128);
        assert!(declaration.zones.is_empty() && declaration.composition.is_none());
        assert!(declaration
            .validate(|action| (1..=129).any(|index| action == format!("action-{index}"))));
    }
    let observed = probe_command_uses(
        &core,
        &captured,
        &[json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"action-129"})],
    );
    let selection = &observed["selections"][0];
    let mut context = selection["context"].clone();
    context["invocation_source"] = json!("plugin_ui");
    let invoked = core.request(
        "plugin.action.invoke",
        json!({"plugin_id":selection["plugin_id"],
        "action_id":selection["action_id"],"context":context}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let logs = core.request("plugin.log.list", json!({"plugin_id":"demo.owner"}));
        if logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|log| log["log_id"] == invoked["log"]["log_id"] && log["status"] == "succeeded")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "action 129 actual owner did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let text = std::fs::read_to_string(plugin.join("executed.jsonl")).unwrap();
    let records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["owner"], "demo.owner");
    assert_eq!(records[0]["action"], "action-129");
    for field in ["workspace_id", "tab_id", "pane_id"] {
        assert_eq!(records[0]["context"][field], captured[field]);
    }
}

#[test]
fn c16_confirmation_copies_the_current_owner_export_without_an_outer_action() {
    use trailboss_runtime::protocol::widgets::Node;
    let core = support::Core::new();
    let owner = core.home.join("confirmation-owner");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::write(owner.join("trailboss-plugin.toml"),
        "id = \"fixture.confirmation\"\nname = \"Confirmation owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"close-space\"\ntitle = \"Close owned space\"\nconfirmation_fragment = \"commands.confirm-space\"\ncommand = [\"python3\", \"record.py\"]\n[[ui]]\nid = \"confirmation\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"confirmation.py\"]\n").unwrap();
    std::fs::write(
        owner.join("record.py"),
        "raise SystemExit('opening a confirmation must never invoke its owner')\n",
    )
    .unwrap();
    let confirmation = json!({"type":"column","children":[
        {"type":"label","text":"Owner confirmation","role":"heading"},
        {"type":"row","children":[{"type":"label","text":"Cancel","role":"primary"}],"action":"host:cancel"},
        {"type":"row","children":[{"type":"label","text":"Confirm original owner","role":"primary"}],"action":"close-space"}
    ]});
    let declaration = json!({"herdr":"widget_tree","version":1,"zones":[],
        "fragments":[{"name":"commands.confirm-space","priority":0,"tree":confirmation}]});
    std::fs::write(
        owner.join("confirmation.py"),
        format!("print({:?}, flush=True)\n", declaration.to_string()),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let logs = core.request(
            "plugin.log.list",
            json!({"plugin_id":"fixture.confirmation"}),
        );
        if logs["logs"].as_array().unwrap().iter().any(|log| {
            log["status"] == "succeeded"
                && log["command"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|arg| arg == "confirmation.py")
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "actual confirmation producer did not complete: {logs}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let catalog = core.request("ui.command.list", json!({}));
    let command = catalog["commands"].as_array().unwrap().iter().find(|command|
        command["key"] == json!({"kind":"plugin","plugin_id":"fixture.confirmation","action_id":"close-space"})).unwrap();
    let mut client = core.attach();
    while client.boot_id.is_empty() || client.declarations.is_none() {
        assert!(
            Instant::now() < deadline,
            "actual confirmation projection missing"
        );
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    let native = NativeCore::default();
    let (provider, tree) = native
        .fragment(declarations, command["fragment_name"].as_str().unwrap())
        .unwrap();
    assert_eq!(provider, "fixture.confirmation");
    let Node::Row {
        action,
        context,
        menu,
        ..
    } = tree
    else {
        panic!("generated owner confirmation must be an ordinary row");
    };
    assert_eq!(
        action, &None,
        "opening confirmation must not execute the destructive action"
    );
    assert_eq!(
        context,
        &Context::default(),
        "owner export must not replace captured scope"
    );
    let expected: Node = serde_json::from_value(confirmation).unwrap();
    assert_eq!(menu.as_deref().expect("owner confirmation menu"), &expected);
    let logs = core.request(
        "plugin.log.list",
        json!({"plugin_id":"fixture.confirmation"}),
    );
    assert!(!logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|log| log["command"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "record.py")));
}

#[test]
fn c16_invalid_or_ambiguous_owner_confirmations_withdraw_and_ordinary_publication_recovers() {
    use trailboss_runtime::protocol::widgets::Node;
    let core = support::Core::new();
    let owner = core.home.join("confirmation-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let header = "id = \"fixture.confirmation\"\nname = \"Confirmation owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"close-space\"\ntitle = \"Close owned space\"\nconfirmation_fragment = \"commands.confirm-space\"\ncommand = [\"python3\", \"record.py\"]\n[[actions]]\nid = \"other\"\ntitle = \"Other action\"\ncommand = [\"python3\", \"record.py\"]\n";
    std::fs::write(owner.join("trailboss-plugin.toml"), format!("{header}[[ui]]\nid = \"confirmation\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"confirmation.py\", \"current.json\"]\nstream = true\n[[ui]]\nid = \"duplicate\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"confirmation.py\", \"duplicate.json\"]\nstream = true\n")).unwrap();
    std::fs::write(owner.join("confirmation.py"),
        "import pathlib, sys, time\npath = pathlib.Path(sys.argv[1])\nlast = None\nwhile True:\n text = path.read_text()\n if text != last:\n  print(text, flush=True)\n  last = text\n time.sleep(0.02)\n").unwrap();
    std::fs::write(
        owner.join("record.py"),
        "raise SystemExit('unavailable confirmation must not dispatch')\n",
    )
    .unwrap();
    let label = |text: &str| json!({"type":"label","text":text,"role":"primary"});
    let terminal = |action: &str| json!({"type":"row","children":[label(action)],"action":action});
    let valid = json!({"type":"column","children":[label("Owner confirmation"),
        terminal("host:cancel"),terminal("close-space")]});
    let publication = |marker: &str, tree: Option<Value>| {
        json!({
            "herdr":"widget_tree","version":1,
            "zones":[{"zone":"footer","tree":label(marker)}],
            "fragments":tree.map(|tree| vec![json!({"name":"commands.confirm-space","priority":0,"tree":tree})]).unwrap_or_default()
        })
    };
    std::fs::write(
        owner.join("current.json"),
        publication("pending", None).to_string(),
    )
    .unwrap();
    std::fs::write(
        owner.join("duplicate.json"),
        publication("duplicate-empty", None).to_string(),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));

    // A higher-priority, valid export from another owner must never satisfy the
    // annotation while this owner's own export is absent.
    let foreign = core.home.join("foreign-confirmation");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("trailboss-plugin.toml"),
        "id = \"fixture.foreign\"\nname = \"Foreign confirmation\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"close-space\"\ntitle = \"Foreign close\"\ncommand = [\"python3\", \"foreign.py\"]\n[[ui]]\nid = \"confirmation\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"foreign.py\"]\n").unwrap();
    let mut foreign_declaration = publication("foreign", Some(valid.clone()));
    foreign_declaration["fragments"][0]["priority"] = json!(1000000);
    std::fs::write(
        foreign.join("foreign.py"),
        format!("print({:?}, flush=True)\n", foreign_declaration.to_string()),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":foreign,"enabled":true}));
    let observed = |marker: &str| {
        let mut client = core.attach();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(declarations) = &client.declarations {
                if declarations.iter().any(|(provider, declaration)| {
                    provider == "fixture.confirmation"
                        && declaration.zones.iter().any(
                            |zone| matches!(&zone.tree, Node::Label { text, .. } if text == marker),
                        )
                }) {
                    return declarations.clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "ordinary producer state {marker} was not projected"
            );
            client.receive().unwrap();
        }
    };
    let key = json!({"kind":"plugin","plugin_id":"fixture.confirmation","action_id":"close-space"});
    let mut locator = None;
    let mut check = |marker: &str, usable: bool| {
        let declarations = observed(marker);
        let catalog = core.request("ui.command.list", json!({}));
        let command = catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == key)
            .unwrap();
        assert_eq!(command["title"], "Close owned space");
        if let Some(previous) = &locator {
            assert_eq!(command["fragment_name"].as_str().unwrap(), previous);
        } else {
            locator = Some(command["fragment_name"].as_str().unwrap().to_owned());
        }
        let native = NativeCore::default();
        let resolved = native.fragment(&declarations, command["fragment_name"].as_str().unwrap());
        if usable {
            assert!(command.get("diagnostic").is_none(), "{marker}: {command}");
            let (provider, tree) =
                resolved.expect("valid ordinary owner publication restores its Use");
            assert_eq!(provider, "fixture.confirmation");
            let Node::Row {
                action,
                context,
                menu,
                ..
            } = tree
            else {
                panic!("confirmation wrapper")
            };
            assert!(action.is_none());
            assert_eq!(context, &Context::default());
            assert_eq!(
                menu.as_deref(),
                Some(&serde_json::from_value::<Node>(valid.clone()).unwrap())
            );
        } else {
            let diagnostic = command["diagnostic"]
                .as_str()
                .expect("unavailable owner/action diagnostic");
            assert!(
                diagnostic.contains("fixture.confirmation/close-space"),
                "{diagnostic}"
            );
            assert!(
                resolved.is_none(),
                "{marker}: no direct-action fallback or priority substitution"
            );
        }
    };
    check("pending", false);
    std::fs::write(
        owner.join("current.json"),
        publication("valid-first", Some(valid.clone())).to_string(),
    )
    .unwrap();
    check("valid-first", true);
    let mut wrong_action = valid.clone();
    wrong_action["children"][2]["action"] = json!("other");
    let mut target_override = valid.clone();
    target_override["children"][2]["context"] = json!({"tab_id":"replacement-target"});
    let mut row_menu = valid.clone();
    row_menu["children"][2]["menu"] = json!({"type":"column","children":[terminal("close-space")]});
    let mut extra_affirmative = valid.clone();
    extra_affirmative["children"]
        .as_array_mut()
        .unwrap()
        .push(terminal("close-space"));
    let mut no_cancel = valid.clone();
    no_cancel["children"].as_array_mut().unwrap().remove(1);
    for (index, tree) in [
        None,
        Some(wrong_action),
        Some(target_override),
        Some(row_menu),
        Some(extra_affirmative),
        Some(no_cancel),
        Some(json!({"type":"column","children":[{"type":"input"},terminal("host:cancel"),terminal("close-space")]})),
        Some(json!({"type":"column","children":[{"type":"use","name":"commands.confirm-space"},terminal("host:cancel"),terminal("close-space")]})),
        Some(json!({"type":"column","children":[{"type":"column","children":[terminal("host:cancel"),terminal("close-space")]}]})),
    ].into_iter().enumerate() {
        let marker = format!("invalid-{index}");
        std::fs::write(owner.join("current.json"), publication(&marker, tree).to_string()).unwrap();
        check(&marker, false);
    }
    std::fs::write(
        owner.join("current.json"),
        publication("before-duplicate", Some(valid.clone())).to_string(),
    )
    .unwrap();
    check("before-duplicate", true);
    std::fs::write(
        owner.join("duplicate.json"),
        publication("duplicate-present", Some(valid.clone())).to_string(),
    )
    .unwrap();
    check("duplicate-present", false);
    std::fs::write(
        owner.join("duplicate.json"),
        publication("duplicate-withdrawn", None).to_string(),
    )
    .unwrap();
    check("duplicate-withdrawn", true);
    let action_list = core.request(
        "plugin.action.list",
        json!({"plugin_id":"fixture.confirmation"}),
    );
    assert!(action_list["actions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|action| action.get("confirmation_fragment").is_none()
            && action.get("diagnostic").is_none()));
    let logs = core.request(
        "plugin.log.list",
        json!({"plugin_id":"fixture.confirmation"}),
    );
    assert!(!logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|log| log["command"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "record.py")));
}

#[test]
fn c17_actual_paired_revisions_refresh_owners_and_restart_invalidates_old_locators() {
    use std::os::unix::net::UnixStream;
    use trailboss_runtime::protocol::{
        self,
        endpoint::{
            EndpointClientHello, EndpointPluginUi, ENDPOINT_HELLO_KIND, ENDPOINT_SNAPSHOT_KIND,
            PLUGIN_UI_KIND,
        },
        widgets::{Declaration, Node},
        ClientMessage, ClientShellSnapshot, ClientSurfaceSize, ServerMessage,
    };
    fn connect(core: &support::Core) -> UnixStream {
        let mut stream = UnixStream::connect(core.home.join(format!(
            "trailboss/sessions/{}/trailboss-client.sock",
            core.name
        )))
        .unwrap();
        stream
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
            surface_active: false,
            surface_reuse: false,
            surface_delta: false,
            surface_scroll: false,
            snapshot_codecs: vec!["shell.snapshot.v1".into()],
            surface_codecs: vec!["shell.surface.v1".into()],
            input_codecs: vec!["shell.input.semantic.v1".into()],
            blob_codecs: vec!["shell.blob.v1".into()],
        };
        protocol::write_message(
            &mut stream,
            &ClientMessage::EndpointControl {
                kind: ENDPOINT_HELLO_KIND.into(),
                data: serde_json::to_string(&hello).unwrap(),
            },
        )
        .unwrap();
        stream
    }
    fn paired(
        stream: &mut UnixStream,
        ready: impl Fn(&EndpointPluginUi) -> bool,
    ) -> (ClientShellSnapshot, EndpointPluginUi) {
        let mut snapshot: Option<ClientShellSnapshot> = None;
        let mut pending_ui: Option<EndpointPluginUi> = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "matching refreshed snapshot/UI pair did not arrive"
            );
            let message: ServerMessage =
                protocol::read_message(stream, protocol::MAX_GRAPHICS_FRAME_SIZE).unwrap();
            if let ServerMessage::EndpointControl { kind, data } = message {
                if kind == ENDPOINT_SNAPSHOT_KIND {
                    snapshot = Some(serde_json::from_str(&data).unwrap());
                } else if kind == PLUGIN_UI_KIND {
                    pending_ui = Some(serde_json::from_str(&data).unwrap());
                }
                // The existing reliable control batch publishes UI before its
                // snapshot; consumers pair identities, not message ordering.
                if let (Some(snapshot), Some(ui)) = (&snapshot, &pending_ui) {
                    if ui.boot_id == snapshot.boot_id
                        && ui.revision == snapshot.revision
                        && ready(ui)
                    {
                        return (snapshot.clone(), pending_ui.take().unwrap());
                    }
                }
            }
        }
    }
    fn declarations(ui: &EndpointPluginUi) -> Vec<(String, Declaration)> {
        ui.ui
            .iter()
            .filter_map(|item| {
                item.declaration
                    .as_ref()
                    .map(|tree| (item.plugin_id.clone(), tree.clone()))
            })
            .collect()
    }
    fn assert_catalog(catalog: &Value, ui: &EndpointPluginUi) {
        assert_eq!(catalog["boot_id"], ui.boot_id);
        let declarations = declarations(ui);
        let native = NativeCore::default();
        let mut issued = std::collections::BTreeSet::new();
        for command in catalog["commands"].as_array().unwrap() {
            let name = command["fragment_name"].as_str().unwrap();
            assert!(
                issued.insert(name),
                "distinct structured commands must not share a locator"
            );
            let (owner, _) = native
                .fragment(&declarations, name)
                .expect("current advertised locator resolves");
            let expected = command["key"]["plugin_id"]
                .as_str()
                .unwrap_or("trailboss.core");
            assert_eq!(
                owner, expected,
                "only the actual owner may provide this current locator"
            );
        }
    }
    let mut core = support::Core::new();
    let target = core.request(
        "workspace.create",
        json!({"label":"Captured","cwd":core.home,"focus":true}),
    );
    let owner = core.home.join("refresh-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let manifest = |title: &str, added: bool| {
        format!(
        "id = \"fixture.refresh\"\nname = \"Refresh owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"first\"\ntitle = \"{title}\"\ncommand = [\"python3\", \"record.py\"]\n{}",
        if added { "[[actions]]\nid = \"second\"\ntitle = \"New live command\"\ncommand = [\"python3\", \"record.py\"]\n" } else { "" })
    };
    std::fs::write(
        owner.join("trailboss-plugin.toml"),
        manifest("Original owner title", false),
    )
    .unwrap();
    std::fs::write(owner.join("record.py"),
        "import json, os\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let before = core.request("ui.command.list", json!({}));
    let mut stream = connect(&core);
    let (before_snapshot, before_ui) = paired(&mut stream, |_| true);
    assert_catalog(&before, &before_ui);
    let first_key = json!({"kind":"plugin","plugin_id":"fixture.refresh","action_id":"first"});
    let second_key = json!({"kind":"plugin","plugin_id":"fixture.refresh","action_id":"second"});
    let first = before["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["key"] == first_key)
        .unwrap();
    let old_declarations = declarations(&before_ui);
    std::fs::write(
        owner.join("trailboss-plugin.toml"),
        manifest("Refreshed owner title", true),
    )
    .unwrap();
    let refreshed = core.request("ui.command.list", json!({}));
    let second = refreshed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["key"] == second_key)
        .unwrap();
    let renamed = refreshed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["key"] == first_key)
        .unwrap();
    assert_eq!(first["fragment_name"], renamed["fragment_name"]);
    assert_eq!(renamed["title"], "Refreshed owner title");
    let second_name = second["fragment_name"].as_str().unwrap();
    let native = NativeCore::default();
    assert!(
        native.fragment(&old_declarations, second_name).is_none(),
        "a fresh catalog with a pending old projection must not guess an executable row"
    );
    assert!(
        !owner.join("executed.jsonl").exists(),
        "unresolved pending Use must not dispatch"
    );
    let (refreshed_snapshot, refreshed_ui) = paired(&mut stream, |ui| {
        ui.ui
            .iter()
            .filter_map(|item| item.declaration.as_ref())
            .flat_map(|tree| &tree.fragments)
            .any(|fragment| fragment.name == second_name)
    });
    assert!(refreshed_snapshot.revision > before_snapshot.revision);
    assert_catalog(&refreshed, &refreshed_ui);
    let refreshed_declarations = declarations(&refreshed_ui);
    let (_, renamed_tree) = native
        .fragment(
            &refreshed_declarations,
            renamed["fragment_name"].as_str().unwrap(),
        )
        .unwrap();
    let Node::Row { children, .. } = renamed_tree else {
        panic!("ordinary generated row")
    };
    assert!(matches!(&children[0], Node::Label { text, .. } if text == "Refreshed owner title"));
    let old_names: Vec<_> = refreshed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["fragment_name"].as_str().unwrap().to_owned())
        .collect();
    drop(stream);
    core.restart();
    let restarted = core.request("ui.command.list", json!({}));
    assert_ne!(
        refreshed["boot_id"], restarted["boot_id"],
        "new Core process must have a new actual boot identity"
    );
    let mut stream = connect(&core);
    let (_, restarted_ui) = paired(&mut stream, |_| true);
    assert_catalog(&restarted, &restarted_ui);
    let restarted_declarations = declarations(&restarted_ui);
    for old_name in old_names {
        assert!(
            native
                .fragment(&restarted_declarations, &old_name)
                .is_none(),
            "old-boot locator must never retarget to a current command"
        );
    }
    let current = restarted["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["key"] == second_key)
        .unwrap();
    assert_ne!(second["fragment_name"], current["fragment_name"]);
    let (provider, tree) = native
        .fragment(
            &restarted_declarations,
            current["fragment_name"].as_str().unwrap(),
        )
        .unwrap();
    let Node::Row {
        action: Some(action),
        ..
    } = tree
    else {
        panic!("current actual owner action")
    };
    let captured = json!({"workspace_id":target["workspace"]["workspace_id"],
        "tab_id":target["tab"]["tab_id"],"pane_id":target["root_pane"]["pane_id"],"invocation_source":"plugin_ui"});
    core.request(
        "plugin.action.invoke",
        json!({"plugin_id":provider,"action_id":action,"context":captured}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let records = loop {
        if let Ok(text) = std::fs::read_to_string(owner.join("executed.jsonl")) {
            let records: Vec<Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            if !records.is_empty() {
                break records;
            }
        }
        assert!(
            Instant::now() < deadline,
            "current locator's actual owner did not execute"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(records.len(), 1, "only current selection invokes once");
    assert_eq!(records[0]["owner"], "fixture.refresh");
    assert_eq!(records[0]["action"], "second");
    for field in ["workspace_id", "tab_id", "pane_id"] {
        assert_eq!(records[0]["context"][field], captured[field]);
    }
}

#[test]
fn c19_twenty_thousand_valid_actions_surface_a_complete_publication_failure_without_fallback() {
    use std::fmt::Write as _;
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
        protocol::MAX_FRAME_SIZE,
    };
    let core = support::Core::new();
    let survivor = core.request(
        "workspace.create",
        json!({"label":"Survivor","cwd":core.home,"focus":true}),
    );
    let mut live = core.attach();
    let ready = Instant::now() + Duration::from_secs(5);
    while live.declarations.is_none() {
        assert!(
            Instant::now() < ready,
            "initial complete Core-only publication missing"
        );
        live.receive().unwrap();
    }
    let previous = live.declarations.clone();
    let owner = core.home.join("oversized-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let mut manifest = String::with_capacity(3 * 1024 * 1024);
    manifest.push_str("id = \"fixture.oversized\"\nname = \"Oversized owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n");
    for index in 0..20000 {
        writeln!(manifest, "[[actions]]\nid = \"action-{index:05}\"\ntitle = \"Valid action {index:05}\"\ncommand = [\"python3\", \"record.py\"]").unwrap();
    }
    std::fs::write(owner.join("trailboss-plugin.toml"), manifest).unwrap();
    std::fs::write(
        owner.join("record.py"),
        "from pathlib import Path\nPath('unexpected-fallback').write_text('invoked')\n",
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let legacy = core.request(
        "plugin.action.list",
        json!({"plugin_id":"fixture.oversized"}),
    );
    assert_eq!(
        legacy["actions"].as_array().unwrap().len(),
        20000,
        "all fixture declarations must be accepted as valid, not filtered to manufacture a limit"
    );
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    ));
    let request: Request = serde_json::from_value(json!({
        "id":"complete-discovery", "method":"ui.command.list", "params":{},
    }))
    .unwrap();
    let reply = api
        .request_value_with_timeout(&request, Duration::from_secs(10))
        .unwrap();
    let error = reply.get("error").expect(
        "the oversized complete projection must fail, not advertise a successful usable catalog",
    );
    assert!(
        reply.get("result").is_none(),
        "a failed complete publication has no partial success"
    );
    let bound = format!("exceeds maximum {MAX_FRAME_SIZE}");
    assert!(
        error["message"].as_str().unwrap().contains(&bound),
        "{error}"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            Instant::now() < deadline,
            "already-connected endpoint must surface complete publication failure"
        );
        if let Err(error) = live.receive() {
            assert!(error.to_string().contains(&bound), "{error}");
            break;
        }
        assert_eq!(
            live.declarations, previous,
            "an oversized refresh must not substitute a partial successful owner set"
        );
    }
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            Instant::now() < deadline,
            "oversized owner declarations must surface a bounded endpoint failure"
        );
        if let Err(error) = client.receive() {
            assert!(
                error.to_string().contains(&bound),
                "actual endpoint error must expose the existing frame bound: {error}"
            );
            break;
        }
        assert!(client.declarations.is_none(),
            "an incomplete generated owner set must not be presented as a usable declaration projection");
    }
    assert!(
        !owner.join("unexpected-fallback").exists(),
        "publication failure must never invoke an action"
    );
    let after = core.request("session.snapshot", json!({}));
    assert!(after["snapshot"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|workspace| workspace["workspace_id"] == survivor["workspace"]["workspace_id"]));
}

#[test]
fn c20_native_nested_use_dispatches_explicit_child_scope_after_other_focus() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use trailboss::terminal::{
        menus::{Controls, Response},
        Rect,
    };
    let core = support::Core::new();
    let parent = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":parent["tab"]["tab_id"],"label":"Parent"}),
    );
    let child = core.request(
        "workspace.create",
        json!({"label":"ChildScope","cwd":core.home,"focus":false}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":child["tab"]["tab_id"],"label":"Child"}),
    );
    let other = core.request(
        "workspace.create",
        json!({"label":"Other","cwd":core.home,"focus":false}),
    );
    let captured = |resource: &Value| {
        json!({
            "workspace_id":resource["workspace"]["workspace_id"],
            "tab_id":resource["tab"]["tab_id"],"pane_id":resource["root_pane"]["pane_id"],
        })
    };
    let parent_context = captured(&parent);
    let child_context = captured(&child);
    let owner = core.home.join("nested-owner");
    let consumer = core.home.join("nested-consumer");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::create_dir_all(&consumer).unwrap();
    std::fs::write(owner.join("trailboss-plugin.toml"),
        "id = \"demo.owner\"\nname = \"Nested action owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"record-target\"\ntitle = \"Record explicit child\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n").unwrap();
    std::fs::write(owner.join("record.py"),
        "import json, os\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let catalog = core.request("ui.command.list", json!({}));
    let command = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| {
            command["key"]
                == json!({"kind":"plugin","plugin_id":"demo.owner","action_id":"record-target"})
        })
        .unwrap();
    let label = |text: &str| json!({"type":"label","text":text,"role":"primary"});
    let declaration = json!({"herdr":"widget_tree","version":1,"zones":[],
    "fragments":[{"name":"command-palette","priority":1000000,"tree":{
        "type":"column","children":[{
            "type":"row","children":[label("Review Parent")],"context":parent_context,
            "menu":{"type":"column","children":[{
                "type":"row","children":[label("ChildScope Child")],"context":child_context,
                "menu":{"type":"column","children":[{"type":"use","name":command["fragment_name"]}]}
            }]}
        }]
    }}]});
    std::fs::write(consumer.join("trailboss-plugin.toml"),
        "id = \"fixture.nested-consumer\"\nname = \"Nested neutral consumer\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"menus\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"consumer.py\"]\n").unwrap();
    std::fs::write(
        consumer.join("consumer.py"),
        format!("print({:?}, flush=True)\n", declaration.to_string()),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":consumer,"enabled":true}));
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if client.declarations.as_ref().is_some_and(|declarations| {
            declarations.iter().any(|(provider, declaration)| {
                provider == "fixture.nested-consumer"
                    && declaration
                        .fragments
                        .iter()
                        .any(|fragment| fragment.name == "command-palette")
            })
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "actual ordinary nested consumer not published"
        );
        client.receive().unwrap();
    }
    let mut native = NativeCore::default();
    native.update_session(
        parent_context["workspace_id"].as_str().map(str::to_owned),
        parent_context["tab_id"].as_str().map(str::to_owned),
        true,
    );
    let mut controls = Controls::default();
    let viewport = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    assert!(controls.open_palette(&native, client.declarations.as_ref().unwrap(), None));
    for _ in 0..2 {
        assert!(matches!(
            controls.key(
                &native,
                client.declarations.as_ref().unwrap(),
                viewport,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Response::Consumed
        ));
    }
    assert!(
        !owner.join("executed.jsonl").exists(),
        "opening target menus must not invoke an action"
    );
    core.request(
        "workspace.focus",
        json!({"workspace_id":other["workspace"]["workspace_id"]}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while client
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.focused_tab_id.as_deref())
        != other["tab"]["tab_id"].as_str()
    {
        assert!(
            Instant::now() < deadline,
            "actual later Other focus not received"
        );
        client.receive().unwrap();
    }
    let snapshot = client.snapshot.as_ref().unwrap();
    native.update_session(
        snapshot.focused_workspace_id.clone(),
        snapshot.focused_tab_id.clone(),
        true,
    );
    let Response::Invoke(selected) = controls.key(
        &native,
        client.declarations.as_ref().unwrap(),
        viewport,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ) else {
        panic!("nested ordinary Use must yield its true owner action");
    };
    assert_eq!(selected.plugin, "demo.owner");
    assert_eq!(selected.action, "record-target");
    assert!(matches!(
        native.invoke(&selected.action, &selected.context, Some(snapshot)),
        Effect::Plugin
    ));
    client
        .input()
        .unwrap()
        .invoke(
            &client.boot_id,
            &selected.plugin,
            &selected.action,
            &selected.context,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let records = loop {
        if let Ok(text) = std::fs::read_to_string(owner.join("executed.jsonl")) {
            let records: Vec<Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            if !records.is_empty() {
                break records;
            }
        }
        assert!(
            Instant::now() < deadline,
            "actual nested owner invocation did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["owner"], "demo.owner");
    assert_eq!(records[0]["action"], "record-target");
    for field in ["workspace_id", "tab_id", "pane_id"] {
        assert_eq!(records[0]["context"][field], child_context[field]);
        assert_ne!(records[0]["context"][field], parent_context[field]);
        assert_ne!(records[0]["context"][field], captured(&other)[field]);
    }
}

#[test]
fn c21_empty_eligible_plugin_set_keeps_full_core_inventory_and_real_targeted_focus() {
    let core = support::Core::new();
    let target = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":target["tab"]["tab_id"],"label":"Target"}),
    );
    let other = core.request(
        "workspace.create",
        json!({"label":"Other","cwd":core.home,"focus":true}),
    );
    let context = json!({
        "workspace_id":target["workspace"]["workspace_id"],
        "tab_id":target["tab"]["tab_id"],"pane_id":target["root_pane"]["pane_id"],
    });
    let retired = core.home.join("retired-owner");
    std::fs::create_dir_all(&retired).unwrap();
    std::fs::write(retired.join("trailboss-plugin.toml"),
        "id = \"fixture.retired\"\nname = \"Previously eligible owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"retired\"\ntitle = \"Retired plugin command\"\ncommand = [\"python3\", \"action.py\"]\n").unwrap();
    std::fs::write(
        retired.join("action.py"),
        "open('executed','w').write('unexpected invocation')\n",
    )
    .unwrap();
    core.request("plugin.link", json!({"path":retired,"enabled":true}));
    let stale_key = json!({"kind":"plugin","plugin_id":"fixture.retired","action_id":"retired"});
    assert!(core.request("ui.command.list", json!({}))["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["key"] == stale_key));
    core.request("plugin.disable", json!({"plugin_id":"fixture.retired"}));
    let focus = json!({"kind":"core","action_id":"host:focus","mode":"targeted"});
    let observed = probe_command_uses(&core, &context, &[focus]);
    let commands = observed["catalog"]["commands"].as_array().unwrap();
    // Source-independent inventory, including both distinct focus modes.
    let inventory = [
        ("host:new-space", None),
        ("host:new", None),
        ("host:new-tab", None),
        ("host:rename-space", None),
        ("host:rename-tab", None),
        ("host:close-tab", None),
        ("host:focus", Some("targeted")),
        ("host:focus", Some("previous")),
        ("host:restart-idle-agents", None),
        ("host:detach", None),
    ];
    assert_eq!(
        commands.len(),
        inventory.len(),
        "empty eligible plugin set must retain the complete Core inventory"
    );
    assert!(
        commands
            .iter()
            .all(|command| command["key"]["kind"] == "core"),
        "no stale plugin command may survive the transition to an empty eligible set"
    );
    for (action, mode) in inventory {
        let mut key = json!({"kind":"core","action_id":action});
        if let Some(mode) = mode {
            key["mode"] = json!(mode);
        }
        assert_eq!(
            commands
                .iter()
                .filter(|command| command["key"] == key)
                .count(),
            1,
            "missing or duplicated Core command in the empty eligible set: {key}"
        );
    }
    let selections = observed["selections"].as_array().unwrap();
    assert_eq!(selections.len(), 1);
    let selected = &selections[0];
    assert_eq!(selected["plugin_id"], "trailboss.core");
    assert_eq!(selected["action_id"], "host:focus");
    assert_eq!(selected["context"], context);
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.boot_id.is_empty() || client.snapshot.is_none() {
        assert!(
            Instant::now() < deadline,
            "native Core snapshot was not received"
        );
        client.receive().unwrap();
    }
    let snapshot = client.snapshot.as_ref().unwrap();
    assert_eq!(
        snapshot.focused_tab_id.as_deref(),
        other["tab"]["tab_id"].as_str(),
        "the operation must change focus, not merely assert an already-focused target"
    );
    let mut native = NativeCore::default();
    native.update_session(
        snapshot.focused_workspace_id.clone(),
        snapshot.focused_tab_id.clone(),
        true,
    );
    let captured: Context = serde_json::from_value(selected["context"].clone()).unwrap();
    let Effect::Request { method, params } = native.invoke(
        selected["action_id"].as_str().unwrap(),
        &captured,
        Some(snapshot),
    ) else {
        panic!("ordinary generated Use did not select existing native Core dispatch");
    };
    client
        .input()
        .unwrap()
        .request(&client.boot_id, method, params)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.snapshot.as_ref().unwrap().focused_tab_id.as_deref()
        != target["tab"]["tab_id"].as_str()
        && Instant::now() < deadline
    {
        if client.receive().is_err() {
            break;
        }
    }
    assert_eq!(
        client.snapshot.as_ref().unwrap().focused_tab_id.as_deref(),
        target["tab"]["tab_id"].as_str(),
        "actual Core operation did not focus the captured target"
    );
    assert_eq!(
        client
            .snapshot
            .as_ref()
            .unwrap()
            .focused_workspace_id
            .as_deref(),
        target["workspace"]["workspace_id"].as_str()
    );
    let snapshot = core.request("session.snapshot", json!({}));
    assert!(snapshot["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tab| tab["tab_id"] == target["tab"]["tab_id"] && tab["focused"] == true));
    assert!(
        !retired.join("executed").exists(),
        "a disabled owner must not run as fallback"
    );
    assert!(
        !core.home.join("consumer/invocations").exists(),
        "the actionless neutral consumer must not become an action owner"
    );
}

#[test]
fn c22_printable_250_scalar_titles_keep_metadata_and_invoke_distinct_native_owners() {
    unusual_title_native_invocations(
        [
            "界".repeat(250),
            format!("{}別", "界".repeat(249)),
            "界".repeat(200),
        ],
        [format!("{}…", "界".repeat(199)), "界".repeat(200)],
    );
}

#[test]
fn c22_embedded_controls_escape_at_token_boundaries_without_changing_owner_or_metadata() {
    let prefix = format!("Read\n\u{1b}[31m {}", "界".repeat(181));
    unusual_title_native_invocations(
        [
            format!("{prefix}\n{} first", "界".repeat(50)),
            format!("{prefix}\n{} second", "界".repeat(50)),
            format!("Read\n\u{1b}[31m {}\n終", "界".repeat(180)),
        ],
        [
            format!("Read\\n\\u{{1b}}[31m {}…", "界".repeat(181)),
            format!("Read\\n\\u{{1b}}[31m {}\\n終", "界".repeat(180)),
        ],
    );
}

fn unusual_title_native_invocations(titles: [String; 3], display: [String; 2]) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use trailboss::terminal::{
        menus::{Controls, Response},
        Rect,
    };
    use trailboss_runtime::protocol::widgets::{Declaration, Node};
    let core = support::Core::new();
    let target = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let other = core.request(
        "workspace.create",
        json!({"label":"Other","cwd":core.home,"focus":true}),
    );
    let context = json!({"workspace_id":target["workspace"]["workspace_id"],
        "tab_id":target["tab"]["tab_id"],"pane_id":target["root_pane"]["pane_id"]});
    let owner = core.home.join("unusual-titles");
    std::fs::create_dir_all(&owner).unwrap();
    let actions = ["first", "second", "unshortened"];
    let mut manifest = String::from(
        "id = \"fixture.titles\"\nname = \"Unusual title owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n");
    for (action, title) in actions.iter().zip(&titles) {
        manifest.push_str(&format!(
            "[[actions]]\nid = {action:?}\ntitle = {}\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n",
            json!(format!("  {title}  "))));
    }
    std::fs::write(owner.join("trailboss-plugin.toml"), manifest).unwrap();
    std::fs::write(owner.join("record.py"),
        "import json, os\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let legacy = core.request("plugin.action.list", json!({}));
    for (action, title) in actions.iter().zip(&titles) {
        assert!(
            legacy["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["plugin_id"] == "fixture.titles"
                    && item["action_id"] == *action
                    && item["title"] == *title),
            "the unchanged legacy manifest normalizer must accept and retain {action}'s title"
        );
    }
    let catalog = core.request("ui.command.list", json!({}));
    let commands: Vec<_> = actions
        .iter()
        .zip(&titles)
        .map(|(action, title)| {
            let key = json!({"kind":"plugin","plugin_id":"fixture.titles","action_id":action});
            let command = catalog["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|command| command["key"] == key)
                .expect("valid unusual title omitted from discovery");
            assert_eq!(
                command["title"], *title,
                "discovery metadata must keep the full normalized title"
            );
            assert!(
                command["diagnostic"].is_null(),
                "displayable title must not make an action unavailable"
            );
            command
        })
        .collect();
    assert_ne!(commands[0]["key"], commands[1]["key"]);
    assert_ne!(commands[0]["fragment_name"], commands[1]["fragment_name"]);
    let consumer = core.home.join("title-consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    let declaration = json!({"herdr":"widget_tree","version":1,"zones":[],
    "fragments":[{"name":"command-palette","priority":1000000,"tree":{"type":"column","children":[{
        "type":"row","children":[{"type":"label","text":"Captured target","role":"primary"}],
        "context":context,"menu":{"type":"column","children":commands.iter().map(|command|
            json!({"type":"use","name":command["fragment_name"]})).collect::<Vec<_>>()}
    }]}}]});
    std::fs::write(consumer.join("trailboss-plugin.toml"),
        "id = \"fixture.title-consumer\"\nname = \"Neutral title consumer\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"commands\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"consumer.py\"]\n").unwrap();
    std::fs::write(
        consumer.join("consumer.py"),
        format!("print({:?}, flush=True)\n", declaration.to_string()),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":consumer,"enabled":true}));
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.boot_id.is_empty()
        || client.snapshot.is_none()
        || !client.declarations.as_ref().is_some_and(|declarations| {
            declarations.iter().any(|(owner, declaration)| {
                owner == "fixture.title-consumer"
                    && declaration
                        .fragments
                        .iter()
                        .any(|fragment| fragment.name == "command-palette")
            })
        })
    {
        assert!(
            Instant::now() < deadline,
            "actual native unusual-title publication not received"
        );
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    let mut native = NativeCore::default();
    let snapshot = client.snapshot.as_ref().unwrap();
    assert_eq!(
        snapshot.focused_tab_id.as_deref(),
        other["tab"]["tab_id"].as_str()
    );
    native.update_session(
        snapshot.focused_workspace_id.clone(),
        snapshot.focused_tab_id.clone(),
        true,
    );
    for (index, command) in commands.iter().enumerate() {
        let (actual_owner, tree) = native
            .fragment(declarations, command["fragment_name"].as_str().unwrap())
            .expect("issued unusual-title Use must resolve");
        assert_eq!(actual_owner, "fixture.titles");
        let Node::Row { children, .. } = tree else {
            panic!("generated action must remain an ordinary Row")
        };
        let Node::Label { text, .. } = &children[0] else {
            panic!("generated action must have its real display label")
        };
        assert_eq!(
            text,
            &display[usize::from(index == 2)],
            "safe display must preserve Unicode scalars and whole escape tokens"
        );
        assert!(text.chars().count() <= 200 && !text.chars().any(char::is_control));
        assert_eq!(
            text.ends_with('…'),
            index != 2,
            "ellipsis is only for a shortened label"
        );
    }
    for invalid in ["x".repeat(201), "Read\n\u{1b}[31m title".to_owned()] {
        let ordinary: Declaration = serde_json::from_value(json!({
            "herdr":"widget_tree","version":1,"zones":[{"zone":"list",
                "tree":{"type":"label","text":invalid,"role":"primary"}}]
        }))
        .unwrap();
        assert!(
            !ordinary.validate_producer(|_| false),
            "display formatting is not permission to accept arbitrary invalid producer labels"
        );
    }
    let viewport = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let mut input = client.input().unwrap();
    for (index, action) in actions.iter().enumerate() {
        let mut controls = Controls::default();
        assert!(controls.open_palette(&native, declarations, None));
        assert!(matches!(
            controls.key(
                &native,
                declarations,
                viewport,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Response::Consumed
        ));
        for _ in 0..index {
            assert!(matches!(
                controls.key(
                    &native,
                    declarations,
                    viewport,
                    KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
                ),
                Response::Consumed
            ));
        }
        let Response::Invoke(selected) = controls.key(
            &native,
            declarations,
            viewport,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ) else {
            panic!("native ordinary Use did not select the unusual-title owner action");
        };
        assert!(matches!(
            native.invoke(&selected.action, &selected.context, Some(snapshot)),
            Effect::Plugin
        ));
        input
            .invoke(
                &client.boot_id,
                &selected.plugin,
                &selected.action,
                &selected.context,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let logs = core.request("plugin.log.list", json!({"plugin_id":"fixture.titles"}));
            if logs["logs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|log| log["action_id"] == *action && log["status"] == "succeeded")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "real unusual-title action {action} did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let records: Vec<Value> = std::fs::read_to_string(owner.join("executed.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), actions.len());
    for action in actions {
        let matching: Vec<_> = records
            .iter()
            .filter(|record| record["action"] == action)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "a display-identical row must invoke only its actual action once"
        );
        assert_eq!(matching[0]["owner"], "fixture.titles");
        for field in ["workspace_id", "tab_id", "pane_id"] {
            assert_eq!(matching[0]["context"][field], context[field]);
        }
    }
}
