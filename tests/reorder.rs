use serde_json::json;
use trailboss::{
    core::Core,
    sidebar::Declarations,
    terminal::{
        reorder::{move_request, Drag},
        Rect,
    },
};
use trailboss_runtime::protocol::{widgets::Context, ClientShellSnapshot};
fn context(space: &str, tab: Option<&str>) -> Context {
    Context {
        workspace_id: Some(space.into()),
        tab_id: tab.map(str::to_owned),
        pane_id: None,
    }
}
fn snapshot() -> ClientShellSnapshot {
    serde_json::from_value(json!({"boot_id":"boot","revision":1,"config_diagnostic":null,"product_announcement":null,"update_available":null,"update_install_command":"","server_keybindings_toml":null,"latest_release_notes_available":false,"integration_updates_available":false,"worktree_directory":"","release_notes":null,"focused_workspace_id":"a","focused_tab_id":"t1","focused_pane_id":null,"tab_bar_right":[],"tab_bar_right_separator":"","agent_view_label":null,"agent_order":[],"workspaces":[space("a"),space("b"),space("c")],"tabs":[tab("t1","a"),tab("t2","a"),tab("t3","a"),tab("other","b")],"panes":[],"agents":[],"commands":[]})).unwrap()
}
fn space(id: &str) -> serde_json::Value {
    json!({"workspace_id":id,"active_tab_id":"","new_workspace_cwd":"","number":1,"label":"Duplicate","custom_label":false,"branch":null,"git_ahead_behind":null,"tokens":[],"worktree":null,"focused":false,"agent_status":"idle"})
}
fn tab(id: &str, space: &str) -> serde_json::Value {
    json!({"tab_id":id,"workspace_id":space,"number":1,"label":"Duplicate","custom_label":false,"zoomed":false,"focused":false,"agent_status":"idle"})
}
fn declarations(tabs: bool) -> Declarations {
    let children: Vec<_> = if tabs {
        ["t1","t2","t3"].iter().map(|id|json!({"type":"row","action":"custom:select","context":{"workspace_id":"a","tab_id":id},"children":[{"type":"label","text":"Duplicate","role":"label"}],"menu":{"type":"column","children":[]}})).collect()
    } else {
        ["a","b","c"].iter().map(|id|json!({"type":"row","action":"custom:select","context":{"workspace_id":id},"children":[{"type":"label","text":"Duplicate","role":"label"}],"menu":{"type":"column","children":[]}})).collect()
    };
    vec![("arbitrary.provider".into(),serde_json::from_value(json!({"herdr":"widget_tree","version":1,"zones":[],"composition":{"priority":0,"tree":{"type":"row","children":[{"type":"flex","weight":1,"child":{"type":"column","children":children}},{"type":"flex","weight":1,"child":{"type":"surface"}}]}}})).unwrap())]
}
fn expanded_declarations(s: &ClientShellSnapshot) -> Declarations {
    let mut children =
        vec![json!({"type":"row","children":[{"type":"label","text":"Title","role":"title"}]})];
    for (index, workspace) in s.workspaces.iter().enumerate() {
        if index != 0 {
            children
                .push(json!({"type":"row","children":[{"type":"label","text":"","role":"muted"}]}));
        }
        let id = &workspace.workspace_id;
        children.push(json!({"type":"row","action":"custom:select","context":{"workspace_id":id},"children":[{"type":"label","text":"Duplicate","role":"label"}]}));
        for tab in s.tabs.iter().filter(|tab| tab.workspace_id == *id) {
            children.push(json!({"type":"row","action":"custom:select","context":{"workspace_id":id,"tab_id":tab.tab_id},"children":[{"type":"label","text":"Duplicate","role":"label"}]}));
        }
        if id == "c" {
            children.push(json!({"type":"row","action":"custom:focus-pane","context":{"workspace_id":id,"tab_id":"t3","pane_id":"private"},"children":[{"type":"label","text":"Pane","role":"muted"}]}));
        }
    }
    let mut d = declarations(false);
    d[0].1.composition.as_mut().unwrap().tree = serde_json::from_value(json!({"type":"row","children":[{"type":"flex","weight":1,"child":{"type":"column","children":children}},{"type":"flex","weight":1,"child":{"type":"surface"}}]})).unwrap();
    d
}
fn expanded_snapshot() -> ClientShellSnapshot {
    let mut s = snapshot();
    for tab in &mut s.tabs {
        if tab.workspace_id == "a" {
            tab.workspace_id = "c".into();
        }
    }
    s
}
#[test]
fn space_drag_downward_crosses_expanded_final_group_and_drops_after_all_children() {
    let (core, mut s, v) = (Core::default(), expanded_snapshot(), Rect::viewport(30, 14));
    let d = expanded_declarations(&s);
    let mut drag = Drag::default();
    assert!(drag.press(&core, &d, v, &s, 1, 1));
    for y in 2..=11 {
        drag.motion(&core, &d, v, &s, 1, y);
        assert!(drag.captured(), "cancelled while crossing group at row {y}");
        assert!([1, 3, 6, 11].contains(&drag.indicator().unwrap().y));
        drag.reconcile(&core, &d, v, &s);
        assert!(drag.captured());
    }
    assert_eq!(drag.indicator().unwrap().y, 11);
    // The source is retained by ID, while endpoint indices come from release state.
    s.workspaces
        .insert(0, serde_json::from_value(space("new")).unwrap());
    let result = drag.release(&core, &d, v, &s, 1, 11);
    assert!(result.click.is_none());
    assert_eq!(
        result.movement.unwrap().params,
        json!({"workspace_id":"a","insert_index":4})
    );
    assert!(!drag.captured());
    assert!(drag.indicator().is_none());
}
#[test]
fn expanded_space_drag_upward_targets_root_boundary_not_child_rows() {
    let (core, s, v) = (Core::default(), expanded_snapshot(), Rect::viewport(30, 14));
    let d = expanded_declarations(&s);
    let mut drag = Drag::default();
    assert!(!drag.press(&core, &d, v, &s, 1, 10)); // Pane is not draggable.
    assert!(drag.press(&core, &d, v, &s, 1, 6));
    for y in (1..6).rev() {
        drag.motion(&core, &d, v, &s, 1, y);
        assert!(drag.captured());
        assert!([1, 3, 6, 11].contains(&drag.indicator().unwrap().y));
    }
    assert_eq!(drag.indicator().unwrap().y, 1);
    let result = drag.release(&core, &d, v, &s, 1, 1);
    assert_eq!(
        result.movement.unwrap().params,
        json!({"workspace_id":"c","insert_index":0})
    );
}
#[test]
fn expanded_space_drag_cancels_outside_group_or_surface_boundary() {
    let (core, s, v) = (Core::default(), expanded_snapshot(), Rect::viewport(30, 14));
    let d = expanded_declarations(&s);
    for (x, y) in [(1, 12), (15, 11), (1, 14)] {
        let mut drag = Drag::default();
        assert!(drag.press(&core, &d, v, &s, 1, 1));
        drag.motion(&core, &d, v, &s, 1, 11);
        assert_eq!(drag.indicator().unwrap().y, 11);
        drag.motion(&core, &d, v, &s, x, y);
        assert!(!drag.captured());
        assert!(drag.indicator().is_none());
        let result = drag.release(&core, &d, v, &s, 1, 11);
        assert!(result.movement.is_none());
        assert!(result.click.is_none());
    }
    // A clipped final group has no drawable after-group slot.
    let clipped = Rect::viewport(30, 11);
    let mut drag = Drag::default();
    assert!(drag.press(&core, &d, clipped, &s, 1, 1));
    drag.motion(&core, &d, clipped, &s, 1, 10);
    assert!(drag.captured());
    assert!([1, 3, 6].contains(&drag.indicator().unwrap().y));
    assert!(drag.indicator().unwrap().y < clipped.height);
}
#[test]
fn expanded_linked_final_space_keeps_marker_after_entire_block() {
    let (core, mut s, v) = (Core::default(), expanded_snapshot(), Rect::viewport(30, 14));
    s.workspaces.swap(1, 2); // a, c (root), b (linked child).
    s.workspaces[1].worktree = Some(
        serde_json::from_value(json!({"key":"repo","label":"root","is_linked_worktree":false}))
            .unwrap(),
    );
    s.workspaces[2].worktree = Some(
        serde_json::from_value(json!({"key":"repo","label":"child","is_linked_worktree":true}))
            .unwrap(),
    );
    let d = expanded_declarations(&s);
    let mut drag = Drag::default();
    assert!(drag.press(&core, &d, v, &s, 1, 1));
    for y in 2..=11 {
        drag.motion(&core, &d, v, &s, 1, y);
        assert!(drag.captured());
        assert!([1, 3, 11].contains(&drag.indicator().unwrap().y));
    }
    assert_eq!(drag.indicator().unwrap().y, 11);
    assert_eq!(
        drag.release(&core, &d, v, &s, 1, 11)
            .movement
            .unwrap()
            .params,
        json!({"workspace_id":"a","insert_index":3})
    );
    assert!(drag.press(&core, &d, v, &s, 1, 3));
    drag.motion(&core, &d, v, &s, 1, 1);
    let movement = drag.release(&core, &d, v, &s, 1, 1).movement.unwrap();
    assert_eq!(movement.method, "workspace.move_block");
    assert_eq!(
        movement.params,
        json!({"workspace_ids":["c","b"],"before_workspace_id":"a"})
    );
}

#[test]
fn clicks_wait_for_release_and_menu_and_pane_do_not_capture() {
    let (core, d, s, v) = (
        Core::default(),
        declarations(false),
        snapshot(),
        Rect::viewport(30, 8),
    );
    let mut drag = Drag::default();
    assert!(!drag.press(&core, &d, v, &s, 13, 0));
    assert!(!drag.press(&core, &d, v, &s, 15, 0));
    assert!(drag.press(&core, &d, v, &s, 1, 0));
    assert!(drag.pending());
    assert!(drag.indicator().is_none());
    let result = drag.release(&core, &d, v, &s, 1, 0);
    assert!(result.movement.is_none());
    let click = result.click.unwrap();
    assert_eq!(click.plugin, "arbitrary.provider");
    assert_eq!(click.context.workspace_id.as_deref(), Some("a"));
    assert!(!drag.captured());
}
#[test]
fn one_cell_drag_shows_clipped_line_and_reorders_only_on_release() {
    let (core, d, s, v) = (
        Core::default(),
        declarations(false),
        snapshot(),
        Rect::viewport(30, 8),
    );
    let mut drag = Drag::default();
    assert!(drag.press(&core, &d, v, &s, 1, 0));
    drag.motion(&core, &d, v, &s, 2, 0);
    assert!(!drag.pending());
    assert_eq!(drag.indicator().unwrap().y, 0);
    drag.motion(&core, &d, v, &s, 2, 3);
    let line = drag.indicator().unwrap();
    assert_eq!(line.y, 3);
    assert!(line.x + line.width <= 15);
    let result = drag.release(&core, &d, v, &s, 2, 3);
    assert!(result.click.is_none());
    assert_eq!(
        result.movement.unwrap().params,
        json!({"workspace_id":"a","insert_index":3})
    );
    assert!(drag.indicator().is_none());
}
#[test]
fn invalid_drop_boot_removed_source_resize_and_cancel_clear() {
    let (core, d, s, v) = (
        Core::default(),
        declarations(false),
        snapshot(),
        Rect::viewport(30, 8),
    );
    for kind in 0..5 {
        let mut drag = Drag::default();
        drag.press(&core, &d, v, &s, 1, 0);
        drag.motion(&core, &d, v, &s, 1, 3);
        match kind {
            0 => {
                let r = drag.release(&core, &d, v, &s, 15, 3);
                assert!(r.movement.is_none());
                assert!(r.click.is_none());
            }
            1 => {
                let mut changed = s.clone();
                changed.boot_id = "new".into();
                assert!(drag.reconcile(&core, &d, v, &changed));
            }
            2 => {
                let mut changed = s.clone();
                changed.workspaces.remove(0);
                assert!(drag.reconcile(&core, &d, v, &changed));
            }
            3 => {
                assert!(drag.reconcile(&core, &d, Rect::viewport(20, 8), &s));
            }
            _ => {
                assert!(drag.cancel());
            }
        }
        assert!(!drag.captured());
        assert!(drag.indicator().is_none());
    }
}
#[test]
fn tab_indices_are_live_workspace_local_and_suppress_noops() {
    let mut s = snapshot();
    let source = context("a", Some("t1"));
    assert!(move_request(&s, &source, Some("t1")).is_none());
    assert!(move_request(&s, &source, Some("t2")).is_none());
    assert!(move_request(&s, &source, Some("other")).is_none());
    assert_eq!(
        move_request(&s, &source, None).unwrap().params["insert_index"],
        3
    );
    s.tabs.swap(0, 2);
    assert_eq!(
        move_request(&s, &source, Some("t3")).unwrap().params["insert_index"],
        0
    );
    let (core, d, v) = (Core::default(), declarations(true), Rect::viewport(30, 8));
    let mut drag = Drag::default();
    drag.press(&core, &d, v, &s, 1, 2);
    drag.motion(&core, &d, v, &s, 1, 0);
    assert!(drag.release(&core, &d, v, &s, 1, 0).movement.is_some());
}
#[test]
fn linked_groups_move_as_whole_and_cannot_be_split() {
    let mut s = snapshot();
    s.workspaces[0].worktree = Some(
        serde_json::from_value(json!({"key":"repo","label":"repo","is_linked_worktree":false}))
            .unwrap(),
    );
    s.workspaces[1].worktree = Some(
        serde_json::from_value(json!({"key":"repo","label":"branch","is_linked_worktree":true}))
            .unwrap(),
    );
    assert!(move_request(&s, &context("b", None), None).is_none());
    assert!(move_request(&s, &context("c", None), Some("b")).is_none());
    assert!(move_request(&s, &context("a", None), Some("c")).is_none());
    let request = move_request(&s, &context("a", None), None).unwrap();
    assert_eq!(request.method, "workspace.move_block");
    assert_eq!(
        request.params,
        json!({"workspace_ids":["a","b"],"before_workspace_id":null})
    );
    // Noncontiguous linked children are gathered, exactly like upstream.
    s.workspaces.swap(1, 2);
    assert_eq!(
        move_request(&s, &context("a", None), None).unwrap().params["workspace_ids"],
        json!(["a", "b"])
    );
}

#[test]
fn private_core_applies_generated_workspace_and_tab_orders() {
    use std::{
        path::PathBuf,
        process::{Child, Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    struct Fixture {
        child: Child,
        home: PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }
    let home = std::env::temp_dir().join(format!(
        "reorder-proof-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let executable = std::env::var_os("TRAILBOSS_REORDER_CORE_BIN")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_trailboss").into());
    let mut command = Command::new(executable);
    command.args(["server", "--session", "reorder-proof"]);
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        command.env(key, &home);
    }
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
        command.env_remove(key);
    }
    command
        .env("SHELL", "/bin/sh")
        .env("TRAILBOSS_CONFIG_PATH", home.join("config.toml"))
        .env("TRAILBOSS_STARTUP_CWD", &home)
        .current_dir(&home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let fixture = Fixture {
        child: command.spawn().unwrap(),
        home: home.clone(),
    };
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(
        home.join("trailboss/sessions/reorder-proof/trailboss.sock"),
    ));
    let request = |method: &str, params: serde_json::Value| -> Request {
        serde_json::from_value(json!({"id":"reorder-proof","method":method,"params":params}))
            .unwrap()
    };
    let ready = request("session.snapshot", json!({}));
    let deadline = Instant::now() + Duration::from_secs(10);
    while api
        .request_value_with_timeout(&ready, Duration::from_millis(100))
        .is_err()
    {
        assert!(Instant::now() < deadline, "private Core not ready");
        std::thread::sleep(Duration::from_millis(20));
    }
    let call = |method: &str, params: serde_json::Value| {
        let reply = api
            .request_value_with_timeout(&request(method, params), Duration::from_secs(5))
            .unwrap();
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    };
    for label in ["A", "B", "C"] {
        call(
            "workspace.create",
            json!({"label":label,"cwd":home,"focus":false}),
        );
    }
    let list = call("workspace.list", json!({}));
    let spaces = list["workspaces"].as_array().unwrap();
    let ids: Vec<String> = spaces
        .iter()
        .map(|w| w["workspace_id"].as_str().unwrap().into())
        .collect();
    let mut s = snapshot();
    s.workspaces = ids
        .iter()
        .map(|id| serde_json::from_value(space(id)).unwrap())
        .collect();
    let moved = move_request(&s, &context(&ids[0], None), None).unwrap();
    call(moved.method, moved.params);
    let list = call("workspace.list", json!({}));
    assert_eq!(
        list["workspaces"].as_array().unwrap().last().unwrap()["workspace_id"],
        ids[0]
    );
    for label in ["one", "two", "three"] {
        call(
            "tab.create",
            json!({"workspace_id":ids[0],"label":label,"cwd":home,"focus":false}),
        );
    }
    let list = call("tab.list", json!({"workspace_id":ids[0]}));
    let tabs = list["tabs"].as_array().unwrap();
    let tids: Vec<String> = tabs
        .iter()
        .map(|t| t["tab_id"].as_str().unwrap().into())
        .collect();
    s.tabs = tids
        .iter()
        .map(|id| serde_json::from_value(tab(id, &ids[0])).unwrap())
        .collect();
    let moved = move_request(&s, &context(&ids[0], Some(&tids[0])), None).unwrap();
    call(moved.method, moved.params);
    let list = call("tab.list", json!({"workspace_id":ids[0]}));
    assert_eq!(
        list["tabs"].as_array().unwrap().last().unwrap()["tab_id"],
        tids[0]
    );
    // The unchanged r12 API also accepts whole-group requests, without a server cutover.
    s.workspaces[0].worktree = Some(
        serde_json::from_value(json!({"key":"proof","label":"proof","is_linked_worktree":false}))
            .unwrap(),
    );
    s.workspaces[1].worktree = Some(
        serde_json::from_value(json!({"key":"proof","label":"proof","is_linked_worktree":true}))
            .unwrap(),
    );
    let moved = move_request(&s, &context(&ids[0], None), None).unwrap();
    call(moved.method, moved.params);
    let list = call("workspace.list", json!({}));
    let result = list["workspaces"].as_array().unwrap();
    assert_eq!(result[result.len() - 2]["workspace_id"], ids[0]);
    assert_eq!(result[result.len() - 1]["workspace_id"], ids[1]);
    call("server.stop", json!({}));
    drop(fixture);
}
#[test]
fn indicator_restores_labels_after_move_cancel_and_invalid_drop() {
    use trailboss::{terminal::FrameRenderer, Cell, Surface};
    let (mut core, d, s, v) = (
        Core::default(),
        declarations(false),
        snapshot(),
        Rect::viewport(30, 8),
    );
    core.has_pane = true;
    let surface = Surface {
        pane_id: "private".into(),
        width: 15,
        height: 8,
        cells: (0..120)
            .map(|_| Cell {
                symbol: "P".into(),
                foreground: 0,
                background: 0,
                modifiers: 0,
                hyperlink: None,
            })
            .collect(),
        cursor: None,
        focused: true,
        mouse_reporting: false,
        hyperlinks: Vec::new(),
        graphics: Default::default(),
        graphics_scope: String::new(),
    };
    let mut renderer = FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(30, 8, 0).unwrap();
    let paint = |renderer: &mut FrameRenderer, screen: &mut ghostty_vt::Terminal| {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(&mut bytes, Some(&surface), &d, &core, v)
            .unwrap();
        screen.write(&bytes);
    };
    let rows = |screen: &ghostty_vt::Terminal| -> Vec<String> {
        screen
            .screen_text_rows()
            .unwrap()
            .iter()
            .map(|r| {
                r.cells
                    .iter()
                    .map(|c| {
                        c.graphemes
                            .first()
                            .and_then(|c| char::from_u32(*c))
                            .unwrap_or(' ')
                    })
                    .collect()
            })
            .collect()
    };
    paint(&mut renderer, &mut screen);
    let baseline = rows(&screen);
    renderer.reorder.press(&core, &d, v, &s, 1, 0);
    renderer.reorder.motion(&core, &d, v, &s, 1, 1);
    paint(&mut renderer, &mut screen);
    assert!(rows(&screen)[1].starts_with("───────────────"));
    assert_eq!(&rows(&screen)[1][45..], "P".repeat(15));
    renderer.reorder.motion(&core, &d, v, &s, 1, 2);
    paint(&mut renderer, &mut screen);
    assert_eq!(rows(&screen)[1], baseline[1]);
    renderer.reorder.cancel();
    paint(&mut renderer, &mut screen);
    assert_eq!(rows(&screen), baseline);
    renderer.reorder.press(&core, &d, v, &s, 1, 0);
    renderer.reorder.motion(&core, &d, v, &s, 1, 1);
    paint(&mut renderer, &mut screen);
    renderer.reorder.release(&core, &d, v, &s, 15, 1);
    paint(&mut renderer, &mut screen);
    assert_eq!(rows(&screen), baseline);
}
#[test]
fn narrow_viewports_and_collapsed_tabs_never_invent_destinations() {
    let (core, mut d, s) = (Core::default(), declarations(true), snapshot());
    for width in 2..8 {
        let v = Rect::viewport(width, 8);
        let mut drag = Drag::default();
        assert!(drag.press(&core, &d, v, &s, 0, 0));
        drag.motion(&core, &d, v, &s, 0, 2);
        let line = drag.indicator().unwrap();
        assert!(line.x + line.width <= width / 2);
        drag.cancel();
    }
    let v = Rect::viewport(30, 8);
    let mut drag = Drag::default();
    drag.press(&core, &d, v, &s, 1, 1);
    drag.motion(&core, &d, v, &s, 1, 0);
    d[0].1.composition.as_mut().unwrap().tree=serde_json::from_value(json!({"type":"row","children":[{"type":"flex","weight":1,"child":{"type":"column","children":[]}},{"type":"flex","weight":1,"child":{"type":"surface"}}]})).unwrap();
    assert!(drag.reconcile(&core, &d, v, &s));
    assert!(drag.indicator().is_none());
    assert!(drag.release(&core, &d, v, &s, 1, 0).movement.is_none());
}
