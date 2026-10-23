use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::time::{Duration, Instant};
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

fn acknowledge(core: &mut Core, s: &ClientShellSnapshot) -> Option<Effect> {
    let id = core
        .tab_cycle_request_id()
        .expect("an actual focus request");
    let request_id = format!("tab-cycle-{id}");
    let response = serde_json::to_vec(&json!({
        "id": request_id,
        "result": {"tab": {"tab_id": s.focused_tab_id}},
    }))
    .unwrap();
    core.tab_cycle_response(&request_id, true, &response, Some(s))
}

fn confirm(core: &mut Core, tab: &str) {
    visit(core, tab);
    assert!(acknowledge(core, &snapshot(Some(tab), &["A", "B", "C", "D"])).is_none());
}

#[test]
fn confirmed_previews_cycle_c_to_b_to_a_across_spaces_without_toggling() {
    let mut core = visited();
    assert_eq!(
        core.cycle_tab(&snapshot(Some("C"), &["A", "B", "C"])),
        Some("B")
    );
    visit(&mut core, "B");
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
    focus_request(core.request_tab_cycle(&s), "B");
    assert!(matches!(core.request_tab_cycle(&s), Effect::Consumed));
    assert!(core.finish_tab_cycle());
    visit(&mut core, "B");
    focus_request(acknowledge(&mut core, &s).unwrap(), "A");
    confirm(&mut core, "A");
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
            Instant::now(),
        )
        .unwrap(),
        "A",
    );
    confirm(&mut core, "A");
    assert!(core
        .tab_cycle_key(
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            true,
            Some(&s),
            Instant::now(),
        )
        .is_none());
    assert!(core.tab_cycle_hint().is_some());
    focus_request(
        core.tab_cycle_key(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            false,
            Some(&s),
            Instant::now(),
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
        let effect = core.tab_cycle_key(
            KeyEvent::new(code, KeyModifiers::NONE),
            false,
            Some(&s),
            Instant::now(),
        );
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

#[test]
fn direct_f6_previews_the_frozen_mru_order_without_a_prefix() {
    let mut core = visited();
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    for tab in ["B", "A", "C"] {
        focus_request(
            core.tab_cycle_key(
                KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE),
                false,
                Some(&s),
                Instant::now(),
            )
            .expect("F6 must directly navigate without entering a prefix mode"),
            tab,
        );
        confirm(&mut core, tab);
    }
}

#[test]
fn last_f6_resets_idle_commit_and_the_next_burst_uses_only_the_final_visit() {
    let mut core = visited();
    let mut s = snapshot(Some("C"), &["A", "B", "C"]);
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    focus_request(
        core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
        "B",
    );
    confirm(&mut core, "B");
    s.focused_tab_id = Some("B".into());
    focus_request(
        core.tab_cycle_key(key, false, Some(&s), start + Duration::from_millis(600))
            .unwrap(),
        "A",
    );
    confirm(&mut core, "A");
    s.focused_tab_id = Some("A".into());
    assert!(!core.tick_tab_cycle(start + Duration::from_millis(800)));
    assert!(!core.tick_tab_cycle(start + Duration::from_millis(1399)));
    assert!(core.tick_tab_cycle(start + Duration::from_millis(1400)));
    assert_eq!(core.tab_cycle_hint(), None);
    assert!(!core.tick_tab_cycle(start + Duration::from_secs(2)));
    for tab in ["C", "B"] {
        focus_request(
            core.tab_cycle_key(key, false, Some(&s), start + Duration::from_secs(2))
                .unwrap(),
            tab,
        );
        confirm(&mut core, tab);
        s.focused_tab_id = Some(tab.into());
    }
}

#[test]
fn f6_never_captures_normal_input_or_modified_function_keys() {
    let start = Instant::now();
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    for key in [
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::F(6), KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::F(6), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::F(6), KeyModifiers::ALT),
    ] {
        let mut core = visited();
        focus_request(
            core.tab_cycle_key(
                KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE),
                false,
                Some(&s),
                start,
            )
            .unwrap(),
            "B",
        );
        confirm(&mut core, "B");
        assert!(core
            .tab_cycle_key(key, key.modifiers == KeyModifiers::CONTROL, Some(&s), start)
            .is_none());
        assert_eq!(core.tab_cycle_hint(), None);
        visit(&mut core, "A");
        assert!(!core.tick_tab_cycle(start + Duration::from_secs(2)));
        assert_eq!(
            core.previous_tab(&snapshot(Some("A"), &["A", "B", "C"])),
            Some("B")
        );
    }
}

#[test]
fn deleted_preview_and_later_deliberate_focus_cancel_the_old_timer() {
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    for focus in [None, Some("A"), Some("D")] {
        let mut core = visited();
        let s = snapshot(Some("C"), &["A", "B", "C", "D"]);
        focus_request(
            core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
            "B",
        );
        visit(&mut core, "B");
        if let Some(tab) = focus {
            visit(&mut core, tab);
        } else {
            core.reconcile_tab_history(&snapshot(Some("C"), &["A", "C"]));
            visit(&mut core, "C");
        }
        assert_eq!(core.tab_cycle_hint(), None);
        assert!(!core.tick_tab_cycle(start + Duration::from_secs(2)));
        let current = focus.unwrap_or("C");
        let live = if focus.is_some() {
            &["A", "B", "C", "D"][..]
        } else {
            &["A", "C"][..]
        };
        let next_snapshot = snapshot(Some(current), live);
        assert!(matches!(
            core.tab_cycle_key(
                key,
                false,
                Some(&next_snapshot),
                start + Duration::from_secs(2),
            ),
            Some(Effect::Consumed)
        ));
        // The canceled command still holds the endpoint lease until its reply.
        focus_request(
            acknowledge(&mut core, &next_snapshot).unwrap(),
            if focus.is_some() { "C" } else { "A" },
        );
    }
}

#[test]
fn close_before_confirmation_and_core_reset_discard_pending_selection() {
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    for reset in [false, true] {
        let mut core = visited();
        focus_request(
            core.tab_cycle_key(
                key,
                false,
                Some(&snapshot(Some("C"), &["A", "B", "C"])),
                start,
            )
            .unwrap(),
            "B",
        );
        if reset {
            core.clear_tab_history();
        } else {
            core.reconcile_tab_history(&snapshot(Some("C"), &["A", "C"]));
        }
        assert!(!core.tick_tab_cycle(start + Duration::from_secs(2)));
        visit(&mut core, "A");
        assert_eq!(
            core.previous_tab(&snapshot(Some("A"), &["A", "C"])),
            if reset { None } else { Some("C") }
        );
    }
}

#[test]
fn coalesced_wrapped_previews_cannot_hide_a_later_deliberate_visit() {
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    for (steps, final_tab) in [(3, "C"), (4, "B")] {
        let mut core = visited();
        let s = snapshot(Some("C"), &["A", "B", "C"]);
        for tab in ["B", "A", "C", "B"].into_iter().take(steps) {
            focus_request(
                core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
                tab,
            );
            assert!(acknowledge(&mut core, &s).is_none());
        }
        // Native snapshots can coalesce a full wrap into unchanged focus,
        // which does not produce an update_session notification.
        if final_tab != "C" {
            visit(&mut core, final_tab);
        }
        assert!(core.tick_tab_cycle(start + Duration::from_millis(800)));
        visit(&mut core, "A");
        focus_request(
            core.tab_cycle_key(
                key,
                false,
                Some(&snapshot(Some("A"), &["A", "B", "C"])),
                start + Duration::from_secs(2),
            )
            .unwrap(),
            final_tab,
        );
    }
}

#[test]
fn delayed_wrap_confirmations_after_timeout_do_not_become_deliberate_visits() {
    let mut core = visited();
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    focus_request(
        core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
        "B",
    );
    for _ in 0..2 {
        assert!(matches!(
            core.tab_cycle_key(key, false, Some(&s), start),
            Some(Effect::Consumed)
        ));
    }
    assert!(core.tick_tab_cycle(start + Duration::from_millis(800)));
    visit(&mut core, "B");
    focus_request(acknowledge(&mut core, &s).unwrap(), "C");
    confirm(&mut core, "C");
    focus_request(
        core.tab_cycle_key(key, false, Some(&s), start + Duration::from_secs(2))
            .unwrap(),
        "B",
    );
}

#[test]
fn superseded_responses_cannot_dispatch_old_unsent_navigation() {
    let s = snapshot(Some("C"), &["A", "B", "C", "D"]);
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    for cancel in ["key", "pointer", "boot", "close"] {
        let mut core = visited();
        focus_request(
            core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
            "B",
        );
        let old_request = format!("tab-cycle-{}", core.tab_cycle_request_id().unwrap());
        assert!(matches!(
            core.tab_cycle_key(key, false, Some(&s), start),
            Some(Effect::Consumed)
        ));
        assert!(core.tick_tab_cycle(start + Duration::from_secs(1)));
        match cancel {
            "key" => assert!(core
                .tab_cycle_key(
                    KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                    false,
                    Some(&s),
                    start + Duration::from_secs(1),
                )
                .is_none()),
            "pointer" => {
                core.interrupt_tab_cycle();
            }
            "boot" => core.clear_tab_history(),
            "close" => core.reconcile_tab_history(&snapshot(Some("C"), &["B", "C", "D"])),
            _ => unreachable!(),
        }
        visit(&mut core, "D");
        assert!(core
            .tab_cycle_response(&old_request, true, br#"{"result":{}}"#, Some(&s))
            .is_none());
        let effect = core.tab_cycle_key(key, false, Some(&s), start + Duration::from_secs(2));
        if cancel == "boot" {
            assert!(matches!(effect, Some(Effect::Consumed)));
        } else {
            focus_request(effect.unwrap(), "C");
        }
    }
}

#[test]
fn response_identity_and_final_chunk_control_queued_navigation() {
    let mut core = visited();
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    let start = Instant::now();
    focus_request(
        core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
        "B",
    );
    let request = format!("tab-cycle-{}", core.tab_cycle_request_id().unwrap());
    assert!(matches!(
        core.tab_cycle_key(key, false, Some(&s), start),
        Some(Effect::Consumed)
    ));
    assert!(core
        .tab_cycle_response("tab-cycle-999", true, br#"{"result":{}}"#, Some(&s))
        .is_none());
    assert!(core
        .tab_cycle_response(&request, false, br#"{"res"#, Some(&s))
        .is_none());
    visit(&mut core, "B");
    focus_request(
        core.tab_cycle_response(&request, true, br#"ult":{}}"#, Some(&s))
            .unwrap(),
        "A",
    );
    confirm(&mut core, "A");
    assert!(core.tick_tab_cycle(start + Duration::from_secs(1)));
    focus_request(
        core.tab_cycle_key(
            key,
            false,
            Some(&snapshot(Some("A"), &["A", "B", "C"])),
            start + Duration::from_secs(2),
        )
        .unwrap(),
        "C",
    );
}

#[test]
fn rejected_final_focus_keeps_the_displayed_tab_as_the_current_visit() {
    let mut core = visited();
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    let start = Instant::now();
    focus_request(
        core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
        "B",
    );
    assert!(matches!(
        core.tab_cycle_key(key, false, Some(&s), start),
        Some(Effect::Consumed)
    ));
    visit(&mut core, "B");
    focus_request(acknowledge(&mut core, &s).unwrap(), "A");
    let request = format!("tab-cycle-{}", core.tab_cycle_request_id().unwrap());
    assert!(core
        .tab_cycle_response(
            &request,
            true,
            br#"{"error":{"code":"not_found"}}"#,
            Some(&s)
        )
        .is_none());
    assert!(!core.tick_tab_cycle(start + Duration::from_secs(1)));
    focus_request(
        core.tab_cycle_key(
            key,
            false,
            Some(&snapshot(Some("B"), &["A", "B", "C"])),
            start + Duration::from_secs(2),
        )
        .unwrap(),
        "C",
    );
}

#[test]
fn normal_interaction_before_response_retains_the_displayed_preview() {
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    for keyboard in [true, false] {
        let mut core = visited();
        focus_request(
            core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
            "B",
        );
        let request = format!("tab-cycle-{}", core.tab_cycle_request_id().unwrap());
        visit(&mut core, "B");
        if keyboard {
            assert!(core
                .tab_cycle_key(
                    KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                    false,
                    Some(&s),
                    start,
                )
                .is_none());
        } else {
            core.interrupt_tab_cycle();
        }
        assert!(core
            .tab_cycle_response(&request, true, br#"{"result":{}}"#, Some(&s))
            .is_none());
        assert!(!core.tick_tab_cycle(start + Duration::from_secs(1)));
        focus_request(
            core.tab_cycle_key(
                key,
                false,
                Some(&snapshot(Some("B"), &["A", "B", "C"])),
                start + Duration::from_secs(2),
            )
            .unwrap(),
            "C",
        );
    }
}

#[test]
fn canceled_navigation_keeps_the_endpoint_lease_until_its_final_response() {
    let s = snapshot(Some("C"), &["A", "B", "C"]);
    let start = Instant::now();
    let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    for response in [
        br#"{"result":{}}"#.as_slice(),
        br#"{"error":{"code":"not_found"}}"#.as_slice(),
    ] {
        let mut core = visited();
        focus_request(
            core.tab_cycle_key(key, false, Some(&s), start).unwrap(),
            "B",
        );
        let old = format!("tab-cycle-{}", core.tab_cycle_request_id().unwrap());
        core.interrupt_tab_cycle();
        assert!(
            matches!(
                core.tab_cycle_key(key, false, Some(&s), start + Duration::from_millis(100)),
                Some(Effect::Consumed)
            ),
            "a new burst must not send while the old endpoint command is outstanding"
        );
        focus_request(
            core.tab_cycle_response(&old, true, response, Some(&s))
                .unwrap(),
            "B",
        );
        confirm(&mut core, "B");
        assert!(core.tick_tab_cycle(start + Duration::from_secs(1)));
        focus_request(
            core.tab_cycle_key(
                key,
                false,
                Some(&snapshot(Some("B"), &["A", "B", "C"])),
                start + Duration::from_secs(2),
            )
            .unwrap(),
            "C",
        );
    }
}
