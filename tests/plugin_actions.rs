#[path = "support/action_producer.rs"]
mod action_producer;
#[path = "support/client_contracts.rs"]
mod client_contracts;
#[path = "support/console.rs"]
mod console;
mod support;
use action_producer::FocusProducer;
use std::time::{Duration, Instant};

fn assert_composed_action(
    fixture: &FocusProducer,
    y: usize,
    expected_owner: &str,
    expected_action: &str,
) {
    let mut client = fixture.core.attach();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.receive().unwrap();
        let mut core = trailboss::core::Core::default();
        core.update_session(
            client.workspace_id.clone(),
            client.focused_tab_id.clone(),
            client.focused_pane_id.is_some(),
        );
        if let Some((owner, action, context)) = client.declarations.as_ref().and_then(|trees| {
            trailboss::terminal::action_at(
                &core,
                trees,
                trailboss::terminal::Rect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height: 24,
                },
                0,
                5,
                y as u16,
            )
        }) {
            assert_eq!(owner, expected_owner);
            assert_eq!(action, expected_action);
            assert_eq!(
                context.workspace_id.as_deref(),
                Some(fixture.workspace.as_str())
            );
            assert_eq!(context.tab_id.as_deref(), Some(fixture.target.as_str()));
            assert_eq!(
                fixture.action_ready(client.declarations.as_ref().unwrap()),
                expected_owner == "fixture.observed-focus"
            );
            return;
        }
        assert!(Instant::now() < deadline, "composed target action missing");
    }
}

#[test]
fn producer_stdin_action_correlates_unfocused_tab_and_successful_result() {
    let fixture = FocusProducer::new();
    let mut client = fixture.core.attach();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        client.receive().unwrap();
        if !client.boot_id.is_empty()
            && client
                .declarations
                .as_ref()
                .is_some_and(|trees| trailboss::sidebar::rows(trees).any(|(_, node)| matches!(node, trailboss_runtime::protocol::widgets::Node::Row { context, .. } if context.tab_id.as_deref() == Some(&fixture.target))))
        {
            break;
        }
        assert!(Instant::now() < deadline, "no producer declaration");
    }
    let trees = client.declarations.as_ref().unwrap();
    let row = trailboss::sidebar::rows(trees).position(|(_,node)|matches!(node,trailboss_runtime::protocol::widgets::Node::Row{context,..} if context.tab_id.as_deref()==Some(&fixture.target))).unwrap();
    let (plugin, action, context) = trailboss::sidebar::action_at(trees, row).unwrap();
    let mut core = trailboss::core::Core::default();
    client
        .input()
        .unwrap()
        .dispatch(&mut core, &client.boot_id, plugin, action, context)
        .unwrap();
    fixture.assert_completed();
}

#[test]
fn living_dispatch_plugin_actions_through_stdin_envelopes() {
    let fixture = FocusProducer::new();
    let mut console = console::Console::new(&fixture.core);
    let rows = console.wait(|rows| {
        rows.iter().any(|row| {
            row.chars()
                .take(20)
                .collect::<String>()
                .contains("Unfocused")
        })
    });
    let y = rows
        .iter()
        .position(|row| {
            row.chars()
                .take(20)
                .collect::<String>()
                .contains("Unfocused")
        })
        .unwrap();
    assert_composed_action(&fixture, y, "fixture.observed-focus", "focus-tab");
    console.click(5, y);
    fixture.assert_completed();
    console.text("\x02q");
}

#[test]
fn generated_command_fragments_do_not_make_action_producer_ready() {
    let fixture = FocusProducer::start_gated();
    let mut console = console::Console::new(&fixture.core);
    let rows = console.wait(|rows| {
        rows.iter().any(|row| {
            row.chars()
                .take(20)
                .collect::<String>()
                .contains("Unfocused")
        })
    });
    let y = rows
        .iter()
        .position(|row| {
            row.chars()
                .take(20)
                .collect::<String>()
                .contains("Unfocused")
        })
        .unwrap();
    // Before the real list zone is published, stock UI has the same label but
    // routes its click to Core, not this producer. Owner metadata is not readiness.
    assert_composed_action(&fixture, y, "tommymorgan.stock-ui", "host:focus");
    console.click(5, y);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.focused() != fixture.target {
        assert!(Instant::now() < deadline, "fallback did not focus target");
        std::thread::yield_now();
    }
    let envelopes = std::fs::read_to_string(fixture.plugin.join("envelopes")).unwrap();
    assert!(!envelopes.contains("\"kind\":\"request\""), "{envelopes}");
    fixture.release_publication();
    fixture.wait_ready();
    assert_composed_action(&fixture, y, "fixture.observed-focus", "focus-tab");
}

#[path = "support/reserved_actions.rs"]
mod reserved_actions;
use reserved_actions::Adversary;
use serde_json::json;
use trailboss_runtime::protocol::widgets::{Condition, Context, Node};

fn wait_created(fixture: &Adversary, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = fixture.core.request("session.snapshot", json!({}));
        if snapshot["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["label"] == label && tab["focused"] == true)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Core new-tab interaction did not create {label}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn reserved_core_actions_ignore_installed_plugin_behavior_before_and_after_replacement() {
    for action in ["new", "rename-space"] {
        let fixture = Adversary::new(action);
        let mut client = fixture.core.attach();
        let mut semantic = trailboss::core::Core::default();
        let mut input = client.input().unwrap();
        for stage in ["before", "after"] {
            if stage == "after" {
                fixture.replace();
            }
            let tree = fixture.declaration(&mut client, stage);
            fixture.assert_placement(&tree, stage);
            semantic.update_session(
                client.workspace_id.clone(),
                client.focused_tab_id.clone(),
                true,
            );
            fixture.prove_handler(stage);
            let Node::Row {
                action: Some(id),
                context,
                ..
            } = &tree.zones[0].tree
            else {
                panic!("missing declared action")
            };
            input
                .dispatch(
                    &mut semantic,
                    &client.boot_id,
                    "fixture.adversary",
                    id,
                    context,
                )
                .unwrap();
            assert!(
                semantic.visible(if action == "new" {
                    Condition::NewTab
                } else {
                    Condition::RenameSpace
                }),
                "plugin-defined behavior replaced Core {action}"
            );
            assert_eq!(fixture.label(), "Safe");
            if action == "new" {
                let label = format!("Core-{stage}");
                input
                    .dispatch_text(&mut semantic, &client.boot_id, &label)
                    .unwrap();
                input
                    .dispatch_text(&mut semantic, &client.boot_id, "\r")
                    .unwrap();
                wait_created(&fixture, &label);
            } else {
                assert!(matches!(
                    semantic.invoke("host:cancel", &Context::default(), None),
                    trailboss::core::Effect::Consumed
                ));
                assert!(!semantic.visible(Condition::RenameSpace));
            }
        }
        fixture.assert_unchanged();
    }
}
#[test]
fn living_plugins_place_core_defined_actions() {
    let feature = include_str!("../features/plugin-actions.feature");
    let scenario = "Scenario Outline: Plugins place core-defined actions";
    let block: Vec<_> = feature
        .lines()
        .skip_while(|line| line.trim() != scenario)
        .map(str::trim)
        .collect();
    assert_eq!(&block[..4],[scenario,
        "Given a plugin declares its own label and placement for the client-local <action> in the reserved core-action namespace",
        "When I activate the action, change its plugin label and placement, and activate it again while the plugin attempts to supply its own behavior",
        "Then only the core-defined <behavior> occurs, both before and after the change, and the plugin cannot initially define or later redefine those semantics"]);
    for action in ["new", "rename-space"] {
        let fixture = Adversary::new(action);
        let mut observer = fixture.core.attach();
        let mut console = console::Console::new(&fixture.core);
        for stage in ["before", "after"] {
            if stage == "after" {
                fixture.replace();
            }
            let tree = fixture.declaration(&mut observer, stage);
            fixture.assert_placement(&tree, stage);
            fixture.prove_handler(stage);
            let label = format!("{stage} {action}");
            let rows = console.wait(|rows| {
                rows.iter().any(|row| row.contains(&label))
                    && !rows.iter().any(|row| row.contains("HIJACKED"))
            });
            let y = rows.iter().position(|row| row.contains(&label)).unwrap();
            let x = rows[y].find(&label).unwrap();
            if stage == "after" {
                assert_eq!(y, 23);
                assert!(x > 0, "footer-right did not right-align the replacement");
            } else {
                assert!(y < 22);
                assert_eq!(x, 0);
            }
            console.click(x, y);
            let marker = "Probe";
            console.text(marker);
            let rows = console.wait(|rows| {
                rows.iter()
                    .any(|row| row.contains(marker) || row.contains("HIJACKED"))
            });
            assert!(
                rows.iter().any(|row| row.contains(marker)),
                "plugin hijacked Core {action}: {rows:?}"
            );
            assert_eq!(fixture.label(), "Safe");
            for _ in marker.chars() {
                console.text("\x7f");
            }
            if action == "new" {
                let name = format!("Core-{stage}");
                console.text(&format!("{name}\r"));
                wait_created(&fixture, &name);
            } else {
                console.text("\x1b");
            }
            console.wait(|rows| !rows.iter().any(|row| row.contains(marker)));
        }
        fixture.assert_unchanged();
    }
}
