use serde_json::{json, Value};
use trailboss::{
    core::Core,
    sidebar::Declarations,
    terminal::{self, split::SidebarResize, Rect},
};

fn declaration(
    id: &str,
    priority: i32,
    tree: Value,
) -> (String, trailboss_runtime::protocol::widgets::Declaration) {
    (
        id.into(),
        serde_json::from_value(json!({
            "herdr":"widget_tree", "version":1, "zones":[],
            "composition":{"priority":priority,"tree":tree}
        }))
        .unwrap(),
    )
}

fn split_tree(left: u16, right: u16) -> Value {
    json!({"type":"row","children":[
        {"type":"flex","weight":left,"child":{
            "type":"column","children":[
                {"type":"row","action":"select-tab","context":{"tab_id":"tab-a"},"children":[
                    {"type":"label","text":"Tab","role":"tab"}
                ]}
            ]
        }},
        {"type":"flex","weight":right,"child":{"type":"surface"}}
    ]})
}

fn declarations() -> Declarations {
    vec![declaration("arbitrary-provider", 1, split_tree(1, 3))]
}

fn sidebar_width(core: &Core, declarations: &Declarations, viewport: Rect) -> u16 {
    terminal::surface_rect(core, declarations, viewport)
        .unwrap()
        .x
        - viewport.x
}

#[test]
fn only_first_pane_column_captures_without_changing_default_layout_or_sidebar_actions() {
    let core = Core::default();
    let mut declarations = declarations();
    let original = declarations.clone();
    let viewport = Rect {
        x: 5,
        y: 7,
        width: 100,
        height: 20,
    };
    let mut resize = SidebarResize::default();
    assert!(!resize.apply(&core, &mut declarations, viewport));
    assert!(!resize.begin(&core, &declarations, viewport, 29, 7));
    assert!(!resize.begin(&core, &declarations, viewport, 31, 7));
    assert!(!resize.begin(&core, &declarations, viewport, 30, 6));
    assert!(!resize.begin(&core, &declarations, viewport, 30, 27));
    assert!(!resize.drag_to(50, viewport));
    assert!(!resize.finish());
    assert!(resize.begin(&core, &declarations, viewport, 30, 26));
    assert_eq!(declarations, original);
    assert!(resize.drag_to(45, viewport));
    assert!(resize.apply(&core, &mut declarations, viewport));
    assert_eq!(
        terminal::surface_rect(&core, &declarations, viewport),
        Some(Rect {
            x: 45,
            y: 7,
            width: 60,
            height: 20,
        })
    );
    let (provider, action, context) =
        terminal::action_at(&core, &declarations, viewport, 0, 44, 7).unwrap();
    assert_eq!((provider, action), ("arbitrary-provider", "select-tab"));
    assert_eq!(context.tab_id.as_deref(), Some("tab-a"));
    assert!(terminal::action_at(&core, &declarations, viewport, 0, 45, 7).is_none());
    assert!(resize.drag_to(45, viewport));
    assert!(!resize.apply(&core, &mut declarations, viewport));
    assert!(resize.finish());
    assert!(!resize.finish());
}

#[test]
fn drag_clamps_both_sides_and_tiny_viewports_never_override_or_start_capture() {
    let core = Core::default();
    let mut declarations = declarations();
    let viewport = Rect::viewport(100, 20);
    let mut resize = SidebarResize::default();
    assert!(resize.begin(&core, &declarations, viewport, 25, 0));
    assert!(resize.drag_to(0, viewport));
    assert!(resize.apply(&core, &mut declarations, viewport));
    assert_eq!(sidebar_width(&core, &declarations, viewport), 12);
    assert!(resize.drag_to(u16::MAX, viewport));
    assert!(resize.apply(&core, &mut declarations, viewport));
    assert_eq!(sidebar_width(&core, &declarations, viewport), 88);
    for width in 2..24 {
        let narrow = Rect::viewport(width, 20);
        resize.apply(&core, &mut declarations, narrow);
        assert_eq!(
            sidebar_width(&core, &declarations, narrow),
            width - width / 2
        );
    }
    for width in 0..2 {
        let tiny = Rect::viewport(width, 20);
        let original = declarations.clone();
        assert!(!resize.apply(&core, &mut declarations, tiny));
        assert!(resize.drag_to(0, tiny));
        assert_eq!(declarations, original);
    }
    assert!(resize.finish());
    for width in 0..2 {
        let tiny = Rect::viewport(width, 20);
        assert!(!resize.begin(&core, &declarations, tiny, 0, 0));
        assert!(!resize.drag_to(0, tiny));
    }
}

#[test]
fn desired_cell_width_survives_declaration_refresh_and_viewport_shrink_then_expand() {
    let core = Core::default();
    let mut declarations = declarations();
    let viewport = Rect::viewport(100, 20);
    let mut resize = SidebarResize::default();
    assert!(resize.begin(&core, &declarations, viewport, 25, 0));
    assert!(resize.drag_to(60, viewport));
    resize.finish();
    resize.apply(&core, &mut declarations, viewport);
    declarations = vec![declaration("new-provider", 10, split_tree(7, 9))];
    assert!(resize.apply(&core, &mut declarations, viewport));
    assert_eq!(sidebar_width(&core, &declarations, viewport), 60);
    for width in [30, 20, 2, 1, 0, 100, 200] {
        let resized = Rect::viewport(width, 20);
        resize.apply(&core, &mut declarations, resized);
        if width >= 2 {
            let minimum = 12.min(width / 2);
            assert_eq!(
                sidebar_width(&core, &declarations, resized),
                60.clamp(minimum, width - minimum)
            );
        }
    }
    assert_eq!(
        sidebar_width(&core, &declarations, Rect::viewport(200, 20)),
        60
    );
}

#[test]
fn winning_nonmatching_compositions_remain_untouched_and_precedence_belongs_to_core() {
    let core = Core::default();
    let mut declarations = declarations();
    let viewport = Rect::viewport(100, 20);
    let mut resize = SidebarResize::default();
    assert!(resize.begin(&core, &declarations, viewport, 25, 0));
    resize.drag_to(40, viewport);
    resize.finish();
    declarations.push(declaration("higher-priority", 2, json!({"type":"surface"})));
    let original = declarations.clone();
    assert!(!resize.apply(&core, &mut declarations, viewport));
    assert!(!resize.begin(&core, &declarations, viewport, 25, 0));
    assert_eq!(declarations, original);
    assert_eq!(
        terminal::surface_rect(&core, &declarations, viewport),
        Some(viewport)
    );
    declarations[1] = declaration("z-provider", 1, split_tree(3, 1));
    assert!(resize.apply(&core, &mut declarations, viewport));
    assert_eq!(declarations[0], original[0]);
    assert_eq!(sidebar_width(&core, &declarations, viewport), 40);
    declarations[1] = declaration(
        "z-provider",
        1,
        json!({"type":"row","children":[
            {"type":"surface"}, {"type":"surface"}
        ]}),
    );
    let original = declarations.clone();
    assert!(!resize.apply(&core, &mut declarations, viewport));
    assert_eq!(declarations, original);
}

#[test]
fn modal_excludes_capture_and_large_flex_weights_keep_exact_boundary() {
    let core = Core::default();
    let viewport = Rect::viewport(100, 20);
    let mut resize = SidebarResize::default();
    let mut tree = split_tree(u16::MAX, u16::MAX);
    let declarations = vec![declaration("large-weights", 1, tree.clone())];
    assert!(!resize.begin(&core, &declarations, viewport, 49, 0));
    assert!(resize.begin(&core, &declarations, viewport, 50, 0));
    resize.finish();
    tree["children"][0]["child"] = json!({"type":"modal","child":{"type":"input"}});
    let declarations = vec![declaration("modal-provider", 1, tree)];
    assert!(!resize.begin(&core, &declarations, viewport, 50, 0));
    assert!(!resize.drag_to(60, viewport));
}

#[test]
fn resize_pointer_tracks_moved_geometry_and_stays_active_until_release() {
    let core = Core::default();
    let viewport = Rect::viewport(80, 24);
    let mut declarations = declarations();
    let mut resize = SidebarResize::default();
    assert!(!resize.resize_cursor(&core, &declarations, viewport, None));
    assert!(resize.resize_cursor(&core, &declarations, viewport, Some((20, 12))));
    assert!(!resize.resize_cursor(&core, &declarations, viewport, Some((19, 12))));
    assert!(resize.begin(&core, &declarations, viewport, 20, 12));
    assert!(resize.resize_cursor(&core, &declarations, viewport, Some((100, 12))));
    resize.drag_to(30, viewport);
    resize.apply(&core, &mut declarations, viewport);
    resize.finish();
    assert!(!resize.resize_cursor(&core, &declarations, viewport, Some((100, 12))));
    assert!(!resize.resize_cursor(&core, &declarations, viewport, Some((20, 12))));
    assert!(resize.resize_cursor(&core, &declarations, viewport, Some((30, 12))));
    declarations[0].1.composition.as_mut().unwrap().tree =
        serde_json::from_value(json!({"type":"modal","child":{"type":"input"}})).unwrap();
    assert!(!resize.resize_cursor(&core, &declarations, viewport, Some((30, 12))));
}
