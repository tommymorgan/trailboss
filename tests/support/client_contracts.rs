use super::support;
use serde_json::{json, Value};
use std::{
    process::Command,
    time::{Duration, Instant},
};

fn consume_contract() {
    let core = support::Core::new();
    let first = core.request(
        "workspace.create",
        json!({"label":"Neutral workspace","cwd":core.home,"focus":true}),
    );
    let workspace = first["workspace"]["workspace_id"].as_str().unwrap();
    let original = first["tab"]["tab_id"].as_str().unwrap();
    let second = core.request(
        "tab.create",
        json!({"workspace_id":workspace,"label":"Neutral target","cwd":core.home,"focus":false}),
    );
    let target = second["tab"]["tab_id"].as_str().unwrap();
    let pane = second["root_pane"]["pane_id"].as_str().unwrap();
    let root = env!("CARGO_MANIFEST_DIR");
    core.request(
        "plugin.link",
        json!({"path":format!("{root}/plugins/agent-tree"),"enabled":true}),
    );
    core.request(
        "plugin.link",
        json!({"path":format!("{root}/plugins/stock-ui"),"enabled":true}),
    );
    let tone_plugin = core.home.join("semantic-tone");
    std::fs::create_dir_all(&tone_plugin).unwrap();
    std::fs::write(tone_plugin.join("trailboss-plugin.toml"), "id = \"fixture.semantic-tone\"\nname = \"Semantic tone\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tone\"\nsurface = \"sidebar\"\ncommand = [\"sh\", \"producer.sh\"]\nstream = true\n").unwrap();
    std::fs::write(tone_plugin.join("producer.sh"), "printf '%s\\n' '{\"herdr\":\"widget_tree\",\"version\":1,\"zones\":[{\"zone\":\"footer\",\"tree\":{\"type\":\"label\",\"text\":\"Neutral tone\",\"role\":\"muted\",\"tone\":\"fg-cyan\"}}]}'\nwhile read -r line; do :; done\n").unwrap();
    core.request("plugin.link", json!({"path":tone_plugin,"enabled":true}));
    let malicious = core.home.join("terminal-role");
    std::fs::create_dir_all(&malicious).unwrap();
    let manifest = std::fs::read_to_string(tone_plugin.join("trailboss-plugin.toml"))
        .unwrap()
        .replace("fixture.semantic-tone", "fixture.terminal-role");
    std::fs::write(malicious.join("trailboss-plugin.toml"), manifest).unwrap();
    let bad = json!({"herdr":"widget_tree","version":1,"zones":[{"zone":"footer_right","tree":{"type":"label","text":"Reject terminal role","role":"\u{1b}[31m"}}]});
    std::fs::write(
        malicious.join("producer.sh"),
        format!("printf '%s\\n' '{bad}'\nwhile read -r line; do :; done\n"),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":malicious,"enabled":true}));
    core.request("pane.send_text", json!({"pane_id":pane,"text":"printf '\\033[2J\\033[H\\033[1;38;2;17;34;51mneutral-%s\\033[0m\\033[4;7H\\033[6 q' live; sleep 30\n"}));
    let output = core.home.join("neutral-wire.json");
    let socket = core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    ));
    let mut child = Command::new("python3")
        .args([
            format!("{root}/tests/support/neutral_consumer.py"),
            socket.to_string_lossy().into_owned(),
            target.to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "independent consumer failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("independent consumer exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let state: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(state["action"]["action_id"], "focus-tab");
    assert_eq!(state["action"]["context"]["tab_id"], target);
    assert_eq!(state["snapshots"][0]["focused_tab_id"], original);
    assert_eq!(
        state["snapshots"].as_array().unwrap().last().unwrap()["focused_tab_id"],
        target
    );
    let actual = core.request("session.snapshot", json!({}));
    assert!(actual["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tab| tab["tab_id"] == target && tab["focused"] == true));
    let declarations = state["declarations"].as_array().unwrap();
    let agent = declarations
        .iter()
        .find(|item| item["plugin_id"] == "tommymorgan.agent-tree")
        .unwrap();
    assert_eq!(agent["declaration"]["zones"][0]["tree"]["type"], "column");
    let stock = declarations
        .iter()
        .find(|item| item["plugin_id"] == "tommymorgan.stock-ui")
        .unwrap();
    assert_eq!(stock["declaration"]["composition"]["tree"]["type"], "row");
    let tone = declarations
        .iter()
        .find(|item| item["plugin_id"] == "fixture.semantic-tone")
        .unwrap();
    assert_eq!(
        tone["declaration"]["zones"][0]["tree"],
        json!({"type":"label","text":"Neutral tone","role":"muted","tone":"fg-cyan"})
    );
    fn neutral(value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        !["x", "y", "width", "height", "rect", "fg", "bg", "modifier", "ratatui"]
                            .contains(&key.as_str()),
                        "renderer field: {key}"
                    );
                    neutral(value);
                }
            }
            Value::Array(array) => {
                for value in array {
                    neutral(value);
                }
            }
            Value::String(text) => assert!(
                !text.chars().any(char::is_control),
                "terminal control in declaration"
            ),
            _ => {}
        }
    }
    for item in state["declaration_history"].as_array().unwrap() {
        neutral(&item["declaration"]);
    }
    let frame = state["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|frame| {
            frame["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| cell["symbol"].as_str().unwrap())
                .collect::<String>()
                .contains("neutral-live")
        })
        .unwrap();
    assert_eq!(frame["cursor"], json!([6, 3, 1, 6]));
    let cell = frame["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|cell| cell["symbol"] == "n")
        .unwrap();
    assert_eq!(cell["fg"], 0x02112233u32);
    assert_eq!(cell["modifier"].as_u64().unwrap() & 1, 1);
    assert_eq!(frame["panes"][0]["id"], pane);
    assert_eq!(frame["panes"][0]["focused"], 1);
}

#[test]
fn independent_consumer_obtains_declarations_surface_and_core_action_effect() {
    consume_contract();
}

#[test]
fn living_consume_shared_ui_and_pane_state_independently_of_terminal_renderer() {
    let feature = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/features/client-contracts.feature"
    ))
    .unwrap();
    let steps: Vec<_> = feature
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("Scenario:")
                || line.starts_with("Given ")
                || line.starts_with("When ")
                || line.starts_with("Then ")
        })
        .collect();
    assert_eq!(steps,[
        "Scenario: Consume shared UI and pane state independently of the terminal renderer",
        "Given the shared Core owns the live session state and action semantics, each client owns its renderer, and the Core publishes widget declarations and pane Surface state through the shared client contract",
        "When an independent protocol consumer decodes that stream using the documented contract without the terminal renderer",
        "Then it obtains the declared structure, action identities, semantic styling and pane terminal state, the declaration payload itself contains no ratatui types, terminal escape sequences or fixed terminal-cell geometry, and Core-owned state and action semantics remain independent of each client-owned renderer"
    ]);
    consume_contract();
}
