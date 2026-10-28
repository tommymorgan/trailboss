#[path = "support/restart.rs"]
mod restart;
mod support;

use serde_json::{json, Value};
use std::{
    process::Command,
    time::{Duration, Instant},
};
use trailboss::core::{Core as NativeCore, Effect};
use trailboss_runtime::protocol::widgets::Context;

#[path = "support/console.rs"]
mod console;

#[test]
fn dedicated_palette_hides_targetless_operations_and_this_clients_unavailable_previous_visit() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use trailboss::{
        sidebar::Declarations,
        terminal::{menus::Response, FrameRenderer, Rect},
    };
    use trailboss_runtime::protocol::widgets::Node;

    fn search(
        renderer: &mut FrameRenderer,
        native: &NativeCore,
        declarations: &Declarations,
        text: &str,
    ) {
        for character in text.chars() {
            renderer.controls.key(
                native,
                declarations,
                Rect::viewport(100, 30),
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
            );
        }
    }
    fn enter(
        renderer: &mut FrameRenderer,
        native: &NativeCore,
        declarations: &Declarations,
    ) -> Response {
        renderer.controls.key(
            native,
            declarations,
            Rect::viewport(100, 30),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
    }
    fn rows(
        renderer: &mut FrameRenderer,
        native: &NativeCore,
        declarations: &Declarations,
    ) -> Vec<String> {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(
                &mut bytes,
                None,
                declarations,
                native,
                Rect::viewport(100, 30),
            )
            .unwrap();
        let mut screen = ghostty_vt::Terminal::new(100, 30, 0).unwrap();
        screen.write(&bytes);
        screen
            .screen_text_rows()
            .unwrap()
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .flat_map(|cell| cell.graphemes.iter())
                    .filter_map(|codepoint| char::from_u32(*codepoint))
                    .collect()
            })
            .collect()
    }
    fn uses(node: &Node, names: &mut Vec<String>) {
        match node {
            Node::Use { name } => names.push(name.clone()),
            Node::Column { children } | Node::List { entries: children } => {
                for child in children {
                    uses(child, names);
                }
            }
            Node::Row { children, menu, .. } => {
                for child in children {
                    uses(child, names);
                }
                if let Some(menu) = menu {
                    uses(menu, names);
                }
            }
            _ => {}
        }
    }
    fn has_tab_context(node: &Node, tab: &str) -> bool {
        match node {
            Node::Row {
                context,
                children,
                menu,
                ..
            } => {
                context.tab_id.as_deref() == Some(tab)
                    || children.iter().any(|child| has_tab_context(child, tab))
                    || menu
                        .as_deref()
                        .is_some_and(|menu| has_tab_context(menu, tab))
            }
            Node::Column { children } | Node::List { entries: children } => {
                children.iter().any(|child| has_tab_context(child, tab))
            }
            Node::Flex { child, .. } | Node::Modal { child } => has_tab_context(child, tab),
            _ => false,
        }
    }

    let core = support::Core::new();
    let resource = core.request(
        "workspace.create",
        json!({
            "label":"Visibility workspace","cwd":core.home,"focus":true
        }),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":resource["tab"]["tab_id"],"label":"First"}),
    );
    let second = core.request("tab.create", json!({
        "workspace_id":resource["workspace"]["workspace_id"],"label":"Second","cwd":core.home,"focus":true
    }));
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        core.request(
            "plugin.link",
            json!({
                "path":format!("{}/plugins/{plugin}", env!("CARGO_MANIFEST_DIR")),"enabled":true
            }),
        );
    }
    let catalog = core.request("ui.command.list", json!({}));
    let command = |action: &str, mode: Option<&str>| {
        let mut key = json!({"kind":"core","action_id":action});
        if let Some(mode) = mode {
            key["mode"] = json!(mode);
        }
        catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == key)
            .unwrap()
            .clone()
    };
    let new_tab = command("host:new-tab", None);
    let rename_tab = command("host:rename-tab", None);
    let previous = command("host:focus", Some("previous"));
    let restart = command("host:restart-idle-agents", None);
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(8);
    while client.declarations.as_ref().is_none_or(|declarations| {
        NativeCore::default()
            .fragment(declarations, "command-palette")
            .is_none()
            || !declarations.iter().any(|(owner, declaration)| {
                owner == "tommymorgan.stock-ui" && declaration.composition.is_some()
            })
    }) || client.snapshot.is_none()
    {
        assert!(
            Instant::now() < deadline,
            "actual dedicated consumer and stock application composition did not publish"
        );
        client.receive().unwrap();
    }
    let mut native = NativeCore::default();
    native.update_session(
        resource["workspace"]["workspace_id"]
            .as_str()
            .map(str::to_owned),
        second["tab"]["tab_id"].as_str().map(str::to_owned),
        true,
    );
    let declarations = client.declarations.as_ref().unwrap();
    assert_eq!(
        native.fragment(declarations, "command-palette").unwrap().0,
        "tommymorgan.command-palette"
    );
    // With resources available, these entries navigate to targets. They must
    // not become executable operations by borrowing the observer/current tab.
    for (unavailable, workspace_chosen) in [(&new_tab, false), (&rename_tab, true)] {
        let mut renderer = FrameRenderer::default();
        assert!(renderer.controls.open_palette(&native, declarations, None));
        search(
            &mut renderer,
            &native,
            declarations,
            unavailable["title"].as_str().unwrap(),
        );
        assert!(matches!(
            enter(&mut renderer, &native, declarations),
            Response::Consumed
        ));
        if workspace_chosen {
            search(&mut renderer, &native, declarations, "Visibility workspace");
            assert!(matches!(
                enter(&mut renderer, &native, declarations),
                Response::Consumed
            ));
        }
        assert!(!native.editing(), "missing target opened a name flow");
        search(
            &mut renderer,
            &native,
            declarations,
            unavailable["title"].as_str().unwrap(),
        );
        let painted = rows(&mut renderer, &native, declarations);
        assert!(
            painted.iter().any(|row| row.contains("No matches")),
            "targetless operation leaf must be absent, not disabled: {painted:?}"
        );
        assert!(matches!(
            enter(&mut renderer, &native, declarations),
            Response::Consumed
        ));
        assert!(!native.editing());

        // A valid explicit choice must still reach its normal name operation.
        let mut valid = FrameRenderer::default();
        assert!(valid.controls.open_palette(&native, declarations, None));
        for title in [
            unavailable["title"].as_str().unwrap(),
            "Visibility workspace",
        ] {
            search(&mut valid, &native, declarations, title);
            assert!(matches!(
                enter(&mut valid, &native, declarations),
                Response::Consumed
            ));
        }
        if workspace_chosen {
            search(&mut valid, &native, declarations, "First");
            assert!(matches!(
                enter(&mut valid, &native, declarations),
                Response::Consumed
            ));
        }
        let Response::Prepare(selected) = enter(&mut valid, &native, declarations) else {
            panic!("valid explicit target lost its existing name operation");
        };
        assert_eq!(selected.action, unavailable["key"]["action_id"]);
        assert_eq!(
            selected.context.workspace_id.as_deref(),
            resource["workspace"]["workspace_id"].as_str()
        );
        if workspace_chosen {
            assert_eq!(
                selected.context.tab_id.as_deref(),
                resource["tab"]["tab_id"].as_str()
            );
            assert_ne!(
                selected.context.tab_id.as_deref(),
                native.focused_tab_id.as_deref()
            );
        }
    }

    // Empty resources are reachable after closing the owned spaces while both
    // endpoints remain attached. At that boundary even navigation must vanish.
    // Stock composition above is essential: the non-composed headless fallback
    // deliberately creates a default space for attached shell clients.
    let original_root = native
        .fragment(declarations, "command-palette")
        .unwrap()
        .1
        .clone();
    let before_close = core.request("session.snapshot", json!({}));
    let resource_summary = |response: &Value| {
        json!({
            "focused_workspace_id":response["snapshot"]["focused_workspace_id"],
            "workspace_ids":response["snapshot"]["workspaces"].as_array().unwrap().iter()
                .map(|workspace| workspace["workspace_id"].clone()).collect::<Vec<_>>()
        })
    };
    let mut close_results = Vec::new();
    for workspace in before_close["snapshot"]["workspaces"].as_array().unwrap() {
        let workspace_id = workspace["workspace_id"].as_str().unwrap_or_else(|| {
            panic!("actual workspace snapshot lacks stable workspace_id: {workspace}")
        });
        let closed = core.request("workspace.close", json!({"workspace_id":workspace_id}));
        let after_close = core.request("session.snapshot", json!({}));
        assert_eq!(
            closed["type"],
            "ok",
            "owned close failed; target={workspace_id}, reply={closed}, before={}, after={}",
            resource_summary(&before_close),
            resource_summary(&after_close)
        );
        close_results.push(json!({"workspace_id":workspace_id,"reply":closed,"after":resource_summary(&after_close)}));
    }
    let after_close = core.request("session.snapshot", json!({}));
    assert!(after_close["snapshot"]["workspaces"].as_array().unwrap().is_empty(),
        "actual resources remained after successful stable-ID closes; before={}, closes={close_results:?}, after={}",
        resource_summary(&before_close), resource_summary(&after_close));
    let deadline = Instant::now() + Duration::from_secs(8);
    while client
        .snapshot
        .as_ref()
        .is_none_or(|snapshot| !snapshot.workspaces.is_empty())
        || client.declarations.as_ref().is_none_or(|declarations| {
            native
                .fragment(declarations, "command-palette")
                .is_none_or(|(_, tree)| {
                    let mut names = Vec::new();
                    uses(tree, &mut names);
                    tree == &original_root
                        || names.iter().any(|name| {
                            *name == new_tab["fragment_name"]
                                || *name == rename_tab["fragment_name"]
                        })
                })
        })
    {
        assert!(Instant::now() < deadline,
            "dedicated consumer did not remove targeted branches after all stable-ID closes; endpoint_workspace_count={:?}, issued_uses={:?}",
            client.snapshot.as_ref().map(|snapshot| snapshot.workspaces.len()),
            client.declarations.as_ref().and_then(|declarations| {
                native.fragment(declarations, "command-palette").map(|(_, tree)| {
                    let mut names = Vec::new();
                    uses(tree, &mut names);
                    names
                })
            }));
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    let empty_root = native
        .fragment(declarations, "command-palette")
        .unwrap()
        .1
        .clone();
    let mut names = Vec::new();
    uses(&empty_root, &mut names);
    assert!(!names.iter().any(|name| *name == new_tab["fragment_name"]));
    assert!(!names
        .iter()
        .any(|name| *name == rename_tab["fragment_name"]));
    let empty_native = NativeCore::default();
    let mut available = FrameRenderer::default();
    available
        .controls
        .open_palette(&empty_native, declarations, None);
    search(
        &mut available,
        &empty_native,
        declarations,
        restart["title"].as_str().unwrap(),
    );
    let Response::Invoke(selected) = enter(&mut available, &empty_native, declarations) else {
        panic!("satisfied global command disappeared in an empty session");
    };
    let mut empty_native = empty_native;
    let Effect::Request { method, params } =
        empty_native.invoke(&selected.action, &selected.context, None)
    else {
        panic!("eligible global operation did not reach Core");
    };
    assert_eq!(method, "agent.restart_idle");
    assert_eq!(core.request(method, params)["type"], "agent_restart_idle");

    // Two clients observe the same actual live snapshot. Only the client that
    // visited both tabs may display Previous; server/observer state is not MRU.
    let first = core.request(
        "workspace.create",
        json!({
            "label":"History workspace","cwd":core.home,"focus":true
        }),
    );
    let mut experienced = NativeCore::default();
    experienced.update_session(
        first["workspace"]["workspace_id"]
            .as_str()
            .map(str::to_owned),
        first["tab"]["tab_id"].as_str().map(str::to_owned),
        true,
    );
    let second = core.request("tab.create", json!({
        "workspace_id":first["workspace"]["workspace_id"],"label":"History second","cwd":core.home,"focus":true
    }));
    experienced.update_session(
        first["workspace"]["workspace_id"]
            .as_str()
            .map(str::to_owned),
        second["tab"]["tab_id"].as_str().map(str::to_owned),
        true,
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while client.snapshot.as_ref().is_none_or(|snapshot| {
        snapshot.focused_tab_id.as_deref() != second["tab"]["tab_id"].as_str()
    }) || client.declarations.as_ref().is_none_or(|declarations| {
        native
            .fragment(declarations, "command-palette")
            .is_none_or(|(_, tree)| {
                tree == &empty_root
                    || !has_tab_context(tree, second["tab"]["tab_id"].as_str().unwrap())
            })
    }) {
        assert!(
            Instant::now() < deadline,
            "actual history snapshot did not arrive"
        );
        client.receive().unwrap();
    }
    let snapshot = client.snapshot.as_ref().unwrap();
    let declarations = client.declarations.as_ref().unwrap();
    let mut fresh = NativeCore::default();
    fresh.update_session(
        snapshot.focused_workspace_id.clone(),
        snapshot.focused_tab_id.clone(),
        true,
    );
    assert!(fresh.previous_tab(snapshot).is_none());
    assert_eq!(
        experienced.previous_tab(snapshot),
        first["tab"]["tab_id"].as_str()
    );
    let menu_context: Context = serde_json::from_value(json!({
        "workspace_id":first["workspace"]["workspace_id"],"tab_id":first["tab"]["tab_id"],
        "pane_id":first["root_pane"]["pane_id"]
    }))
    .unwrap();
    let mut unavailable = FrameRenderer::default();
    unavailable
        .controls
        .open_palette(&fresh, declarations, Some(&menu_context));
    search(
        &mut unavailable,
        &fresh,
        declarations,
        previous["title"].as_str().unwrap(),
    );
    let painted = rows(&mut unavailable, &fresh, declarations);
    assert!(
        painted.iter().any(|row| row.contains("No matches")),
        "fresh client must not render Previous as a disabled/no-op row: {painted:?}"
    );
    assert!(matches!(
        enter(&mut unavailable, &fresh, declarations),
        Response::Consumed
    ));
    let targeted = command("host:focus", Some("targeted"));
    let mut explicit = FrameRenderer::default();
    explicit
        .controls
        .open_palette(&fresh, declarations, Some(&menu_context));
    for title in [
        targeted["title"].as_str().unwrap(),
        "History workspace",
        "History second",
    ] {
        search(&mut explicit, &fresh, declarations, title);
        let painted = rows(&mut explicit, &fresh, declarations);
        assert!(
            !painted.iter().any(|row| row.contains("No matches")),
            "actual target navigation lost {title:?}: {painted:?}"
        );
        assert!(matches!(
            enter(&mut explicit, &fresh, declarations),
            Response::Consumed
        ));
    }
    let Response::Invoke(selected) = enter(&mut explicit, &fresh, declarations) else {
        panic!(
            "no Previous visit must not hide explicitly targeted Focus: {:?}",
            rows(&mut explicit, &fresh, declarations)
        );
    };
    assert_eq!(
        selected.context.tab_id.as_deref(),
        second["tab"]["tab_id"].as_str()
    );
    let mut available = FrameRenderer::default();
    available
        .controls
        .open_palette(&experienced, declarations, Some(&menu_context));
    search(
        &mut available,
        &experienced,
        declarations,
        previous["title"].as_str().unwrap(),
    );
    let Response::Invoke(selected) = enter(&mut available, &experienced, declarations) else {
        panic!("this client's eligible previous visit was hidden");
    };
    let Effect::Request { method, params } =
        experienced.invoke(&selected.action, &selected.context, Some(snapshot))
    else {
        panic!("eligible previous selection did not use this client's history");
    };
    assert_eq!(method, "tab.focus");
    assert_eq!(params["tab_id"], first["tab"]["tab_id"]);
    core.request(method, params);
    assert_eq!(
        core.request("session.snapshot", json!({}))["snapshot"]["focused_tab_id"],
        first["tab"]["tab_id"]
    );
    // Exercise the executable's own snapshot ingestion as well as the native
    // controller seam: no direct history-pruning helper drives this PTY.
    let mut live = console::Console::new(&core);
    for (tab, pane, marker) in [
        (
            &first["tab"]["tab_id"],
            &first["root_pane"]["pane_id"],
            "NATIVE_HISTORY_FIRST",
        ),
        (
            &second["tab"]["tab_id"],
            &second["root_pane"]["pane_id"],
            "NATIVE_HISTORY_SECOND",
        ),
        (
            &first["tab"]["tab_id"],
            &first["root_pane"]["pane_id"],
            "NATIVE_HISTORY_RETURNED",
        ),
    ] {
        core.request("tab.focus", json!({"tab_id":tab}));
        core.request(
            "pane.send_text",
            json!({"pane_id":pane,"text":format!("printf '{marker}\\n'\n")}),
        );
        live.wait(|rows| rows.iter().any(|row| row.trim_end().ends_with(marker)));
    }
    let previous_title = previous["title"].as_str().unwrap();
    live.text("\x1bOP");
    live.text(previous_title);
    live.wait(|rows| {
        rows.iter()
            .filter(|row| row.contains(previous_title))
            .count()
            >= 2
    });
    live.text("\x1b");
    live.wait(|rows| !rows.iter().any(|row| row.contains("Enter select")));
    core.request("tab.close", json!({"tab_id":second["tab"]["tab_id"]}));
    let deadline = Instant::now() + Duration::from_secs(8);
    while client.snapshot.as_ref().is_none_or(|snapshot| {
        snapshot.focused_tab_id.as_deref() != first["tab"]["tab_id"].as_str()
            || snapshot
                .tabs
                .iter()
                .any(|tab| Some(tab.tab_id.as_str()) == second["tab"]["tab_id"].as_str())
    }) {
        assert!(
            Instant::now() < deadline,
            "actual closed visit remained in the client snapshot"
        );
        client.receive().unwrap();
    }
    let snapshot = client.snapshot.as_ref().unwrap();
    experienced.update_session(
        snapshot.focused_workspace_id.clone(),
        snapshot.focused_tab_id.clone(),
        true,
    );
    experienced.reconcile_tab_history(snapshot);
    assert!(experienced.previous_tab(snapshot).is_none());
    let declarations = client.declarations.as_ref().unwrap();
    let mut closed_visit = FrameRenderer::default();
    closed_visit
        .controls
        .open_palette(&experienced, declarations, Some(&menu_context));
    search(
        &mut closed_visit,
        &experienced,
        declarations,
        previous["title"].as_str().unwrap(),
    );
    let painted = rows(&mut closed_visit, &experienced, declarations);
    assert!(
        painted.iter().any(|row| row.contains("No matches")),
        "a closed previous visit must not leave an executable-looking row: {painted:?}"
    );
    assert!(matches!(
        enter(&mut closed_visit, &experienced, declarations),
        Response::Consumed
    ));
    core.request(
        "pane.send_text",
        json!({
            "pane_id":first["root_pane"]["pane_id"],
            "text":"printf 'NATIVE_CLOSED_HISTORY_READY\\n'\n"
        }),
    );
    live.wait(|rows| {
        rows.iter()
            .any(|row| row.trim_end().ends_with("NATIVE_CLOSED_HISTORY_READY"))
    });
    live.text("\x1bOP");
    live.text(previous_title);
    live.wait(|rows| rows.iter().any(|row| row.contains("No matches")));
    live.text("\r");
    assert_eq!(
        core.request("session.snapshot", json!({}))["snapshot"]["focused_tab_id"],
        first["tab"]["tab_id"]
    );
}

#[test]
fn discovered_palette_actions_keep_existing_stream_and_spawn_execution_paths() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use trailboss::terminal::{
        menus::{Controls, Response},
        Rect,
    };

    struct EndpointTap(std::process::Child);
    impl Drop for EndpointTap {
        fn drop(&mut self) {
            drop(self.0.stdin.take());
            let _ = self.0.wait();
        }
    }
    fn records(path: &std::path::Path) -> Vec<Value> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => panic!("cannot read {}: {error}", path.display()),
        };
        // The independent relay writes live JSONL. Observe only complete records.
        text.split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    for stream in [true, false] {
        let core = support::Core::new();
        let origin = core.request(
            "workspace.create",
            json!({
                "label":"Execution origin","cwd":core.home,"focus":true
            }),
        );
        let captured = core.request(
            "workspace.create",
            json!({
                "label":"Execution target","cwd":core.home,"focus":false
            }),
        );
        let target: Context = serde_json::from_value(json!({
            "workspace_id":captured["workspace"]["workspace_id"],
            "tab_id":captured["tab"]["tab_id"],"pane_id":captured["root_pane"]["pane_id"]
        }))
        .unwrap();
        let owner = core.home.join("execution-owner");
        std::fs::create_dir_all(&owner).unwrap();
        let mut manifest = "id = \"fixture.execution-owner\"\nname = \"Execution owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"route-probe\"\ntitle = \"Route probe\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"spawn.py\"]\n".to_owned();
        if stream {
            manifest.push_str("[[ui]]\nid = \"action-stream\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"stream.py\"]\nstream = true\nstream_actions = true\n");
        }
        std::fs::write(owner.join("trailboss-plugin.toml"), manifest).unwrap();
        std::fs::write(owner.join("spawn.py"), r#"import json,os
with open('spawn.jsonl','a') as output:
    output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],
        'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON']),'ppid':os.getppid()})+'\n')
print('spawn-owner-completed',flush=True)
"#).unwrap();
        std::fs::write(owner.join("stream.py"), r#"import json,os,sys
print(json.dumps({'herdr':'widget_tree','version':1,'zones':[],'fragments':[
    {'name':'fixture.execution-stream.ready','priority':0,
     'tree':{'type':'label','role':'label','text':'Owner stream ready'}}]}),flush=True)
for line in sys.stdin:
    envelope=json.loads(line)
    with open('stream.jsonl','a') as output:
        output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'ppid':os.getppid(),'envelope':envelope})+'\n')
    print(json.dumps({'herdr':'action_result','id':envelope['id'],'ok':True}),flush=True)
"#).unwrap();
        core.request("plugin.link", json!({"path":owner,"enabled":true}));
        core.request(
            "plugin.link",
            json!({
                "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/sidebar-menus"),"enabled":true
            }),
        );
        let endpoint = core.home.join(format!(
            "trailboss/sessions/{}/trailboss-client.sock",
            core.name
        ));
        let child = Command::new("python3").args(["-u", "-c",
            "import sys,select\nfrom pathlib import Path\nsys.path.insert(0,sys.argv[1])\nfrom command_interactions import OwnedEndpointTap\nevidence=Path(sys.argv[3]); tap=OwnedEndpointTap(Path(sys.argv[2]),evidence)\n(evidence/'tap-ready').touch()\ntry:\n while not select.select([sys.stdin],[],[],0.02)[0]:\n  assert not tap.errors,tap.errors\nfinally: tap.close()\n",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support")])
            .arg(&endpoint).arg(&core.home).stdin(std::process::Stdio::piped()).spawn().unwrap();
        let _tap = EndpointTap(child);
        let deadline = Instant::now() + Duration::from_secs(8);
        while !core.home.join("tap-ready").exists() {
            assert!(
                Instant::now() < deadline,
                "transparent endpoint relay did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(
            origin["workspace"]["workspace_id"]
                .as_str()
                .map(str::to_owned),
            origin["tab"]["tab_id"].as_str().map(str::to_owned),
            true,
        );
        let key = json!({"kind":"plugin","plugin_id":"fixture.execution-owner","action_id":"route-probe"});
        let catalog = core.request("ui.command.list", json!({}));
        let command = catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == key)
            .expect("independent owner action was not discovered");
        assert_eq!(command["title"], "Route probe");
        let locator = command["fragment_name"].as_str().unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while client.declarations.as_ref().is_none_or(|declarations| {
            native.fragment(declarations, locator).is_none()
                || native
                    .fragment(declarations, "commands.confirm-space")
                    .is_none()
                || native
                    .fragment(declarations, "commands.confirm-tab")
                    .is_none()
                || (stream
                    && native
                        .fragment(declarations, "fixture.execution-stream.ready")
                        .is_none())
        }) {
            assert!(
                Instant::now() < deadline,
                "existing execution owner prerequisites missing"
            );
            client.receive().unwrap();
        }
        core.request("plugin.link", json!({
            "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/command-palette"),"enabled":true
        }));
        let deadline = Instant::now() + Duration::from_secs(8);
        while client
            .declarations
            .as_ref()
            .is_none_or(|declarations| native.fragment(declarations, "command-palette").is_none())
        {
            assert!(
                Instant::now() < deadline,
                "dedicated palette did not publish"
            );
            client.receive().unwrap();
        }
        let declarations = client.declarations.as_ref().unwrap();
        assert_eq!(
            native.fragment(declarations, "command-palette").unwrap().0,
            "tommymorgan.command-palette"
        );
        let mut controls = Controls::default();
        assert!(controls.open_palette(&native, declarations, Some(&target)));
        for character in "Route probe".chars() {
            controls.key(
                &native,
                declarations,
                Rect::viewport(100, 30),
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
            );
        }
        let Response::Invoke(selected) = controls.key(
            &native,
            declarations,
            Rect::viewport(100, 30),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ) else {
            panic!("discovered operation did not reach native plugin dispatch");
        };
        assert!(matches!(
            native.invoke(&selected.action, &selected.context, None),
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
                None,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (issued, reply) =
            loop {
                let events = records(&core.home.join("native-endpoint/operations.jsonl"));
                let requests: Vec<_> = events
                    .iter()
                    .filter(|event| {
                        event["direction"] == "client"
                            && event["request"]["method"] == "plugin.action.invoke"
                    })
                    .collect();
                assert!(
                    requests.len() <= 1,
                    "one selection issued duplicate owner requests"
                );
                if let Some(issued) = requests.first() {
                    let chunks: Vec<_> = events
                        .iter()
                        .filter(|event| {
                            event["direction"] == "core"
                                && event["connection"] == issued["connection"]
                                && event["boot_id"] == issued["boot_id"]
                                && event["request_id"] == issued["request"]["id"]
                        })
                        .collect();
                    if chunks
                        .last()
                        .is_some_and(|chunk| chunk["final_chunk"] == true)
                    {
                        let mut bytes = Vec::new();
                        for chunk in chunks {
                            let hex = chunk["data_hex"].as_str().unwrap();
                            bytes.extend((0..hex.len()).step_by(2).map(|index| {
                                u8::from_str_radix(&hex[index..index + 2], 16).unwrap()
                            }));
                        }
                        break (
                            (*issued).clone(),
                            serde_json::from_slice::<Value>(&bytes).unwrap(),
                        );
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "actual owner invocation received no correlated reply: {events:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
        assert_eq!(
            issued["request"]["params"]["plugin_id"],
            "fixture.execution-owner"
        );
        assert_eq!(issued["request"]["params"]["action_id"], "route-probe");
        assert_eq!(reply["id"], issued["request"]["id"]);
        assert!(
            reply.get("error").is_none(),
            "owner route returned its normal error: {reply}"
        );
        assert_eq!(reply["result"]["type"], "plugin_action_invoked");
        let expected_context = serde_json::to_value(&target).unwrap();
        for field in ["workspace_id", "tab_id", "pane_id"] {
            assert_eq!(
                issued["request"]["params"]["context"][field], expected_context[field],
                "native request lost captured {field}: issued={issued}; actual_reply={reply}"
            );
            assert_eq!(
                reply["result"]["context"][field], expected_context[field],
                "owner route resolved a different {field}: {reply}"
            );
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let finished = loop {
            let logs = core.request(
                "plugin.log.list",
                json!({"plugin_id":"fixture.execution-owner"}),
            );
            let matching: Vec<_> = logs["logs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|log| log["log_id"] == reply["result"]["log"]["log_id"])
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "owner reply/log correlation lost: {logs}"
            );
            if matching[0]["status"] != "running" {
                break matching[0].clone();
            }
            assert!(
                Instant::now() < deadline,
                "owner's existing completion did not settle: {logs}"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(finished["plugin_id"], "fixture.execution-owner");
        assert_eq!(finished["action_id"], "route-probe");
        assert_eq!(finished["status"], "succeeded", "{finished}");
        let spawned = records(&owner.join("spawn.jsonl"));
        let forwarded = records(&owner.join("stream.jsonl"));
        let effect = if stream {
            assert!(
                spawned.is_empty(),
                "stream route must not execute the manifest command vector"
            );
            assert_eq!(forwarded.len(), 1);
            assert_eq!(forwarded[0]["envelope"]["id"], finished["log_id"]);
            assert_eq!(forwarded[0]["envelope"]["action_id"], "route-probe");
            &forwarded[0]
        } else {
            assert!(forwarded.is_empty());
            assert_eq!(spawned.len(), 1);
            assert_eq!(spawned[0]["action"], "route-probe");
            assert!(finished["stdout"]
                .as_str()
                .unwrap()
                .contains("spawn-owner-completed"));
            &spawned[0]
        };
        assert_eq!(effect["owner"], "fixture.execution-owner");
        assert_eq!(
            effect["ppid"],
            core.child.id(),
            "Core, not the palette, must own execution"
        );
        let actual_context = if stream {
            &effect["envelope"]["context"]
        } else {
            &effect["context"]
        };
        assert_eq!(
            actual_context["workspace_id"],
            captured["workspace"]["workspace_id"]
        );
        assert_eq!(actual_context["tab_id"], captured["tab"]["tab_id"]);
        assert_eq!(actual_context["pane_id"], captured["root_pane"]["pane_id"]);
    }
}

#[test]
fn selected_blue_refresh_use_invokes_only_its_owner_once_on_captured_target() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use trailboss::terminal::{
        menus::{Controls, Response},
        Rect,
    };
    use trailboss_runtime::protocol::widgets::Node;

    let core = support::Core::new();
    let origin = core.request(
        "workspace.create",
        json!({
            "label":"Invocation origin","cwd":core.home,"focus":true
        }),
    );
    let captured = core.request(
        "workspace.create",
        json!({
            "label":"Explicit target","cwd":core.home,"focus":false
        }),
    );
    let target: Context = serde_json::from_value(json!({
        "workspace_id":captured["workspace"]["workspace_id"],
        "tab_id":captured["tab"]["tab_id"],"pane_id":captured["root_pane"]["pane_id"]
    }))
    .unwrap();
    for color in ["red", "blue"] {
        let root = core.home.join(color);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("trailboss-plugin.toml"), format!(
            "id = \"fixture.{color}\"\nname = \"{color} owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"refresh\"\ntitle = \"Refresh\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n"
        )).unwrap();
        std::fs::write(root.join("record.py"), "import json,os\nwith open('executed.jsonl','a') as f:\n f.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
        core.request("plugin.link", json!({"path":root,"enabled":true}));
    }
    core.request(
        "plugin.link",
        json!({
            "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/sidebar-menus"),"enabled":true
        }),
    );
    let catalog = core.request("ui.command.list", json!({}));
    let mut locators = Vec::new();
    for owner in ["fixture.red", "fixture.blue"] {
        let key = json!({"kind":"plugin","plugin_id":owner,"action_id":"refresh"});
        let entries: Vec<_> = catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|command| command["key"] == key)
            .collect();
        assert_eq!(entries.len(), 1, "{owner}");
        assert_eq!(entries[0]["title"], "Refresh");
        locators.push((
            owner,
            entries[0]["fragment_name"].as_str().unwrap().to_owned(),
        ));
    }
    assert_ne!(locators[0].1, locators[1].1);
    let mut client = core.attach();
    let mut native = NativeCore::default();
    native.update_session(
        origin["workspace"]["workspace_id"]
            .as_str()
            .map(str::to_owned),
        origin["tab"]["tab_id"].as_str().map(str::to_owned),
        true,
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while client.declarations.as_ref().is_none_or(|declarations| {
        locators
            .iter()
            .any(|(_, name)| native.fragment(declarations, name).is_none())
            || native
                .fragment(declarations, "commands.confirm-space")
                .is_none()
            || native
                .fragment(declarations, "commands.confirm-tab")
                .is_none()
    }) {
        assert!(
            Instant::now() < deadline,
            "existing owner exports did not become ready"
        );
        client.receive().unwrap();
    }
    for (expected_owner, name) in &locators {
        assert_eq!(
            native
                .fragment(client.declarations.as_ref().unwrap(), name)
                .unwrap()
                .0,
            *expected_owner
        );
    }
    core.request(
        "plugin.link",
        json!({
            "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/command-palette"),"enabled":true
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while client
        .declarations
        .as_ref()
        .is_none_or(|declarations| native.fragment(declarations, "command-palette").is_none())
    {
        assert!(
            Instant::now() < deadline,
            "dedicated consumer did not publish"
        );
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    let (provider, tree) = native.fragment(declarations, "command-palette").unwrap();
    assert_eq!(provider, "tommymorgan.command-palette");
    let Node::Column { children } = tree else {
        panic!("dedicated consumer did not publish its normal command list");
    };
    // Read the actual consumer's order only to navigate duplicate display
    // titles. Expected owner/action/target remain independent constants.
    let refresh_order: Vec<_> = children
        .iter()
        .filter_map(|node| {
            let Node::Use { name } = node else {
                return None;
            };
            locators
                .iter()
                .find(|(_, locator)| locator == name)
                .map(|(owner, _)| *owner)
        })
        .collect();
    assert_eq!(refresh_order.len(), 2);
    let blue_position = refresh_order
        .iter()
        .position(|owner| *owner == "fixture.blue")
        .expect("actual consumer lost Blue's owner-qualified Use");
    let mut controls = Controls::default();
    assert!(controls.open_palette(&native, declarations, Some(&target)));
    for character in "Refresh".chars() {
        assert!(matches!(
            controls.key(
                &native,
                declarations,
                Rect::viewport(100, 30),
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)
            ),
            Response::Consumed
        ));
    }
    for _ in 0..blue_position {
        controls.key(
            &native,
            declarations,
            Rect::viewport(100, 30),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );
    }
    let Response::Invoke(selected) = controls.key(
        &native,
        declarations,
        Rect::viewport(100, 30),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ) else {
        panic!("actual Blue Use did not produce an owner invocation");
    };
    assert!(!controls.modal(), "selection must close command search");
    assert!(matches!(
        native.invoke(&selected.action, &selected.context, None),
        Effect::Plugin
    ));
    // Exercise the production native endpoint sender, not a hand-assembled API
    // request or an echo of the selected owner/context.
    client
        .input()
        .unwrap()
        .invoke(
            &client.boot_id,
            &selected.plugin,
            &selected.action,
            &selected.context,
            None,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let logs = loop {
        let logs = core.request("plugin.log.list", json!({"limit":100}));
        if logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|log| log["action_id"] == "refresh" && log["status"] != "running")
        {
            break logs;
        }
        assert!(
            Instant::now() < deadline,
            "selected owner={} action={} context={:?} produced no completed invocation: {logs}",
            selected.plugin,
            selected.action,
            selected.context
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let executed: Vec<_> = logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|log| log["action_id"] == "refresh")
        .collect();
    assert_eq!(executed.len(), 1, "duplicate owner invocation: {logs}");
    assert_eq!(
        executed[0]["plugin_id"], "fixture.blue",
        "wrong actual execution owner: {logs}"
    );
    assert_eq!(executed[0]["status"], "succeeded", "{logs}");
    assert_eq!(selected.plugin, "fixture.blue");
    assert_eq!(selected.action, "refresh");
    assert_eq!(selected.context, target);
    let records: Vec<Value> = std::fs::read_to_string(core.home.join("blue/executed.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["owner"], "fixture.blue");
    assert_eq!(records[0]["action"], "refresh");
    assert_eq!(
        records[0]["context"]["workspace_id"],
        captured["workspace"]["workspace_id"]
    );
    assert_eq!(records[0]["context"]["tab_id"], captured["tab"]["tab_id"]);
    assert_eq!(
        records[0]["context"]["pane_id"],
        captured["root_pane"]["pane_id"]
    );
    assert!(!core.home.join("red/executed.jsonl").exists());
    assert!(!logs["logs"].as_array().unwrap().iter().any(|log| {
        log["action_id"] == "refresh"
            && (log["plugin_id"] == "fixture.red"
                || log["plugin_id"] == "tommymorgan.command-palette")
    }));
}

#[test]
fn dedicated_palette_omits_core_known_unavailable_actions_without_disabled_rows() {
    use trailboss::sidebar::Declarations;
    use trailboss_runtime::protocol::widgets::Node;

    fn inspect(
        node: &Node,
        scope: (&NativeCore, &Declarations, &str),
        depth: usize,
        actions: &mut Vec<(String, String)>,
        labels: &mut Vec<String>,
        locators: &mut Vec<String>,
    ) {
        assert!(depth < 32, "cyclic palette Use");
        let (native, declarations, owner) = scope;
        match node {
            Node::Use { name } => {
                locators.push(name.clone());
                if let Some((provider, tree)) = native.fragment(declarations, name) {
                    inspect(
                        tree,
                        (native, declarations, provider),
                        depth + 1,
                        actions,
                        labels,
                        locators,
                    );
                }
            }
            Node::Label { text, .. } => labels.push(text.clone()),
            Node::Row {
                children,
                action,
                menu,
                ..
            } => {
                if let Some(action) = action {
                    actions.push((owner.to_owned(), action.clone()));
                }
                for child in children {
                    inspect(child, scope, depth + 1, actions, labels, locators);
                }
                if let Some(menu) = menu {
                    inspect(menu, scope, depth + 1, actions, labels, locators);
                }
            }
            Node::Column { children } | Node::List { entries: children } => {
                for child in children {
                    inspect(child, scope, depth + 1, actions, labels, locators);
                }
            }
            Node::Flex { child, .. } | Node::When { child, .. } | Node::Modal { child } => {
                inspect(child, scope, depth + 1, actions, labels, locators);
            }
            Node::Zone {
                fallback: Some(child),
                ..
            } => {
                inspect(child, scope, depth + 1, actions, labels, locators);
            }
            _ => {}
        }
    }

    for state in [
        "plugin disabled",
        "action unsupported on this platform",
        "plugin manifest unavailable",
    ] {
        let core = support::Core::new();
        let resource = core.request(
            "workspace.create",
            json!({
                "label":"Availability context","cwd":core.home,"focus":true
            }),
        );
        let owner = core.home.join("availability-owner");
        let control = core.home.join("availability-control");
        std::fs::create_dir_all(&owner).unwrap();
        std::fs::create_dir_all(&control).unwrap();
        // These independent manifests establish one candidate and one eligible
        // positive control, rather than deriving expected actions from discovery.
        let manifest = "id = \"fixture.availability\"\nname = \"Availability owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"candidate\"\ntitle = \"Unavailable candidate\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n";
        let ui = "[[ui]]\nid = \"existing-action\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"producer.py\"]\nstream = true\n";
        std::fs::write(
            owner.join("trailboss-plugin.toml"),
            format!("{manifest}{ui}"),
        )
        .unwrap();
        std::fs::write(control.join("trailboss-plugin.toml"), "id = \"fixture.availability-control\"\nname = \"Availability control\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"control\"\ntitle = \"Available control\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n").unwrap();
        for root in [&owner, &control] {
            std::fs::write(root.join("record.py"), "import json,os\nwith open('executed.jsonl','a') as f:\n f.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
        }
        // An ordinary owner export is deliberately not a catalog eligibility
        // flag. Unsupported actions can still have declared UI; discovery must
        // not mistake that UI for an available command.
        let declaration = json!({"herdr":"widget_tree","version":1,"zones":[],
        "fragments":[{"name":"fixture.availability-candidate","priority":0,"tree":{
            "type":"row","action":"candidate","context":{
                "workspace_id":resource["workspace"]["workspace_id"],
                "tab_id":resource["tab"]["tab_id"],"pane_id":resource["root_pane"]["pane_id"]
            },"children":[{"type":"label","text":"Unavailable candidate","role":"label"}]
        }}]});
        std::fs::write(
            owner.join("producer.py"),
            format!(
                "import sys\nprint({:?},flush=True)\nfor line in sys.stdin: pass\n",
                declaration.to_string()
            ),
        )
        .unwrap();
        for path in [&owner, &control] {
            core.request("plugin.link", json!({"path":path,"enabled":true}));
        }
        core.request(
            "plugin.link",
            json!({
                "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/sidebar-menus"),"enabled":true
            }),
        );
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(
            resource["workspace"]["workspace_id"]
                .as_str()
                .map(str::to_owned),
            resource["tab"]["tab_id"].as_str().map(str::to_owned),
            true,
        );
        let deadline = Instant::now() + Duration::from_secs(8);
        while client.declarations.as_ref().is_none_or(|declarations| {
            [
                "fixture.availability-candidate",
                "commands.confirm-space",
                "commands.confirm-tab",
            ]
            .iter()
            .any(|name| native.fragment(declarations, name).is_none())
        }) {
            assert!(
                Instant::now() < deadline,
                "{state}: original owner prerequisites missing"
            );
            client.receive().unwrap();
        }
        let candidate =
            json!({"kind":"plugin","plugin_id":"fixture.availability","action_id":"candidate"});
        let eligible = json!({"kind":"plugin","plugin_id":"fixture.availability-control","action_id":"control"});
        let initial = core.request("ui.command.list", json!({}));
        let old_locator = initial["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == candidate)
            .expect("candidate must be registered and available before applying its state")
            ["fragment_name"]
            .as_str()
            .unwrap()
            .to_owned();
        match state {
            "plugin disabled" => {
                core.request(
                    "plugin.disable",
                    json!({"plugin_id":"fixture.availability"}),
                );
            }
            "action unsupported on this platform" => {
                let unsupported = if cfg!(target_os = "windows") {
                    "linux"
                } else {
                    "windows"
                };
                std::fs::write(
                    owner.join("trailboss-plugin.toml"),
                    format!("{manifest}platforms = [{unsupported:?}]\n{ui}"),
                )
                .unwrap();
            }
            "plugin manifest unavailable" => {
                std::fs::remove_file(owner.join("trailboss-plugin.toml")).unwrap();
            }
            _ => unreachable!(),
        }
        // This real API refresh establishes Core's authoritative availability
        // before the dedicated consumer starts. No open selection is exercised.
        let current = core.request("ui.command.list", json!({}));
        let commands = current["commands"].as_array().unwrap();
        assert!(
            !commands.iter().any(|command| command["key"] == candidate),
            "{state}"
        );
        let control_locator = commands
            .iter()
            .find(|command| command["key"] == eligible)
            .expect("independent eligible control disappeared")["fragment_name"]
            .as_str()
            .unwrap();
        core.request("plugin.link", json!({
            "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/command-palette"),"enabled":true
        }));
        let deadline = Instant::now() + Duration::from_secs(8);
        while client
            .declarations
            .as_ref()
            .is_none_or(|declarations| native.fragment(declarations, "command-palette").is_none())
        {
            assert!(
                Instant::now() < deadline,
                "{state}: dedicated consumer did not publish"
            );
            client.receive().unwrap();
        }
        let declarations = client.declarations.as_ref().unwrap();
        let (provider, tree) = native.fragment(declarations, "command-palette").unwrap();
        assert_eq!(provider, "tommymorgan.command-palette");
        let mut actions = Vec::new();
        let mut labels = Vec::new();
        let mut locators = Vec::new();
        inspect(
            tree,
            (&native, declarations, provider),
            0,
            &mut actions,
            &mut labels,
            &mut locators,
        );
        assert!(
            actions.contains(&(
                "fixture.availability-control".to_owned(),
                "control".to_owned()
            )),
            "{state}: eligible control must remain usable"
        );
        assert!(
            locators.iter().any(|name| name == control_locator),
            "{state}"
        );
        assert!(
            !locators.contains(&old_locator),
            "{state}: unavailable command Use survived"
        );
        assert!(
            !actions.contains(&("fixture.availability".to_owned(), "candidate".to_owned())),
            "{state}: unavailable action remains in the actual palette"
        );
        assert!(
            !labels
                .iter()
                .any(|label| label.contains("Unavailable candidate")),
            "{state}: unavailable command was rendered as a disabled/explanatory row: {labels:?}"
        );
    }
}

#[test]
fn palette_consumer_retains_independent_catalog_authorities_and_targets() {
    use trailboss::sidebar::Declarations;
    use trailboss_runtime::protocol::widgets::Node;

    fn uses(node: &Node, inherited: &Context, found: &mut Vec<(String, Context)>) {
        match node {
            Node::Use { name } => found.push((name.clone(), inherited.clone())),
            Node::Column { children } | Node::List { entries: children } => {
                for child in children {
                    uses(child, inherited, found);
                }
            }
            Node::Row {
                children,
                context,
                menu,
                ..
            } => {
                let context = if context == &Context::default() {
                    inherited
                } else {
                    context
                };
                for child in children {
                    uses(child, context, found);
                }
                if let Some(menu) = menu {
                    uses(menu, context, found);
                }
            }
            _ => {}
        }
    }
    fn action_contexts(
        node: &Node,
        inherited: &Context,
        scope: (&NativeCore, &Declarations),
        wanted: &str,
        depth: usize,
        found: &mut Vec<Context>,
    ) {
        assert!(depth < 32, "cyclic owner fragment");
        let (native, declarations) = scope;
        match node {
            Node::Use { name } => {
                let (_, tree) = native
                    .fragment(declarations, name)
                    .expect("unresolvable owner Use");
                action_contexts(tree, inherited, scope, wanted, depth + 1, found);
            }
            Node::Column { children } | Node::List { entries: children } => {
                for child in children {
                    action_contexts(child, inherited, scope, wanted, depth + 1, found);
                }
            }
            Node::Row {
                children,
                action,
                context,
                menu,
            } => {
                let context = if context == &Context::default() {
                    inherited
                } else {
                    context
                };
                if action.as_deref() == Some(wanted) {
                    found.push(context.clone());
                }
                for child in children {
                    action_contexts(child, context, scope, wanted, depth + 1, found);
                }
                if let Some(menu) = menu {
                    action_contexts(menu, context, scope, wanted, depth + 1, found);
                }
            }
            _ => {}
        }
    }

    // Accepted planning inventory, not CORE_COMMANDS, parser membership, RPC
    // methods, or values learned from this test's catalog response.
    let core_inventory = [
        ("host:new-space", None, "global"),
        ("host:new", None, "client_local"),
        ("host:new-tab", None, "workspace"),
        ("host:rename-space", None, "workspace"),
        ("host:rename-tab", None, "tab"),
        ("host:close-tab", None, "tab"),
        ("host:focus", Some("targeted"), "tab"),
        ("host:focus", Some("previous"), "client_local"),
        ("host:restart-idle-agents", None, "global"),
        ("host:detach", None, "client_local"),
    ];
    let core = support::Core::new();
    let mut plugin_inventory = Vec::new();
    let mut targets = Vec::new();
    for color in ["red", "blue", "green"] {
        let resource = core.request(
            "workspace.create",
            json!({"label":format!("Authority {color}"),"cwd":core.home,"focus":true}),
        );
        let context: Context = serde_json::from_value(json!({
            "workspace_id":resource["workspace"]["workspace_id"],
            "tab_id":resource["tab"]["tab_id"],"pane_id":resource["root_pane"]["pane_id"]
        }))
        .unwrap();
        let owner = format!("fixture.{color}");
        let root = core.home.join(color);
        std::fs::create_dir_all(&root).unwrap();
        let actions = [
            ("refresh".to_owned(), "Refresh".to_owned()),
            (format!("inspect-{color}"), format!("Inspect {color}")),
        ];
        let mut manifest = format!(
            "id = \"{owner}\"\nname = \"Authority {color}\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n"
        );
        let mut rows = Vec::new();
        for (action, title) in actions {
            manifest.push_str(&format!(
                "[[actions]]\nid = \"{action}\"\ntitle = \"{title}\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n"
            ));
            rows.push(json!({"type":"row","action":action,"context":context,
                "children":[{"type":"label","text":title,"role":"label"}]}));
            plugin_inventory.push((owner.clone(), action, title, context.clone()));
        }
        manifest.push_str("[[ui]]\nid = \"authority\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"producer.py\"]\nstream = true\n");
        std::fs::write(root.join("trailboss-plugin.toml"), manifest).unwrap();
        std::fs::write(root.join("record.py"), "import json,os\nwith open('executed.jsonl','a') as f:\n f.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
        let declaration = json!({"herdr":"widget_tree","version":1,"zones":[
            {"zone":"footer","tree":{"type":"column","children":rows}}
        ]});
        std::fs::write(
            root.join("producer.py"),
            format!(
                "import sys\nprint({:?},flush=True)\nfor line in sys.stdin: pass\n",
                declaration.to_string()
            ),
        )
        .unwrap();
        core.request("plugin.link", json!({"path":root,"enabled":true}));
        targets.push(context);
    }
    core.request(
        "plugin.link",
        json!({
            "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/sidebar-menus"),"enabled":true
        }),
    );
    let mut client = core.attach();
    let mut native = NativeCore::default();
    for target in &targets {
        native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
    }
    // Establish owner exports before starting the consumer. Eligibility is not
    // inferred from the palette rows whose completeness this witness tests.
    let deadline = Instant::now() + Duration::from_secs(8);
    let catalog = loop {
        let catalog = core.request("ui.command.list", json!({}));
        if client.declarations.as_ref().is_some_and(|declarations| {
            catalog["commands"]
                .as_array()
                .unwrap()
                .iter()
                .all(|command| {
                    command["diagnostic"].is_null()
                        && native
                            .fragment(declarations, command["fragment_name"].as_str().unwrap())
                            .is_some()
                })
                && plugin_inventory.iter().all(|(owner, action, _, _)| {
                    declarations.iter().any(|(provider, declaration)| {
                        provider == owner
                            && declaration.zones.iter().any(|zone| {
                                let mut contexts = Vec::new();
                                action_contexts(
                                    &zone.tree,
                                    &Context::default(),
                                    (&native, declarations),
                                    action,
                                    0,
                                    &mut contexts,
                                );
                                !contexts.is_empty()
                            })
                    })
                })
        }) {
            break catalog;
        }
        assert!(
            Instant::now() < deadline,
            "independent owner prerequisites not ready: {catalog}"
        );
        client.receive().unwrap_or_else(|error| {
            let log = std::fs::read_to_string(core.home.join(format!(
                "trailboss/sessions/{}/trailboss-server.log", core.name
            ))).unwrap_or_else(|read_error| format!("server log unavailable: {read_error}"));
            let stream_errors: Vec<_> = log.lines()
                .filter(|line| line.contains("plugin UI stream stopped"))
                .take(6).collect();
            let diagnostics: Vec<_> = catalog["commands"].as_array().unwrap().iter()
                .filter(|command| !command["diagnostic"].is_null())
                .map(|command| json!({"key":command["key"],"diagnostic":command["diagnostic"]}))
                .collect();
            let owners: Vec<_> = client.declarations.as_ref().map(|declarations| {
                declarations.iter().map(|(owner, declaration)| (owner, declaration.zones.len())).collect()
            }).unwrap_or_default();
            panic!("owner readiness receive failed: {error}; diagnostics={diagnostics:?}; owner_zone_counts={owners:?}; producer_errors={stream_errors:?}");
        });
    };
    let commands = catalog["commands"].as_array().unwrap();
    let core_keys: std::collections::BTreeSet<_> = commands
        .iter()
        .filter(|command| command["key"]["kind"] == "core")
        .map(|command| {
            (
                command["key"]["action_id"].as_str().unwrap(),
                command["key"]["mode"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        core_keys,
        core_inventory
            .iter()
            .map(|(action, mode, _)| (*action, *mode))
            .collect()
    );
    for forbidden in [
        "host:create",
        "host:cancel",
        "pane.send_text",
        "pane.send_key",
        "pane.send_bytes",
    ] {
        assert!(
            !commands
                .iter()
                .any(|command| command["key"]["action_id"] == forbidden),
            "interaction control/raw PTY RPC became a standalone command: {forbidden}"
        );
    }
    let mut expected = Vec::new();
    for (action, mode, target) in core_inventory {
        let mut key = json!({"kind":"core","action_id":action});
        if let Some(mode) = mode {
            key["mode"] = json!(mode);
        }
        let matching: Vec<_> = commands
            .iter()
            .filter(|command| command["key"] == key)
            .collect();
        assert_eq!(matching.len(), 1, "{key}");
        assert_eq!(matching[0]["target"], target, "{key}");
        let contexts = if matches!(target, "workspace" | "tab") {
            targets
                .iter()
                .map(|context| Context {
                    workspace_id: context.workspace_id.clone(),
                    tab_id: (target == "tab").then(|| context.tab_id.clone()).flatten(),
                    ..Context::default()
                })
                .collect::<Vec<_>>()
        } else {
            vec![Context::default()]
        };
        for context in contexts {
            expected.push((
                key.clone(),
                "trailboss.core".to_owned(),
                action.to_owned(),
                context,
            ));
        }
    }
    for (owner, action, title, context) in &plugin_inventory {
        let key = json!({"kind":"plugin","plugin_id":owner,"action_id":action});
        let matching: Vec<_> = commands
            .iter()
            .filter(|command| command["key"] == key)
            .collect();
        assert_eq!(matching.len(), 1, "{key}");
        assert_eq!(matching[0]["title"], *title, "{key}");
        assert_eq!(matching[0]["target"], "owner", "{key}");
        expected.push((key, owner.clone(), action.clone(), context.clone()));
    }
    assert_eq!(
        plugin_inventory
            .iter()
            .filter(|(_, _, title, _)| title == "Refresh")
            .count(),
        3
    );
    core.request(
        "plugin.link",
        json!({
            "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/command-palette"),"enabled":true
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while client
        .declarations
        .as_ref()
        .is_none_or(|declarations| native.fragment(declarations, "command-palette").is_none())
    {
        assert!(
            Instant::now() < deadline,
            "actual dedicated consumer did not publish"
        );
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    let (provider, tree) = native.fragment(declarations, "command-palette").unwrap();
    assert_eq!(provider, "tommymorgan.command-palette");
    let mut consumed = Vec::new();
    uses(tree, &Context::default(), &mut consumed);
    // Join opaque issued locators to metadata, never manufacture expectations
    // from that metadata. All expected identities/targets were established above.
    for (key, owner, action, context) in expected {
        let command = commands
            .iter()
            .find(|command| command["key"] == key)
            .unwrap();
        let locator = command["fragment_name"].as_str().unwrap();
        let matching: Vec<_> = consumed
            .iter()
            .filter(|(name, inherited)| {
                if name != locator {
                    return false;
                }
                let (actual_owner, fragment) = native.fragment(declarations, name).unwrap();
                assert_eq!(actual_owner, owner, "{key}");
                let mut contexts = Vec::new();
                action_contexts(
                    fragment,
                    inherited,
                    (&native, declarations),
                    &action,
                    0,
                    &mut contexts,
                );
                contexts.contains(&context)
            })
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "dedicated consumer lost/merged {key} at {context:?}"
        );
    }
    for (name, _) in consumed {
        let matching: Vec<_> = commands
            .iter()
            .filter(|command| command["fragment_name"] == name)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "consumer synthesized a non-catalog command: {name}"
        );
    }
}

#[test]
fn legacy_generation_one_consumer_preserves_action_response_and_running_shell() {
    let core = support::Core::new();
    let original = core.request(
        "workspace.create",
        json!({"label":"Legacy workspace","cwd":core.home,"focus":true}),
    );
    let target = core.request(
        "tab.create",
        json!({"workspace_id":original["workspace"]["workspace_id"],"label":"Legacy target","cwd":core.home,"focus":false}),
    );
    let tab = target["tab"]["tab_id"].as_str().unwrap();
    let pane = target["root_pane"]["pane_id"].as_str().unwrap();
    let root = env!("CARGO_MANIFEST_DIR");
    for plugin in ["agent-tree", "stock-ui", "sidebar-menus"] {
        core.request(
            "plugin.link",
            json!({"path":format!("{root}/plugins/{plugin}"),"enabled":true}),
        );
    }
    // Retain the old consumer's existing plain-Surface fixture and readiness
    // prerequisite. The actual focus action comes from the bundled agent tree.
    let tone = core.home.join("semantic-tone");
    std::fs::create_dir_all(&tone).unwrap();
    std::fs::write(tone.join("trailboss-plugin.toml"), "id = \"fixture.semantic-tone\"\nname = \"Semantic tone\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tone\"\nsurface = \"sidebar\"\ncommand = [\"sh\", \"producer.sh\"]\nstream = true\n").unwrap();
    std::fs::write(tone.join("producer.sh"), "printf '%s\\n' '{\"herdr\":\"widget_tree\",\"version\":1,\"zones\":[{\"zone\":\"footer\",\"tree\":{\"type\":\"label\",\"text\":\"Neutral tone\",\"role\":\"muted\",\"tone\":\"fg-cyan\"}}]}'\nwhile read -r line; do :; done\n").unwrap();
    core.request("plugin.link", json!({"path":tone,"enabled":true}));
    let before_pid = core.home.join("legacy-shell-before");
    let after_pid = core.home.join("legacy-shell-after");
    core.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":format!(
            "printf '%s' \"$$\" > '{}'; printf '\\033[2J\\033[H\\033[1;38;2;17;34;51mneutral-%s\\033[0m\\033[4;7H\\033[6 q' live; read legacy_hold\n",
            before_pid.display()
        )}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !before_pid.exists() {
        assert!(
            Instant::now() < deadline,
            "original shell did not reach its read"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert_eq!(before["focused_tab_id"], original["tab"]["tab_id"]);
    let output = core.home.join("legacy-consumer.json");
    let socket = core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    ));
    // Reuse the pre-discovery Python consumer unchanged: its handwritten
    // framing/field decoder imports no application or shared Rust codec.
    let mut child = Command::new("python3")
        .args([
            format!("{root}/tests/support/neutral_consumer.py"),
            socket.to_string_lossy().into_owned(),
            tab.to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "generation-one consumer failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("generation-one consumer exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let state: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(state["action"]["plugin_id"], "tommymorgan.agent-tree");
    assert_eq!(state["action"]["action_id"], "focus-tab");
    assert_eq!(state["action"]["context"]["tab_id"], tab);
    assert_eq!(state["action"]["context"]["invocation_source"], "plugin_ui");
    let snapshots = state["snapshots"].as_array().unwrap();
    let boot = snapshots[0]["boot_id"].as_str().unwrap();
    assert!(!boot.is_empty());
    assert_eq!(snapshots[0]["focused_tab_id"], original["tab"]["tab_id"]);
    assert_eq!(snapshots.last().unwrap()["focused_tab_id"], tab);
    assert!(snapshots.iter().all(|snapshot| snapshot["boot_id"] == boot));
    let responses: Vec<_> = state["responses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|response| response["id"] == "widget-action-1")
        .collect();
    assert!(
        !responses.is_empty(),
        "legacy request received no correlated response"
    );
    assert!(responses.iter().all(|response| response["boot"] == boot));
    assert_eq!(responses.last().unwrap()["final"], 1);
    let bytes: String = responses
        .iter()
        .map(|response| response["data"].as_str().unwrap())
        .collect();
    let reply: Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(reply["id"], "widget-action-1");
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(reply["result"]["type"], "plugin_action_invoked");
    assert_eq!(reply["result"]["context"]["tab_id"], tab);
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
        .expect("unchanged Surface layout did not decode the running shell");
    assert_eq!(frame["boot"], boot);
    assert_eq!(frame["panes"][0]["id"], pane);
    assert_eq!(frame["panes"][0]["focused"], 1);
    assert_eq!(frame["cursor"], json!([6, 3, 1, 6]));
    let actual = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert_eq!(actual["focused_tab_id"], tab);
    assert_eq!(actual["focused_pane_id"], pane);
    let identities = |snapshot: &Value| -> std::collections::BTreeMap<String, Value> {
        snapshot["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pane| {
                (
                    pane["pane_id"].as_str().unwrap().to_owned(),
                    json!({
                        "terminal_id":pane["terminal_id"],
                        "workspace_id":pane["workspace_id"],"tab_id":pane["tab_id"]
                    }),
                )
            })
            .collect()
    };
    assert_eq!(identities(&actual), identities(&before));
    // Release the same shell's pending read and obtain its PID again. A new
    // shell or pane that merely renders equivalent text cannot satisfy this.
    core.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":format!(
            "released\nprintf '%s' \"$$\" > '{}'\n", after_pid.display()
        )}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !after_pid.exists() {
        assert!(
            Instant::now() < deadline,
            "original shell did not continue after focus"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let original_pid = std::fs::read_to_string(before_pid).unwrap();
    assert!(original_pid.parse::<u32>().unwrap() > 0);
    assert_eq!(std::fs::read_to_string(after_pid).unwrap(), original_pid);
}

#[test]
fn core_public_operations_preserve_captured_targets_and_unrelated_resources() {
    use std::collections::BTreeMap;

    // Observe durable resource identity, ownership and names, not incidental
    // focus flags or shell/agent status that may change between snapshots.
    fn resources(snapshot: &Value) -> BTreeMap<&'static str, BTreeMap<String, Value>> {
        [
            ("workspaces", "workspace_id", &["workspace_id", "label"][..]),
            ("tabs", "tab_id", &["tab_id", "workspace_id", "label"][..]),
            (
                "panes",
                "pane_id",
                &["pane_id", "terminal_id", "workspace_id", "tab_id"][..],
            ),
        ]
        .into_iter()
        .map(|(kind, id, fields)| {
            let rows = snapshot[kind]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    let identity = row[id].as_str().unwrap().to_owned();
                    let values = fields
                        .iter()
                        .map(|field| ((*field).to_owned(), row[*field].clone()))
                        .collect();
                    (identity, Value::Object(values))
                })
                .collect();
            (kind, rows)
        })
        .collect()
    }

    // This is the existing public operation inventory, independent of discovery
    // or a palette tree. Requests produced by NativeCore run on an owned Core.
    for action in [
        "host:new-space",
        "host:new",
        "host:new-tab",
        "host:rename-space",
        "host:rename-tab",
        "host:close-tab",
        "host:focus",
        "host:restart-idle-agents",
        "host:detach",
    ] {
        let core = support::Core::new();
        let alpha = core.request(
            "workspace.create",
            json!({"label":"Alpha","cwd":core.home,"focus":true}),
        );
        let beta = core.request(
            "workspace.create",
            json!({"label":"Beta","cwd":core.home,"focus":false}),
        );
        for (resource, label) in [(&alpha, "A1"), (&beta, "B1")] {
            core.request(
                "tab.rename",
                json!({"tab_id":resource["tab"]["tab_id"],"label":label}),
            );
        }
        if action == "host:close-tab" {
            // Keep the target space live so its other tab is an explicit
            // unrelated-resource witness, not a last-tab policy assumption.
            core.request(
                "tab.create",
                json!({"workspace_id":beta["workspace"]["workspace_id"],"label":"B2","focus":false}),
            );
        }
        core.request("tab.focus", json!({"tab_id":alpha["tab"]["tab_id"]}));
        let workspace = beta["workspace"]["workspace_id"].as_str().unwrap();
        let tab = beta["tab"]["tab_id"].as_str().unwrap();
        let pane = beta["root_pane"]["pane_id"].as_str().unwrap();
        let context: Context = serde_json::from_value(json!({
            "workspace_id":workspace,"tab_id":tab,"pane_id":pane
        }))
        .unwrap();
        let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
        assert_eq!(before["focused_tab_id"], alpha["tab"]["tab_id"], "{action}");
        assert!(before["agents"].as_array().unwrap().is_empty(), "{action}");
        let mut expected = resources(&before);
        let mut native = NativeCore::default();
        let initial = if action == "host:new" { &beta } else { &alpha };
        native.update_session(
            initial["workspace"]["workspace_id"]
                .as_str()
                .map(str::to_owned),
            initial["tab"]["tab_id"].as_str().map(str::to_owned),
            true,
        );
        let mut attached = (action == "host:detach").then(|| core.attach());
        if let Some(client) = attached.as_mut() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while client.boot_id.is_empty() {
                assert!(Instant::now() < deadline, "detach client did not attach");
                client.receive().unwrap();
            }
        }
        let mut effect = native.invoke(action, &context, None);
        if matches!(action, "host:new" | "host:rename-space" | "host:rename-tab") {
            assert!(matches!(effect, Effect::Consumed), "{action}");
            assert!(
                native.editing(),
                "{action} must retain its existing name interaction"
            );
            // Move native focus after opening the interaction. Submission must
            // still use the captured scope, not whichever tab is current now.
            let changed = if action == "host:new" { &alpha } else { &beta };
            native.update_session(
                changed["workspace"]["workspace_id"]
                    .as_str()
                    .map(str::to_owned),
                changed["tab"]["tab_id"].as_str().map(str::to_owned),
                true,
            );
            assert!(matches!(
                native.text("Core semantics result"),
                Some(Effect::Consumed)
            ));
            effect = native
                .text("\r")
                .expect("existing name interaction must submit");
        }
        assert!(!native.editing(), "{action} left an interaction open");
        let reply = match effect {
            Effect::Request { method, params } => core.request(method, params),
            Effect::Consumed if action == "host:detach" => {
                assert!(native.detach_requested);
                attached
                    .as_mut()
                    .unwrap()
                    .input()
                    .unwrap()
                    .detach()
                    .unwrap();
                Value::Null
            }
            _ => panic!("{action} did not reach its existing native operation"),
        };
        if action == "host:restart-idle-agents" {
            assert_eq!(reply["type"], "agent_restart_idle");
            assert_eq!(reply["results"], json!([]));
        }
        let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
        let actual = resources(&after);
        match action {
            "host:rename-space" => {
                expected
                    .get_mut("workspaces")
                    .unwrap()
                    .get_mut(workspace)
                    .unwrap()["label"] = json!("Core semantics result");
            }
            "host:rename-tab" => {
                expected.get_mut("tabs").unwrap().get_mut(tab).unwrap()["label"] =
                    json!("Core semantics result");
            }
            "host:close-tab" => {
                expected.get_mut("tabs").unwrap().remove(tab).unwrap();
                expected.get_mut("panes").unwrap().remove(pane).unwrap();
            }
            "host:new-space" | "host:new" | "host:new-tab" => {
                let new_workspace = if action == "host:new-space" {
                    let added: Vec<_> = actual["workspaces"]
                        .iter()
                        .filter(|(id, _)| !expected["workspaces"].contains_key(*id))
                        .collect();
                    assert_eq!(added.len(), 1, "{action}");
                    assert_eq!(added[0].1["label"], "New space", "{action}");
                    added[0].0.as_str()
                } else {
                    workspace
                };
                let tabs: Vec<_> = actual["tabs"]
                    .iter()
                    .filter(|(id, _)| !expected["tabs"].contains_key(*id))
                    .collect();
                assert_eq!(tabs.len(), 1, "{action}");
                assert_eq!(tabs[0].1["workspace_id"], new_workspace, "{action}");
                assert_eq!(
                    tabs[0].1["label"],
                    match action {
                        "host:new" => json!("Core semantics result"),
                        "host:new-tab" => json!("New tab"),
                        _ => alpha["tab"]["label"].clone(),
                    },
                    "{action}"
                );
                let panes: Vec<_> = actual["panes"]
                    .iter()
                    .filter(|(id, _)| !expected["panes"].contains_key(*id))
                    .collect();
                assert_eq!(panes.len(), 1, "{action}");
                assert_eq!(panes[0].1["workspace_id"], new_workspace, "{action}");
                assert_eq!(panes[0].1["tab_id"], tabs[0].0.as_str(), "{action}");
                assert_eq!(after["focused_tab_id"], tabs[0].0.as_str(), "{action}");
                // Core allocates these IDs. Only admit the independently
                // checked additions; every pre-existing row must match below.
                for kind in ["workspaces", "tabs", "panes"] {
                    let additions: Vec<_> = actual[kind]
                        .iter()
                        .filter(|(id, _)| !expected[kind].contains_key(*id))
                        .map(|(id, row)| (id.clone(), row.clone()))
                        .collect();
                    assert_eq!(
                        additions.len(),
                        usize::from(kind != "workspaces" || action == "host:new-space"),
                        "{action}: unexpected {kind} additions"
                    );
                    expected.get_mut(kind).unwrap().extend(additions);
                }
            }
            _ => {}
        }
        assert_eq!(
            actual, expected,
            "{action} changed unrelated resource state"
        );
        if action == "host:focus" {
            assert_eq!(after["focused_workspace_id"], workspace);
            assert_eq!(after["focused_tab_id"], tab);
            assert_eq!(after["focused_pane_id"], pane);
        } else if !matches!(action, "host:new-space" | "host:new" | "host:new-tab") {
            assert_eq!(after["focused_tab_id"], alpha["tab"]["tab_id"], "{action}");
        }
    }
}

#[test]
fn palette_discovery_finds_and_runs_independent_inventory_without_sidebar_focus() {
    use trailboss::sidebar::Declarations;
    use trailboss_runtime::protocol::widgets::Node;

    struct EndpointTap(std::process::Child);
    impl Drop for EndpointTap {
        fn drop(&mut self) {
            drop(self.0.stdin.take());
            let _ = self.0.wait();
        }
    }

    // This is the accepted user-operation inventory, not CORE_COMMANDS, the
    // discovery response, or a fixture palette assembled from that response.
    let inventory = [
        ("fixture.red", "record-red", None, "owner"),
        ("fixture.blue", "record-blue", None, "owner"),
        ("fixture.green", "record-green", None, "owner"),
        (
            "tommymorgan.sidebar-menus",
            "close-space",
            None,
            "workspace.close",
        ),
        ("trailboss.core", "host:new-space", None, "workspace.create"),
        ("trailboss.core", "host:new", None, "tab.create"),
        ("trailboss.core", "host:new-tab", None, "tab.create"),
        (
            "trailboss.core",
            "host:rename-space",
            None,
            "workspace.rename",
        ),
        ("trailboss.core", "host:rename-tab", None, "tab.rename"),
        ("trailboss.core", "host:close-tab", None, "tab.close"),
        (
            "trailboss.core",
            "host:focus",
            Some("targeted"),
            "tab.focus",
        ),
        ("trailboss.core", "host:focus", Some("previous"), "previous"),
        (
            "trailboss.core",
            "host:restart-idle-agents",
            None,
            "restart",
        ),
        ("trailboss.core", "host:detach", None, "detach"),
    ];

    // Read public declaration structure only to navigate producer-chosen target
    // menus. Expected operations/owners above remain independent of that tree.
    fn paths(
        native: &NativeCore,
        declarations: &Declarations,
        node: &Node,
        scope: (&str, &Context),
        route: &[String],
        depth: usize,
        found: &mut Vec<(String, String, Context, Vec<String>)>,
    ) {
        let (provider, inherited) = scope;
        assert!(depth < 32, "cyclic public palette declaration");
        match node {
            Node::Use { name } => {
                if let Some((owner, tree)) = native.fragment(declarations, name) {
                    paths(
                        native,
                        declarations,
                        tree,
                        (owner, inherited),
                        route,
                        depth + 1,
                        found,
                    );
                }
            }
            Node::Column { children } | Node::List { entries: children } => {
                for child in children {
                    paths(
                        native,
                        declarations,
                        child,
                        (provider, inherited),
                        route,
                        depth + 1,
                        found,
                    );
                }
            }
            Node::Row {
                children,
                action,
                context,
                menu,
            } => {
                let context = if context == &Context::default() {
                    inherited
                } else {
                    context
                };
                let title = children
                    .iter()
                    .filter_map(|child| match child {
                        Node::Label { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let mut route = route.to_vec();
                route.push(title);
                if let Some(action) = action {
                    found.push((
                        provider.to_owned(),
                        action.clone(),
                        context.clone(),
                        route.clone(),
                    ));
                }
                if let Some(menu) = menu {
                    paths(
                        native,
                        declarations,
                        menu,
                        (provider, context),
                        &route,
                        depth + 1,
                        found,
                    );
                }
            }
            _ => {}
        }
    }

    for (owner, action, mode, outcome) in inventory {
        let core = support::Core::new();
        let review = core.request(
            "workspace.create",
            json!({"label":"Palette Review","cwd":core.home,"focus":true}),
        );
        let workspace = review["workspace"]["workspace_id"].as_str().unwrap();
        let tab = review["tab"]["tab_id"].as_str().unwrap();
        let pane = review["root_pane"]["pane_id"].as_str().unwrap();
        let other = core.request(
            "workspace.create",
            json!({"label":"Palette Other","cwd":core.home,"focus":false}),
        );
        for color in ["red", "blue", "green"] {
            let root = core.home.join(color);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("trailboss-plugin.toml"), format!(
                "id = \"fixture.{color}\"\nname = \"Palette {color} owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"record-{color}\"\ntitle = \"Record {color} target\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n")).unwrap();
            std::fs::write(root.join("record.py"),
                "import json,os\nwith open('executed.jsonl','a') as f:\n f.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
            core.request("plugin.link", json!({"path":root,"enabled":true}));
        }
        // An ordinary composition replacement hides the sidebar without
        // replacing, registering or manufacturing a command-palette fragment.
        let hidden = core.home.join("surface-only");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("trailboss-plugin.toml"),
            "id = \"fixture.surface-only\"\nname = \"Hidden sidebar\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"surface\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"surface.py\"]\n").unwrap();
        std::fs::write(hidden.join("surface.py"),
            "print('{\"herdr\":\"widget_tree\",\"version\":1,\"zones\":[],\"composition\":{\"priority\":1000000,\"tree\":{\"type\":\"surface\"}}}',flush=True)\n").unwrap();
        core.request("plugin.link", json!({"path":hidden,"enabled":true}));
        // Establish existing owners before starting the default palette. A
        // confirmation-bearing action is not eligible while its ordinary
        // owner export is still pending; the first consumer root may correctly
        // withhold it. This waits for owner prerequisites, never palette rows.
        core.request(
            "plugin.link",
            json!({
                "path":concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/sidebar-menus"),
                "enabled":true
            }),
        );
        let mut owners = core.attach();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let catalog = core.request("ui.command.list", json!({}));
            if owners.declarations.as_ref().is_some_and(|declarations| {
                catalog["commands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|command| {
                        command["diagnostic"].is_null()
                            && command["fragment_name"].as_str().is_some_and(|name| {
                                NativeCore::default().fragment(declarations, name).is_some()
                            })
                    })
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "existing owner prerequisites unavailable: {catalog}"
            );
            owners.receive().unwrap_or_else(|error| {
                panic!("existing owners failed before palette startup: {error}; catalog={catalog}; logs={}",
                    core.request("plugin.log.list", json!({"limit":200})))
            });
        }
        drop(owners);
        // Reuse the transparent native-endpoint witness: an empty agent list
        // alone cannot prove that Restart was actually dispatched.
        let _tap = if outcome == "restart" {
            let endpoint = core.home.join(format!(
                "trailboss/sessions/{}/trailboss-client.sock",
                core.name
            ));
            let child = Command::new("python3").args(["-u", "-c",
                "import sys,select,json\nfrom pathlib import Path\nsys.path.insert(0,sys.argv[1])\nfrom command_interactions import OwnedEndpointTap\nevidence=Path(sys.argv[3]); tap=OwnedEndpointTap(Path(sys.argv[2]),evidence)\n(evidence/'tap-ready').touch()\ntry:\n while not select.select([sys.stdin],[],[],0.02)[0]:\n  result=tap.restart_result()\n  if result: (evidence/'restart-result.json').write_text(json.dumps(result))\nfinally: tap.close()\n",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support")])
                .arg(endpoint).arg(&core.home).stdin(std::process::Stdio::piped()).spawn().unwrap();
            let tap = EndpointTap(child);
            let deadline = Instant::now() + Duration::from_secs(8);
            while !core.home.join("tap-ready").exists() {
                assert!(
                    Instant::now() < deadline,
                    "owned endpoint tap failed to start"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Some(tap)
        } else {
            None
        };
        let mut console = console::Console::new(&core);
        let mut observer = core.attach();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            observer.receive().unwrap_or_else(|error| {
                panic!(
                    "default-producer readiness failed before inventory assertion: {error}; \
                     received declarations={:?}; registered plugins={}; producer logs={}",
                    observer.declarations,
                    core.request("plugin.list", json!({})),
                    core.request("plugin.log.list", json!({"limit":200})),
                )
            });
            if observer.declarations.as_ref().is_some_and(|declarations| {
                declarations
                    .iter()
                    .any(|(id, d)| id == "fixture.surface-only" && d.composition.is_some())
                    && declarations
                        .iter()
                        .any(|(_, d)| d.fragments.iter().any(|f| f.name == "command-palette"))
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "default palette never published");
        }
        let catalog = core.request("ui.command.list", json!({}));
        let key = if owner == "trailboss.core" {
            let mut key = json!({"kind":"core","action_id":action});
            if let Some(mode) = mode {
                key["mode"] = json!(mode);
            }
            key
        } else {
            json!({"kind":"plugin","plugin_id":owner,"action_id":action})
        };
        let command = catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == key)
            .expect("independent eligible operation missing from Core");
        let declarations = observer.declarations.as_ref().unwrap();
        let native = NativeCore::default();
        let (provider, root) = native.fragment(declarations, "command-palette").unwrap();
        let mut available = Vec::new();
        paths(
            &native,
            declarations,
            root,
            (provider, &Context::default()),
            &[],
            0,
            &mut available,
        );
        let selection = available.iter().find(|(actual_owner, actual_action, context, _)| {
            actual_owner == owner && actual_action == action
                && if mode == Some("previous") { context.tab_id.is_none() }
                else if action == "host:focus" { context.tab_id.as_deref() == Some(tab) }
                else { context.workspace_id.as_deref().is_none_or(|id| id == workspace)
                    && context.tab_id.as_deref().is_none_or(|id| id == tab) }
        }).unwrap_or_else(|| panic!("default palette omits independent eligible command {key}; metadata={command}; available={available:?}"));
        // Owner-bound commands may have equal labels for different captured
        // targets. Navigate their actual declaration order, not the first
        // text match, while keeping the expected owner/action inventory above.
        let root_match = available
            .iter()
            .filter(|(actual_owner, actual_action, _, route)| {
                actual_owner == owner && actual_action == action && route == &selection.3
            })
            .position(|candidate| candidate == selection)
            .unwrap();
        assert!(
            selection
                .3
                .iter()
                .any(|title| title == command["title"].as_str().unwrap()),
            "palette must expose the authoritative command title: {command}"
        );
        let providers: Vec<_> = declarations
            .iter()
            .filter(|(_, d)| d.fragments.iter().any(|f| f.name == "command-palette"))
            .collect();
        assert_eq!(providers.len(), 1, "competing bundled palette providers");
        assert_ne!(
            provider, "tommymorgan.sidebar-menus",
            "sidebar must no longer own the default palette"
        );
        drop(observer);
        // Observe a later Surface update on this native connection only after
        // Core published the palette, so F1 cannot race its declarations.
        core.request(
            "pane.send_text",
            json!({"pane_id":pane,"text":"printf 'PALETTE_OWNED_READY\\n'\n"}),
        );
        console.wait(|rows| {
            rows.iter()
                .any(|row| row.starts_with("PALETTE_OWNED_READY"))
        });
        if mode == Some("previous") {
            core.request("tab.focus", json!({"tab_id":other["tab"]["tab_id"]}));
            std::thread::sleep(Duration::from_millis(200));
            core.request("tab.focus", json!({"tab_id":tab}));
            std::thread::sleep(Duration::from_millis(200));
        }
        if mode == Some("targeted") {
            core.request("tab.focus", json!({"tab_id":other["tab"]["tab_id"]}));
            std::thread::sleep(Duration::from_millis(200));
        }
        console.text("\x1bOP");
        for (index, label) in selection.3.iter().enumerate() {
            if matches!(outcome, "workspace.close" | "tab.close") && index + 1 == selection.3.len()
            {
                console.wait(|rows| {
                    rows.iter().any(|row| row.contains(label))
                        && rows.iter().any(|row| row.contains("Cancel"))
                });
                console.text("\x1b[B");
            } else {
                console.text(label);
                console.wait(|rows| rows.iter().any(|row| row.contains(label)));
                if index == 0 {
                    for _ in 0..root_match {
                        console.text("\x1b[B");
                    }
                }
            }
            console.text("\r");
        }
        if matches!(
            outcome,
            "workspace.create" | "tab.create" | "workspace.rename" | "tab.rename"
        ) {
            console.text("Palette Named Result");
            console.wait(|rows| rows.iter().any(|row| row.contains("Palette Named Result")));
            console.text("\r");
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
            let complete = match outcome {
                "owner" => {
                    let records = core
                        .home
                        .join(owner.strip_prefix("fixture.").unwrap())
                        .join("executed.jsonl");
                    if let Some(text) = std::fs::read_to_string(records)
                        .ok()
                        .filter(|text| text.ends_with('\n'))
                    {
                        let rows: Vec<Value> = text
                            .lines()
                            .map(|line| serde_json::from_str(line).unwrap())
                            .collect();
                        assert_eq!(rows.len(), 1);
                        assert_eq!(rows[0]["owner"], owner);
                        assert_eq!(rows[0]["action"], action);
                        assert_eq!(rows[0]["context"]["workspace_id"], workspace);
                        assert_eq!(rows[0]["context"]["tab_id"], tab);
                        assert_eq!(rows[0]["context"]["pane_id"], pane);
                        for color in ["red", "blue", "green"] {
                            if owner != format!("fixture.{color}") {
                                assert!(
                                    !core.home.join(color).join("executed.jsonl").exists(),
                                    "selection was dispatched to an unrelated owner"
                                );
                            }
                        }
                        true
                    } else {
                        false
                    }
                }
                "workspace.create" | "workspace.rename" => {
                    snapshot["workspaces"].as_array().unwrap().iter().any(|w| {
                        w["label"] == "Palette Named Result"
                            && (outcome == "workspace.create" || w["workspace_id"] == workspace)
                    })
                }
                "tab.create" | "tab.rename" => {
                    snapshot["tabs"].as_array().unwrap().iter().any(|t| {
                        t["label"] == "Palette Named Result"
                            && t["workspace_id"] == workspace
                            && (outcome == "tab.create" || t["tab_id"] == tab)
                    })
                }
                "tab.close" => !snapshot["tabs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["tab_id"] == tab),
                "workspace.close" => {
                    let gone = !snapshot["workspaces"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|w| w["workspace_id"] == workspace);
                    let logs = core.request("plugin.log.list", json!({"plugin_id":owner}));
                    let succeeded = logs["logs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|log| log["action_id"] == action && log["status"] == "succeeded")
                        .count();
                    assert!(succeeded <= 1, "duplicate owner invocation: {logs}");
                    gone && succeeded == 1
                }
                "tab.focus" => snapshot["focused_tab_id"] == tab,
                "previous" => snapshot["focused_tab_id"] == other["tab"]["tab_id"],
                "detach" => console.child.try_wait().unwrap().is_some(),
                "restart" => core.home.join("restart-result.json").is_file(),
                _ => unreachable!(),
            };
            if complete {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "native selection did not perform {key} on its captured target: {snapshot}; route={:?}; screen={:?}; owner logs={}",
                selection.3,
                console.rows(),
                core.request("plugin.log.list", json!({"plugin_id":owner,"limit":100}))
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        if outcome != "detach" {
            core.request(
                "plugin.disable",
                json!({"plugin_id":"fixture.surface-only"}),
            );
            console.wait(|rows| rows.iter().any(|row| row.starts_with('▏')));
            console.text("\x1b[<35;20;24M");
            let rows = console.wait(|rows| rows[23].contains('⋯'));
            let x = rows[23].chars().position(|ch| ch == '⋯').unwrap();
            console.click(x, 23);
            console.wait(|rows| rows.iter().any(|row| row.contains("Detach")));
            console.text("\x1b");
        }
    }
}

#[test]
fn reserved_core_action_collision_is_discoverable_but_cannot_detach_through_plugin_use() {
    let core = support::Core::new();
    let owner = core.home.join("reserved-owner");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::write(
        owner.join("trailboss-plugin.toml"),
        "id = \"fixture.reserved\"\nname = \"Reserved owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"host:detach\"\ntitle = \"Plugin detach collision\"\ncommand = [\"true\"]\n",
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let catalog = core.request("ui.command.list", json!({}));
    let command = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| {
            command["key"]
                == json!({"kind":"plugin","plugin_id":"fixture.reserved","action_id":"host:detach"})
        })
        .expect("manifest declaration remains discoverable");
    let locator = command["fragment_name"].as_str().unwrap();
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.boot_id.is_empty() || client.declarations.is_none() {
        client.receive().unwrap();
        assert!(Instant::now() < deadline);
    }
    assert!(
        NativeCore::default()
            .fragment(client.declarations.as_ref().unwrap(), locator)
            .is_none(),
        "a plugin locator must not resolve to the reserved native detach operation"
    );
    assert!(
        command["diagnostic"]
            .as_str()
            .is_some_and(|diagnostic| diagnostic.contains("fixture.reserved/host:detach")),
        "unresolved owner must be reported without suppressing metadata"
    );
    let legacy = core.request("plugin.action.list", json!({}));
    assert!(
        legacy["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["plugin_id"] == "fixture.reserved"
                && action["action_id"] == "host:detach"),
        "legacy action registration must not change"
    );
}

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
    // The stream reader must observe complete declarations, not truncation
    // between opening and writing a fixture update.
    let publish = |file: &str, value: Value| {
        let target = owner.join(file);
        let pending = target.with_extension("next");
        std::fs::write(&pending, value.to_string()).unwrap();
        std::fs::rename(pending, target).unwrap();
    };
    publish("current.json", publication("pending", None));
    publish("duplicate.json", publication("duplicate-empty", None));
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
            client.receive().unwrap_or_else(|error| {
                panic!("ordinary producer state {marker} was not projected: {error}")
            });
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
    publish(
        "current.json",
        publication("valid-first", Some(valid.clone())),
    );
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
        publish("current.json", publication(&marker, tree));
        check(&marker, false);
    }
    publish(
        "current.json",
        publication("before-duplicate", Some(valid.clone())),
    );
    check("before-duplicate", true);
    publish(
        "duplicate.json",
        publication("duplicate-present", Some(valid.clone())),
    );
    check("duplicate-present", false);
    publish("duplicate.json", publication("duplicate-withdrawn", None));
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
            None,
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
                None,
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

#[test]
fn c24_same_captured_scope_keeps_second_confirmation_locator_through_real_refresh() {
    confirmation_locator_refresh(true);
}

#[test]
fn c24_default_scope_shortened_identical_labels_keep_second_confirmation_through_refresh() {
    confirmation_locator_refresh(false);
}

fn confirmation_locator_refresh(captured_scope: bool) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use trailboss::{
        sidebar::Declarations,
        terminal::{menus::Response, FrameRenderer, Rect},
    };
    use trailboss_runtime::protocol::widgets::Node;
    let core = support::Core::new();
    let target = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let other = core.request(
        "workspace.create",
        json!({"label":"Other","cwd":core.home,"focus":false}),
    );
    let context = if captured_scope {
        json!({"workspace_id":target["workspace"]["workspace_id"],
            "tab_id":target["tab"]["tab_id"],"pane_id":target["root_pane"]["pane_id"]})
    } else {
        json!({"workspace_id":null,"tab_id":null,"pane_id":null})
    };
    let owner = core.home.join("confirmation-owner");
    let consumer = core.home.join("confirmation-consumer");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::create_dir_all(&consumer).unwrap();
    let mut manifest = String::from(
        "id = \"fixture.destructive\"\nname = \"Confirmation identity owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n");
    for action in ["first", "second"] {
        let title = if captured_scope {
            "Remove owned fixture".to_owned()
        } else {
            format!("{} {action}", "界".repeat(250))
        };
        manifest.push_str(&format!(
            "[[actions]]\nid = {action:?}\ntitle = {}\ncontexts = [\"pane\"]\nconfirmation_fragment = \"confirm.{action}\"\ncommand = [\"python3\", \"delete.py\"]\n",
            json!(title)));
        std::fs::write(
            owner.join(format!("{action}-state")),
            format!("original {action}"),
        )
        .unwrap();
    }
    manifest.push_str("[[ui]]\nid = \"confirmations\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"publish.py\"]\nstream = true\n");
    std::fs::write(owner.join("trailboss-plugin.toml"), manifest).unwrap();
    std::fs::write(owner.join("delete.py"),
        "import json, os, pathlib\naction = os.environ['HERDR_PLUGIN_ACTION_ID']\npathlib.Path(action+'-state').unlink()\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':action,'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
    let publisher = "import pathlib, time\npath = pathlib.Path('ui.json')\nlast = None\nwhile True:\n text = path.read_text()\n if text != last:\n  print(text, flush=True)\n  last = text\n time.sleep(0.02)\n";
    std::fs::write(owner.join("publish.py"), publisher).unwrap();
    std::fs::write(consumer.join("publish.py"), publisher).unwrap();
    let owner_ui = |stage: &str, reverse: bool| {
        let mut actions = vec!["first", "second"];
        if reverse {
            actions.reverse();
        }
        json!({"herdr":"widget_tree","version":1,"zones":[],
            "fragments":actions.into_iter().map(|action| json!({
                "name":format!("confirm.{action}"),"priority":0,
                "tree":{"type":"column","children":[
                    {"type":"label","text":format!("Confirm {action} {stage}"),"role":"primary"},
                    {"type":"row","children":[{"type":"label","text":"Cancel","role":"primary"}],"action":"host:cancel"},
                    {"type":"row","children":[{"type":"label","text":"Accept","role":"primary"}],"action":action}
                ]}
            })).collect::<Vec<_>>()})
    };
    std::fs::write(
        owner.join("ui.json"),
        owner_ui("initial", false).to_string(),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let keys: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|action| json!({"kind":"plugin","plugin_id":"fixture.destructive","action_id":action}))
        .collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    let commands = loop {
        let catalog = core.request("ui.command.list", json!({}));
        let commands: Vec<Value> = keys
            .iter()
            .filter_map(|key| {
                catalog["commands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|command| command["key"] == *key && command["diagnostic"].is_null())
                    .cloned()
            })
            .collect();
        if commands.len() == 2 {
            break commands;
        }
        assert!(
            Instant::now() < deadline,
            "actual same-owner confirmation exports were not published"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_ne!(commands[0]["fragment_name"], commands[1]["fragment_name"]);
    if !captured_scope {
        assert_ne!(
            commands[0]["title"], commands[1]["title"],
            "default-scope fixture must exercise distinct titles shortened to the same label"
        );
    }
    let names: Vec<String> = commands
        .iter()
        .map(|command| command["fragment_name"].as_str().unwrap().to_owned())
        .collect();
    let consumer_ui = |reverse: bool| {
        let order = if reverse { [1, 0] } else { [0, 1] };
        let uses = json!({"type":"column","children":order.into_iter().map(|index|
            json!({"type":"use","name":names[index]})).collect::<Vec<_>>()});
        let tree = if captured_scope {
            json!({"type":"column","children":[{
                "type":"row","children":[{"type":"label","text":"Captured target","role":"primary"}],
                "context":context,"menu":uses
            }]})
        } else {
            uses
        };
        json!({"herdr":"widget_tree","version":1,"zones":[],
            "fragments":[{"name":"command-palette","priority":1000000,"tree":tree}]})
    };
    std::fs::write(consumer.join("trailboss-plugin.toml"),
        "id = \"fixture.confirmation-consumer\"\nname = \"Neutral confirmation consumer\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"commands\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"publish.py\"]\nstream = true\n").unwrap();
    std::fs::write(consumer.join("ui.json"), consumer_ui(false).to_string()).unwrap();
    core.request("plugin.link", json!({"path":consumer,"enabled":true}));
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.boot_id.is_empty()
        || client.snapshot.is_none()
        || !client.declarations.as_ref().is_some_and(|declarations| {
            declarations.iter().any(|(owner, declaration)| {
                owner == "fixture.confirmation-consumer"
                    && declaration
                        .fragments
                        .iter()
                        .any(|fragment| fragment.name == "command-palette")
            })
        })
    {
        assert!(
            Instant::now() < deadline,
            "actual native confirmation consumer was not published"
        );
        client.receive().unwrap();
    }
    let mut native = NativeCore::default();
    let snapshot = client.snapshot.as_ref().unwrap();
    native.update_session(
        snapshot.focused_workspace_id.clone(),
        snapshot.focused_tab_id.clone(),
        true,
    );
    let declarations = client.declarations.as_ref().unwrap();
    let labels: Vec<_> = names
        .iter()
        .map(|name| {
            let (provider, tree) = native.fragment(declarations, name).unwrap();
            assert_eq!(provider, "fixture.destructive");
            let Node::Row {
                action, children, ..
            } = tree
            else {
                panic!("generated confirmation must be an ordinary Row")
            };
            assert!(
                action.is_none(),
                "a generated outer confirmation must have no destructive action"
            );
            let Node::Label { text, .. } = &children[0] else {
                panic!("missing generated display label")
            };
            text.clone()
        })
        .collect();
    assert_eq!(
        labels[0], labels[1],
        "both real issued commands must be display-identical"
    );
    assert_eq!(labels[0].ends_with('…'), !captured_scope);
    let viewport = Rect::viewport(100, 30);
    let mut renderer = FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(100, 30, 0).unwrap();
    let mut render =
        |renderer: &mut FrameRenderer, native: &NativeCore, declarations: &Declarations| {
            let mut bytes = Vec::new();
            renderer
                .draw_frame(&mut bytes, None, declarations, native, viewport)
                .unwrap();
            screen.write(&bytes);
            screen
                .screen_text_rows()
                .unwrap()
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .flat_map(|cell| cell.graphemes.iter())
                        .filter_map(|cp| char::from_u32(*cp))
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
    assert!(renderer.controls.open_palette(&native, declarations, None));
    if captured_scope {
        assert!(matches!(
            renderer.controls.key(
                &native,
                declarations,
                viewport,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Response::Consumed
        ));
    }
    assert!(matches!(
        renderer.controls.key(
            &native,
            declarations,
            viewport,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
        ),
        Response::Consumed
    ));
    assert!(matches!(
        renderer.controls.key(
            &native,
            declarations,
            viewport,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        Response::Consumed
    ));
    assert!(render(&mut renderer, &native, declarations).contains("Confirm second initial"),
        "native UI must open the second issued command's actual confirmation, not unwind an ambiguous path");
    assert!(!owner.join("executed.jsonl").exists());
    assert_eq!(
        std::fs::read_to_string(owner.join("first-state")).unwrap(),
        "original first"
    );
    assert_eq!(
        std::fs::read_to_string(owner.join("second-state")).unwrap(),
        "original second"
    );
    core.request(
        "workspace.focus",
        json!({"workspace_id":other["workspace"]["workspace_id"]}),
    );
    for (path, declaration) in [
        (owner.join("ui.json"), owner_ui("refreshed", true)),
        (consumer.join("ui.json"), consumer_ui(true)),
    ] {
        let pending = path.with_extension("next");
        std::fs::write(&pending, declaration.to_string()).unwrap();
        std::fs::rename(pending, path).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let refreshed = client.declarations.as_ref().is_some_and(|declarations| {
            let Some((_, fragment)) = declarations
                .iter()
                .filter(|(owner, _)| owner == "fixture.confirmation-consumer")
                .flat_map(|(owner, declaration)| {
                    declaration
                        .fragments
                        .iter()
                        .map(move |fragment| (owner, fragment))
                })
                .find(|(_, fragment)| fragment.name == "command-palette")
            else {
                return false;
            };
            let tree = serde_json::to_value(&fragment.tree).unwrap();
            let children = if captured_scope {
                &tree["children"][0]["menu"]["children"]
            } else {
                &tree["children"]
            };
            let reordered = children.as_array().is_some_and(|children| {
                children.len() == 2
                    && children[0]["name"] == names[1]
                    && children[1]["name"] == names[0]
            });
            let second_updated =
                native
                    .fragment(declarations, &names[1])
                    .is_some_and(|(_, tree)| {
                        let tree = serde_json::to_value(tree).unwrap();
                        tree["menu"]["children"][0]["text"] == "Confirm second refreshed"
                    });
            reordered && second_updated
        });
        let focused_other = client
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.focused_tab_id.as_deref())
            == other["tab"]["tab_id"].as_str();
        if refreshed && focused_other {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "actual reordered Uses, refreshed owner confirmation and later focus were not received"
        );
        client.receive().unwrap();
    }
    let snapshot = client.snapshot.as_ref().unwrap();
    native.update_session(
        snapshot.focused_workspace_id.clone(),
        snapshot.focused_tab_id.clone(),
        true,
    );
    let declarations = client.declarations.as_ref().unwrap();
    assert!(
        render(&mut renderer, &native, declarations).contains("Confirm second refreshed"),
        "native refresh must retain the second locator's confirmation ancestry after reorder"
    );
    assert!(matches!(
        renderer.controls.key(
            &native,
            declarations,
            viewport,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
        ),
        Response::Consumed
    ));
    let Response::Invoke(selected) = renderer.controls.key(
        &native,
        declarations,
        viewport,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ) else {
        panic!("accepting the retained confirmation must invoke its real owner");
    };
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
            None,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let logs = core.request(
            "plugin.log.list",
            json!({"plugin_id":"fixture.destructive"}),
        );
        if logs["logs"].as_array().unwrap().iter().any(|log| {
            matches!(log["action_id"].as_str(), Some("first" | "second"))
                && log["status"] == "succeeded"
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "real destructive fixture action did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(owner.join("first-state")).unwrap(),
        "original first",
        "acceptance must not mutate the display-identical first action's fixture state"
    );
    assert!(
        !owner.join("second-state").exists(),
        "only the second action must delete its fixture state"
    );
    let records: Vec<Value> = std::fs::read_to_string(owner.join("executed.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["owner"], "fixture.destructive");
    assert_eq!(records[0]["action"], "second");
    for field in ["workspace_id", "tab_id", "pane_id"] {
        assert_eq!(records[0]["context"][field], context[field],
            "confirmation acceptance must retain the original captured/default context, never later focus");
    }
}

#[test]
fn large_catalog_keeps_every_owner_action_and_unrelated_core_commands_searchable() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use trailboss::terminal::{
        menus::{Controls, Response},
        Rect,
    };

    let core = support::Core::new();
    let owner = core.home.join("large-catalog-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let mut manifest = String::from(
        "id = \"fixture.large-catalog\"\nname = \"Large catalog\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n",
    );
    for index in 0..700 {
        manifest.push_str(&format!(
            "[[actions]]\nid = \"action{index:03}\"\ntitle = \"Capacity {index:03}\"\ncommand = [\"sh\", \"record.sh\"]\n"
        ));
    }
    std::fs::write(owner.join("trailboss-plugin.toml"), manifest).unwrap();
    std::fs::write(
        owner.join("record.sh"),
        "printf '%s\\n' \"$HERDR_PLUGIN_ACTION_ID\" >> executed\n",
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    for plugin in ["stock-ui", "command-palette"] {
        core.request(
            "plugin.link",
            json!({"path":format!("{}/plugins/{plugin}",env!("CARGO_MANIFEST_DIR")),"enabled":true}),
        );
    }
    let mut client = core.attach();
    let native = NativeCore::default();
    let choose = |declarations: &trailboss::sidebar::Declarations, query: &str| {
        let mut controls = Controls::default();
        if !controls.open_palette(&native, declarations, None) {
            return Response::Pass;
        }
        controls.paste(query);
        controls.key(
            &native,
            declarations,
            Rect::viewport(100, 30),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if client.declarations.as_ref().is_some_and(|declarations| {
            matches!(choose(declarations, "Capacity 699"),
                Response::Invoke(action) if action.plugin == "fixture.large-catalog" && action.action == "action699")
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "supported large catalog made the dedicated palette unavailable"
        );
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    for index in 0..700 {
        let Response::Invoke(action) = choose(declarations, &format!("Capacity {index:03}")) else {
            panic!("catalog action {index} was lost");
        };
        assert_eq!(action.plugin, "fixture.large-catalog");
        assert_eq!(action.action, format!("action{index:03}"));
        if index == 0 || index == 699 {
            client
                .input()
                .unwrap()
                .invoke(
                    &client.boot_id,
                    &action.plugin,
                    &action.action,
                    &action.context,
                    None,
                )
                .unwrap();
        }
    }
    let Response::Prepare(action) = choose(declarations, "New space") else {
        panic!("large owner catalog hid the unrelated Core creation flow");
    };
    assert_eq!(action.plugin, "trailboss.core");
    assert_eq!(action.action, "host:new-space");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let receipt = std::fs::read_to_string(owner.join("executed")).unwrap_or_default();
        let mut actions = receipt.lines().collect::<Vec<_>>();
        actions.sort_unstable();
        if actions == ["action000", "action699"] && receipt.ends_with('\n') {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "first and last owner actions did not execute: {receipt:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn captured_owner_targets_are_visible_before_the_selected_workspace_is_invoked() {
    captured_owner_targets(None);
}

#[test]
fn long_control_bearing_pane_names_keep_owner_commands_searchable() {
    captured_owner_targets(Some((format!("Needle\n{}", "界".repeat(220)), None)));
}

#[test]
fn pane_renames_refresh_captured_owner_captions_and_search() {
    captured_owner_targets(Some((
        "PreviousPane".to_owned(),
        Some("RenamedPane".to_owned()),
    )));
}

fn captured_owner_targets(pane_names: Option<(String, Option<String>)>) {
    let core = support::Core::new();
    let alpha = core.request(
        "workspace.create",
        json!({"label":"Alpha","cwd":core.home,"focus":true}),
    );
    let beta = core.request(
        "workspace.create",
        json!({"label":"Beta","cwd":core.home,"focus":false}),
    );
    if let Some((initial, _)) = &pane_names {
        core.request(
            "pane.rename",
            json!({"pane_id":alpha["root_pane"]["pane_id"],"label":initial}),
        );
    }
    let owner = core.home.join("named-target-owner");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::write(owner.join("trailboss-plugin.toml"),
        format!("id = \"fixture.named-target\"\nname = \"Named targets\"\nversion = \"1.0.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"archive\"\ntitle = \"Archive\"\ncommand = [\"python3\", {:?}]\n[[ui]]\nid = \"targets\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"targets.py\"]\n",owner.join("record.py").to_str().unwrap())).unwrap();
    std::fs::write(owner.join("record.py"),
        format!("import json,os\nwith open({:?},'a') as output:\n output.write(json.dumps(json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON']))+'\\n')\n",owner.join("executed.jsonl").to_str().unwrap())).unwrap();
    let publication = json!({
        "herdr":"widget_tree","version":1,"zones":[],
        "fragments":[{"name":"fixture.targets","priority":0,"tree":{"type":"column","children":[
            {"type":"row","action":"archive",
             "context":{"workspace_id":alpha["workspace"]["workspace_id"],
                        "tab_id":alpha["tab"]["tab_id"],"pane_id":alpha["root_pane"]["pane_id"]},
             "children":[{"type":"label","text":"Archive Alpha","role":"label"}]},
            {"type":"row","action":"archive",
             "context":{"workspace_id":beta["workspace"]["workspace_id"],
                        "tab_id":beta["tab"]["tab_id"],"pane_id":beta["root_pane"]["pane_id"]},
             "children":[{"type":"label","text":"Archive Beta","role":"label"}]}
        ]}}]
    });
    std::fs::write(
        owner.join("targets.py"),
        format!("print({:?})\n", publication.to_string()),
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    core.request(
        "plugin.link",
        json!({
            "path":concat!(env!("CARGO_MANIFEST_DIR"),"/plugins/command-palette"),"enabled":true
        }),
    );
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(8);
    while client.declarations.as_ref().is_none_or(|declarations| {
        NativeCore::default()
            .fragment(declarations, "command-palette")
            .is_none()
            || !declarations
                .iter()
                .any(|(provider, _)| provider == "fixture.named-target")
    }) {
        assert!(Instant::now() < deadline, "owner palette did not publish");
        client.receive().unwrap();
    }
    let mut native = console::Console::new(&core);
    native.wait(|rows| rows.iter().any(|row| row.contains("Alpha")));
    native.text("\x1bOPArchive");
    native.wait(|rows| {
        ["Alpha", "Beta"].iter().all(|target| {
            rows.iter().any(|row| {
                row.contains(target)
                    && (row.contains("Archive")
                        || (*target == "Alpha"
                            && pane_names
                                .as_ref()
                                .is_some_and(|(_, rename)| rename.is_none())))
            })
        })
    });
    if let Some((_, renamed)) = &pane_names {
        let query = renamed.as_deref().unwrap_or("Needle");
        if let Some(renamed) = renamed {
            core.request(
                "pane.rename",
                json!({"pane_id":alpha["root_pane"]["pane_id"],"label":renamed}),
            );
            native.wait(|rows| {
                rows.iter()
                    .any(|row| row.contains("Archive") && row.contains(renamed))
            });
        }
        for _ in "Archive".chars() {
            native.text("\x7f");
        }
        native.text(query);
        native.wait(|rows| {
            rows.iter().any(|row| row.contains("Alpha"))
                && !rows
                    .iter()
                    .any(|row| row.contains("Archive") && row.contains("Beta"))
        });
        for _ in query.chars() {
            native.text("\x7f");
        }
        native.text("Archive");
    }
    for _ in "Archive".chars() {
        native.text("\x7f");
    }
    native.text("Beta Archive");
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("Archive") && row.contains("Beta"))
            && !rows
                .iter()
                .any(|row| row.contains("Archive") && row.contains("Alpha"))
    });
    native.text("\r");
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("Commands") && row.contains("Beta") && row.contains("Archive"))
    });
    native.text("\r");
    let deadline = Instant::now() + Duration::from_secs(5);
    let receipts = loop {
        let text = std::fs::read_to_string(owner.join("executed.jsonl")).unwrap_or_default();
        if text.ends_with('\n') {
            break text
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>();
        }
        assert!(
            Instant::now() < deadline,
            "selected named target did not execute; logs={}; screen={:?}",
            core.request("plugin.log.list", json!({"limit":20})),
            native.rows()
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0]["workspace_id"],
        beta["workspace"]["workspace_id"]
    );
    assert_ne!(
        receipts[0]["workspace_id"],
        alpha["workspace"]["workspace_id"]
    );
}

#[test]
fn palette_observer_and_catalog_follow_the_api_socket_override() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use trailboss_runtime::protocol::widgets::{Declaration, Node};
    let core = support::Core::new();
    let inherited = support::Core::new();
    for (session, label, plugin) in [
        (&core, "OverrideTarget", "fixture.selected-session"),
        (&inherited, "InheritedTarget", "fixture.inherited-session"),
    ] {
        session.request(
            "workspace.create",
            json!({"label":label,"cwd":session.home,"focus":true}),
        );
        let owner = session.home.join("argument-owner");
        std::fs::create_dir_all(&owner).unwrap();
        std::fs::write(owner.join("trailboss-plugin.toml"), format!(
            "id = {plugin:?}\nname = \"Session argument owner\"\nversion = \"1.0.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"raw\"\ntitle = {label:?}\nargument_input = {{ kind = \"raw_tail\" }}\nargument_hint = \"[session name]\"\ncommand = [\"true\"]\n"
        )).unwrap();
        session.request("plugin.link", json!({"path":owner,"enabled":true}));
    }
    let api = core
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
    let selected_environment = |command: &mut Command| {
        for key in support::HOMES {
            command.env(key, &inherited.home);
        }
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            let key = key.to_string_lossy();
            key.starts_with("HERDR_") || key.starts_with("TRAILBOSS_")
        }) {
            command.env_remove(key);
        }
        command
            .env("TRAILBOSS_SESSION", &inherited.name)
            .env(
                "HERDR_CLIENT_SOCKET_PATH",
                inherited.home.join(format!(
                    "trailboss/sessions/{}/trailboss-client.sock",
                    inherited.name
                )),
            )
            .env("HERDR_SOCKET_PATH", &api);
    };
    // Use the independent consumer's real JSON API decoder with the resolved
    // environment, not a direct request to an implicitly selected test Core.
    let mut probe = Command::new("python3");
    selected_environment(&mut probe);
    let output = probe
        .args(["-B", "-c", "import json, os, sys; sys.path.insert(0, sys.argv[1]); from command_consumer import catalog; print(json.dumps(catalog(os.environ['HERDR_SOCKET_PATH'])))"])
        .arg(format!("{}/tests/support", env!("CARGO_MANIFEST_DIR")))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let catalog: Value = serde_json::from_slice(&output.stdout).unwrap();
    let inherited_catalog = inherited.request("ui.command.list", json!({}));
    assert_ne!(catalog["boot_id"], inherited_catalog["boot_id"]);
    assert!(catalog["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("plugin_action_arguments_v1")));
    let selected_command = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["key"]["plugin_id"] == "fixture.selected-session")
        .expect("selected socket must discover its own owner");
    assert!(!catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["key"]["plugin_id"] == "fixture.inherited-session"));
    assert!(inherited_catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["key"]["plugin_id"] == "fixture.inherited-session"));
    assert!(!inherited_catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["key"]["plugin_id"] == "fixture.selected-session"));
    assert_eq!(
        selected_command["argument_input"],
        json!({"kind":"raw_tail"})
    );
    assert_eq!(selected_command["argument_hint"], "[session name]");
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.command_ui.as_ref().is_none_or(|ui| {
        !ui.ui.iter().any(|item| {
            item.plugin_id == "fixture.selected-session"
                && item
                    .command_inputs
                    .iter()
                    .any(|input| input.fragment_name == selected_command["fragment_name"])
        })
    }) {
        assert!(
            Instant::now() < deadline,
            "selected command input projection missing"
        );
        client.receive().unwrap();
    }
    let ui = client.command_ui.as_ref().unwrap();
    let snapshot = client.snapshot.as_ref().unwrap();
    assert_eq!(ui.boot_id, catalog["boot_id"]);
    assert_eq!(ui.boot_id, snapshot.boot_id);
    assert_eq!(ui.revision, snapshot.revision);
    assert!(!ui
        .ui
        .iter()
        .any(|item| item.plugin_id == "fixture.inherited-session"));
    let input = ui
        .ui
        .iter()
        .filter(|item| item.plugin_id == "fixture.selected-session")
        .flat_map(|item| &item.command_inputs)
        .find(|input| input.fragment_name == selected_command["fragment_name"])
        .unwrap();
    assert_eq!(input.title, "OverrideTarget");
    assert_eq!(input.argument_hint.as_deref(), Some("[session name]"));
    assert_eq!(
        serde_json::to_value(&input.argument_input).unwrap(),
        json!({"kind":"raw_tail"})
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_trailboss-command-palette"));
    selected_environment(&mut command);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line);
        let _ = send.send((result, line));
    });
    let publication = receive.recv_timeout(Duration::from_secs(5));
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    reader.join().unwrap();
    let (read, line) = publication
        .unwrap_or_else(|error| panic!("override palette did not publish: {error}; {output:?}"));
    read.unwrap();
    let declaration: Declaration = serde_json::from_str(&line)
        .unwrap_or_else(|error| panic!("override palette publication: {error}; {output:?}"));
    let mut nodes: Vec<_> = declaration.fragments.iter().map(|f| &f.tree).collect();
    let mut target_visible = false;
    let mut selected_use = false;
    while let Some(node) = nodes.pop() {
        match node {
            Node::Label { text, .. } => {
                target_visible |= text == "OverrideTarget";
                assert_ne!(text, "InheritedTarget");
            }
            Node::Use { name } => {
                selected_use |=
                    name.as_str() == selected_command["fragment_name"].as_str().unwrap();
            }
            Node::Column { children }
            | Node::Row { children, .. }
            | Node::List { entries: children } => nodes.extend(children),
            _ => {}
        }
        if let Node::Row {
            menu: Some(menu), ..
        } = node
        {
            nodes.push(menu);
        }
    }
    assert!(
        target_visible,
        "palette observer must use the API-selected Core"
    );
    assert!(
        selected_use,
        "palette must use the selected catalog's canonical command"
    );
}

#[test]
fn raw_arguments_create_owned_tabs_without_losing_empty_quotes_spaces_or_unicode() {
    use std::os::unix::fs::PermissionsExt;
    for stream in [false, true] {
        // Seed only the child Core, never the parallel test runner's environment.
        let name = format!(
            "arg-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let home = std::env::temp_dir().join(&name);
        std::fs::create_dir_all(&home).unwrap();
        let launcher = home.join("core-with-inherited-arguments");
        std::fs::write(&launcher, format!(
            "#!/bin/sh\nexport HERDR_PLUGIN_ARGUMENTS_JSON='\"inherited poison\"'\nexec '{}' \"$@\"\n",
            support::executable().display()
        )).unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
        let core = support::Core::launch_with_executable(home, name, launcher);
        let target = core.request(
            "workspace.create",
            json!({"label":"Argument target","cwd":core.home,"focus":true}),
        );
        let other = core.request(
            "workspace.create",
            json!({"label":"Unrelated focus","cwd":core.home,"focus":true}),
        );
        let owner = core.home.join("argument-owner");
        std::fs::create_dir_all(&owner).unwrap();
        let mut manifest = "id = \"fixture.arguments\"\nname = \"Argument owner\"\nversion = \"1.0.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"create\"\ntitle = \"Create from tail\"\nargument_input = { kind = \"raw_tail\" }\ncommand = [\"python3\", \"owner.py\", \"spawn\"]\n".to_owned();
        if stream {
            manifest.push_str("[[ui]]\nid = \"owner\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"owner.py\", \"stream\"]\nstream = true\nstream_actions = true\n");
        }
        std::fs::write(owner.join("trailboss-plugin.toml"), manifest).unwrap();
        std::fs::write(owner.join("owner.py"), r#"import json,os,socket,sys
def create(context, present, tail):
    label = 'Present[' + tail + ']' if present else 'Absent'
    request = {'id':'owned-tail','method':'tab.create','params':{
        'workspace_id':context['workspace_id'],'label':label,'focus':False}}
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect(os.environ['HERDR_SOCKET_PATH'])
        connection.sendall((json.dumps(request)+'\n').encode())
        reply = json.loads(connection.makefile('rb').readline())
        assert 'error' not in reply, reply
if sys.argv[1] == 'stream':
    print(json.dumps({'herdr':'widget_tree','version':1,'zones':[],'fragments':[
        {'name':'fixture.arguments.ready','priority':0,'tree':{'type':'label','role':'label','text':'Ready'}}]}),flush=True)
    for line in sys.stdin:
        envelope=json.loads(line)
        create(envelope['context'], 'arguments' in envelope, envelope.get('arguments'))
        print(json.dumps({'herdr':'action_result','id':envelope['id'],'ok':True}),flush=True)
else:
    present = 'HERDR_PLUGIN_ARGUMENTS_JSON' in os.environ
    create(json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON']), present,
           json.loads(os.environ['HERDR_PLUGIN_ARGUMENTS_JSON']) if present else None)
"#).unwrap();
        core.request("plugin.link", json!({"path":owner,"enabled":true}));
        let mut client = core.attach();
        if stream {
            let deadline = Instant::now() + Duration::from_secs(5);
            while client.declarations.as_ref().is_none_or(|declarations| {
                NativeCore::default()
                    .fragment(declarations, "fixture.arguments.ready")
                    .is_none()
            }) {
                assert!(
                    Instant::now() < deadline,
                    "owner stream did not become ready"
                );
                client.receive().unwrap();
            }
        }
        let tails = [
            None,
            Some(""),
            Some("  \"quoted value\"  C:\\work\\note café 界 🦀  "),
        ];
        for tail in tails {
            let mut params = json!({"plugin_id":"fixture.arguments","action_id":"create",
                "context":{"workspace_id":target["workspace"]["workspace_id"],
                    "tab_id":target["tab"]["tab_id"],"pane_id":target["root_pane"]["pane_id"]}});
            if let Some(tail) = tail {
                params["arguments"] = json!(tail);
            }
            core.request("plugin.action.invoke", params);
            let expected = tail
                .map(|tail| format!("Present[{tail}]"))
                .unwrap_or_else(|| "Absent".into());
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let snapshot = core.request("session.snapshot", json!({}));
                let tabs = snapshot["snapshot"]["tabs"].as_array().unwrap();
                if tabs.iter().any(|tab| {
                    tab["label"] == expected
                        && tab["workspace_id"] == target["workspace"]["workspace_id"]
                }) {
                    assert!(!tabs.iter().any(|tab| tab["label"] == expected
                        && tab["workspace_id"] == other["workspace"]["workspace_id"]));
                    break;
                }
                assert!(Instant::now() < deadline, "owner failed to create {expected:?}; stream={stream}; snapshot={snapshot}; logs={}", core.request("plugin.log.list", json!({"limit":20})));
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let snapshot = core.request("session.snapshot", json!({}));
        assert_eq!(
            snapshot["snapshot"]["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|tab| tab["workspace_id"] == target["workspace"]["workspace_id"])
                .count(),
            4,
            "each invocation must create exactly one tab, with no fallback or duplicate dispatch"
        );
    }
}

#[test]
fn unsupported_argument_descriptors_reject_before_owner_resource_operations() {
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    let core = support::Core::new();
    let owner = core.home.join("unsupported-argument-owner");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::write(owner.join("trailboss-plugin.toml"),
        "id = \"fixture.unsupported-arguments\"\nname = \"Unsupported arguments\"\nversion = \"1.0.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"legacy\"\ntitle = \"Legacy\"\ncommand = [\"python3\", \"owner.py\"]\n[[actions]]\nid = \"future\"\ntitle = \"Future descriptor\"\nargument_input = { kind = \"future_input\" }\ncommand = [\"python3\", \"owner.py\"]\n").unwrap();
    std::fs::write(
        owner.join("owner.py"),
        r#"import json,os,socket
with socket.socket(socket.AF_UNIX) as connection:
    connection.settimeout(5)
    connection.connect(os.environ['HERDR_SOCKET_PATH'])
    connection.sendall((json.dumps({'id':'forbidden-effect','method':'workspace.create',
        'params':{'label':'Owner operation happened','focus':False}})+'\n').encode())
    assert 'error' not in json.loads(connection.makefile('rb').readline())
"#,
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let before = core.request("session.snapshot", json!({}));
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    ));
    for action in ["legacy", "future"] {
        for tail in ["", "not accepted"] {
            let request: Request = serde_json::from_value(json!({"id":"reject-arguments","method":"plugin.action.invoke",
                "params":{"plugin_id":"fixture.unsupported-arguments","action_id":action,"arguments":tail}})).unwrap();
            let reply = api
                .request_value_with_timeout(&request, Duration::from_secs(5))
                .unwrap();
            assert_eq!(
                reply["error"]["code"], "unsupported_plugin_arguments",
                "{reply}"
            );
        }
    }
    assert_eq!(
        core.request("session.snapshot", json!({}))["snapshot"]["workspaces"],
        before["snapshot"]["workspaces"]
    );
    assert!(
        core.request("plugin.log.list", json!({"limit":20}))["logs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|log| log["action_id"].is_null()),
        "rejection must precede even scheduling an owner action"
    );
    // A successful legacy call is an ordering barrier and proves the fixture
    // really can perform the operation the rejected requests must not perform.
    core.request(
        "plugin.action.invoke",
        json!({"plugin_id":"fixture.unsupported-arguments","action_id":"legacy"}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = core.request("session.snapshot", json!({}));
        let count = snapshot["snapshot"]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|workspace| workspace["label"] == "Owner operation happened")
            .count();
        if count > 0 {
            assert_eq!(count, 1);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "legacy fixture could not create its resource"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn argument_catalog_preserves_owner_text_but_projects_safe_optional_input_metadata() {
    use trailboss_runtime::{api_schema::UiCommandInfo, protocol::endpoint::EndpointPluginUi};
    let core = support::Core::new();
    core.request(
        "workspace.create",
        json!({"label":"Metadata target","cwd":core.home,"focus":true}),
    );
    let owner = core.home.join("argument-metadata-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let title = "Create\n界";
    let hint = "\"name\"  [café]\tmore";
    std::fs::write(owner.join("trailboss-plugin.toml"), format!(
        "id = \"fixture.argument-metadata\"\nname = \"Argument metadata\"\nversion = \"1.0.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"raw\"\ntitle = {title:?}\nargument_input = {{ kind = \"raw_tail\" }}\nargument_hint = {hint:?}\ncommand = [\"true\"]\n[[actions]]\nid = \"no-hint\"\ntitle = \"No hint\"\nargument_input = {{ kind = \"raw_tail\" }}\ncommand = [\"true\"]\n[[actions]]\nid = \"legacy\"\ntitle = \"Legacy\"\ncommand = [\"true\"]\n"
    )).unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let catalog = core.request("ui.command.list", json!({}));
    assert!(catalog["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("plugin_action_arguments_v1")));
    let command = |action: &str| {
        catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| {
                command["key"]["plugin_id"] == "fixture.argument-metadata"
                    && command["key"]["action_id"] == action
            })
            .unwrap()
    };
    assert_eq!(command("raw")["title"], title);
    assert_eq!(command("raw")["argument_hint"], hint);
    assert_eq!(command("raw")["argument_input"], json!({"kind":"raw_tail"}));
    assert!(command("no-hint").get("argument_hint").is_none());
    assert!(command("legacy").get("argument_input").is_none());
    let mut legacy_command = command("raw").clone();
    legacy_command
        .as_object_mut()
        .unwrap()
        .remove("argument_input");
    legacy_command
        .as_object_mut()
        .unwrap()
        .remove("argument_hint");
    let legacy_command: UiCommandInfo = serde_json::from_value(legacy_command).unwrap();
    assert!(legacy_command.argument_input.is_none() && legacy_command.argument_hint.is_none());

    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.command_ui.as_ref().is_none_or(|ui| {
        !ui.ui.iter().any(|item| {
            item.plugin_id == "fixture.argument-metadata"
                && item
                    .command_inputs
                    .iter()
                    .any(|input| input.fragment_name == command("raw")["fragment_name"])
        })
    }) {
        assert!(
            Instant::now() < deadline,
            "capable endpoint did not receive command input metadata"
        );
        client.receive().unwrap();
    }
    let ui = client.command_ui.as_ref().unwrap();
    let snapshot = client.snapshot.as_ref().unwrap();
    assert_eq!(ui.boot_id, catalog["boot_id"]);
    assert_eq!(ui.boot_id, snapshot.boot_id);
    assert_eq!(ui.revision, snapshot.revision);
    let inputs: Vec<_> = ui
        .ui
        .iter()
        .filter(|item| item.plugin_id == "fixture.argument-metadata")
        .flat_map(|item| &item.command_inputs)
        .collect();
    let raw = inputs
        .iter()
        .find(|input| input.fragment_name == command("raw")["fragment_name"])
        .unwrap();
    assert_eq!(raw.title, "Create\\n界");
    assert_eq!(
        raw.argument_hint.as_deref(),
        Some("\"name\"  [café]\\tmore")
    );
    assert_eq!(
        serde_json::to_value(&raw.argument_input).unwrap(),
        json!({"kind":"raw_tail"})
    );
    let no_hint = inputs
        .iter()
        .find(|input| input.fragment_name == command("no-hint")["fragment_name"])
        .unwrap();
    assert!(
        no_hint.argument_hint.is_none(),
        "missing hints must not become inferred syntax"
    );
    assert!(!inputs
        .iter()
        .any(|input| input.fragment_name == command("legacy")["fragment_name"]));
    // Old endpoint payloads omit the entire extension. They must still decode,
    // retaining their existing declaration and revision/boot identity.
    let mut old_payload = serde_json::to_value(ui).unwrap();
    for item in old_payload["ui"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("command_inputs");
    }
    let old_ui: EndpointPluginUi = serde_json::from_value(old_payload).unwrap();
    assert_eq!(old_ui.boot_id, ui.boot_id);
    assert_eq!(old_ui.revision, ui.revision);
    for (old, current) in old_ui.ui.iter().zip(&ui.ui) {
        assert!(old.command_inputs.is_empty());
        assert_eq!(old.declaration, current.declaration);
    }
    let mut old_catalog = catalog.clone();
    old_catalog.as_object_mut().unwrap().remove("capabilities");
    for command in old_catalog["commands"].as_array_mut().unwrap() {
        command.as_object_mut().unwrap().remove("argument_input");
        command.as_object_mut().unwrap().remove("argument_hint");
    }
    let old_catalog: trailboss_runtime::api_schema::ResponseResult =
        serde_json::from_value(old_catalog).unwrap();
    let trailboss_runtime::api_schema::ResponseResult::UiCommandList {
        boot_id,
        capabilities,
        commands,
    } = old_catalog
    else {
        panic!("older catalog lost its response identity");
    };
    assert_eq!(boot_id, ui.boot_id);
    assert!(
        capabilities.is_empty(),
        "absence must not imply raw argument capability"
    );
    assert!(commands
        .iter()
        .all(|command| command.argument_input.is_none() && command.argument_hint.is_none()));

    // Downgrade only optional extension fields on actual Core frames. Both
    // cases traverse the native client's real generation-1 socket decoder:
    // metadata without negotiation, and negotiation without metadata.
    for omit_capability in [true, false] {
        use std::os::unix::net::{UnixListener, UnixStream};
        use trailboss_runtime::protocol::{self, endpoint::*, ClientMessage, ServerMessage};
        let socket = core
            .home
            .join(format!("older-endpoint-{omit_capability}.sock"));
        let listener = UnixListener::bind(&socket).unwrap();
        let upstream = core.home.join(format!(
            "trailboss/sessions/{}/trailboss-client.sock",
            core.name
        ));
        let raw_locator = command("raw")["fragment_name"].as_str().unwrap().to_owned();
        let relay = std::thread::spawn(move || {
            let (mut downstream, _) = listener.accept().unwrap();
            downstream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut upstream = UnixStream::connect(upstream).unwrap();
            upstream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let hello: ClientMessage =
                protocol::read_message(&mut downstream, protocol::MAX_GRAPHICS_FRAME_SIZE).unwrap();
            protocol::write_message(&mut upstream, &hello).unwrap();
            let mut snapshot_pair = None;
            let mut ui_pair = None;
            let mut welcome_seen = false;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(
                    Instant::now() < deadline,
                    "older endpoint fixture never paired"
                );
                let mut message: ServerMessage =
                    protocol::read_message(&mut upstream, protocol::MAX_GRAPHICS_FRAME_SIZE)
                        .unwrap();
                if let ServerMessage::EndpointControl { kind, data } = &mut message {
                    let mut payload: Value = serde_json::from_str(data).unwrap();
                    if kind == ENDPOINT_WELCOME_KIND {
                        assert!(payload["capabilities"]
                            .as_array()
                            .unwrap()
                            .contains(&json!("plugin_action_arguments_v1")));
                        welcome_seen = true;
                        if omit_capability {
                            payload.as_object_mut().unwrap().remove("capabilities");
                        }
                    } else if kind == ENDPOINT_SNAPSHOT_KIND {
                        snapshot_pair =
                            Some((payload["boot_id"].clone(), payload["revision"].clone()));
                    } else if kind == PLUGIN_UI_KIND {
                        // Require real canonical input metadata before removing
                        // anything; an empty upstream cannot satisfy this case.
                        let has_raw = payload["ui"].as_array().unwrap().iter().any(|item| {
                            item["plugin_id"] == "fixture.argument-metadata"
                                && item["command_inputs"].as_array().is_some_and(|inputs| {
                                    inputs
                                        .iter()
                                        .any(|input| input["fragment_name"] == raw_locator)
                                })
                        });
                        if has_raw {
                            ui_pair =
                                Some((payload["boot_id"].clone(), payload["revision"].clone()));
                        }
                        if !omit_capability {
                            for item in payload["ui"].as_array_mut().unwrap() {
                                item.as_object_mut().unwrap().remove("command_inputs");
                            }
                        }
                    }
                    *data = payload.to_string();
                }
                protocol::write_message(&mut downstream, &message).unwrap();
                if welcome_seen && ui_pair == snapshot_pair {
                    if let Some(pair) = ui_pair.take() {
                        return pair;
                    }
                }
            }
        });
        let mut older = trailboss::Client::attach(&socket, 80, 24).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while older.command_ui.is_none() {
            assert!(
                Instant::now() < deadline,
                "older native client did not pair projection"
            );
            older.receive().unwrap();
        }
        let (boot, revision) = relay.join().unwrap();
        let old_ui = older.command_ui.as_ref().unwrap();
        assert_eq!(old_ui.boot_id, boot);
        assert_eq!(old_ui.boot_id, catalog["boot_id"]);
        assert_eq!(json!(old_ui.revision), revision);
        assert_eq!(old_ui.revision, older.snapshot.as_ref().unwrap().revision);
        assert!(
            old_ui.ui.iter().all(|item| item.command_inputs.is_empty()),
            "optional fields alone must not infer raw argument entry"
        );
        let declarations = older.declarations.as_ref().unwrap();
        assert!(
            NativeCore::default()
                .fragment(
                    declarations,
                    command("raw")["fragment_name"].as_str().unwrap()
                )
                .is_some(),
            "legacy command remains usable without inferred argument metadata"
        );
    }
}
