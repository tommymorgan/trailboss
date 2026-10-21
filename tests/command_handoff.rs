#[path = "support/console.rs"]
mod console;
mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use trailboss::{
    core::{Core as NativeCore, Effect},
    sidebar::Declarations,
    terminal::{menus::Response, FrameRenderer, Rect},
};
use trailboss_runtime::protocol::widgets::{Context, Node};

type Route = Vec<(String, usize)>;
type PublishedAction = (String, String, Context, Route);

fn key(
    renderer: &mut FrameRenderer,
    native: &NativeCore,
    declarations: &Declarations,
    code: KeyCode,
) -> Response {
    renderer.controls.key(
        native,
        declarations,
        Rect::viewport(100, 30),
        KeyEvent::new(code, KeyModifiers::NONE),
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
                .filter_map(|cp| char::from_u32(*cp))
                .collect()
        })
        .collect()
}

// Navigate the real producer's public tree, without deriving expected owners,
// operations or targets from discovery. The action menu is not a new test form.
fn paths(
    native: &NativeCore,
    declarations: &Declarations,
    node: &Node,
    owner: &str,
    inherited: &Context,
    route: &Route,
    depth: usize,
    ordinals: &mut HashMap<(Route, String), usize>,
    found: &mut Vec<PublishedAction>,
) {
    assert!(depth < 32, "cyclic public declaration");
    match node {
        Node::Use { name } => {
            if let Some((provider, tree)) = native.fragment(declarations, name) {
                paths(
                    native,
                    declarations,
                    tree,
                    provider,
                    inherited,
                    route,
                    depth + 1,
                    ordinals,
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
                    owner,
                    inherited,
                    route,
                    depth + 1,
                    ordinals,
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
            let context = if *context == Context::default() {
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
            // Equal command titles can bind different owner contexts. Preserve
            // the occurrence within this exact parent route, not just its text.
            let ordinal = ordinals.entry((route.clone(), title.clone())).or_default();
            let step = (title, *ordinal);
            *ordinal += 1;
            let mut route = route.clone();
            route.push(step);
            if let Some(action) = action {
                found.push((
                    owner.to_owned(),
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
                    owner,
                    context,
                    &route,
                    depth + 1,
                    ordinals,
                    found,
                );
            }
        }
        _ => {}
    }
}
fn available(native: &NativeCore, declarations: &Declarations) -> Vec<PublishedAction> {
    let mut result = Vec::new();
    if let Some((owner, root)) = native.fragment(declarations, "command-palette") {
        paths(
            native,
            declarations,
            root,
            owner,
            &Context::default(),
            &Vec::new(),
            0,
            &mut HashMap::new(),
            &mut result,
        );
    }
    result
}
fn select(
    renderer: &mut FrameRenderer,
    native: &NativeCore,
    declarations: &Declarations,
    route: &[(String, usize)],
) -> Response {
    let mut response = Response::Pass;
    for (label, ordinal) in route {
        for character in label.chars() {
            key(renderer, native, declarations, KeyCode::Char(character));
        }
        for _ in 0..*ordinal {
            key(renderer, native, declarations, KeyCode::Down);
        }
        response = key(renderer, native, declarations, KeyCode::Enter);
    }
    response
}
fn link(core: &support::Core, plugin: &str) {
    core.request(
        "plugin.link",
        json!({"path":format!("{}/plugins/{plugin}", env!("CARGO_MANIFEST_DIR")),"enabled":true}),
    );
}
fn context(resource: &Value) -> Context {
    serde_json::from_value(json!({"workspace_id":resource["workspace"]["workspace_id"],
        "tab_id":resource["tab"]["tab_id"],"pane_id":resource["root_pane"]["pane_id"]}))
    .unwrap()
}
fn matching<'a>(
    routes: &'a [PublishedAction],
    owner: &str,
    action: &str,
    target: &Context,
) -> Option<&'a PublishedAction> {
    routes.iter().find(|(provider, operation, captured, _)| {
        provider == owner
            && operation == action
            && captured
                .workspace_id
                .as_ref()
                .is_none_or(|id| Some(id) == target.workspace_id.as_ref())
            && captured
                .tab_id
                .as_ref()
                .is_none_or(|id| Some(id) == target.tab_id.as_ref())
    })
}

#[test]
fn existing_input_handoffs_core_names_and_confirmations_keep_captured_targets() {
    for (owner, action, method, confirmation) in [
        (
            "trailboss.core",
            "host:new-space",
            "workspace.create",
            false,
        ),
        ("trailboss.core", "host:new-tab", "tab.create", false),
        (
            "trailboss.core",
            "host:rename-space",
            "workspace.rename",
            false,
        ),
        ("trailboss.core", "host:rename-tab", "tab.rename", false),
        (
            "tommymorgan.sidebar-menus",
            "close-space",
            "workspace.close",
            true,
        ),
        ("trailboss.core", "host:close-tab", "tab.close", true),
    ] {
        let core = support::Core::new();
        let captured = core.request(
            "workspace.create",
            json!({"label":"Captured space","cwd":core.home,"focus":true}),
        );
        core.request(
            "tab.rename",
            json!({"tab_id":captured["tab"]["tab_id"],"label":"Captured tab"}),
        );
        if method == "tab.close" {
            core.request(
                "tab.create",
                json!({
                    "workspace_id":captured["workspace"]["workspace_id"],
                    "label":"Surviving sibling","cwd":core.home,"focus":false
                }),
            );
        }
        let unrelated = core.request(
            "workspace.create",
            json!({"label":"Unrelated space","cwd":core.home,"focus":false}),
        );
        core.request(
            "tab.rename",
            json!({"tab_id":unrelated["tab"]["tab_id"],"label":"Unrelated tab"}),
        );
        let target = context(&captured);
        for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
            link(&core, plugin);
        }
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
        let deadline = Instant::now() + Duration::from_secs(8);
        let route = loop {
            if let Some(declarations) = &client.declarations {
                if let Some(selection) =
                    matching(&available(&native, declarations), owner, action, &target)
                {
                    break selection.3.clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "real consumer did not publish {owner}/{action}"
            );
            client.receive().unwrap();
        };
        let declarations = client.declarations.as_ref().unwrap();
        let mut renderer = FrameRenderer::default();
        assert!(renderer
            .controls
            .open_palette(&native, declarations, Some(&target)));
        let route = if confirmation {
            &route[..route.len() - 1]
        } else {
            &route[..]
        };
        let selected = select(&mut renderer, &native, declarations, route);
        if confirmation {
            assert!(matches!(selected, Response::Consumed));
        } else {
            let Response::Prepare(prepared) = selected else {
                panic!("{action} skipped its existing native name flow")
            };
            assert_eq!(prepared.action, action);
            assert!(matches!(
                native.prepare_palette(&prepared.action, &prepared.context),
                Effect::Consumed
            ));
            assert!(native.editing());
            assert!(
                native.input_text.is_empty(),
                "search text must not become the name draft"
            );
        }
        // Search must already be gone while the original interaction is open.
        // A palette-specific confirmation/search form still accepts this paste.
        assert!(
            !renderer.controls.paste("must-not-enter-search"),
            "{action} retained command search instead of handing off"
        );
        let dialog = rows(&mut renderer, &native, declarations);
        assert!(
            !dialog.iter().any(|row| row.contains("Commands")),
            "{action}: {dialog:?}"
        );
        assert!(
            dialog.iter().any(|row| row.contains("Cancel")),
            "{action}: {dialog:?}"
        );
        if confirmation {
            // Ordinary popups indicate selection with their existing color,
            // not the name dialog's caret. Prove the default action directly.
            let mut cancel = FrameRenderer::default();
            assert!(cancel
                .controls
                .open_palette(&native, declarations, Some(&target)));
            assert!(matches!(
                select(&mut cancel, &native, declarations, route),
                Response::Consumed
            ));
            let Response::Invoke(default) = key(&mut cancel, &native, declarations, KeyCode::Enter)
            else {
                panic!("existing confirmation did not expose its default Cancel action")
            };
            assert_eq!(
                default.action, "host:cancel",
                "{action} confirmation must default to Cancel"
            );
        } else {
            assert!(
                dialog.iter().any(|row| row.contains('▏')),
                "existing native input caret missing: {dialog:?}"
            );
        }
        let before = core.request("session.snapshot", json!({}));
        core.request("tab.focus", json!({"tab_id":unrelated["tab"]["tab_id"]}));
        native.update_session(
            unrelated["workspace"]["workspace_id"]
                .as_str()
                .map(str::to_owned),
            unrelated["tab"]["tab_id"].as_str().map(str::to_owned),
            true,
        );
        let effect = if confirmation {
            key(&mut renderer, &native, declarations, KeyCode::Down);
            let Response::Invoke(submit) =
                key(&mut renderer, &native, declarations, KeyCode::Enter)
            else {
                panic!("existing {action} confirmation did not submit")
            };
            assert_eq!(submit.plugin, owner);
            assert_eq!(submit.action, action);
            assert_eq!(submit.context.workspace_id, target.workspace_id);
            if action == "host:close-tab" {
                assert_eq!(submit.context.tab_id, target.tab_id);
            }
            let effect = native.invoke(&submit.action, &submit.context, None);
            if matches!(effect, Effect::Plugin) {
                client
                    .input()
                    .unwrap()
                    .invoke(
                        &client.boot_id,
                        &submit.plugin,
                        &submit.action,
                        &submit.context,
                    )
                    .unwrap();
            }
            effect
        } else {
            assert!(matches!(
                native.text("Accepted captured name"),
                Some(Effect::Consumed)
            ));
            let typed = rows(&mut renderer, &native, declarations);
            assert!(
                typed
                    .iter()
                    .any(|row| row.contains("Accepted captured name▏")),
                "shared name field lost input: {typed:?}"
            );
            match key(&mut renderer, &native, declarations, KeyCode::Enter) {
                Response::Pass => native.text("\r").expect("native input did not submit"),
                Response::Invoke(submit) => native.invoke(&submit.action, &submit.context, None),
                _ => panic!("native name submission retained a palette handler"),
            }
        };
        if let Effect::Request {
            method: actual,
            params,
        } = effect
        {
            assert_eq!(actual, method);
            if matches!(
                method,
                "tab.create" | "workspace.rename" | "workspace.close"
            ) {
                assert_eq!(
                    params["workspace_id"],
                    captured["workspace"]["workspace_id"]
                );
            }
            if matches!(method, "tab.rename" | "tab.close") {
                assert_eq!(params["tab_id"], captured["tab"]["tab_id"]);
            }
            if !confirmation {
                assert_eq!(params["label"], "Accepted captured name");
            }
            core.request(actual, params);
        } else {
            assert!(confirmation && owner == "tommymorgan.sidebar-menus");
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        let after = loop {
            let snapshot = core.request("session.snapshot", json!({}));
            if method != "workspace.close"
                || !snapshot["snapshot"]["workspaces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|workspace| {
                        workspace["workspace_id"] == captured["workspace"]["workspace_id"]
                    })
            {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "original close-space owner did not close captured space"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let spaces_before = before["snapshot"]["workspaces"].as_array().unwrap();
        let tabs_before = before["snapshot"]["tabs"].as_array().unwrap();
        let spaces_after = after["snapshot"]["workspaces"].as_array().unwrap();
        let tabs_after = after["snapshot"]["tabs"].as_array().unwrap();
        assert_eq!(
            spaces_after.len() as isize - spaces_before.len() as isize,
            if method == "workspace.create" {
                1
            } else if method == "workspace.close" {
                -1
            } else {
                0
            },
            "{action}"
        );
        assert_eq!(
            tabs_after.len() as isize - tabs_before.len() as isize,
            if matches!(method, "workspace.create" | "tab.create") {
                1
            } else if confirmation {
                -1
            } else {
                0
            },
            "{action}"
        );
        if method == "workspace.create" {
            let created = spaces_after
                .iter()
                .find(|new| {
                    !spaces_before
                        .iter()
                        .any(|old| old["workspace_id"] == new["workspace_id"])
                })
                .unwrap();
            assert_eq!(created["label"], "Accepted captured name");
        }
        if method == "tab.create" {
            let created = tabs_after
                .iter()
                .find(|new| !tabs_before.iter().any(|old| old["tab_id"] == new["tab_id"]))
                .unwrap();
            assert_eq!(created["label"], "Accepted captured name");
            assert_eq!(
                created["workspace_id"],
                captured["workspace"]["workspace_id"]
            );
        }
        for old in spaces_before {
            if old["workspace_id"] == captured["workspace"]["workspace_id"]
                && method == "workspace.close"
            {
                continue;
            }
            let new = spaces_after
                .iter()
                .find(|new| new["workspace_id"] == old["workspace_id"])
                .unwrap();
            let expected = if method == "workspace.rename"
                && old["workspace_id"] == captured["workspace"]["workspace_id"]
            {
                json!("Accepted captured name")
            } else {
                old["label"].clone()
            };
            assert_eq!(
                new["label"], expected,
                "{action} changed an unrelated workspace"
            );
        }
        for old in tabs_before {
            if old["tab_id"] == captured["tab"]["tab_id"] && confirmation {
                continue;
            }
            let new = tabs_after
                .iter()
                .find(|new| new["tab_id"] == old["tab_id"])
                .unwrap();
            let expected = if method == "tab.rename" && old["tab_id"] == captured["tab"]["tab_id"] {
                json!("Accepted captured name")
            } else {
                old["label"].clone()
            };
            assert_eq!(new["label"], expected, "{action} changed an unrelated tab");
        }
        if !confirmation {
            assert!(!native.editing());
            assert!(
                !matches!(native.text("\r"), Some(Effect::Request { .. })),
                "one name submit must not submit twice"
            );
        }
        assert!(renderer
            .controls
            .open_palette(&native, declarations, Some(&target)));
        let reopened = rows(&mut renderer, &native, declarations);
        assert!(
            !reopened
                .iter()
                .any(|row| row.contains("must-not-enter-search")
                    || row.contains("Accepted captured name")),
            "completed interaction leaked a draft into reopened search: {reopened:?}"
        );
    }
}

fn register_input_owner(core: &support::Core) -> std::path::PathBuf {
    let owner = core.home.join("input-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let trace = owner.join("interaction.jsonl");
    let socket = core
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", core.name));
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/command_input_owner.py"
    );
    std::fs::write(owner.join("trailboss-plugin.toml"), format!(
        "id = \"fixture.input-owner\"\nname = \"Existing input owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n\
         [[actions]]\nid = \"ask-name\"\ntitle = \"Owner asks for a name\"\ncontexts = [\"pane\"]\ncommand = {}\n\
         [[panes]]\nid = \"name\"\ntitle = \"Existing owner name\"\nplacement = \"tab\"\ncommand = {}\n",
        json!(["python3", "-u", script, "open", socket.to_str().unwrap(), trace.to_str().unwrap()]),
        json!(["python3", "-u", script, "pane"]),
    )).unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    trace
}

#[test]
fn existing_input_handoffs_real_plugin_owner_pane_reuses_ordinary_interaction() {
    fn events(path: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    let core = support::Core::new();
    let captured = core.request(
        "workspace.create",
        json!({"label":"Capture","cwd":core.home,"focus":true}),
    );
    let unrelated = core.request(
        "workspace.create",
        json!({"label":"Elsewhere","cwd":core.home,"focus":false}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":captured["tab"]["tab_id"],"label":"Target"}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":unrelated["tab"]["tab_id"],"label":"Other"}),
    );
    let target = context(&captured);
    let trace = register_input_owner(&core);
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        link(&core, plugin);
    }
    let mut client = core.attach();
    let mut native = NativeCore::default();
    native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if client.declarations.as_ref().is_some_and(|declarations| {
            matching(
                &available(&native, declarations),
                "fixture.input-owner",
                "ask-name",
                &target,
            )
            .is_some()
                && declarations.iter().any(|(owner, declaration)| {
                    owner == "tommymorgan.stock-ui" && declaration.composition.is_some()
                })
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "real owner action and stock composition did not publish"
        );
        client.receive().unwrap();
    }
    let mut console = console::Console::new(&core);
    for (round, accepted) in [None, Some("Ordinary"), Some("Palette")]
        .into_iter()
        .enumerate()
    {
        macro_rules! observe {
            ($boundary:expr, $predicate:expr) => {{
                let observation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| console.wait($predicate)));
                assert!(observation.is_ok(),
                    "owner UI boundary={} round={round}; receipts={:?}; owner_logs={}; panes={}; global_snapshot={}; observer_boot={:?}; observer_snapshot={:?}; native_workspace={:?}; native_tab={:?}; screen={:?}",
                    $boundary, events(&trace), core.request("plugin.log.list", json!({"limit":20})),
                    core.request("pane.list", json!({})), core.request("session.snapshot", json!({})),
                    client.boot_id, client.snapshot, native.workspace_id, native.focused_tab_id, console.rows());
            }};
        }
        macro_rules! choose_tab {
            ($label:expr) => {{
                let needle = format!("○ {}", $label);
                observe!("native-sidebar-tab-visible", |rows| rows
                    .iter()
                    .any(|row| row.contains(&needle)));
                let screen = console.rows();
                let y = screen.iter().position(|row| row.contains(&needle)).unwrap();
                let byte = screen[y].find(&needle).unwrap();
                let x = screen[y][..byte].chars().count() + 2;
                console.click(x, y);
            }};
        }
        choose_tab!(if round < 2 { "Target" } else { "Ordinary" });
        core.request("tab.focus", json!({"tab_id":captured["tab"]["tab_id"]}));
        core.request(
            "pane.send_text",
            json!({"pane_id":captured["root_pane"]["pane_id"],
            "text":format!("printf 'OWNER_TARGET_READY_{round}\\n'\n")}),
        );
        observe!("target-shell-marker", |rows| rows.iter().any(|row| row
            .trim_end()
            .ends_with(&format!("OWNER_TARGET_READY_{round}"))));
        let before = core.request("session.snapshot", json!({}));
        if round < 2 {
            // Establish the existing interaction through its ordinary public
            // owner-action endpoint before the palette is allowed to invoke it.
            // Do not bypass the action by opening the fixture pane ourselves.
            client
                .input()
                .unwrap()
                .invoke(&client.boot_id, "fixture.input-owner", "ask-name", &target)
                .unwrap();
        } else {
            let declarations = client.declarations.as_ref().unwrap();
            let route = matching(
                &available(&native, declarations),
                "fixture.input-owner",
                "ask-name",
                &target,
            )
            .unwrap()
            .3
            .clone();
            let mut renderer = FrameRenderer::default();
            assert!(renderer
                .controls
                .open_palette(&native, declarations, Some(&target)));
            let Response::Invoke(selected) = select(&mut renderer, &native, declarations, &route)
            else {
                panic!("discovered plugin action did not transfer to its ordinary owner")
            };
            assert_eq!(selected.plugin, "fixture.input-owner");
            assert_eq!(selected.action, "ask-name");
            assert_eq!(selected.context, target);
            assert!(
                !renderer.controls.modal(),
                "search must close before owner invocation opens its pane"
            );
            assert!(
                !renderer.controls.paste("must-not-enter-search"),
                "search query must be cleared before owner invocation"
            );
            assert!(!rows(&mut renderer, &native, declarations)
                .iter()
                .any(|row| row.contains("Commands")));
            core.request("tab.focus", json!({"tab_id":unrelated["tab"]["tab_id"]}));
            native.update_session(
                unrelated["workspace"]["workspace_id"]
                    .as_str()
                    .map(str::to_owned),
                unrelated["tab"]["tab_id"].as_str().map(str::to_owned),
                true,
            );
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
                )
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let receipts = events(&trace);
            let failed = receipts.iter().any(|event| event["kind"] == "failure");
            let pane_started = receipts
                .iter()
                .filter(|event| event["kind"] == "pane")
                .count()
                == round + 1;
            if pane_started {
                break;
            }
            assert!(!failed && Instant::now() < deadline,
                "ordinary owner setup failed before any palette assertion; round={round}; boot_id={:?}; receipts={receipts:?}; owner_logs={}; panes={}; snapshot={}",
                client.boot_id, core.request("plugin.log.list", json!({"limit":20})),
                core.request("pane.list", json!({})), core.request("session.snapshot", json!({})));
            std::thread::sleep(Duration::from_millis(10));
        }
        let observed = events(&trace);
        assert_eq!(
            observed
                .iter()
                .filter(|event| event["kind"] == "invoke")
                .count(),
            round + 1
        );
        let pane = observed
            .iter()
            .rev()
            .find(|event| event["kind"] == "pane")
            .unwrap();
        assert_eq!(pane["owner"], "fixture.input-owner");
        assert_eq!(pane["entrypoint"], "name");
        assert_eq!(pane["target"], captured["tab"]["tab_id"]);
        let pane_identity = core.request("pane.get", json!({"pane_id":pane["pane"]}));
        let owner_tab = &pane_identity["pane"]["tab_id"];
        assert_ne!(
            owner_tab, &captured["tab"]["tab_id"],
            "fixture must collect input in a real owner-owned pane"
        );
        let owner_label = format!("Input{round}");
        core.request(
            "tab.rename",
            json!({"tab_id":owner_tab,"label":owner_label}),
        );
        // Plugin pane opening changes global focus, not another native
        // connection's view. Enter through that client's ordinary sidebar.
        // The existing per-client contract is locked by
        // client_shell_tab_focus_changes_only_the_source_connection.
        choose_tab!(&owner_label);
        observe!("initial-owner-prompt", |rows| rows
            .iter()
            .any(|row| row.contains("OWNER NAME (type cancel to dismiss):")));
        // Return to the existing owner pane after an unrelated focus change;
        // the pane is not the object that its authoritative submit will rename.
        choose_tab!("Other");
        core.request(
            "pane.send_text",
            json!({"pane_id":unrelated["root_pane"]["pane_id"],
            "text":format!("printf 'OWNER_OTHER_READY_{round}\\n'\n")}),
        );
        observe!("unrelated-native-focus", |rows| rows.iter().any(|row| row
            .trim_end()
            .ends_with(&format!("OWNER_OTHER_READY_{round}"))));
        choose_tab!(&owner_label);
        observe!("refocused-owner-prompt", |rows| rows
            .iter()
            .any(|row| row.contains("OWNER NAME (type cancel to dismiss):")));
        if let Some(name) = accepted {
            console.text("\r");
            observe!("owner-validation-and-reprompt", |rows| rows
                .iter()
                .any(|row| row.contains("OWNER REJECTED EMPTY NAME"))
                && rows
                    .iter()
                    .filter(|row| row.contains("OWNER NAME (type cancel to dismiss):"))
                    .count()
                    >= 2);
            let rejected = core.request("tab.get", json!({"tab_id":captured["tab"]["tab_id"]}));
            assert_eq!(
                rejected["tab"]["label"],
                if round == 1 { "Target" } else { "Ordinary" }
            );
            console.text(&format!("{name}\r"));
            observe!("owner-submit-receipt", |rows| rows
                .iter()
                .any(|row| row.contains(&format!("OWNER SAVED {name}"))));
        } else {
            console.text("cancel\r");
            observe!("owner-cancel-receipt", |rows| rows
                .iter()
                .any(|row| row.contains("OWNER CANCELLED")));
        }
        let after = core.request("session.snapshot", json!({}));
        for old in before["snapshot"]["tabs"].as_array().unwrap() {
            let current = after["snapshot"]["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tab| tab["tab_id"] == old["tab_id"])
                .unwrap();
            let expected = if old["tab_id"] == captured["tab"]["tab_id"] {
                accepted
                    .map(|name| json!(name))
                    .unwrap_or_else(|| old["label"].clone())
            } else {
                old["label"].clone()
            };
            assert_eq!(
                current["label"], expected,
                "owner submit changed another tab"
            );
        }
        let observed = events(&trace);
        assert_eq!(
            observed
                .iter()
                .filter(|event| event["kind"] == "submit")
                .count(),
            round
        );
        assert_eq!(
            observed
                .iter()
                .filter(|event| event["kind"] == "cancel")
                .count(),
            1
        );
        assert_eq!(
            observed
                .iter()
                .filter(|event| event["kind"] == "reject")
                .count(),
            round
        );
    }
    let observed = events(&trace);
    let invokes: Vec<_> = observed
        .iter()
        .filter(|event| event["kind"] == "invoke")
        .collect();
    assert_eq!(
        invokes.len(),
        3,
        "ordinary and palette routes must each invoke once, with no fallback/retry"
    );
    for invoke in invokes {
        assert_eq!(invoke["owner"], "fixture.input-owner");
        assert_eq!(invoke["action"], "ask-name");
        assert_eq!(
            invoke["context"]["workspace_id"],
            captured["workspace"]["workspace_id"]
        );
        assert_eq!(invoke["context"]["tab_id"], captured["tab"]["tab_id"]);
        assert_eq!(
            invoke["context"]["pane_id"],
            captured["root_pane"]["pane_id"]
        );
    }
    let submits: Vec<_> = observed
        .iter()
        .filter(|event| event["kind"] == "submit")
        .collect();
    assert_eq!(
        submits
            .iter()
            .map(|event| event["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Ordinary", "Palette"]
    );
    assert!(submits
        .iter()
        .all(|event| event["target"] == captured["tab"]["tab_id"]));
}

fn cancellation_resources(core: &support::Core) -> Value {
    let snapshot = core.request("session.snapshot", json!({}));
    let workspaces: Vec<_> = snapshot["snapshot"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|workspace| json!([workspace["workspace_id"], workspace["label"]]))
        .collect();
    let tabs: Vec<_> = snapshot["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tab| json!([tab["tab_id"], tab["workspace_id"], tab["label"]]))
        .collect();
    json!({"workspaces":workspaces, "tabs":tabs})
}

#[test]
fn cancelling_existing_core_flows_discards_drafts_without_reopening_search() {
    for (owner, action, confirmation) in [
        ("trailboss.core", "host:new-space", false),
        ("trailboss.core", "host:new-tab", false),
        ("trailboss.core", "host:rename-space", false),
        ("trailboss.core", "host:rename-tab", false),
        ("tommymorgan.sidebar-menus", "close-space", true),
        ("trailboss.core", "host:close-tab", true),
    ] {
        let core = support::Core::new();
        let captured = core.request(
            "workspace.create",
            json!({"label":"Origin","cwd":core.home,"focus":true}),
        );
        let unrelated = core.request(
            "workspace.create",
            json!({"label":"Other","cwd":core.home,"focus":false}),
        );
        let target = context(&captured);
        for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
            link(&core, plugin);
        }
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
        let deadline = Instant::now() + Duration::from_secs(8);
        let route = loop {
            if let Some(declarations) = &client.declarations {
                if let Some(selection) =
                    matching(&available(&native, declarations), owner, action, &target)
                {
                    if declarations.iter().any(|(provider, declaration)| {
                        provider == "tommymorgan.stock-ui" && declaration.composition.is_some()
                    }) && [
                        "commands.name-new",
                        "commands.name-existing",
                        "sidebar.confirm-close-space",
                        "commands.confirm-tab",
                    ]
                    .iter()
                    .all(|name| native.fragment(declarations, name).is_some())
                    {
                        break selection.3.clone();
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "missing existing owner flow {owner}/{action}"
            );
            client.receive().unwrap();
        };
        for discovered in [false, true] {
            for escape in [false, true] {
                let before = cancellation_resources(&core);
                let mut declarations = client.declarations.as_ref().unwrap().clone();
                let mut renderer = FrameRenderer::default();
                let retained_new_space_controller = !discovered && action == "host:new-space";
                if discovered {
                    assert!(renderer
                        .controls
                        .open_palette(&native, &declarations, Some(&target)));
                    let selected_route = if confirmation {
                        &route[..route.len() - 1]
                    } else {
                        &route[..]
                    };
                    let response = select(&mut renderer, &native, &declarations, selected_route);
                    if confirmation {
                        assert!(matches!(response, Response::Consumed));
                    } else {
                        let Response::Prepare(prepared) = response else {
                            panic!("{action} skipped existing input")
                        };
                        assert_eq!(
                            prepared.action, action,
                            "{action}: prepared a different operation"
                        );
                        match action {
                            "host:new-space" => assert_eq!(
                                prepared.context,
                                Context::default(),
                                "{action}: global command gained an unused target"
                            ),
                            "host:new-tab" | "host:rename-space" => assert_eq!(
                                prepared.context.workspace_id, target.workspace_id,
                                "{action}: lost captured workspace"
                            ),
                            "host:rename-tab" => {
                                assert_eq!(
                                    prepared.context.workspace_id, target.workspace_id,
                                    "{action}: lost captured workspace"
                                );
                                assert_eq!(
                                    prepared.context.tab_id, target.tab_id,
                                    "{action}: lost captured tab"
                                );
                            }
                            _ => unreachable!("unexpected named action {action}"),
                        }
                        assert!(matches!(
                            native.prepare_palette(&prepared.action, &prepared.context),
                            Effect::Consumed
                        ));
                    }
                } else if confirmation {
                    // Exercise the retained public confirmation through the ordinary
                    // row-menu controller, not through command search or fake exports.
                    let name = if action == "close-space" {
                        "sidebar.confirm-close-space"
                    } else {
                        "commands.confirm-tab"
                    };
                    assert!(native.fragment(&declarations, name).is_some());
                    declarations.push(("fixture.original-confirmation".into(), serde_json::from_value(json!({
                        "herdr":"widget_tree","version":1,"zones":[],
                        "composition":{"priority":10000,"tree":{"type":"column","children":[
                            {"type":"row","context":target,"children":[{"type":"label","text":"Original confirmation","role":"label"}],
                             "menu":{"type":"use","name":name}},
                            {"type":"flex","weight":1,"child":{"type":"surface"}}
                        ]}}
                    })).unwrap()));
                    assert!(matches!(
                        renderer.controls.click(
                            &native,
                            &declarations,
                            Rect::viewport(100, 30),
                            98,
                            0,
                            false
                        ),
                        Response::Consumed
                    ));
                } else {
                    if retained_new_space_controller {
                        // There was no standalone named NewSpace entrypoint. This
                        // is parity of its retained old-palette controller only.
                        let catalog = core.request("ui.command.list", json!({}));
                        let command = catalog["commands"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|command| {
                                command["key"]["kind"] == "core"
                                    && command["key"]["action_id"] == action
                            })
                            .unwrap();
                        let issued = native
                            .fragment(&declarations, command["fragment_name"].as_str().unwrap())
                            .unwrap();
                        assert_eq!(issued.0, "trailboss.core");
                        assert!(matches!(
                            native.prepare_palette(action, &Context::default()),
                            Effect::Consumed
                        ));
                    } else {
                        let original = if action == "host:new-tab" {
                            "host:new"
                        } else {
                            action
                        };
                        assert!(matches!(
                            native.invoke(original, &target, None),
                            Effect::Consumed
                        ));
                    }
                }
                assert!(!renderer.controls.paste("must-not-reopen-search"));
                if !confirmation {
                    assert!(native.editing());
                    assert!(native.input_text.is_empty());
                    assert!(matches!(
                        native.text("Unsubmitted draft"),
                        Some(Effect::Consumed)
                    ));
                    assert_eq!(native.input_text, "Unsubmitted draft");
                }
                let dialog = rows(&mut renderer, &native, &declarations);
                if !retained_new_space_controller {
                    assert!(
                        !dialog.iter().any(|row| row.contains("Commands")),
                        "{action}: {dialog:?}"
                    );
                    assert!(
                        dialog.iter().any(|row| row.contains("Cancel")),
                        "{action}: {dialog:?}"
                    );
                }
                let response = if retained_new_space_controller {
                    if escape {
                        assert!(matches!(native.text("\u{1b}"), Some(Effect::Consumed)));
                    } else {
                        assert!(matches!(
                            native.invoke("host:cancel", &target, None),
                            Effect::Consumed
                        ));
                    }
                    Response::Consumed
                } else if escape {
                    key(&mut renderer, &native, &declarations, KeyCode::Esc)
                } else if confirmation {
                    key(&mut renderer, &native, &declarations, KeyCode::Enter)
                } else if !discovered && action == "host:new-tab" {
                    // The original NewTab editor is inline, not a modal.
                    let screen = &dialog;
                    let y = screen
                        .iter()
                        .position(|row| row.contains("Cancel"))
                        .unwrap();
                    let byte = screen[y].find("Cancel").unwrap();
                    let x = screen[y][..byte].chars().count();
                    let (_, cancel, context) = trailboss::terminal::action_at(
                        &native,
                        &declarations,
                        Rect::viewport(100, 30),
                        renderer.controls.list_offset,
                        x.try_into().unwrap(),
                        y.try_into().unwrap(),
                    )
                    .expect("ordinary inline Cancel must have a public action");
                    assert_eq!(cancel, "host:cancel");
                    assert!(matches!(
                        native.invoke(cancel, context, None),
                        Effect::Consumed
                    ));
                    Response::Consumed
                } else {
                    key(&mut renderer, &native, &declarations, KeyCode::Tab);
                    key(&mut renderer, &native, &declarations, KeyCode::Tab);
                    key(&mut renderer, &native, &declarations, KeyCode::Enter)
                };
                match response {
                    Response::Invoke(cancel) => {
                        assert_eq!(cancel.action, "host:cancel", "unsubmitted {action} must never invoke its mutation");
                        assert!(matches!(native.invoke(&cancel.action, &cancel.context, None), Effect::Consumed));
                    }
                    Response::Pass if escape => {
                        assert!(matches!(native.text("\u{1b}"), Some(Effect::Consumed)));
                    }
                    Response::Consumed => {}
                    _ => panic!("existing cancellation was not handled: {action}, discovered={discovered}, escape={escape}"),
                }
                assert!(
                    !native.editing(),
                    "{action} left its input interaction active"
                );
                assert!(
                    native.input_text.is_empty(),
                    "{action} retained its unsubmitted draft"
                );
                renderer
                    .controls
                    .reconcile(&native, &declarations, Rect::viewport(100, 30));
                assert!(
                    !renderer.controls.modal(),
                    "{action} reopened its palette or modal"
                );
                assert!(!renderer.controls.paste("must-not-reopen-search"));
                let screen = rows(&mut renderer, &native, &declarations);
                assert!(
                    !screen
                        .iter()
                        .any(|row| row.contains("Commands") || row.contains("Unsubmitted draft")),
                    "{action}: {screen:?}"
                );
                assert_eq!(native.workspace_id, target.workspace_id);
                assert_eq!(native.focused_tab_id, target.tab_id);
                assert!(
                    native.text("resume-pane-input").is_none(),
                    "{action} still captured pane input"
                );
                assert_eq!(
                    cancellation_resources(&core),
                    before,
                    "{action} mutated resources without submit"
                );
                let origin = core.request("tab.get", json!({"tab_id":target.tab_id}));
                assert_eq!(
                    origin["tab"]["workspace_id"],
                    captured["workspace"]["workspace_id"]
                );
                assert!(
                    core.request("tab.get", json!({"tab_id":unrelated["tab"]["tab_id"]}))["tab"]
                        .is_object()
                );
            }
        }
        if action == "host:new-space" {
            // The genuine ordinary NewSpace endpoint remains immediate; no
            // standalone named flow is invented to satisfy the parity witness.
            let Effect::Request { method, params } = native.invoke("host:new-space", &target, None)
            else {
                panic!("ordinary NewSpace stopped being immediate")
            };
            assert_eq!(method, "workspace.create");
            let created = core.request(method, params);
            assert!(created["workspace"]["workspace_id"].is_string());
            assert!(!native.editing());
        }
    }
}

#[test]
fn cancelling_existing_plugin_owner_flow_discards_draft_on_both_entry_routes() {
    fn events(path: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    let core = support::Core::new();
    let captured = core.request(
        "workspace.create",
        json!({"label":"Capture","cwd":core.home,"focus":true}),
    );
    let unrelated = core.request(
        "workspace.create",
        json!({"label":"Elsewhere","cwd":core.home,"focus":false}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":captured["tab"]["tab_id"],"label":"Target"}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":unrelated["tab"]["tab_id"],"label":"Other"}),
    );
    let target = context(&captured);
    let trace = register_input_owner(&core);
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        link(&core, plugin);
    }
    let mut client = core.attach();
    let mut native = NativeCore::default();
    native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
    let deadline = Instant::now() + Duration::from_secs(8);
    let route = loop {
        if let Some(declarations) = &client.declarations {
            if let Some(selection) = matching(
                &available(&native, declarations),
                "fixture.input-owner",
                "ask-name",
                &target,
            ) {
                if declarations.iter().any(|(provider, declaration)| {
                    provider == "tommymorgan.stock-ui" && declaration.composition.is_some()
                }) {
                    break selection.3.clone();
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "existing owner action did not publish"
        );
        client.receive().unwrap();
    };
    let mut console = console::Console::new(&core);
    for (round, discovered) in [false, true].into_iter().enumerate() {
        macro_rules! observe {
            ($boundary:expr, $predicate:expr) => {{
                let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| console.wait($predicate)));
                assert!(observed.is_ok(),
                    "cancel boundary={} discovered={discovered}; receipts={:?}; logs={}; panes={}; snapshot={}; screen={:?}",
                    $boundary, events(&trace), core.request("plugin.log.list", json!({"limit":20})),
                    core.request("pane.list", json!({})), core.request("session.snapshot", json!({})), console.rows());
            }};
        }
        macro_rules! choose_tab {
            ($label:expr) => {{
                let needle = format!("○ {}", $label);
                observe!("ordinary-sidebar-tab", |rows| rows
                    .iter()
                    .any(|row| row.contains(&needle)));
                let screen = console.rows();
                let y = screen.iter().position(|row| row.contains(&needle)).unwrap();
                let byte = screen[y].find(&needle).unwrap();
                let x = screen[y][..byte].chars().count() + 2;
                console.click(x, y);
            }};
        }
        choose_tab!("Target");
        core.request("tab.focus", json!({"tab_id":captured["tab"]["tab_id"]}));
        core.request(
            "pane.send_text",
            json!({"pane_id":captured["root_pane"]["pane_id"],
            "text":format!("printf 'CANCEL_ORIGIN_READY_{round}\\n'\n")}),
        );
        observe!("valid-origin", |rows| rows.iter().any(|row| row
            .trim_end()
            .ends_with(&format!("CANCEL_ORIGIN_READY_{round}"))));
        let declarations = client.declarations.as_ref().unwrap();
        let mut renderer = FrameRenderer::default();
        if discovered {
            assert!(renderer
                .controls
                .open_palette(&native, declarations, Some(&target)));
            let Response::Invoke(selected) = select(&mut renderer, &native, declarations, &route)
            else {
                panic!("palette did not transfer to the existing owner")
            };
            assert_eq!(selected.plugin, "fixture.input-owner");
            assert_eq!(selected.action, "ask-name");
            assert_eq!(selected.context, target);
            assert!(!renderer.controls.modal());
            assert!(!renderer.controls.paste("must-not-be-a-query"));
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
        } else {
            client
                .input()
                .unwrap()
                .invoke(&client.boot_id, "fixture.input-owner", "ask-name", &target)
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        let pane = loop {
            let receipts = events(&trace);
            let panes: Vec<_> = receipts
                .iter()
                .filter(|event| event["kind"] == "pane")
                .collect();
            if panes.len() == round + 1 {
                break (*panes.last().unwrap()).clone();
            }
            assert!(
                !receipts.iter().any(|event| event["kind"] == "failure")
                    && Instant::now() < deadline,
                "owner did not open; receipts={receipts:?}; logs={}",
                core.request("plugin.log.list", json!({"limit":20}))
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(pane["owner"], "fixture.input-owner");
        assert_eq!(pane["entrypoint"], "name");
        assert_eq!(pane["target"], captured["tab"]["tab_id"]);
        let identity = core.request("pane.get", json!({"pane_id":pane["pane"]}));
        let owner_tab = &identity["pane"]["tab_id"];
        assert_ne!(owner_tab, &captured["tab"]["tab_id"]);
        let label = format!("Cancel{round}");
        core.request("tab.rename", json!({"tab_id":owner_tab,"label":label}));
        // Global pane opening does not steal another native client's view.
        // Its ordinary sidebar enters the actual owner-owned PTY.
        choose_tab!(&label);
        observe!("fresh-owner-prompt", |rows| rows
            .iter()
            .any(|row| row.contains("OWNER NAME (type cancel to dismiss):")));
        assert!(!console
            .rows()
            .iter()
            .any(|row| row.contains("Unsubmitted draft")));
        let before = cancellation_resources(&core);
        console.text("Unsubmitted draft");
        observe!("unsubmitted-owner-draft", |rows| rows
            .iter()
            .any(|row| row.contains("Unsubmitted draft")));
        assert!(!events(&trace).iter().any(|event| event["kind"] == "submit"));
        // This owner's existing dismissal is the word "cancel". Its normal
        // canonical-line edit replaces the unsubmitted draft before dismissal;
        // no palette-specific Escape/form protocol is added to that owner.
        console.text("\u{15}cancel\r");
        observe!("ordinary-owner-cancel", |rows| rows
            .iter()
            .any(|row| row.contains("OWNER CANCELLED")));
        assert_eq!(
            cancellation_resources(&core),
            before,
            "cancel mutated the target, another resource, or the owner pane"
        );
        let receipts = events(&trace);
        assert_eq!(
            receipts
                .iter()
                .filter(|event| event["kind"] == "cancel")
                .count(),
            round + 1
        );
        assert_eq!(
            receipts
                .iter()
                .filter(|event| event["kind"] == "invoke")
                .count(),
            round + 1
        );
        assert!(!receipts.iter().any(|event| event["kind"] == "submit"
            || event["kind"] == "reject"
            || event["kind"] == "failure"));
        // Existing cancellation leaves this owner pane alive. Prove that
        // subsequent native input still reaches it, without reopening search.
        console.text(&format!("OWNER_RESUMED_{round}"));
        observe!("valid-existing-owner-origin", |rows| rows
            .iter()
            .any(|row| row.contains("OWNER CANCELLED"))
            && rows
                .iter()
                .any(|row| row.contains(&format!("OWNER_RESUMED_{round}"))));
        assert!(!console.rows().iter().any(|row| row.contains("Commands")));
        assert!(!renderer.controls.modal());
        assert!(!renderer.controls.paste("must-not-reopen-search"));
        assert!(!rows(&mut renderer, &native, declarations)
            .iter()
            .any(|row| row.contains("Commands")));
        assert_eq!(
            core.request("tab.get", json!({"tab_id":captured["tab"]["tab_id"]}))["tab"]["label"],
            "Target"
        );
        assert_eq!(
            core.request("tab.get", json!({"tab_id":unrelated["tab"]["tab_id"]}))["tab"]["label"],
            "Other"
        );
    }
    for invoke in events(&trace)
        .iter()
        .filter(|event| event["kind"] == "invoke")
    {
        assert_eq!(invoke["owner"], "fixture.input-owner");
        assert_eq!(invoke["action"], "ask-name");
        assert_eq!(
            invoke["context"]["workspace_id"],
            captured["workspace"]["workspace_id"]
        );
        assert_eq!(invoke["context"]["tab_id"], captured["tab"]["tab_id"]);
        assert_eq!(
            invoke["context"]["pane_id"],
            captured["root_pane"]["pane_id"]
        );
    }
}

#[test]
fn unavailable_open_selection_disappears_without_activating_a_neighbor_or_replacement_owner() {
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    fn paint(
        renderer: &mut FrameRenderer,
        terminal: &mut ghostty_vt::Terminal,
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
        terminal.write(&bytes);
        terminal
            .screen_text_rows()
            .unwrap()
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .flat_map(|cell| cell.graphemes.iter())
                    .filter_map(|cp| char::from_u32(*cp))
                    .collect()
            })
            .collect()
    }
    fn receipts(path: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn await_receipts(path: &std::path::Path, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while receipts(path).len() != count {
            assert!(
                Instant::now() < deadline,
                "owner receipt count != {count}: {:?}",
                receipts(path)
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    for change in [
        "target is removed",
        "supplying plugin disappears",
        "supplying plugin becomes disabled",
    ] {
        let target_removed = change == "target is removed";
        let core = support::Core::new();
        let captured = core.request(
            "workspace.create",
            json!({"label":"Live selection","cwd":core.home,"focus":false}),
        );
        core.request(
            "tab.rename",
            json!({"tab_id":captured["tab"]["tab_id"],"label":"Chosen"}),
        );
        let sibling = core.request(
            "tab.create",
            json!({"workspace_id":captured["workspace"]["workspace_id"],
            "label":"Chosen","cwd":core.home,"focus":false}),
        );
        let origin = core.request(
            "workspace.create",
            json!({"label":"Unrelated origin","cwd":core.home,"focus":true}),
        );
        let target = context(&captured);
        let owner = core.home.join("stale-owner");
        let replacement = core.home.join("stale-replacement");
        let mut invocation_context = serde_json::to_value(&target).unwrap();
        invocation_context["invocation_source"] = json!("plugin_ui");
        if !target_removed {
            // Same title/action/target, distinct real owners. Both can execute;
            // losing one must not silently select the otherwise usable other.
            for (path, provider) in [
                (&owner, "fixture.stale-a"),
                (&replacement, "fixture.stale-b"),
            ] {
                std::fs::create_dir_all(path).unwrap();
                std::fs::write(path.join("trailboss-plugin.toml"), format!(
                    "id = {provider:?}\nname = \"Live selection owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n\
                     [[actions]]\nid = \"probe\"\ntitle = \"Unavailable candidate\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n"
                )).unwrap();
                std::fs::write(path.join("record.py"),
                    "import json,os\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
                core.request("plugin.link", json!({"path":path,"enabled":true}));
                core.request(
                    "plugin.action.invoke",
                    json!({"plugin_id":provider,"action_id":"probe","context":invocation_context}),
                );
                await_receipts(&path.join("executed.jsonl"), 1);
                let executed = receipts(&path.join("executed.jsonl"));
                assert_eq!(executed[0]["owner"], provider);
                assert_eq!(executed[0]["action"], "probe");
                assert_eq!(executed[0]["context"]["tab_id"], captured["tab"]["tab_id"]);
            }
        }
        for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
            link(&core, plugin);
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
        let candidate = |entry: &PublishedAction| {
            if target_removed {
                entry.0 == "trailboss.core"
                    && entry.1 == "host:rename-tab"
                    && entry.2.tab_id == target.tab_id
            } else {
                entry.0 == "fixture.stale-a" && entry.1 == "probe"
            }
        };
        let neighbor = |entry: &PublishedAction| {
            if target_removed {
                entry.0 == "trailboss.core"
                    && entry.1 == "host:rename-tab"
                    && entry.2.tab_id.as_deref() == sibling["tab"]["tab_id"].as_str()
            } else {
                entry.0 == "fixture.stale-b" && entry.1 == "probe"
            }
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        let route = loop {
            if let Some(declarations) = &client.declarations {
                let actions = available(&native, declarations);
                if let Some(selected) = actions.iter().find(|entry| candidate(entry)) {
                    if actions.iter().any(&neighbor)
                        && declarations.iter().any(|(provider, declaration)| {
                            provider == "tommymorgan.stock-ui" && declaration.composition.is_some()
                        })
                    {
                        break selected.3.clone();
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "{change}: real candidate and independent neighbor did not publish"
            );
            client.receive().unwrap();
        };
        let declarations = client.declarations.as_ref().unwrap();
        // Capture a genuine native invocation before withdrawal. Deliver it
        // afterwards to verify the existing authoritative backend precondition.
        let mut issued_renderer = FrameRenderer::default();
        assert!(issued_renderer
            .controls
            .open_palette(&native, declarations, Some(&target)));
        let issued = match select(&mut issued_renderer, &native, declarations, &route) {
            Response::Prepare(action) if target_removed => action,
            Response::Invoke(action) if !target_removed => action,
            _ => panic!("{change}: real owner route did not issue its original action"),
        };
        assert_eq!(
            issued.plugin,
            if target_removed {
                "trailboss.core"
            } else {
                "fixture.stale-a"
            }
        );
        assert_eq!(
            issued.action,
            if target_removed {
                "host:rename-tab"
            } else {
                "probe"
            }
        );
        assert_eq!(
            issued.context.workspace_id, target.workspace_id,
            "{change}: lost explicit workspace"
        );
        assert_eq!(
            issued.context.tab_id, target.tab_id,
            "{change}: lost explicit tab"
        );
        let delayed: Request = if target_removed {
            let mut original = NativeCore::default();
            original.update_session(
                native.workspace_id.clone(),
                native.focused_tab_id.clone(),
                true,
            );
            assert!(matches!(
                original.prepare_palette(&issued.action, &issued.context),
                Effect::Consumed
            ));
            original.text("Delayed original rename");
            let Some(Effect::Request { method, params }) = original.text("\r") else {
                panic!("original native rename did not issue its existing API request")
            };
            assert_eq!(method, "tab.rename");
            assert_eq!(params["tab_id"], captured["tab"]["tab_id"]);
            serde_json::from_value(json!({"id":"delayed-original","method":method,"params":params}))
                .unwrap()
        } else {
            serde_json::from_value(json!({"id":"delayed-original","method":"plugin.action.invoke",
                "params":{"plugin_id":issued.plugin,"action_id":issued.action,"context":invocation_context}})).unwrap()
        };
        let mut renderer = FrameRenderer::default();
        let mut terminal = ghostty_vt::Terminal::new(100, 30, 0).unwrap();
        assert!(renderer
            .controls
            .open_palette(&native, declarations, Some(&target)));
        select(
            &mut renderer,
            &native,
            declarations,
            &route[..route.len() - 1],
        );
        let (leaf, ordinal) = route.last().unwrap();
        for character in leaf.chars() {
            key(
                &mut renderer,
                &native,
                declarations,
                KeyCode::Char(character),
            );
        }
        for _ in 0..*ordinal {
            key(&mut renderer, &native, declarations, KeyCode::Down);
        }
        let before = paint(&mut renderer, &mut terminal, &native, declarations);
        let visible_before = before.iter().filter(|row| row.contains(leaf)).count();
        assert!(
            visible_before > 0,
            "{change}: selection was not visible: {before:?}"
        );
        match change {
            "target is removed" => {
                core.request("tab.close", json!({"tab_id":captured["tab"]["tab_id"]}));
            }
            "supplying plugin disappears" => {
                core.request("plugin.unlink", json!({"plugin_id":"fixture.stale-a"}));
            }
            "supplying plugin becomes disabled" => {
                core.request("plugin.disable", json!({"plugin_id":"fixture.stale-a"}));
            }
            _ => unreachable!(),
        }
        core.request("ui.command.list", json!({}));
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(declarations) = &client.declarations {
                let actions = available(&native, declarations);
                if !actions.iter().any(&candidate) && actions.iter().any(&neighbor) {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "{change}: current real producer retained the unavailable row"
            );
            client.receive().unwrap();
        }
        let declarations = client.declarations.as_ref().unwrap();
        let after = paint(&mut renderer, &mut terminal, &native, declarations);
        assert!(after.iter().filter(|row| row.contains(leaf)).count() < visible_before,
            "{change}: unavailable row was not removed before activation: before={before:?}, after={after:?}");
        let resources = cancellation_resources(&core);
        assert!(
            matches!(
                key(&mut renderer, &native, declarations, KeyCode::Enter),
                Response::Consumed
            ),
            "{change}: unavailable selection invoked a neighboring row or replacement owner"
        );
        assert!(
            renderer.controls.modal(),
            "{change}: stale activation closed search"
        );
        assert_eq!(
            paint(&mut renderer, &mut terminal, &native, declarations),
            after,
            "{change}: stale activation entered a different target or owner's branch"
        );
        assert_eq!(cancellation_resources(&core), resources);
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(
            core.home
                .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
        ));
        let reply = api
            .request_value_with_timeout(&delayed, Duration::from_secs(5))
            .unwrap();
        let expected_error = match change {
            "target is removed" => "tab_not_found",
            "supplying plugin disappears" => "plugin_not_found",
            "supplying plugin becomes disabled" => "plugin_disabled",
            _ => unreachable!(),
        };
        assert_eq!(
            reply["error"]["code"], expected_error,
            "{change}: changed original invocation precondition: {reply}"
        );
        assert_eq!(
            cancellation_resources(&core),
            resources,
            "{change}: delayed invocation retargeted another resource"
        );
        if !target_removed {
            assert_eq!(
                receipts(&owner.join("executed.jsonl")).len(),
                1,
                "{change}: withdrawn owner executed"
            );
            assert_eq!(
                receipts(&replacement.join("executed.jsonl")).len(),
                1,
                "{change}: replacement owner executed without reselection"
            );
        }
        // A fresh, deliberate selection still reaches the surviving command;
        // avoiding stale activation must not disable the eligible neighbor.
        let next = available(&native, declarations)
            .into_iter()
            .find(&neighbor)
            .unwrap();
        let mut deliberate = FrameRenderer::default();
        assert!(deliberate
            .controls
            .open_palette(&native, declarations, Some(&target)));
        match select(&mut deliberate, &native, declarations, &next.3) {
            Response::Prepare(selected) if target_removed => {
                assert_eq!(
                    selected.context.tab_id.as_deref(),
                    sibling["tab"]["tab_id"].as_str()
                );
                assert!(matches!(
                    native.prepare_palette(&selected.action, &selected.context),
                    Effect::Consumed
                ));
                native.text("Explicit neighbor");
                let Some(Effect::Request { method, params }) = native.text("\r") else {
                    panic!("surviving rename failed")
                };
                assert_eq!(params["tab_id"], sibling["tab"]["tab_id"]);
                core.request(method, params);
                assert_eq!(
                    core.request("tab.get", json!({"tab_id":sibling["tab"]["tab_id"]}))["tab"]
                        ["label"],
                    "Explicit neighbor"
                );
            }
            Response::Invoke(selected) if !target_removed => {
                assert_eq!(selected.plugin, "fixture.stale-b");
                assert_eq!(selected.action, "probe");
                assert_eq!(selected.context.tab_id, target.tab_id);
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
                await_receipts(&replacement.join("executed.jsonl"), 2);
                assert_eq!(receipts(&owner.join("executed.jsonl")).len(), 1);
            }
            _ => panic!("{change}: eligible neighbor could not be deliberately selected"),
        }
        assert_eq!(
            core.request("tab.get", json!({"tab_id":origin["tab"]["tab_id"]}))["tab"]
                ["workspace_id"],
            origin["workspace"]["workspace_id"],
            "{change}: unrelated origin was replaced"
        );
    }
}
#[test]
fn reordered_duplicate_commands_keep_the_selected_owner_and_captured_resource() {
    fn receipts(path: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    let core = support::Core::new();
    let captured = core.request(
        "workspace.create",
        json!({"label":"Captured","cwd":core.home,"focus":false}),
    );
    let replacement = core.request(
        "workspace.create",
        json!({"label":"Replacement","cwd":core.home,"focus":false}),
    );
    let origin = core.request(
        "workspace.create",
        json!({"label":"Origin","cwd":core.home,"focus":true}),
    );
    let target = context(&captured);
    let other = context(&replacement);
    let mut owners = Vec::new();
    for owner in ["fixture.reorder-blue", "fixture.reorder-red"] {
        let directory = core.home.join(owner);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("trailboss-plugin.toml"), format!(
            "id = {owner:?}\nname = \"Reorder owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n\
             [[actions]]\nid = \"probe\"\ntitle = \"Duplicate command\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n"
        )).unwrap();
        std::fs::write(directory.join("record.py"),
            "import json,os\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
        core.request("plugin.link", json!({"path":directory,"enabled":true}));
        owners.push(directory);
    }
    // The public custom provider controls only ordering/context wrappers. Its
    // leaves are unmodified, real Core-issued Uses attributed to the two owners.
    let catalog = core.request("ui.command.list", json!({}));
    let locators: Vec<String> = ["fixture.reorder-blue", "fixture.reorder-red"]
        .iter()
        .map(|owner| {
            let command = catalog["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|command| {
                    command["key"]["plugin_id"] == *owner && command["key"]["action_id"] == "probe"
                })
                .unwrap();
            assert_eq!(command["title"], "Duplicate command");
            command["fragment_name"].as_str().unwrap().to_owned()
        })
        .collect();
    let ordering = core.home.join("ordered-public-palette");
    std::fs::create_dir_all(&ordering).unwrap();
    std::fs::write(ordering.join("trailboss-plugin.toml"),
        "id = \"fixture.reorder-palette\"\nname = \"Ordered public Uses\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n\
         [[ui]]\nid = \"order\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"publish.py\"]\nstream = true\n").unwrap();
    std::fs::write(ordering.join("publish.py"),
        "import json,time\nprevious=None\nwhile True:\n try:\n  value=json.load(open('declaration.json'))\n  text=json.dumps(value)\n  if text!=previous:\n   print(text,flush=True)\n   previous=text\n except (OSError,ValueError): pass\n time.sleep(0.02)\n").unwrap();
    let publish = |reversed: bool| {
        let mut contexts = vec![target.clone(), other.clone()];
        let mut names = locators.clone();
        if reversed {
            contexts.reverse();
            names.reverse();
        }
        let branches: Vec<Value> = contexts
            .into_iter()
            .map(|context| {
                json!({
                    "type":"row", "context":context,
                    "children":[{"type":"label","text":"Same target label","role":"label"}],
                    "menu":{"type":"column","children":names.iter().map(|name|
                        json!({"type":"use","name":name})).collect::<Vec<_>>()}
                })
            })
            .collect();
        std::fs::write(
            ordering.join("declaration.json"),
            serde_json::to_vec(&json!({
                "herdr":"widget_tree","version":1,"zones":[],
                "fragments":[{"name":"command-palette","priority":10000,
                    "tree":{"type":"column","children":branches}}]
            }))
            .unwrap(),
        )
        .unwrap();
    };
    publish(false);
    core.request("plugin.link", json!({"path":ordering,"enabled":true}));
    let mut client = core.attach();
    let mut native = NativeCore::default();
    native.update_session(
        origin["workspace"]["workspace_id"]
            .as_str()
            .map(str::to_owned),
        origin["tab"]["tab_id"].as_str().map(str::to_owned),
        true,
    );
    let wait_order =
        |client: &mut trailboss::Client, reversed: bool| {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                if let Some(declarations) = &client.declarations {
                    let actions = available(&native, declarations);
                    if actions.len() == 4 {
                        let blue =
                            matching(&actions, "fixture.reorder-blue", "probe", &target).unwrap();
                        let expected = usize::from(reversed);
                        if blue.3.iter().all(|(_, ordinal)| *ordinal == expected) {
                            assert!(matching(&actions, "fixture.reorder-red", "probe", &target)
                                .is_some());
                            assert!(matching(&actions, "fixture.reorder-blue", "probe", &other)
                                .is_some());
                            return blue.3.clone();
                        }
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "real ordered owner Uses did not publish: reversed={reversed}"
                );
                client.receive().unwrap();
            }
        };
    let route = wait_order(&mut client, false);
    let mut renderer = FrameRenderer::default();
    let declarations = client.declarations.as_ref().unwrap();
    assert!(renderer
        .controls
        .open_palette(&native, declarations, Some(&context(&origin))));
    assert!(matches!(
        select(
            &mut renderer,
            &native,
            declarations,
            &route[..route.len() - 1]
        ),
        Response::Consumed
    ));
    let (title, ordinal) = route.last().unwrap();
    assert_eq!(title, "Duplicate command");
    assert_eq!(*ordinal, 0);
    for character in title.chars() {
        key(
            &mut renderer,
            &native,
            declarations,
            KeyCode::Char(character),
        );
    }
    renderer
        .controls
        .reconcile(&native, declarations, Rect::viewport(100, 30));
    // Keep the exact Blue leaf selected while BOTH the duplicate owner leaves
    // and equal-label resource ancestors reverse their public order.
    publish(true);
    let reordered = wait_order(&mut client, true);
    assert_ne!(route, reordered);
    let declarations = client.declarations.as_ref().unwrap();
    renderer
        .controls
        .reconcile(&native, declarations, Rect::viewport(100, 30));
    match key(&mut renderer, &native, declarations, KeyCode::Enter) {
        Response::Invoke(selected) => {
            assert_eq!(
                selected.plugin, "fixture.reorder-blue",
                "reorder retargeted to the equal-title owner"
            );
            assert_eq!(selected.action, "probe");
            assert_eq!(
                selected.context, target,
                "reorder replaced the captured resource"
            );
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
            let deadline = Instant::now() + Duration::from_secs(8);
            while receipts(&owners[0].join("executed.jsonl")).is_empty() {
                assert!(
                    Instant::now() < deadline,
                    "selected Blue owner did not execute"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Response::Consumed => {
            // Safe cancellation is allowed; an unobserved background dispatch is not.
            std::thread::sleep(Duration::from_millis(150));
            assert!(receipts(&owners[0].join("executed.jsonl")).is_empty());
        }
        _ => panic!("reordered selection neither retained identity nor canceled safely"),
    }
    let first = receipts(&owners[0].join("executed.jsonl"));
    assert!(first.len() <= 1);
    assert!(
        receipts(&owners[1].join("executed.jsonl")).is_empty(),
        "Red ran without selection"
    );
    // Positive control: both the old and new order still admit a deliberate Blue
    // selection. Owner receipts, not presentation alone, establish the dispatch.
    let mut fresh = FrameRenderer::default();
    assert!(fresh
        .controls
        .open_palette(&native, declarations, Some(&context(&origin))));
    let Response::Invoke(selected) = select(&mut fresh, &native, declarations, &reordered) else {
        panic!("fresh Blue selection after reorder failed")
    };
    assert_eq!(selected.plugin, "fixture.reorder-blue");
    assert_eq!(selected.context, target);
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
    let deadline = Instant::now() + Duration::from_secs(8);
    while receipts(&owners[0].join("executed.jsonl")).len() < first.len() + 1 {
        assert!(
            Instant::now() < deadline,
            "fresh Blue invocation did not reach owner"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let executed = receipts(&owners[0].join("executed.jsonl"));
    assert_eq!(executed.len(), first.len() + 1);
    for event in executed {
        assert_eq!(event["owner"], "fixture.reorder-blue");
        assert_eq!(event["action"], "probe");
        assert_eq!(
            event["context"]["workspace_id"],
            captured["workspace"]["workspace_id"]
        );
        assert_eq!(event["context"]["tab_id"], captured["tab"]["tab_id"]);
        assert_eq!(
            event["context"]["pane_id"],
            captured["root_pane"]["pane_id"]
        );
    }
    assert!(receipts(&owners[1].join("executed.jsonl")).is_empty());
    assert_eq!(
        core.request("tab.get", json!({"tab_id":replacement["tab"]["tab_id"]}))["tab"]["label"],
        replacement["tab"]["label"]
    );
    assert_eq!(
        core.request("tab.get", json!({"tab_id":origin["tab"]["tab_id"]}))["tab"]["label"],
        origin["tab"]["label"]
    );
}

#[test]
fn discovered_rename_rejects_whitespace_without_a_request_then_corrects_in_the_same_flow() {
    for (action, method, collection, identity) in [
        (
            "host:rename-space",
            "workspace.rename",
            "workspaces",
            "workspace_id",
        ),
        ("host:rename-tab", "tab.rename", "tabs", "tab_id"),
    ] {
        let core = support::Core::new();
        let captured = core.request(
            "workspace.create",
            json!({"label":"Captured before correction","cwd":core.home,"focus":false}),
        );
        let unrelated = core.request(
            "workspace.create",
            json!({"label":"Unrelated origin","cwd":core.home,"focus":true}),
        );
        let target = context(&captured);
        for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
            link(&core, plugin);
        }
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(
            unrelated["workspace"]["workspace_id"]
                .as_str()
                .map(str::to_owned),
            unrelated["tab"]["tab_id"].as_str().map(str::to_owned),
            true,
        );
        let deadline = Instant::now() + Duration::from_secs(8);
        let route = loop {
            if let Some(declarations) = &client.declarations {
                if native
                    .fragment(declarations, "commands.name-existing")
                    .is_some()
                    && declarations.iter().any(|(owner, declaration)| {
                        owner == "tommymorgan.stock-ui" && declaration.composition.is_some()
                    })
                {
                    if let Some(command) = matching(
                        &available(&native, declarations),
                        "trailboss.core",
                        action,
                        &target,
                    ) {
                        break command.3.clone();
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "{action}: real rename and shared name interaction did not publish"
            );
            client.receive().unwrap();
        };
        let declarations = client.declarations.as_ref().unwrap();
        let mut renderer = FrameRenderer::default();
        assert!(renderer
            .controls
            .open_palette(&native, declarations, Some(&target)));
        let Response::Prepare(prepared) = select(&mut renderer, &native, declarations, &route)
        else {
            panic!("{action}: discovered rename did not hand off to its existing controller");
        };
        assert_eq!(prepared.plugin, "trailboss.core");
        assert_eq!(prepared.action, action);
        assert!(matches!(
            native.prepare_palette(&prepared.action, &prepared.context),
            Effect::Consumed
        ));
        assert!(native.editing());
        assert!(native.input_text.is_empty());
        let dialog = rows(&mut renderer, &native, declarations);
        assert!(
            dialog.iter().any(|row| row.contains("Cancel")),
            "{action}: {dialog:?}"
        );
        assert!(
            !dialog.iter().any(|row| row.contains("Commands")),
            "{action}: {dialog:?}"
        );
        let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
        assert!(matches!(native.text("   "), Some(Effect::Consumed)));
        // The public native Effect is the actual outbound-request seam. Merely
        // observing an unchanged label would miss a rejected rename RPC.
        let rejected = native.text("\r");
        assert!(
            matches!(rejected, Some(Effect::Consumed)),
            "{action}: whitespace emitted an effect other than local validation consumption"
        );
        assert!(
            native.editing(),
            "{action}: validation dismissed the existing interaction"
        );
        assert_eq!(
            native.input_text, "   ",
            "{action}: rejection replaced the draft/controller"
        );
        let after_rejection = core.request("session.snapshot", json!({}))["snapshot"].clone();
        for kind in ["workspaces", "tabs"] {
            for previous in before[kind].as_array().unwrap() {
                let id = if kind == "tabs" {
                    "tab_id"
                } else {
                    "workspace_id"
                };
                let current = after_rejection[kind]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|item| item[id] == previous[id])
                    .unwrap();
                assert_eq!(
                    current["label"], previous["label"],
                    "{action}: rejection changed a resource"
                );
            }
        }
        // Correct, do not reopen or re-prepare the controller. A later focus
        // change must not turn correction into a rename of the current origin.
        native.update_session(
            unrelated["workspace"]["workspace_id"]
                .as_str()
                .map(str::to_owned),
            unrelated["tab"]["tab_id"].as_str().map(str::to_owned),
            true,
        );
        for _ in 0..3 {
            assert!(matches!(native.text("\x7f"), Some(Effect::Consumed)));
        }
        assert!(native.input_text.is_empty());
        assert!(matches!(
            native.text("Corrected captured name"),
            Some(Effect::Consumed)
        ));
        let Some(Effect::Request {
            method: submitted,
            params,
        }) = native.text("\r")
        else {
            panic!("{action}: correction did not submit through the existing interaction");
        };
        assert_eq!(submitted, method);
        let target_id = if identity == "workspace_id" {
            captured["workspace"]["workspace_id"].clone()
        } else {
            captured["tab"]["tab_id"].clone()
        };
        assert_eq!(params[identity], target_id);
        assert_eq!(params["label"], "Corrected captured name");
        // Only the one emitted request is forwarded, exactly as the native
        // executable does; another Enter must not emit a duplicate request.
        core.request(submitted, params);
        assert!(!native.editing());
        assert!(
            !matches!(native.text("\r"), Some(Effect::Request { .. })),
            "{action}: completed correction emitted a duplicate request"
        );
        let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
        for kind in ["workspaces", "tabs"] {
            let id = if kind == "tabs" {
                "tab_id"
            } else {
                "workspace_id"
            };
            for previous in before[kind].as_array().unwrap() {
                let current = after[kind]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|item| item[id] == previous[id])
                    .unwrap();
                let expected = if kind == collection && previous[id] == target_id {
                    json!("Corrected captured name")
                } else {
                    previous["label"].clone()
                };
                assert_eq!(
                    current["label"], expected,
                    "{action}: corrected submit changed another resource"
                );
            }
        }
    }
}

#[test]
fn discovered_owner_rows_hide_missing_captured_resources_without_authorizing_manifest_categories() {
    for resource in ["workspace", "tab"] {
        let core = support::Core::new();
        let live = core.request(
            "workspace.create",
            json!({"label":"Live owner context","cwd":core.home,"focus":true}),
        );
        let target = context(&live);
        let (invalid, valid, category) = if resource == "workspace" {
            (
                json!({"workspace_id":"missing-captured-workspace"}),
                json!({"workspace_id":target.workspace_id}),
                "pane",
            )
        } else {
            (
                json!({"workspace_id":target.workspace_id,"tab_id":"missing-captured-tab"}),
                json!({"tab_id":target.tab_id}),
                "workspace",
            )
        };
        // The valid row intentionally lacks the resource named by its manifest
        // category. Categories are discovery hints, not extra authorization.
        let mut roots = Vec::new();
        for (owner, title, captured) in [
            (
                "fixture.context-unavailable",
                "Unavailable captured resource",
                &invalid,
            ),
            (
                "fixture.context-eligible",
                "Eligible independent owner",
                &valid,
            ),
        ] {
            let root = core.home.join(owner);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("trailboss-plugin.toml"), format!(
                "id = {owner:?}\nname = \"Captured context owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n\
                 [[actions]]\nid = \"probe\"\ntitle = {title:?}\ncontexts = [{category:?}]\ncommand = [\"python3\", \"record.py\"]\n\
                 [[ui]]\nid = \"ordinary-owner-row\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"publish.py\"]\nstream = true\n"
            )).unwrap();
            std::fs::write(root.join("record.py"),
                "import json,os\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
            std::fs::write(root.join("publish.py"),
                "import sys\nprint(open('declaration.json').read(),flush=True)\nfor line in sys.stdin: pass\n").unwrap();
            std::fs::write(root.join("declaration.json"), serde_json::to_vec(&json!({
                "herdr":"widget_tree","version":1,"zones":[],
                "fragments":[{"name":format!("{owner}.ordinary"),"priority":0,
                    "tree":{"type":"column","children":[{"type":"row","action":"probe","context":captured,
                        "children":[{"type":"label","text":title,"role":"label"}]}]}}]
            })).unwrap()).unwrap();
            core.request("plugin.link", json!({"path":root,"enabled":true}));
            roots.push(root);
        }
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
        // Start the actual default producer only after both ordinary owner
        // contexts have reached Core, avoiding a targetless startup projection.
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if client.declarations.as_ref().is_some_and(|declarations| {
                ["fixture.context-unavailable", "fixture.context-eligible"]
                    .iter()
                    .all(|owner| {
                        native
                            .fragment(declarations, &format!("{owner}.ordinary"))
                            .is_some()
                    })
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{resource}: ordinary owner rows did not publish"
            );
            client.receive().unwrap();
        }
        let catalog = core.request("ui.command.list", json!({}));
        for owner in ["fixture.context-unavailable", "fixture.context-eligible"] {
            let command = catalog["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|command| {
                    command["key"]["plugin_id"] == owner && command["key"]["action_id"] == "probe"
                })
                .unwrap();
            assert!(
                command.get("diagnostic").is_none_or(Value::is_null),
                "{resource}: {command}"
            );
            assert_eq!(command["target"], "owner");
        }
        for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
            link(&core, plugin);
        }
        let expected: Context = serde_json::from_value(valid.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let route = loop {
            if let Some(declarations) = &client.declarations {
                let actions = available(&native, declarations);
                if let Some(eligible) = actions.iter().find(|entry| {
                    entry.0 == "fixture.context-eligible"
                        && entry.1 == "probe"
                        && entry.2 == expected
                }) {
                    assert!(!actions.iter().any(|entry| entry.0 == "fixture.context-unavailable"),
                        "{resource}: missing captured resource remained a discovered row: {actions:?}");
                    break eligible.3.clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "{resource}: eligible owner with nonbinding category disappeared"
            );
            client.receive().unwrap();
        };
        let declarations = client.declarations.as_ref().unwrap();
        let mut renderer = FrameRenderer::default();
        assert!(renderer
            .controls
            .open_palette(&native, declarations, Some(&target)));
        for character in "Unavailable captured resource".chars() {
            key(
                &mut renderer,
                &native,
                declarations,
                KeyCode::Char(character),
            );
        }
        let rendered = rows(&mut renderer, &native, declarations);
        assert!(
            rendered.iter().any(|row| row.contains("No matches")),
            "{resource}: invalid target was displayed, including as a disabled row: {rendered:?}"
        );
        assert!(matches!(
            key(&mut renderer, &native, declarations, KeyCode::Enter),
            Response::Consumed
        ));
        let mut fresh = FrameRenderer::default();
        assert!(fresh
            .controls
            .open_palette(&native, declarations, Some(&target)));
        let Response::Invoke(selected) = select(&mut fresh, &native, declarations, &route) else {
            panic!("{resource}: valid independent owner was not actionable");
        };
        assert_eq!(selected.plugin, "fixture.context-eligible");
        assert_eq!(selected.action, "probe");
        assert_eq!(
            selected.context, expected,
            "{resource}: manifest category replaced the owner's explicit context"
        );
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
        let receipt = roots[1].join("executed.jsonl");
        let deadline = Instant::now() + Duration::from_secs(8);
        while !std::fs::read_to_string(&receipt).is_ok_and(|text| text.ends_with('\n')) {
            assert!(
                Instant::now() < deadline,
                "{resource}: valid owner did not execute"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let events: Vec<Value> = std::fs::read_to_string(receipt)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["owner"], "fixture.context-eligible");
        assert_eq!(events[0]["action"], "probe");
        assert_eq!(
            events[0]["context"]["workspace_id"],
            valid.get("workspace_id").cloned().unwrap_or(Value::Null)
        );
        assert_eq!(
            events[0]["context"]["tab_id"],
            valid.get("tab_id").cloned().unwrap_or(Value::Null)
        );
        assert!(
            !roots[0].join("executed.jsonl").exists(),
            "{resource}: unavailable owner executed"
        );
    }
}

#[test]
fn discovered_previous_uses_client_history_while_rename_keeps_the_menu_target() {
    fn observe_focus(client: &mut trailboss::Client, native: &mut NativeCore, tab: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while client
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.focused_tab_id.as_deref() != Some(tab))
        {
            assert!(
                Instant::now() < deadline,
                "this client's actual focus did not become {tab}"
            );
            client.receive().unwrap();
        }
        let snapshot = client.snapshot.as_ref().unwrap();
        native.update_session(
            snapshot.focused_workspace_id.clone(),
            snapshot.focused_tab_id.clone(),
            snapshot.focused_pane_id.is_some(),
        );
        native.reconcile_tab_history(snapshot);
    }
    fn focus(client: &mut trailboss::Client, native: &mut NativeCore, tab: &str) {
        client
            .input()
            .unwrap()
            .request(&client.boot_id, "tab.focus", json!({"tab_id":tab}))
            .unwrap();
        observe_focus(client, native, tab);
    }
    let core = support::Core::new();
    let a = core.request(
        "workspace.create",
        json!({"label":"History workspace","cwd":core.home,"focus":true}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":a["tab"]["tab_id"],"label":"A"}),
    );
    let b = core.request("tab.create",
        json!({"workspace_id":a["workspace"]["workspace_id"],"label":"B","cwd":core.home,"focus":false}));
    let c = core.request("tab.create",
        json!({"workspace_id":a["workspace"]["workspace_id"],"label":"C","cwd":core.home,"focus":false}));
    let menu_target: Context = serde_json::from_value(json!({
        "workspace_id":a["workspace"]["workspace_id"],"tab_id":b["tab"]["tab_id"],
        "pane_id":b["root_pane"]["pane_id"]
    }))
    .unwrap();
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        link(&core, plugin);
    }
    let mut client = core.attach();
    let mut native = NativeCore::default();
    observe_focus(
        &mut client,
        &mut native,
        a["tab"]["tab_id"].as_str().unwrap(),
    );
    // These visits use this endpoint's ordinary request path, not the global
    // API focus or a menu's captured context. B has never been visited.
    focus(
        &mut client,
        &mut native,
        c["tab"]["tab_id"].as_str().unwrap(),
    );
    focus(
        &mut client,
        &mut native,
        a["tab"]["tab_id"].as_str().unwrap(),
    );
    assert_eq!(
        native.previous_tab(client.snapshot.as_ref().unwrap()),
        c["tab"]["tab_id"].as_str()
    );
    let catalog = core.request("ui.command.list", json!({}));
    let previous = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| {
            command["key"] == json!({"kind":"core","action_id":"host:focus","mode":"previous"})
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let (previous_route, rename_route) = loop {
        if let Some(declarations) = &client.declarations {
            let commands = available(&native, declarations);
            let previous = commands.iter().find(|entry| {
                entry.0 == "trailboss.core"
                    && entry.1 == "host:focus"
                    && entry.2.tab_id.is_none()
                    && entry
                        .3
                        .last()
                        .is_some_and(|step| Some(step.0.as_str()) == previous["title"].as_str())
            });
            let rename = matching(&commands, "trailboss.core", "host:rename-tab", &menu_target);
            if let (Some(previous), Some(rename)) = (previous, rename) {
                if native
                    .fragment(declarations, "commands.name-existing")
                    .is_some()
                {
                    break (previous.3.clone(), rename.3.clone());
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "actual Previous and B rename commands did not publish"
        );
        client.receive().unwrap();
    };
    let mut renderer = FrameRenderer::default();
    let declarations = client.declarations.as_ref().unwrap();
    assert!(renderer
        .controls
        .open_palette(&native, declarations, Some(&menu_target)));
    let Response::Invoke(previous) = select(&mut renderer, &native, declarations, &previous_route)
    else {
        panic!("discovered Previous did not produce its existing native action");
    };
    assert_eq!(previous.plugin, "trailboss.core");
    assert_eq!(previous.action, "host:focus");
    assert_eq!(
        previous.context,
        Context::default(),
        "menu B contaminated the client-local Previous operation"
    );
    let Effect::Request { method, params } = native.invoke(
        &previous.action,
        &previous.context,
        client.snapshot.as_ref(),
    ) else {
        panic!("Previous did not request this client's eligible prior visit");
    };
    assert_eq!(method, "tab.focus");
    assert_eq!(params["tab_id"], c["tab"]["tab_id"]);
    client
        .input()
        .unwrap()
        .request(&client.boot_id, method, params)
        .unwrap();
    observe_focus(
        &mut client,
        &mut native,
        c["tab"]["tab_id"].as_str().unwrap(),
    );
    assert_ne!(
        native.focused_tab_id.as_deref(),
        b["tab"]["tab_id"].as_str()
    );
    // A second discovered operation uses the same B menu context, but unlike
    // Previous its authoritative target is B rather than the client's history.
    let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
    let declarations = client.declarations.as_ref().unwrap();
    assert!(renderer
        .controls
        .open_palette(&native, declarations, Some(&menu_target)));
    let Response::Prepare(rename) = select(&mut renderer, &native, declarations, &rename_route)
    else {
        panic!("discovered B rename did not reuse the existing name interaction");
    };
    assert_eq!(
        rename.context.tab_id.as_deref(),
        b["tab"]["tab_id"].as_str()
    );
    assert!(matches!(
        native.prepare_palette(&rename.action, &rename.context),
        Effect::Consumed
    ));
    assert!(native.editing());
    focus(
        &mut client,
        &mut native,
        a["tab"]["tab_id"].as_str().unwrap(),
    );
    assert!(
        native.editing(),
        "unrelated focus dismissed B's captured interaction"
    );
    assert!(matches!(
        native.text("Renamed B after focus changed"),
        Some(Effect::Consumed)
    ));
    let Some(Effect::Request { method, params }) = native.text("\r") else {
        panic!("existing captured B interaction failed to submit");
    };
    assert_eq!(method, "tab.rename");
    assert_eq!(params["tab_id"], b["tab"]["tab_id"]);
    client
        .input()
        .unwrap()
        .request(&client.boot_id, method, params)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if client.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.tabs.iter().any(|tab| {
                Some(tab.tab_id.as_str()) == b["tab"]["tab_id"].as_str()
                    && tab.label == "Renamed B after focus changed"
            })
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "captured B rename did not reach actual Core"
        );
        client.receive().unwrap();
    }
    let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
    for old in before["tabs"].as_array().unwrap() {
        let current = after["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tab| tab["tab_id"] == old["tab_id"])
            .unwrap();
        let expected = if old["tab_id"] == b["tab"]["tab_id"] {
            json!("Renamed B after focus changed")
        } else {
            old["label"].clone()
        };
        assert_eq!(
            current["label"], expected,
            "captured rename changed another tab"
        );
    }
    assert_eq!(
        native.focused_tab_id.as_deref(),
        a["tab"]["tab_id"].as_str(),
        "B's rename stole this client's unrelated focus"
    );
}

#[test]
fn original_creation_entries_stay_immediate_while_discovered_creation_uses_shared_names() {
    fn resources(snapshot: &Value) -> Value {
        json!({
            "workspaces":snapshot["workspaces"].as_array().unwrap().iter().map(|workspace|
                json!({"id":workspace["workspace_id"],"label":workspace["label"]})).collect::<Vec<_>>(),
            "tabs":snapshot["tabs"].as_array().unwrap().iter().map(|tab|
                json!({"id":tab["tab_id"],"workspace":tab["workspace_id"],"label":tab["label"]})).collect::<Vec<_>>()
        })
    }
    for action in ["host:new-space", "host:new-tab"] {
        let core = support::Core::new();
        let captured = core.request(
            "workspace.create",
            json!({"label":"Shortcut target","cwd":core.home,"focus":true}),
        );
        let target = context(&captured);
        for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
            link(&core, plugin);
        }
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
        let menu_name = if action == "host:new-space" {
            "sidebar.session-menu".to_owned()
        } else {
            format!(
                "sidebar.space-menu:{}",
                target.workspace_id.as_deref().unwrap()
            )
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if client.declarations.as_ref().is_some_and(|declarations| {
                native.fragment(declarations, &menu_name).is_some()
                    && native.fragment(declarations, "commands.name-new").is_some()
                    && matching(
                        &available(&native, declarations),
                        "trailboss.core",
                        action,
                        &target,
                    )
                    .is_some()
                    && declarations.iter().any(|(owner, declaration)| {
                        owner == "tommymorgan.stock-ui" && declaration.composition.is_some()
                    })
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{action}: original and discovered creation entries did not publish"
            );
            client.receive().unwrap();
        }
        let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
        let declarations = client.declarations.as_ref().unwrap();
        let mut original = FrameRenderer::default();
        let painted = rows(&mut original, &native, declarations);
        let viewport = Rect::viewport(100, 30);
        let surface = trailboss::terminal::surface_rect(&native, declarations, viewport).unwrap();
        let x = surface.x.checked_sub(2).unwrap();
        let y = if action == "host:new-space" {
            viewport.height - 1
        } else {
            painted
                .iter()
                .position(|line| line.contains("Shortcut target"))
                .unwrap() as u16
        };
        // Activate the real retained session/workspace row menu, not a fabricated
        // '+' declaration or a newly invented non-palette naming entrypoint.
        original
            .controls
            .hover(&native, declarations, viewport, x, y);
        assert!(matches!(
            original
                .controls
                .click(&native, declarations, viewport, x, y, false),
            Response::Consumed
        ));
        let Response::Invoke(shortcut) = key(&mut original, &native, declarations, KeyCode::Enter)
        else {
            panic!("{action}: retained immediate creation entry did not invoke");
        };
        assert_eq!(shortcut.action, action);
        assert_eq!(shortcut.plugin, "tommymorgan.sidebar-menus");
        if action == "host:new-tab" {
            assert_eq!(shortcut.context.workspace_id, target.workspace_id);
        }
        let Effect::Request { method, params } =
            native.invoke(&shortcut.action, &shortcut.context, None)
        else {
            panic!("{action}: immediate creation unexpectedly opened a name form");
        };
        assert!(
            !native.editing(),
            "{action}: original shortcut gained a name interaction"
        );
        assert_eq!(
            method,
            if action == "host:new-space" {
                "workspace.create"
            } else {
                "tab.create"
            }
        );
        assert_eq!(
            params["label"],
            if action == "host:new-space" {
                "New space"
            } else {
                "New tab"
            }
        );
        assert_eq!(params["focus"], true);
        if action == "host:new-tab" {
            assert_eq!(
                params["workspace_id"],
                captured["workspace"]["workspace_id"]
            );
        }
        let created = core.request(method, params);
        let immediate = core.request("session.snapshot", json!({}))["snapshot"].clone();
        assert_eq!(
            immediate["tabs"].as_array().unwrap().len(),
            before["tabs"].as_array().unwrap().len() + 1
        );
        assert_eq!(
            immediate["workspaces"].as_array().unwrap().len(),
            before["workspaces"].as_array().unwrap().len()
                + usize::from(action == "host:new-space")
        );
        assert_eq!(immediate["focused_tab_id"], created["tab"]["tab_id"]);
        if action == "host:new-space" {
            assert_eq!(created["workspace"]["label"], "New space");
            assert_eq!(
                immediate["focused_workspace_id"],
                created["workspace"]["workspace_id"]
            );
        } else {
            assert_eq!(created["tab"]["label"], "New tab");
            assert_eq!(
                created["tab"]["workspace_id"],
                captured["workspace"]["workspace_id"]
            );
        }
        for cancel in [false, true] {
            let baseline = core.request("session.snapshot", json!({}))["snapshot"].clone();
            let deadline = Instant::now() + Duration::from_secs(8);
            while client.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.tabs.len() != baseline["tabs"].as_array().unwrap().len()
            }) {
                assert!(
                    Instant::now() < deadline,
                    "{action}: actual creation snapshot did not arrive"
                );
                client.receive().unwrap();
            }
            let declarations = client.declarations.as_ref().unwrap();
            let route = matching(
                &available(&native, declarations),
                "trailboss.core",
                action,
                &target,
            )
            .unwrap()
            .3
            .clone();
            let mut named = FrameRenderer::default();
            assert!(named
                .controls
                .open_palette(&native, declarations, Some(&target)));
            let Response::Prepare(prepared) = select(&mut named, &native, declarations, &route)
            else {
                panic!("{action}: discovered name mode did not hand off to the shared controller");
            };
            assert_eq!(prepared.plugin, "trailboss.core");
            assert_eq!(prepared.action, action);
            assert!(matches!(
                native.prepare_palette(&prepared.action, &prepared.context),
                Effect::Consumed
            ));
            assert!(native.editing());
            assert!(
                native.input_text.is_empty(),
                "{action}: command search leaked into the name draft"
            );
            let after_open = core.request("session.snapshot", json!({}))["snapshot"].clone();
            assert_eq!(
                resources(&after_open),
                resources(&baseline),
                "{action}: opening the name interaction created resources"
            );
            assert!(matches!(
                native.text("Chosen named creation"),
                Some(Effect::Consumed)
            ));
            if cancel {
                assert!(matches!(native.text("\x1b"), Some(Effect::Consumed)));
                assert!(!native.editing());
                assert!(native.input_text.is_empty());
                let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
                assert_eq!(
                    resources(&after),
                    resources(&baseline),
                    "{action}: canceled name created or changed resources"
                );
            } else {
                let Some(Effect::Request { method, params }) = native.text("\r") else {
                    panic!("{action}: shared name submission did not create");
                };
                assert_eq!(
                    method,
                    if action == "host:new-space" {
                        "workspace.create"
                    } else {
                        "tab.create"
                    }
                );
                assert_eq!(params["label"], "Chosen named creation");
                if action == "host:new-tab" {
                    assert_eq!(
                        params["workspace_id"],
                        captured["workspace"]["workspace_id"]
                    );
                }
                let created = core.request(method, params);
                let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
                assert_eq!(
                    after["tabs"].as_array().unwrap().len(),
                    baseline["tabs"].as_array().unwrap().len() + 1
                );
                assert_eq!(
                    after["workspaces"].as_array().unwrap().len(),
                    baseline["workspaces"].as_array().unwrap().len()
                        + usize::from(action == "host:new-space")
                );
                assert_eq!(
                    created[if action == "host:new-space" {
                        "workspace"
                    } else {
                        "tab"
                    }]["label"],
                    "Chosen named creation"
                );
                assert!(!native.editing());
            }
        }
    }
}

#[test]
fn discovered_control_character_titles_keep_safe_visible_text_and_original_execution_identity() {
    discovered_display_identity(
        "Read\n\u{1b}[31m receipt".to_owned(),
        "Read\n\u{1b}[31m receipt".to_owned(),
        "Read\\n\\u{1b}[31m receipt".to_owned(),
        false,
    );
}

#[test]
fn discovered_overlong_titles_keep_ellipsis_and_distinct_original_execution_identity() {
    discovered_display_identity(
        format!("Long {} zzzz", "x".repeat(240)),
        format!("Long {} aaaa", "x".repeat(240)),
        format!("Long {}…", "x".repeat(194)),
        true,
    );
}

fn discovered_display_identity(
    original: String,
    decoy: String,
    displayed: String,
    shortened: bool,
) {
    let core = support::Core::new();
    let captured = core.request(
        "workspace.create",
        json!({"label":"Display target","cwd":core.home,"focus":false}),
    );
    let unrelated = core.request(
        "workspace.create",
        json!({"label":"Display unrelated","cwd":core.home,"focus":true}),
    );
    let target = context(&captured);
    let owner = core.home.join("display-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let mut manifest = String::from(
        "id = \"fixture.display-owner\"\nname = \"Display owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n");
    for (action, title) in [("a-decoy", &decoy), ("z-original", &original)] {
        manifest.push_str(&format!(
            "[[actions]]\nid = {action:?}\ntitle = {}\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n",
            json!(title)));
    }
    manifest.push_str("[[ui]]\nid = \"existing-captured-actions\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"publish.py\"]\nstream = true\n");
    std::fs::write(owner.join("trailboss-plugin.toml"), manifest).unwrap();
    std::fs::write(owner.join("record.py"),
        "import json,os\nwith open('executed.jsonl','a') as output:\n output.write(json.dumps({'owner':os.environ['HERDR_PLUGIN_ID'],'action':os.environ['HERDR_PLUGIN_ACTION_ID'],'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])})+'\\n')\n").unwrap();
    // Ordinary owner labels are already valid public labels. Their explicit
    // action contexts must survive the default palette's title presentation.
    std::fs::write(owner.join("declaration.json"), serde_json::to_vec(&json!({
        "herdr":"widget_tree","version":1,"zones":[],
        "fragments":[{"name":"fixture.display-owner.ordinary","priority":0,
            "tree":{"type":"column","children":(["a-decoy","z-original"].iter().map(|action|
                json!({"type":"row","action":action,"context":target,
                    "children":[{"type":"label","text":"Existing owner action","role":"label"}]}))
                .collect::<Vec<_>>())}}]
    })).unwrap()).unwrap();
    std::fs::write(owner.join("publish.py"),
        "import sys\nprint(open('declaration.json').read(),flush=True)\nfor line in sys.stdin: pass\n").unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    let catalog = core.request("ui.command.list", json!({}));
    let mut fragments = Vec::new();
    for (action, title) in [("a-decoy", &decoy), ("z-original", &original)] {
        let expected_key =
            json!({"kind":"plugin","plugin_id":"fixture.display-owner","action_id":action});
        let command = catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == expected_key)
            .unwrap();
        assert_eq!(command["title"], *title, "catalog lost the original title");
        assert_eq!(command["key"], expected_key);
        assert!(command.get("diagnostic").is_none_or(Value::is_null));
        fragments.push(command["fragment_name"].as_str().unwrap().to_owned());
    }
    assert_ne!(
        fragments[0], fragments[1],
        "equal display labels merged structured identities"
    );
    let mut client = core.attach();
    let mut native = NativeCore::default();
    native.update_session(
        unrelated["workspace"]["workspace_id"]
            .as_str()
            .map(str::to_owned),
        unrelated["tab"]["tab_id"].as_str().map(str::to_owned),
        true,
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if client.declarations.as_ref().is_some_and(|declarations| {
            native
                .fragment(declarations, "fixture.display-owner.ordinary")
                .is_some()
                && fragments
                    .iter()
                    .all(|name| native.fragment(declarations, name).is_some())
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "ordinary owner context and real Core-issued display labels did not publish"
        );
        client.receive().unwrap();
    }
    for name in &fragments {
        let (provider, tree) = native
            .fragment(client.declarations.as_ref().unwrap(), name)
            .unwrap();
        assert_eq!(provider, "fixture.display-owner");
        let Node::Row { children, .. } = tree else {
            panic!("Core-issued command is not its ordinary row");
        };
        let Node::Label { text, .. } = &children[0] else {
            panic!("Core-issued title has no display label");
        };
        // Worked expectations above come from the public 200-scalar contract,
        // not a test copy of the application's normalizer.
        assert_eq!(text, &displayed);
        assert!(!text.chars().any(char::is_control));
        assert_eq!(text.ends_with('…'), shortened);
        if shortened {
            assert_eq!(text.chars().count(), 200);
        }
    }
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        link(&core, plugin);
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let route = loop {
        if let Some(declarations) = &client.declarations {
            let actions = available(&native, declarations);
            let original = matching(&actions, "fixture.display-owner", "z-original", &target);
            let decoy = matching(&actions, "fixture.display-owner", "a-decoy", &target);
            if let (Some(original), Some(_)) = (original, decoy) {
                assert_eq!(
                    original.3[0].1, 1,
                    "fixture must select the second equal-display identity"
                );
                break original.3.clone();
            }
        }
        assert!(
            Instant::now() < deadline,
            "actual default palette failed to publish unusual owner titles; logs={}",
            core.request(
                "plugin.log.list",
                json!({"plugin_id":"tommymorgan.command-palette","limit":20})
            )
        );
        let received = client.receive();
        assert!(
            received.is_ok(),
            "default palette publication stalled for unusual titles; receive={received:?}; logs={}",
            core.request(
                "plugin.log.list",
                json!({"plugin_id":"tommymorgan.command-palette","limit":20})
            )
        );
    };
    let declarations = client.declarations.as_ref().unwrap();
    let mut renderer = FrameRenderer::default();
    assert!(renderer
        .controls
        .open_palette(&native, declarations, Some(&context(&unrelated))));
    let mut terminal = ghostty_vt::Terminal::new(100, 30, 0).unwrap();
    let mut paint = |renderer: &mut FrameRenderer| {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(
                &mut bytes,
                None,
                declarations,
                &native,
                Rect::viewport(100, 30),
            )
            .unwrap();
        terminal.write(&bytes);
        terminal
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
    };
    let describe = |response: &Response| match response {
        Response::Pass => json!({"response":"Pass"}),
        Response::Consumed => json!({"response":"Consumed"}),
        Response::Invoke(action) => json!({"response":"Invoke","owner":action.plugin,
            "action":action.action,"context":action.context}),
        Response::Prepare(action) => json!({"response":"Prepare","owner":action.plugin,
            "action":action.action,"context":action.context}),
    };
    // Search using the visible safe text, not the original control bytes or the
    // hidden long-title suffix. Both rows match, so text cannot be identity.
    for character in displayed.chars() {
        key(
            &mut renderer,
            &native,
            declarations,
            KeyCode::Char(character),
        );
    }
    let screen = paint(&mut renderer);
    assert!(
        !screen.iter().any(|line| line.contains("No matches")),
        "{screen:?}"
    );
    if shortened {
        assert!(
            screen
                .iter()
                .filter(|line| line.contains("Long ") && line.contains('…'))
                .count()
                >= 3,
            "query and both selectable native rows must visibly mark truncation: {screen:?}"
        );
    } else {
        assert!(
            screen
                .iter()
                .filter(|line| line.contains("Read\\n\\u{1b}[31m receipt"))
                .count()
                >= 3,
            "query and both native rows must show escaped controls literally: {screen:?}"
        );
    }
    key(&mut renderer, &native, declarations, KeyCode::Down);
    let before_enter = paint(&mut renderer);
    let opened = key(&mut renderer, &native, declarations, KeyCode::Enter);
    let after_open = paint(&mut renderer);
    assert!(
        matches!(opened, Response::Consumed),
        "first Enter: {}",
        describe(&opened)
    );
    let activated = key(&mut renderer, &native, declarations, KeyCode::Enter);
    let after_activation = paint(&mut renderer);
    let Response::Invoke(selected) = activated else {
        let (_, root) = native.fragment(declarations, "command-palette").unwrap();
        panic!(
            "selected displayed row did not invoke its original owner; \
            response={}; route={route:?}; before_enter={before_enter:?}; \
            after_open={after_open:?}; after_activation={after_activation:?}; root={}",
            describe(&activated),
            serde_json::to_value(root).unwrap()
        );
    };
    assert_eq!(selected.plugin, "fixture.display-owner");
    assert_eq!(
        selected.action, "z-original",
        "display text was used instead of original action identity"
    );
    assert_eq!(
        selected.context, target,
        "display selection borrowed the unrelated current context"
    );
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
    let receipt = owner.join("executed.jsonl");
    let deadline = Instant::now() + Duration::from_secs(8);
    while !std::fs::read_to_string(&receipt).is_ok_and(|text| text.ends_with('\n')) {
        assert!(Instant::now() < deadline, "original owner did not execute");
        std::thread::sleep(Duration::from_millis(10));
    }
    let events: Vec<Value> = std::fs::read_to_string(receipt)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["owner"], "fixture.display-owner");
    assert_eq!(events[0]["action"], "z-original");
    assert_eq!(
        events[0]["context"]["workspace_id"],
        captured["workspace"]["workspace_id"]
    );
    assert_eq!(events[0]["context"]["tab_id"], captured["tab"]["tab_id"]);
    assert_eq!(
        events[0]["context"]["pane_id"],
        captured["root_pane"]["pane_id"]
    );
    let after = core.request("ui.command.list", json!({}));
    for (action, title) in [("a-decoy", &decoy), ("z-original", &original)] {
        let expected_key =
            json!({"kind":"plugin","plugin_id":"fixture.display-owner","action_id":action});
        let command = after["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["key"] == expected_key)
            .unwrap();
        assert_eq!(
            command["title"], *title,
            "display/search/invocation changed the owner title"
        );
        assert_eq!(command["key"], expected_key);
    }
}
