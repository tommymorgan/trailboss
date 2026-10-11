use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};
use trailboss::{
    core::Core,
    sidebar::Declarations,
    terminal::{menus::Response, FrameRenderer, Rect},
    Cell, Surface,
};
fn declaration(
    plugin: &str,
    body: Value,
) -> (String, trailboss_runtime::protocol::widgets::Declaration) {
    (plugin.into(), serde_json::from_value(body).unwrap())
}
fn sidebar() -> (String, trailboss_runtime::protocol::widgets::Declaration) {
    declaration(
        "sidebar",
        json!({"herdr":"widget_tree","version":1,"zones":[],"composition":{"priority":0,"tree":{"type":"column","children":[{"type":"row","children":[{"type":"label","text":"Target","role":"section"}],"context":{"workspace_id":"other"},"menu":{"type":"use","name":"example.menu"}},{"type":"flex","weight":1,"child":{"type":"surface"}}]}}}),
    )
}
fn provider(
    id: &str,
    priority: i32,
    target: &str,
) -> (String, trailboss_runtime::protocol::widgets::Declaration) {
    declaration(
        id,
        json!({"herdr":"widget_tree","version":1,"zones":[],"fragments":[{"name":"example.menu","priority":priority,"tree":{"type":"column","children":[{"type":"row","action":"custom","context":{"workspace_id":target},"children":[{"type":"label","text":"Act","role":"label"}]}]}}]}),
    )
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
#[test]
fn menu_actions_use_the_contributing_plugin_and_can_be_replaced_without_sidebar_edits() {
    let core = Core::default();
    let viewport = Rect::viewport(20, 6);
    let mut declarations: Declarations = vec![sidebar(), provider("first", 0, "first-target")];
    let original = declarations[0].clone();
    let mut renderer = FrameRenderer::default();
    assert!(matches!(
        renderer
            .controls
            .click(&core, &declarations, viewport, viewport.width - 2, 0, false),
        Response::Consumed
    ));
    let Response::Invoke(first) =
        renderer
            .controls
            .key(&core, &declarations, viewport, key(KeyCode::Enter))
    else {
        panic!("menu action absent")
    };
    assert_eq!(first.plugin, "first");
    assert_eq!(first.context.workspace_id.as_deref(), Some("first-target"));
    declarations.push(provider("second", 10, "second-target"));
    renderer
        .controls
        .click(&core, &declarations, viewport, viewport.width - 2, 0, false);
    let Response::Invoke(second) =
        renderer
            .controls
            .key(&core, &declarations, viewport, key(KeyCode::Enter))
    else {
        panic!("replacement absent")
    };
    assert_eq!(second.plugin, "second");
    assert_eq!(second.action, "custom");
    assert_eq!(
        second.context.workspace_id.as_deref(),
        Some("second-target")
    );
    assert_eq!(declarations[0], original);
    declarations.push(provider("z-tiebreak", 10, "tie-target"));
    renderer
        .controls
        .click(&core, &declarations, viewport, viewport.width - 2, 0, false);
    let Response::Invoke(tie) =
        renderer
            .controls
            .key(&core, &declarations, viewport, key(KeyCode::Enter))
    else {
        panic!("tie action absent")
    };
    assert_eq!(tie.plugin, "z-tiebreak");
}
fn rows(screen: &ghostty_vt::Terminal) -> Vec<String> {
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
}
fn paint(
    renderer: &mut FrameRenderer,
    screen: &mut ghostty_vt::Terminal,
    surface: &Surface,
    declarations: &Declarations,
    core: &Core,
) {
    let mut bytes = Vec::new();
    renderer
        .draw_frame(
            &mut bytes,
            Some(surface),
            declarations,
            core,
            Rect::viewport(20, 6),
        )
        .unwrap();
    screen.write(&bytes);
}
#[test]
fn hover_and_popup_dismissal_restore_pane_glyphs_and_links_without_linking_chrome() {
    let mut core = Core::default();
    core.has_pane = true;
    let declarations = vec![sidebar(), provider("menus", 0, "other")];
    let mut surface = Surface {
        pane_id: "pane".into(),
        width: 20,
        height: 5,
        cells: (0..100)
            .map(|_| Cell {
                symbol: "A".into(),
                foreground: 0,
                background: 0,
                modifiers: 0,
                hyperlink: Some(0),
            })
            .collect(),
        hyperlinks: vec![Some("https://example.test/complete/pane/target".into())],
        graphics: Default::default(),
        graphics_scope: String::new(),
        cursor: None,
        focused: true,
        mouse_reporting: false,
    };
    let mut renderer = FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(20, 6, 0).unwrap();
    paint(&mut renderer, &mut screen, &surface, &declarations, &core);
    assert!(!rows(&screen)[0].contains('⋯'));
    renderer
        .controls
        .hover(&core, &declarations, Rect::viewport(20, 6), 19, 0);
    paint(&mut renderer, &mut screen, &surface, &declarations, &core);
    assert!(rows(&screen)[0].contains('⋯'));
    renderer.controls.click(
        &core,
        &declarations,
        Rect::viewport(20, 6),
        rows(&screen)[0].chars().position(|ch| ch == '⋯').unwrap() as u16,
        0,
        false,
    );
    paint(&mut renderer, &mut screen, &surface, &declarations, &core);
    assert!(rows(&screen).iter().any(|r| r.contains("Act")));
    for (y, row) in rows(&screen).iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            if ch != 'A' {
                assert_eq!(
                    screen.viewport_hyperlink_uri(x as u16, y as u32).unwrap(),
                    None,
                    "popup/chrome cell {x},{y}"
                );
            }
        }
    }
    surface.cells[35].symbol = "B".into();
    paint(&mut renderer, &mut screen, &surface, &declarations, &core);
    assert!(rows(&screen).iter().any(|r| r.contains("Act")));
    renderer.controls.key(
        &core,
        &declarations,
        Rect::viewport(20, 6),
        key(KeyCode::Esc),
    );
    renderer.controls.key(
        &core,
        &declarations,
        Rect::viewport(20, 6),
        key(KeyCode::Esc),
    );
    renderer
        .controls
        .hover(&core, &declarations, Rect::viewport(20, 6), 19, 5);
    paint(&mut renderer, &mut screen, &surface, &declarations, &core);
    assert!(!rows(&screen)[0].contains('⋯'));
    for (y, row) in rows(&screen).iter().skip(1).enumerate() {
        let expected: String = surface.cells[y * 20..(y + 1) * 20]
            .iter()
            .map(|c| c.symbol.as_str())
            .collect();
        assert_eq!(*row, expected, "popup damage row {y}");
        for x in 0..20 {
            assert_eq!(
                screen
                    .viewport_hyperlink_uri(x, (y + 1) as u32)
                    .unwrap()
                    .as_deref(),
                Some("https://example.test/complete/pane/target")
            );
        }
    }
}
#[test]
fn unresolved_and_cyclic_fragments_do_not_recurse_or_dispatch_hidden_actions() {
    let core = Core::default();
    let viewport = Rect::viewport(20, 6);
    for contributions in [
        vec![],
        vec![declaration(
            "cycle",
            json!({"herdr":"widget_tree","version":1,"zones":[],"fragments":[{"name":"example.menu","priority":0,"tree":{"type":"use","name":"example.menu"}}]}),
        )],
    ] {
        let mut declarations = vec![sidebar()];
        declarations.extend(contributions);
        let mut renderer = FrameRenderer::default();
        let mut bytes = Vec::new();
        renderer
            .draw_frame(&mut bytes, None, &declarations, &core, viewport)
            .unwrap();
        renderer
            .controls
            .click(&core, &declarations, viewport, viewport.width - 2, 0, false);
        assert!(matches!(
            renderer
                .controls
                .key(&core, &declarations, viewport, key(KeyCode::Enter)),
            Response::Pass
        ));
    }
}

#[test]
fn every_row_menu_has_one_boundary_cell_and_only_its_glyph_is_clickable() {
    for context in [
        json!({}),
        json!({"workspace_id":"w"}),
        json!({"workspace_id":"w","tab_id":"t"}),
    ] {
        for width in 1..=20 {
            let core = Core::default();
            let viewport = Rect::viewport(width, 6);
            let declarations = vec![declaration(
                "sidebar",
                json!({"herdr":"widget_tree","version":1,"zones":[],"composition":{"priority":0,"tree":{"type":"row","context":context,"children":[{"type":"label","text":"Long row label","role":"label"}],"menu":{"type":"column","children":[{"type":"row","action":"scoped","context":context,"children":[{"type":"label","text":"Action","role":"label"}]}]}}}}),
            )];
            let mut renderer = FrameRenderer::default();
            renderer
                .controls
                .hover(&core, &declarations, viewport, 0, 0);
            let mut bytes = Vec::new();
            renderer
                .draw_frame(&mut bytes, None, &declarations, &core, viewport)
                .unwrap();
            let mut screen = ghostty_vt::Terminal::new(width, 6, 0).unwrap();
            screen.write(&bytes);
            let row = rows(&screen)[0].clone();
            assert!(matches!(
                renderer
                    .controls
                    .click(&core, &declarations, viewport, width - 1, 0, false),
                Response::Pass
            ));
            if width < 3 {
                assert!(!row.contains('⋯'));
            } else {
                let x = row.chars().position(|ch| ch == '⋯').unwrap() as u16;
                assert_eq!(x, width - 2);
                assert_eq!(row.chars().nth(usize::from(x + 1)), Some(' '));
                assert!(matches!(
                    renderer
                        .controls
                        .click(&core, &declarations, viewport, x, 0, false),
                    Response::Consumed
                ));
                let Response::Invoke(action) =
                    renderer
                        .controls
                        .key(&core, &declarations, viewport, key(KeyCode::Enter))
                else {
                    panic!("menu not actionable")
                };
                assert_eq!(serde_json::to_value(action.context).unwrap(), context);
            }
        }
    }
}

#[test]
fn scrolled_menu_rows_reveal_clickable_ellipses_for_resolved_children() {
    let core = Core::default();
    let viewport = Rect::viewport(20, 6);
    let menu_row = |id: &str| {
        json!({"type":"row","context":{"tab_id":id},
        "children":[{"type":"use","name":"row-label"}],
        "menu":{"type":"column","children":[{"type":"row","action":"inspect",
            "context":{"tab_id":id},"children":[{"type":"label","text":"Inspect","role":"label"}]}]}})
    };
    let declarations = vec![declaration(
        "sidebar",
        json!({"herdr":"widget_tree","version":1,
        "zones":[{"zone":"list","tree":{"type":"column","children":[menu_row("a"),menu_row("b"),menu_row("c"),menu_row("d"),menu_row("e"),menu_row("f")]}}],
        "fragments":[{"name":"row-label","priority":0,"tree":{"type":"label","text":"A long menu row label","role":"label"}}],
        "composition":{"priority":0,"tree":{"type":"column","children":[
            {"type":"flex","weight":1,"child":{"type":"zone","zone":"list"}},
            {"type":"label","text":"Footer","role":"label"}]}}}),
    )];
    let mut renderer = FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(20, 6, 0).unwrap();
    for (offset, target) in [(0, "a"), (1, "b")] {
        renderer.controls.list_offset = offset;
        renderer
            .controls
            .hover(&core, &declarations, viewport, 19, 5);
        let mut bytes = Vec::new();
        renderer
            .draw_frame(&mut bytes, None, &declarations, &core, viewport)
            .unwrap();
        screen.write(&bytes);
        let before_menu = rows(&screen)[0].clone();
        renderer
            .controls
            .hover(&core, &declarations, viewport, 18, 0);
        bytes.clear();
        renderer
            .draw_frame(&mut bytes, None, &declarations, &core, viewport)
            .unwrap();
        screen.write(&bytes);
        assert_eq!(
            rows(&screen)[0].chars().nth(18),
            Some('⋯'),
            "scroll offset {offset}"
        );
        assert!(rows(&screen)[5].starts_with("Footer"));
        assert!(matches!(
            renderer
                .controls
                .click(&core, &declarations, viewport, 18, 0, false),
            Response::Consumed
        ));
        let Response::Invoke(action) =
            renderer
                .controls
                .key(&core, &declarations, viewport, key(KeyCode::Enter))
        else {
            panic!("visible menu did not invoke")
        };
        assert_eq!(action.plugin, "sidebar");
        assert_eq!(action.action, "inspect");
        assert_eq!(action.context.tab_id.as_deref(), Some(target));
        renderer
            .controls
            .hover(&core, &declarations, viewport, 19, 5);
        bytes.clear();
        renderer
            .draw_frame(&mut bytes, None, &declarations, &core, viewport)
            .unwrap();
        screen.write(&bytes);
        assert_eq!(rows(&screen)[0], before_menu);
    }
}

#[test]
fn production_sidebar_menu_cells_follow_scrolled_rows_and_keyboard_focus() {
    let home = std::env::temp_dir().join(format!("menu-projection-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let mut command = std::process::Command::new("bun");
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
    command.env("TRAILBOSS_CONFIG_PATH", home.join("config.toml"));
    let output = command.args(["-e", r#"
        import {declareStock} from './plugins/agent-tree/stock.ts';
        import {declareMenus} from './plugins/sidebar-menus/menus.ts';
        const entity={workspaceId:'w',paneId:null,number:1,position:0,focused:false,agentStatus:'working',agent:null,cwd:null};
        const entities=[{...entity,id:'workspace:w',kind:'workspace',tabId:null,label:'Long workspace title with status'},
            ...Array.from({length:8},(_,i)=>({...entity,id:'tab:t'+i,kind:'tab',tabId:'t'+i,position:i,label:'Long tab title '+i}))];
        console.log(JSON.stringify([['stock',declareStock(entities)],['menus',declareMenus(entities)]]));
    "#]).current_dir(env!("CARGO_MANIFEST_DIR")).output().unwrap();
    std::fs::remove_dir_all(home).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let declarations: Declarations = serde_json::from_slice(&output.stdout).unwrap();
    let mut core = Core::default();
    core.has_pane = true;
    let viewport = Rect::viewport(80, 6);
    let mut renderer = FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(80, 6, 0).unwrap();
    let paint = |renderer: &mut FrameRenderer, screen: &mut ghostty_vt::Terminal| {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(&mut bytes, None, &declarations, &core, viewport)
            .unwrap();
        screen.write(&bytes);
    };
    paint(&mut renderer, &mut screen);
    assert!(!rows(&screen).iter().any(|row| row.contains('⋯')));
    for (offset, y, expected, tab) in [
        (0, 2, "host:new-tab", None),
        (3, 0, "host:rename-tab", Some("t0")),
    ] {
        renderer.controls.list_offset = offset;
        renderer
            .controls
            .hover(&core, &declarations, viewport, 18, y);
        paint(&mut renderer, &mut screen);
        assert_eq!(rows(&screen)[usize::from(y)].chars().nth(18), Some('⋯'));
        assert!(matches!(
            renderer
                .controls
                .click(&core, &declarations, viewport, 18, y, false),
            Response::Consumed
        ));
        let Response::Invoke(action) =
            renderer
                .controls
                .key(&core, &declarations, viewport, key(KeyCode::Enter))
        else {
            panic!("painted menu not actionable")
        };
        assert_eq!(action.plugin, "menus");
        assert_eq!(action.action, expected);
        assert_eq!(action.context.workspace_id.as_deref(), Some("w"));
        assert_eq!(action.context.tab_id.as_deref(), tab);
    }
    renderer
        .controls
        .hover(&core, &declarations, viewport, 79, 5);
    renderer
        .controls
        .focus_next(&core, &declarations, viewport, false);
    paint(&mut renderer, &mut screen);
    assert_eq!(rows(&screen)[0].chars().nth(18), Some('⋯'));
    renderer
        .controls
        .key(&core, &declarations, viewport, key(KeyCode::Esc));
    paint(&mut renderer, &mut screen);
    assert!(!rows(&screen).iter().any(|row| row.contains('⋯')));
    renderer
        .controls
        .hover(&core, &declarations, viewport, 18, 5);
    paint(&mut renderer, &mut screen);
    assert_eq!(rows(&screen)[5].chars().nth(18), Some('⋯'));
    assert!(matches!(
        renderer
            .controls
            .click(&core, &declarations, viewport, 18, 5, false),
        Response::Consumed
    ));
    let Response::Invoke(action) =
        renderer
            .controls
            .key(&core, &declarations, viewport, key(KeyCode::Enter))
    else {
        panic!("footer menu not actionable")
    };
    assert_eq!(
        (action.plugin.as_str(), action.action.as_str()),
        ("menus", "host:new-space")
    );
}

#[test]
fn contextless_shared_popup_actions_keep_the_origin_target() {
    let core = Core::default();
    let viewport = Rect::viewport(20, 6);
    let shared = declaration(
        "buttons",
        json!({"herdr":"widget_tree","version":1,"zones":[],"fragments":[{"name":"example.menu","priority":0,"tree":{"type":"column","children":[{"type":"row","action":"custom","children":[{"type":"label","text":"Act","role":"label"}]}]}}]}),
    );
    let declarations = vec![sidebar(), shared];
    let mut renderer = FrameRenderer::default();
    renderer
        .controls
        .click(&core, &declarations, viewport, 18, 0, false);
    let Response::Invoke(action) =
        renderer
            .controls
            .key(&core, &declarations, viewport, key(KeyCode::Enter))
    else {
        panic!("missing popup action")
    };
    assert_eq!(action.plugin, "buttons");
    assert_eq!(action.context.workspace_id.as_deref(), Some("other"));
}
