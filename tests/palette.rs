use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};
use trailboss::{
    core::Core,
    sidebar::Declarations,
    terminal::{
        menus::{Controls, Response},
        FrameRenderer, Rect,
    },
};
fn fragment(
    id: &str,
    name: &str,
    priority: i32,
    tree: Value,
) -> (String, trailboss_runtime::protocol::widgets::Declaration) {
    (id.into(), serde_json::from_value(json!({"herdr":"widget_tree","version":1,"zones":[],"fragments":[{"name":name,"priority":priority,"tree":tree}]})).unwrap())
}
fn row(label: &str, action: Option<&str>, target: &str, menu: Option<Value>) -> Value {
    json!({"type":"row","children":[{"type":"label","text":label,"role":"label"}],"action":action,"context":{"workspace_id":target},"menu":menu})
}
fn column(children: Vec<Value>) -> Value {
    json!({"type":"column","children":children})
}
fn key(
    controls: &mut Controls,
    core: &Core,
    declarations: &Declarations,
    code: KeyCode,
) -> Response {
    controls.key(
        core,
        declarations,
        Rect::viewport(100, 30),
        KeyEvent::new(code, KeyModifiers::NONE),
    )
}
fn invoked(response: Response) -> trailboss::terminal::menus::Action {
    match response {
        Response::Invoke(action) => action,
        _ => panic!("expected declared action"),
    }
}
#[test]
fn nested_public_fragments_execute_the_winning_provider_not_the_catalog() {
    let core = Core::default();
    let mut controls = Controls::default();
    let declarations = vec![
        fragment(
            "catalog",
            "command-palette",
            0,
            column(vec![row(
                "Expand",
                None,
                "",
                Some(json!({"type":"use","name":"targets"})),
            )]),
        ),
        fragment(
            "lower",
            "targets",
            0,
            column(vec![row("Low", Some("low"), "a", None)]),
        ),
        fragment(
            "winner",
            "targets",
            10,
            column(vec![row("High", Some("expand"), "b", None)]),
        ),
    ];
    assert!(controls.open_palette(&core, &declarations, None));
    assert!(matches!(
        key(&mut controls, &core, &declarations, KeyCode::Enter),
        Response::Consumed
    ));
    assert!(controls.modal());
    let action = invoked(key(&mut controls, &core, &declarations, KeyCode::Enter));
    assert_eq!(action.plugin, "winner");
    assert_eq!(action.action, "expand");
    assert_eq!(action.context.workspace_id.as_deref(), Some("b"));
    assert!(!controls.modal());
}
#[test]
fn subsequence_search_is_case_insensitive_and_prefers_prefix_matches() {
    let core = Core::default();
    let mut controls = Controls::default();
    let declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![
            row("New space", Some("space"), "a", None),
            row("Next step", Some("step"), "b", None),
            row("NS exact", Some("exact"), "c", None),
        ]),
    )];
    controls.open_palette(&core, &declarations, None);
    assert!(controls.paste("NS"));
    assert_eq!(
        invoked(key(&mut controls, &core, &declarations, KeyCode::Enter)).action,
        "exact"
    );
    controls.open_palette(&core, &declarations, None);
    controls.paste("xyz");
    assert!(matches!(
        key(&mut controls, &core, &declarations, KeyCode::Enter),
        Response::Consumed
    ));
    assert!(controls.modal());
}
#[test]
fn removed_target_cannot_execute_a_stale_name_submit() {
    let mut core = Core::default();
    let mut controls = Controls::default();
    let target = row(
        "Rename",
        Some("host:rename-space"),
        "a",
        Some(column(vec![
            json!({"type":"input"}),
            row("Save", Some("host:create"), "a", None),
        ])),
    );
    let mut declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![target]),
    )];
    controls.open_palette(&core, &declarations, None);
    let Response::Prepare(prep) = key(&mut controls, &core, &declarations, KeyCode::Enter) else {
        panic!("expected name preparation");
    };
    core.prepare_palette(&prep.action, &prep.context);
    core.paste("new name");
    declarations[0] = fragment("catalog", "command-palette", 0, column(vec![]));
    let cancel = invoked(key(&mut controls, &core, &declarations, KeyCode::Enter));
    assert_eq!(cancel.action, "host:cancel");
    core.invoke(&cancel.action, &cancel.context, None);
    assert!(!core.editing());
    assert!(controls.modal());
}
#[test]
fn names_pass_through_core_blank_submit_stays_open_and_escape_cancels_only_one_level() {
    let mut core = Core::default();
    let mut controls = Controls::default();
    let declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![row(
            "Rename",
            Some("host:rename-space"),
            "a",
            Some(column(vec![
                json!({"type":"input"}),
                row("Save", Some("host:create"), "a", None),
                row("Cancel", Some("host:cancel"), "a", None),
            ])),
        )]),
    )];
    controls.open_palette(&core, &declarations, None);
    let Response::Prepare(prep) = key(&mut controls, &core, &declarations, KeyCode::Enter) else {
        panic!("expected name preparation");
    };
    core.prepare_palette(&prep.action, &prep.context);
    assert!(controls.modal());
    assert!(!controls.paste("name"));
    assert!(matches!(
        key(&mut controls, &core, &declarations, KeyCode::Char('x')),
        Response::Pass
    ));
    assert!(matches!(
        key(&mut controls, &core, &declarations, KeyCode::Enter),
        Response::Consumed
    ));
    assert!(core.editing());
    assert!(controls.modal());
    let cancel = invoked(key(&mut controls, &core, &declarations, KeyCode::Esc));
    assert_eq!(cancel.action, "host:cancel");
    core.invoke(&cancel.action, &cancel.context, None);
    assert!(!core.editing());
    assert!(controls.modal());
    assert!(matches!(
        key(&mut controls, &core, &declarations, KeyCode::Esc),
        Response::Consumed
    ));
    assert!(!controls.modal());
}
#[test]
fn live_reordering_preserves_selected_stable_id_and_removal_never_executes_the_replacement() {
    let core = Core::default();
    let mut controls = Controls::default();
    let mut declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![
            row("Duplicate", Some("focus"), "a", None),
            row("Duplicate", Some("focus"), "b", None),
        ]),
    )];
    controls.open_palette(&core, &declarations, None);
    key(&mut controls, &core, &declarations, KeyCode::Down);
    controls.reconcile(&core, &declarations, Rect::viewport(100, 30));
    declarations[0] = fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![
            row("Duplicate", Some("focus"), "b", None),
            row("Duplicate", Some("focus"), "a", None),
        ]),
    );
    assert_eq!(
        invoked(key(&mut controls, &core, &declarations, KeyCode::Enter))
            .context
            .workspace_id
            .as_deref(),
        Some("b")
    );
    controls.open_palette(&core, &declarations, None);
    controls.reconcile(&core, &declarations, Rect::viewport(100, 30));
    declarations[0] = fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![row("Duplicate", Some("focus"), "a", None)]),
    );
    assert!(matches!(
        key(&mut controls, &core, &declarations, KeyCode::Enter),
        Response::Consumed
    ));
    assert!(controls.modal());
}
#[test]
fn outside_click_is_consumed_and_zero_narrow_unicode_frames_are_safe() {
    let core = Core::default();
    let declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![row("漢字 λ", Some("act"), "a", None)]),
    )];
    let mut renderer = FrameRenderer::default();
    renderer.controls.open_palette(&core, &declarations, None);
    for (width, height) in [(0, 0), (1, 1), (2, 2), (100, 30)] {
        renderer
            .draw_frame(
                &mut Vec::new(),
                None,
                &declarations,
                &core,
                Rect::viewport(width, height),
            )
            .unwrap();
    }
    assert!(matches!(
        renderer
            .controls
            .click(&core, &declarations, Rect::viewport(100, 30), 0, 29, false),
        Response::Consumed
    ));
    assert!(!renderer.controls.modal());
}
#[test]
fn page_home_end_wheel_and_hover_select_visible_rows_without_leaking_input() {
    let core = Core::default();
    let mut controls = Controls::default();
    let declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(
            (0..40)
                .map(|i| row(&format!("Entry {i}"), Some("pick"), &i.to_string(), None))
                .collect(),
        ),
    )];
    controls.open_palette(&core, &declarations, None);
    key(&mut controls, &core, &declarations, KeyCode::PageDown);
    assert_eq!(
        invoked(key(&mut controls, &core, &declarations, KeyCode::Enter))
            .context
            .workspace_id
            .as_deref(),
        Some("11")
    );
    controls.open_palette(&core, &declarations, None);
    key(&mut controls, &core, &declarations, KeyCode::End);
    key(&mut controls, &core, &declarations, KeyCode::PageUp);
    key(&mut controls, &core, &declarations, KeyCode::Home);
    assert!(matches!(
        controls.wheel(&core, &declarations, Rect::viewport(100, 30), false),
        Response::Consumed
    ));
    assert!(controls.hover(&core, &declarations, Rect::viewport(100, 30), 13, 7));
    assert_eq!(
        invoked(controls.click(&core, &declarations, Rect::viewport(100, 30), 13, 7, false))
            .context
            .workspace_id
            .as_deref(),
        Some("2")
    );
}
#[test]
fn input_field_shows_focus_current_text_and_the_selected_submit_action() {
    let mut core = Core::default();
    let declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![row(
            "Edit value",
            Some("host:new-space"),
            "",
            Some(column(vec![
                json!({"type":"input"}),
                row("Confirm", Some("host:create"), "", None),
                row("Dismiss", Some("host:cancel"), "", None),
            ])),
        )]),
    )];
    let mut renderer = FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(100, 30, 0).unwrap();
    let mut render = |renderer: &mut FrameRenderer, core: &Core| {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(
                &mut bytes,
                None,
                &declarations,
                core,
                Rect::viewport(100, 30),
            )
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
    };
    renderer.controls.open_palette(&core, &declarations, None);
    let picker = render(&mut renderer, &core);
    assert!(picker[3].trim().is_empty());
    let Response::Prepare(prep) = key(&mut renderer.controls, &core, &declarations, KeyCode::Enter)
    else {
        panic!("expected input preparation");
    };
    core.prepare_palette(&prep.action, &prep.context);
    let empty = render(&mut renderer, &core);
    let field = empty[3].trim();
    assert!(field.starts_with('[') && field.ends_with(']'));
    assert!(field.contains('▏'));
    assert!(
        field.chars().any(char::is_alphabetic),
        "empty input needs a typing prompt"
    );
    assert!(empty[16].contains("Confirm"));

    core.paste("Value 🦀");
    let populated = render(&mut renderer, &core);
    assert!(populated[3].contains("Value 🦀▏"));
    key(&mut renderer.controls, &core, &declarations, KeyCode::Down);
    let changed_action = render(&mut renderer, &core);
    assert!(changed_action[3].contains("Value 🦀▏"));
    assert!(changed_action[16].contains("Dismiss"));
    assert!(!changed_action[16].contains("Confirm"));

    core.input_text.clear();
    core.paste(&format!("{}終", "x".repeat(79)));
    let overflow = render(&mut renderer, &core);
    assert!(
        overflow[3].contains("終▏"),
        "long input must keep its insertion point visible"
    );
    assert!(overflow[3].trim().ends_with(']'));
}

#[test]
fn dismissing_palette_restores_retained_chrome_and_pane_cells() {
    let core = Core::default();
    let mut declarations = vec![fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![row("Pick", Some("pick"), "a", None)]),
    )];
    declarations[0].1.composition = Some(serde_json::from_value(json!({"priority":0,"tree":{"type":"column","children":[{"type":"label","text":"Chrome retained","role":"label"},{"type":"flex","weight":1,"child":{"type":"surface"}}]}})).unwrap());
    let surface = trailboss::Surface {
        pane_id: "p".into(),
        width: 40,
        height: 19,
        cells: (0..760)
            .map(|_| trailboss::Cell {
                symbol: "A".into(),
                foreground: 0,
                background: 0,
                modifiers: 0,
                hyperlink: None,
            })
            .collect(),
        hyperlinks: Vec::new(),
        graphics: Default::default(),
        graphics_scope: String::new(),
        cursor: None,
        focused: true,
        mouse_reporting: false,
    };
    let mut renderer = FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(40, 20, 0).unwrap();
    let paint = |renderer: &mut FrameRenderer, screen: &mut ghostty_vt::Terminal| {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(
                &mut bytes,
                Some(&surface),
                &declarations,
                &core,
                Rect::viewport(40, 20),
            )
            .unwrap();
        screen.write(&bytes);
    };
    paint(&mut renderer, &mut screen);
    let baseline = screen.screen_text_rows().unwrap();
    renderer.controls.open_palette(&core, &declarations, None);
    paint(&mut renderer, &mut screen);
    key(&mut renderer.controls, &core, &declarations, KeyCode::Esc);
    paint(&mut renderer, &mut screen);
    let after = screen.screen_text_rows().unwrap();
    for (before, after) in baseline.iter().zip(&after) {
        for (a, b) in before.cells.iter().zip(&after.cells) {
            assert_eq!(a.graphemes, b.graphemes);
        }
    }
}

#[test]
fn switch_tabs_search_keeps_matching_space_headings_and_skips_them_on_keyboard_and_click() {
    let core = Core::default();
    let group = |space: &str, tab: &str, name: &str| {
        column(vec![
            json!({"type":"label","text":space,"role":"heading"}),
            json!({"type":"row","children":[{"type":"label","text":name,"role":"label"}],"action":"host:focus","context":{"workspace_id":space,"tab_id":tab}}),
        ])
    };
    let declarations = vec![fragment(
        "tabs-provider",
        "command-palette",
        0,
        column(vec![row(
            "Switch tab",
            None,
            "",
            Some(column(vec![
                group("Alpha", "a", "Duplicate"),
                group("Beta", "b", "Duplicate"),
                group("Hidden", "c", "Unrelated"),
                column(vec![
                    json!({"type":"label","text":"Empty","role":"heading"}),
                ]),
            ])),
        )]),
    )];
    let mut renderer = FrameRenderer::default();
    renderer.controls.open_palette(&core, &declarations, None);
    key(&mut renderer.controls, &core, &declarations, KeyCode::Enter);
    let mut screen = ghostty_vt::Terminal::new(100, 30, 0).unwrap();
    let mut render = |renderer: &mut FrameRenderer| {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(
                &mut bytes,
                None,
                &declarations,
                &core,
                Rect::viewport(100, 30),
            )
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
    let text = render(&mut renderer);
    assert!(text.contains("Alpha") && text.contains("Beta") && text.contains("Empty"));
    assert!(renderer.controls.paste("duplicate"));
    let text = render(&mut renderer);
    assert!(text.contains("Alpha") && text.contains("Beta") && text.contains("Duplicate"));
    assert!(!text.contains("Hidden") && !text.contains("Empty"));
    // Headings occupy display rows, but never activate or receive hover selection.
    assert!(!renderer
        .controls
        .hover(&core, &declarations, Rect::viewport(100, 30), 13, 5));
    assert!(matches!(
        renderer
            .controls
            .click(&core, &declarations, Rect::viewport(100, 30), 13, 5, false),
        Response::Consumed
    ));
    key(&mut renderer.controls, &core, &declarations, KeyCode::Down);
    let action = invoked(key(
        &mut renderer.controls,
        &core,
        &declarations,
        KeyCode::Enter,
    ));
    assert_eq!(action.plugin, "tabs-provider");
    assert_eq!(action.action, "host:focus");
    assert_eq!(action.context.workspace_id.as_deref(), Some("Beta"));
    assert_eq!(action.context.tab_id.as_deref(), Some("b"));
    assert!(!renderer.controls.modal());
}

#[test]
fn empty_space_headings_and_unmatched_queries_never_activate_or_panic_on_navigation() {
    let core = Core::default();
    let declarations = vec![fragment(
        "tabs",
        "command-palette",
        0,
        column(vec![column(vec![
            json!({"type":"label","text":"Empty space","role":"heading"}),
        ])]),
    )];
    let mut controls = Controls::default();
    controls.open_palette(&core, &declarations, None);
    for query in ["", "missing"] {
        if !query.is_empty() {
            controls.paste(query);
        }
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Enter,
        ] {
            assert!(matches!(
                key(&mut controls, &core, &declarations, code),
                Response::Consumed
            ));
            assert!(controls.modal());
        }
    }
}

#[test]
fn shared_confirmation_inherits_chosen_identity_but_explicit_child_scope_wins() {
    for explicit in [false, true] {
        let core = Core::default();
        let mut controls = Controls::default();
        let child = json!({"type":"row","action":"host:close-tab","context":if explicit {json!({"workspace_id":"override","tab_id":"override-tab"})} else {json!({})},"children":[{"type":"label","text":"Close","role":"label"}]});
        let parent = json!({"type":"row","context":{"workspace_id":"unfocused","tab_id":"chosen-tab"},"children":[{"type":"label","text":"Chosen","role":"label"}],"menu":{"type":"use","name":"confirmation"}});
        let declarations = vec![
            fragment("catalog", "command-palette", 0, column(vec![parent])),
            fragment("buttons", "confirmation", 0, column(vec![child])),
        ];
        controls.open_palette(&core, &declarations, None);
        assert!(matches!(
            key(&mut controls, &core, &declarations, KeyCode::Enter),
            Response::Consumed
        ));
        let result = invoked(key(&mut controls, &core, &declarations, KeyCode::Enter));
        assert_eq!(result.plugin, "buttons");
        assert_eq!(
            result.context.workspace_id.as_deref(),
            Some(if explicit { "override" } else { "unfocused" })
        );
        assert_eq!(
            result.context.tab_id.as_deref(),
            Some(if explicit {
                "override-tab"
            } else {
                "chosen-tab"
            })
        );
    }
}

#[test]
fn ordinary_use_alias_refresh_retains_existing_anonymous_menu_matching() {
    let core = Core::default();
    let ordinary = json!({"type":"row",
    "children":[{"type":"label","text":"Open ordinary","role":"label"}],
    "menu":{"type":"column","children":[
        {"type":"row","children":[{"type":"label","text":"Cancel","role":"label"}],"action":"host:cancel"},
        {"type":"row","children":[{"type":"label","text":"Accept","role":"label"}],"action":"ordinary"}
    ]}});
    let mut declarations = vec![
        fragment(
            "catalog",
            "command-palette",
            0,
            column(vec![json!({"type":"use","name":"ordinary.before"})]),
        ),
        fragment("ordinary-owner", "ordinary.before", 0, ordinary.clone()),
        fragment("ordinary-owner", "ordinary.after", 0, ordinary),
    ];
    let mut controls = Controls::default();
    assert!(controls.open_palette(&core, &declarations, None));
    assert!(matches!(
        key(&mut controls, &core, &declarations, KeyCode::Enter),
        Response::Consumed
    ));
    declarations[0] = fragment(
        "catalog",
        "command-palette",
        0,
        column(vec![json!({"type":"use","name":"ordinary.after"})]),
    );
    controls.reconcile(&core, &declarations, Rect::viewport(100, 30));
    key(&mut controls, &core, &declarations, KeyCode::Down);
    let selected = invoked(key(&mut controls, &core, &declarations, KeyCode::Enter));
    assert_eq!(selected.plugin, "ordinary-owner");
    assert_eq!(selected.action, "ordinary");
}
