use super::support::Core;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use trailboss_runtime::protocol::widgets::{Declaration, Zone};

pub struct Adversary {
    pub core: Core,
    pub plugin: PathBuf,
    pub tab: String,
    pub action: String,
}
impl Adversary {
    pub fn new(action: &str) -> Self {
        let core = Core::new();
        let created = core.request(
            "workspace.create",
            json!({"label":"Safe","cwd":core.home,"focus":true}),
        );
        let tab = created["tab"]["tab_id"].as_str().unwrap().to_owned();
        let workspace = created["workspace"]["workspace_id"].as_str().unwrap();
        core.request("tab.rename", json!({"tab_id":tab,"label":"Safe"}));
        let plugin = core.home.join("adversary");
        std::fs::create_dir_all(&plugin).unwrap();
        let source = format!(
            r#"import {{createTrailbossStore}} from {store:?};
import {{appendFileSync}} from 'node:fs';
const store=createTrailbossStore(process.env.HERDR_SOCKET_PATH);
await store.ready();
function emit(stage) {{
  console.log(JSON.stringify({{herdr:'widget_tree',version:1,zones:[{{zone:stage==='before'?'list':'footer_right',tree:{{type:'row',action:{action:?},context:{{tab_id:{tab:?},workspace_id:{workspace:?}}},children:[{{type:'label',role:'label',text:stage+' '+{short:?}}}]}}}}]}}));
  appendFileSync('emissions',JSON.stringify({{stage,pid:process.pid}})+'\n');
}}
let stage='before';emit(stage);
void (async()=>{{while(!(await Bun.file('replace').exists()))await Bun.sleep(10);stage='after';emit(stage)}})();
let pending='';const decoder=new TextDecoder();
for await(const chunk of Bun.stdin.stream()) {{
  pending+=decoder.decode(chunk,{{stream:true}});let end;
  while((end=pending.indexOf('\n'))>=0) {{
    const action=JSON.parse(pending.slice(0,end));pending=pending.slice(end+1);
    await store.commands.request('tab.rename',{{tab_id:{tab:?},label:'HIJACKED-'+stage}});
    appendFileSync('attempts',JSON.stringify({{stage,action_id:action.action_id,id:action.id}})+'\n');
    console.log(JSON.stringify({{herdr:'action_result',id:action.id,ok:true}}));
  }}
}}
"#,
            store = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/packages/trailboss-store/index.ts"
            ),
            action = format!("host:{action}"),
            short = action
        );
        std::fs::write(plugin.join("producer.ts"), source).unwrap();
        let manifest = format!(
            r#"id = "fixture.adversary"
name = "Reserved action adversary"
version = "0.1.0"
min_herdr_version = "0.9.0"
[[actions]]
id = "host:{action}"
title = "Attempt reserved semantics"
command = ["bun", "producer.ts"]
[[actions]]
id = "probe"
title = "Prove adversary behavior"
command = ["bun", "producer.ts"]
[[ui]]
id = "tree"
surface = "sidebar"
command = ["bun", "producer.ts"]
stream = true
stream_actions = true
"#
        );
        std::fs::write(plugin.join("trailboss-plugin.toml"), manifest).unwrap();
        core.request("plugin.link", json!({"path":plugin,"enabled":true}));
        Self {
            core,
            plugin,
            tab,
            action: action.to_owned(),
        }
    }
    pub fn label(&self) -> String {
        let snapshot = self.core.request("session.snapshot", json!({}));
        snapshot["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tab| tab["tab_id"] == self.tab)
            .unwrap()["label"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    // Positive controls execute exactly the same installed handler body in both revisions.
    pub fn prove_handler(&self, stage: &str) {
        self.core.request(
            "plugin.action.invoke",
            json!({"plugin_id":"fixture.adversary","action_id":"probe"}),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.label() == format!("HIJACKED-{stage}") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "adversary handler did not really change tab state"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self.core
            .request("tab.rename", json!({"tab_id":self.tab,"label":"Safe"}));
    }
    pub fn replace(&self) {
        std::fs::write(
            self.plugin.join("replace"),
            "replace label, placement and handler revision",
        )
        .unwrap();
    }
    pub fn assert_unchanged(&self) {
        assert_eq!(
            self.label(),
            "Safe",
            "plugin hijacked reserved Core behavior"
        );
        let text = std::fs::read_to_string(self.plugin.join("attempts")).unwrap();
        let attempts: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(attempts.len(), 2);
        assert!(
            attempts.iter().all(|entry| entry["action_id"] == "probe"),
            "reserved action entered producer handler: {text}"
        );
        assert_eq!(attempts[0]["stage"], "before");
        assert_eq!(attempts[1]["stage"], "after");
        let emissions = std::fs::read_to_string(self.plugin.join("emissions")).unwrap();
        let emissions: Vec<Value> = emissions
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(emissions.len(), 2);
        assert_eq!(emissions[0]["pid"], emissions[1]["pid"]);
    }
    pub fn declaration(&self, client: &mut trailboss::Client, stage: &str) -> Declaration {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            client.receive().unwrap();
            if !client.boot_id.is_empty() {
                if let Some(tree) = client.declarations.as_ref().and_then(|trees| {
                    trees.iter().find(|(id, tree)| {
                        id == "fixture.adversary"
                            && serde_json::to_string(tree)
                                .unwrap()
                                .contains(&format!("{stage} {}", self.action))
                    })
                }) {
                    return tree.1.clone();
                }
            }
            assert!(Instant::now() < deadline, "adversary declaration missing");
        }
    }
    pub fn assert_placement(&self, tree: &Declaration, stage: &str) {
        assert_eq!(
            tree.zones[0].zone,
            if stage == "before" {
                Zone::List
            } else {
                Zone::FooterRight
            }
        );
        assert!(serde_json::to_string(tree)
            .unwrap()
            .contains(&format!("{stage} {}", self.action)));
    }
}
