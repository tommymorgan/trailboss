use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use trailboss::core::{Core, Effect};
use trailboss_runtime::protocol::{widgets::Context, ClientShellSnapshot};

fn snapshot(focus: Option<&str>, live: &[&str]) -> ClientShellSnapshot {
    serde_json::from_value(json!({
        "boot_id":"boot", "revision":1, "config_diagnostic":null,
        "product_announcement":null, "update_available":null, "update_install_command":"",
        "server_keybindings_toml":null, "latest_release_notes_available":false,
        "integration_updates_available":false, "worktree_directory":"", "release_notes":null,
        "focused_workspace_id":"space", "focused_tab_id":focus, "focused_pane_id":null,
        "tab_bar_right":[], "tab_bar_right_separator":"", "agent_view_label":null,
        "agent_order":[], "workspaces":[], "tabs":live.iter().enumerate().map(|(i,id)| json!({
            "tab_id":id, "workspace_id":format!("space-{i}"), "number":i+1,
            "label":id, "custom_label":false, "zoomed":false, "focused":Some(*id)==focus,
            "agent_status":"idle"
        })).collect::<Vec<_>>(), "panes":[], "agents":[], "commands":[]
    }))
    .unwrap()
}
fn visit(core: &mut Core, tab: &str) {
    core.update_session(Some(format!("space-{tab}")), Some(tab.into()), true);
}
fn visited() -> Core {
    let mut core = Core::default();
    for tab in ["A", "B", "C"] {
        visit(&mut core, tab);
    }
    core
}
fn focus_request(effect: Effect, expected: &str) {
    let Effect::Request { method, params } = effect else {
        panic!("missing focus request")
    };
    assert_eq!(method, "tab.focus");
    assert_eq!(params, json!({"tab_id":expected}));
}

#[test]
fn confirmed_previews_cycle_c_to_b_to_a_across_spaces_without_toggling() {
    let mut core = visited();
    assert_eq!(
        core.cycle_tab(&snapshot(Some("C"), &["A", "B", "C"])),
        Some("B")
    );
    visit(&mut core, "B");
    assert_eq!(core.tab_cycle_hint(), Some("MRU: L next · Enter/Esc done"));
    assert_eq!(
        core.cycle_tab(&snapshot(Some("B"), &["A", "B", "C"])),
        Some("A")
    );
    visit(&mut core, "A");
    assert_eq!(
        core.cycle_tab(&snapshot(Some("A"), &["A", "B", "C"])),
        Some("C")
    );
}

#[test]
fn cycle_skips_candidates_closed_after_start_and_wraps_to_live_origin() {
    let mut core = visited();
    assert_eq!(
        core.cycle_tab(&snapshot(Some("C"), &["A", "B", "C"])),
        Some("B")
    );
    visit(&mut core, "B");
    assert_eq!(core.cycle_tab(&snapshot(Some("B"), &["B", "C"])), Some("C"));
    visit(&mut core, "C");
    assert_eq!(core.cycle_tab(&snapshot(Some("C"), &["B", "C"])), Some("B"));
    assert_eq!(core.cycle_tab(&snapshot(None, &[])), None);
    assert_eq!(core.tab_cycle_hint(), None);
}

#[test]
fn finishing_commits_final_selection_not_intermediate_previews_to_fresh_mru_order() {
    let mut core = visited();
    let mut s = snapshot(Some("C"), &["A", "B", "C"]);
    assert_eq!(core.cycle_tab(&s), Some("B"));
    visit(&mut core, "B");
    assert_eq!(core.cycle_tab(&s), Some("A"));
    visit(&mut core, "A");
    s.focused_tab_id = Some("A".into());
    assert!(core.finish_tab_cycle());
    assert_eq!(core.tab_cycle_hint(), None);
    assert!(!core.finish_tab_cycle());
    assert_eq!(core.cycle_tab(&s), Some("C"));
    visit(&mut core, "C");
    assert_eq!(core.cycle_tab(&s), Some("B"));
}

#[test]
fn delayed_preview_confirmations_after_finish_do_not_reorder_final_history() {
    let mut core = visited();
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    assert_eq!(core.cycle_tab(&s), Some("B"));
    assert_eq!(core.cycle_tab(&s), Some("A"));
    assert!(core.finish_tab_cycle());
    visit(&mut core, "B");
    visit(&mut core, "A");
    assert_eq!(
        core.previous_tab(&snapshot(Some("A"), &["A", "B", "C"])),
        Some("C")
    );
}

#[test]
fn bare_l_and_another_prefix_l_both_advance_the_same_frozen_cycle() {
    let mut core = visited();
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    assert_eq!(core.cycle_tab(&s), Some("B"));
    visit(&mut core, "B");
    focus_request(
        core.tab_cycle_key(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            false,
            Some(&s),
        )
        .unwrap(),
        "A",
    );
    visit(&mut core, "A");
    assert!(core
        .tab_cycle_key(
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            true,
            Some(&s)
        )
        .is_none());
    assert!(core.tab_cycle_hint().is_some());
    focus_request(
        core.tab_cycle_key(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            false,
            Some(&s),
        )
        .unwrap(),
        "C",
    );
}

#[test]
fn enter_and_escape_finish_and_consume_but_other_keys_finish_and_pass_through() {
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    for code in [
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Char('x'),
        KeyCode::Tab,
        KeyCode::Char('p'),
        KeyCode::Char('q'),
    ] {
        let mut core = visited();
        assert_eq!(core.cycle_tab(&s), Some("B"));
        let effect = core.tab_cycle_key(KeyEvent::new(code, KeyModifiers::NONE), false, Some(&s));
        if matches!(code, KeyCode::Enter | KeyCode::Esc) {
            assert!(matches!(effect, Some(Effect::Consumed)));
        } else {
            assert!(effect.is_none());
        }
        assert_eq!(core.tab_cycle_hint(), None);
    }
}

#[test]
fn no_history_single_live_tab_and_boot_reset_do_not_enter_cycle_mode() {
    let mut core = Core::default();
    assert_eq!(core.cycle_tab(&snapshot(None, &[])), None);
    visit(&mut core, "A");
    assert_eq!(core.cycle_tab(&snapshot(Some("A"), &["A"])), None);
    visit(&mut core, "B");
    assert_eq!(core.cycle_tab(&snapshot(Some("B"), &["B"])), None);
    visit(&mut core, "C");
    assert_eq!(
        core.cycle_tab(&snapshot(Some("C"), &["A", "B", "C"])),
        Some("B")
    );
    core.clear_tab_history();
    assert_eq!(core.tab_cycle_hint(), None);
    assert_eq!(
        core.previous_tab(&snapshot(Some("C"), &["A", "B", "C"])),
        None
    );
    assert_eq!(core.cycle_tab(&snapshot(Some("C"), &["A", "B", "C"])), None);
    visit(&mut core, "D");
    assert_eq!(core.cycle_tab(&snapshot(Some("D"), &["C", "D"])), None);
}

#[test]
fn untargeted_focus_is_one_shot_previous_live_visit_and_targeted_focus_is_unchanged() {
    let mut core = visited();
    let s = snapshot(Some("C"), &["A", "C"]);
    focus_request(
        core.invoke("host:focus", &Context::default(), Some(&s)),
        "A",
    );
    assert_eq!(core.tab_cycle_hint(), None);
    focus_request(
        core.invoke("host:focus", &Context::default(), Some(&s)),
        "A",
    );
    let target = Context {
        tab_id: Some("B".into()),
        ..Context::default()
    };
    focus_request(core.invoke("host:focus", &target, Some(&s)), "B");
    assert!(matches!(
        core.invoke("host:focus", &Context::default(), None),
        Effect::Consumed
    ));
    assert!(matches!(
        core.invoke(
            "host:focus",
            &Context::default(),
            Some(&snapshot(Some("C"), &["C"]))
        ),
        Effect::Consumed
    ));
    assert!(matches!(
        core.invoke("plugin:custom", &Context::default(), Some(&s)),
        Effect::Plugin
    ));
}
