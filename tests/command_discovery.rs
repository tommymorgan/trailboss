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
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    for (withdrawal, expected_error) in [
        ("action", "plugin_action_not_found"),
        ("owner", "plugin_not_found"),
        ("manifest", "plugin_manifest_unavailable"),
    ] {
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
