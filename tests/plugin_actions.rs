#[path = "support/action_producer.rs"]
mod action_producer;
#[path = "support/console.rs"]
mod console;
mod support;
use action_producer::FocusProducer;
use std::time::{Duration, Instant};

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
                .is_some_and(|trees| !trees.is_empty())
        {
            break;
        }
        assert!(Instant::now() < deadline, "no producer declaration");
    }
    let trees = client.declarations.as_ref().unwrap();
    let row = herdr_fork::sidebar::rows(trees).position(|(_,node)|matches!(node,herdr_runtime::protocol::widgets::Node::Row{context,..} if context.tab_id.as_deref()==Some(&fixture.target))).unwrap();
    let (plugin, action, context) = herdr_fork::sidebar::action_at(trees, row).unwrap();
    let mut core = herdr_fork::core::Core::default();
    client
        .input()
        .unwrap()
        .dispatch(&mut core, &client.boot_id, plugin, action, context)
        .unwrap();
    fixture.assert_completed();
}

#[test]
fn living_dispatch_plugin_actions_through_stdin_envelopes() {
    let feature = include_str!("../features/plugin-actions.feature");
    let scenario = "Scenario: Dispatch plugin actions through stdin envelopes";
    let steps: Vec<_> = feature
        .lines()
        .skip_while(|line| line.trim() != scenario)
        .take(4)
        .map(str::trim)
        .collect();
    assert_eq!(steps,[scenario,
        "Given a long-lived producer declares a focus-tab action for an unfocused tab",
        "When I activate that declaration",
        "Then the producer receives the correlated action and tab context on stdin and the requested tab becomes focused after its successful action result"]);
    let fixture = FocusProducer::new();
    let mut console = console::Console::new(&fixture.core);
    let rows = console.wait(|rows| {
        rows.iter()
            .any(|row| row.starts_with("  ○ Unfocused target"))
            && rows.iter().any(|row| row.contains(" · herdr"))
    });
    let y = rows
        .iter()
        .position(|row| row.starts_with("  ○ Unfocused target"))
        .unwrap();
    console.click(5, y);
    fixture.assert_completed();
    console.text("\x11");
}

#[path = "support/reserved_actions.rs"]
mod reserved_actions;
use herdr_runtime::protocol::widgets::{Condition, Context, Node};
use reserved_actions::Adversary;
use serde_json::json;

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
    for action in ["new", "menu"] {
        let fixture = Adversary::new(action);
        let mut client = fixture.core.attach();
        let mut semantic = herdr_fork::core::Core::default();
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
                    Condition::Menu
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
                    semantic.invoke("host:cancel", &Context::default()),
                    herdr_fork::core::Effect::Consumed
                ));
                assert!(!semantic.visible(Condition::Menu));
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
    let examples: Vec<_> = block
        .iter()
        .filter(|line| line.starts_with('|'))
        .copied()
        .collect();
    assert_eq!(
        examples,
        [
            "| action | behavior |",
            "| new | new-tab interaction |",
            "| menu | menu interaction |"
        ]
    );
    for action in ["new", "menu"] {
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
            let marker = if action == "new" {
                " Name: "
            } else {
                " Session menu "
            };
            let rows = console.wait(|rows| {
                rows.iter()
                    .any(|row| row.contains(marker) || row.contains("HIJACKED"))
            });
            assert!(
                rows.iter().any(|row| row.contains(marker)),
                "plugin hijacked Core {action}: {rows:?}"
            );
            assert_eq!(fixture.label(), "Safe");
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
