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
