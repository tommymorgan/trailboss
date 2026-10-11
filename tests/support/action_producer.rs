use super::support::Core;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub struct FocusProducer {
    pub core: Core,
    pub plugin: PathBuf,
    pub target: String,
    pub workspace: String,
    pub original: String,
    boot_id: String,
}
impl FocusProducer {
    pub fn new() -> Self {
        let core = Core::new();
        let first = core.request(
            "workspace.create",
            json!({"label":"Action workspace","cwd":core.home,"focus":true}),
        );
        let workspace = first["workspace"]["workspace_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let original = first["tab"]["tab_id"].as_str().unwrap().to_owned();
        let second = core.request("tab.create", json!({"workspace_id":workspace,"label":"Unfocused target","cwd":core.home,"focus":false}));
        let target = second["tab"]["tab_id"].as_str().unwrap().to_owned();
        let plugin = core.home.join("observed-producer");
        std::fs::create_dir_all(&plugin).unwrap();
        let root = env!("CARGO_MANIFEST_DIR");
        // Observe bytes at the real retained producer boundary, without replacing its behavior.
        let source = format!(
            r#"import {{ appendFileSync }} from 'node:fs';
const child = Bun.spawn(['bun', {producer:?}, '--watch'], {{stdin:'pipe',stdout:'pipe',stderr:'inherit'}});
appendFileSync('envelopes', JSON.stringify({{kind:'started',pid:child.pid,socket:process.env.HERDR_SOCKET_PATH}})+'\n');
async function lines(stream, handle) {{
  let buffer = '';
  const decoder = new TextDecoder();
  for await (const bytes of stream) {{
    buffer += decoder.decode(bytes, {{stream:true}});
    let end;
    while ((end = buffer.indexOf('\n')) >= 0) {{
      const line = buffer.slice(0,end); buffer = buffer.slice(end+1);
      await handle(line);
    }}
  }}
}}
void lines(child.stdout, async line => {{
  const value = JSON.parse(line);
  if (value.herdr === 'action_result') appendFileSync('envelopes',JSON.stringify({{kind:'result',...value}})+'\n');
  process.stdout.write(line+'\n');
}});
await lines(Bun.stdin.stream(), async line => {{
  appendFileSync('envelopes',JSON.stringify({{kind:'request',...JSON.parse(line)}})+'\n');
  child.stdin.write(line+'\n'); await child.stdin.flush();
}});
child.kill(); await child.exited;
"#,
            producer = format!("{root}/plugins/agent-tree/agent_tree.ts")
        );
        std::fs::write(plugin.join("observe.ts"), source).unwrap();
        let manifest =
            std::fs::read_to_string(format!("{root}/plugins/agent-tree/trailboss-plugin.toml"))
                .unwrap()
                .replace("tommymorgan.agent-tree", "fixture.observed-focus")
                .replace(
                    r#"command = ["bun", "agent_tree.ts", "--watch"]"#,
                    r#"command = ["bun", "observe.ts"]"#,
                );
        std::fs::write(plugin.join("trailboss-plugin.toml"), manifest).unwrap();
        core.request("plugin.link", json!({"path":plugin,"enabled":true}));
        let mut observer = core.attach();
        let deadline = Instant::now() + Duration::from_secs(5);
        while observer.boot_id.is_empty()
            || observer
                .declarations
                .as_ref()
                .is_none_or(|trees| !trees.iter().any(|(id, _)| id == "fixture.observed-focus"))
        {
            observer.receive().unwrap();
            assert!(Instant::now() < deadline, "Core boot identity missing");
        }
        let boot_id = observer.boot_id.clone();
        let fixture = Self {
            core,
            plugin,
            target,
            workspace,
            original,
            boot_id,
        };
        assert_eq!(
            fixture.focused(),
            fixture.original,
            "Given must be unfocused"
        );
        fixture
    }
    pub fn focused(&self) -> String {
        let snapshot = self.core.request("session.snapshot", json!({}));
        snapshot["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tab| tab["focused"] == true)
            .unwrap()["tab_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    pub fn assert_completed(&self) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let text = std::fs::read_to_string(self.plugin.join("envelopes")).unwrap_or_default();
            let entries: Vec<Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            if let Some(result) = entries.iter().find(|entry| entry["kind"] == "result") {
                let request = entries
                    .iter()
                    .find(|entry| entry["kind"] == "request")
                    .unwrap();
                assert_eq!(request["herdr"], "action");
                assert_eq!(
                    request["action_id"], "focus-tab",
                    "producer action identity"
                );
                assert_eq!(
                    request["context"]["tab_id"], self.target,
                    "producer tab context"
                );
                assert_eq!(request["context"]["workspace_id"], self.workspace);
                assert_eq!(request["context"]["invocation_source"], "plugin_ui");
                let correlation = request["context"]["correlation_id"]
                    .as_str()
                    .expect("producer invocation correlation missing");
                let endpoint = correlation
                    .strip_prefix(&format!("endpoint:{}:", self.boot_id))
                    .expect("producer correlation belongs to a different Core boot");
                let (client, invocation) = endpoint
                    .split_once(':')
                    .expect("producer correlation lacks endpoint client and invocation");
                client
                    .parse::<u64>()
                    .expect("producer correlation has an invalid endpoint client");
                assert_eq!(
                    invocation, "widget-action-1",
                    "producer context must correlate to the activated declaration request"
                );
                assert_eq!(
                    result["id"], request["id"],
                    "result must correlate to stdin request"
                );
                assert_eq!(
                    result["ok"], true,
                    "real producer rejected action: {result}"
                );
                let logs = self.core.request(
                    "plugin.log.list",
                    json!({"plugin_id":"fixture.observed-focus"}),
                );
                let log = logs["logs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|log| log["log_id"] == request["id"])
                    .unwrap();
                if log["status"] == "succeeded" {
                    assert_eq!(
                        self.focused(),
                        self.target,
                        "successful focus result did not focus requested tab"
                    );
                    let started = &entries[0];
                    assert!(
                        std::path::Path::new(&format!("/proc/{}", started["pid"])).exists(),
                        "retained producer exited"
                    );
                    assert_eq!(
                        started["socket"],
                        self.core
                            .home
                            .join(format!(
                                "trailboss/sessions/{}/trailboss.sock",
                                self.core.name
                            ))
                            .to_str()
                            .unwrap()
                    );
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "no correlated successful producer result: {text}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
