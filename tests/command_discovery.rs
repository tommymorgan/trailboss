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
    std::fs::write(owner.join("trailboss-plugin.toml"), "id = \"demo.owner\"\nname = \"Demo owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"record-target\"\ntitle = \"Record target\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n").unwrap();
    std::fs::write(owner.join("record.py"), format!("import json, os\nwith open({:?}, 'a') as output:\n output.write(json.dumps({{'owner':os.environ['HERDR_PLUGIN_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])}})+'\\n')\n", recorded.to_str().unwrap())).unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
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
