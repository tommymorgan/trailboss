use super::Core;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub fn declaration(replaced: bool) -> Value {
    let label = |text| json!({"type":"label","text":text,"role":"label"});
    let tree = if replaced {
        json!({"type":"column","children":[
            {"type":"row","children":[label("NEW")]},
            {"type":"column","children":[label("NESTED"), label("ONLY")]}
        ]})
    } else {
        json!({"type":"row","children":[label("OLD"), label("TREE")]})
    };
    json!({"herdr":"widget_tree","version":1,"zones":[],"composition":{"priority":100,"tree":tree}})
}
pub fn enable(core: &Core) -> PathBuf {
    let plugin = core.home.join("replacement-producer");
    std::fs::create_dir_all(&plugin).unwrap();
    let schema = concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/agent-tree/widgets.ts");
    let source = format!(
        r#"import {{ WidgetDeclaration }} from {schema:?};
const emit = async (tree) => {{
  console.log(JSON.stringify(WidgetDeclaration.parse(tree)));
  await Bun.write("emissions", (await Bun.file("emissions").exists() ? await Bun.file("emissions").text() : "") + `${{process.pid}} ${{process.env.HERDR_SOCKET_PATH}}\n`);
}};
await emit({});
while (!(await Bun.file("replace").exists())) await Bun.sleep(10);
await emit({});
await Bun.stdin.text();
"#,
        declaration(false),
        declaration(true)
    );
    std::fs::write(plugin.join("producer.ts"), source).unwrap();
    std::fs::write(plugin.join("herdr-plugin.toml"), "id = \"fixture.replacement\"\nname = \"Whole tree replacement\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tree\"\nsurface = \"sidebar\"\ncommand = [\"bun\", \"producer.ts\"]\nstream = true\n").unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    plugin
}
pub fn replace(plugin: &std::path::Path) {
    std::fs::write(plugin.join("replace"), "replace whole tree").unwrap();
}
pub fn assert_same_producer_and_socket(core: &Core, plugin: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let emissions = loop {
        let text = std::fs::read_to_string(plugin.join("emissions")).unwrap_or_default();
        if text.lines().count() == 2 {
            break text;
        }
        assert!(
            Instant::now() < deadline,
            "producer did not emit replacement: {text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let lines: Vec<_> = emissions.lines().collect();
    assert_eq!(lines[0], lines[1], "producer process/socket changed");
    let (pid, socket) = lines[0].split_once(' ').unwrap();
    assert!(
        std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "producer exited"
    );
    assert_eq!(
        socket,
        core.home
            .join(format!("herdr-fork/sessions/{}/herdr.sock", core.name))
            .to_str()
            .unwrap()
    );
}
pub fn assert_replacement(rows: &[String]) {
    assert_eq!(rows[0].trim_end(), "NEW");
    assert_eq!(rows[1].trim_end(), "NESTED");
    assert_eq!(rows[2].trim_end(), "ONLY");
    assert!(
        rows[3..].iter().all(|row| row.trim().is_empty()),
        "old content remains: {rows:?}"
    );
    assert!(!rows
        .iter()
        .any(|row| row.contains("OLD") || row.contains("TREE")));
}
