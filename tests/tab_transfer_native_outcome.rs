#![cfg(target_os = "linux")]

#[path = "support/console.rs"]
mod console;
mod support;

use serde_json::json;
use support::Core;

fn open_move(native: &mut console::Console, destination: &Core) {
    native.text("\x1bOP");
    native.wait(|rows| rows.iter().any(|row| row.contains("Commands")));
    native.text("Move to session");
    native.wait(|rows| rows.iter().any(|row| row.contains("Move to session")));
    native.text("\r");
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains(&destination.name) && row.contains("RetryWork"))
    });
}

#[test]
fn definitive_native_rejection_allows_a_subsequent_move() {
    let source = Core::new();
    let destination = Core::launch_with_executable(
        source.home.clone(),
        format!("{}-retry", source.name),
        support::executable(),
        false,
    );
    let from = source.request(
        "workspace.create",
        json!({"label":"RetrySource","cwd":source.home,"focus":true}),
    );
    let to = destination.request(
        "workspace.create",
        json!({"label":"RetryWork","cwd":source.home}),
    );
    let pane = from["root_pane"]["pane_id"].as_str().unwrap();
    source.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":"retry_marker=original; printf 'RETRY_SOURCE_READY\\n'\n"}),
    );
    let mut native = console::Console::new(&source);
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.trim_end().ends_with("RETRY_SOURCE_READY"))
    });
    open_move(&mut native, &destination);
    // Discovery is real; invalidate its selected workspace before submission.
    destination.request(
        "workspace.close",
        json!({"workspace_id":to["workspace"]["workspace_id"]}),
    );
    native.text("RetryWork");
    native.text("\r");
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("Cannot move tab:") && row.contains("workspace"))
    });
    assert!(source.request("tab.list", json!({}))["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tab| tab["tab_id"] == from["tab"]["tab_id"]));
    let replacement = destination.request(
        "workspace.create",
        json!({"label":"RetryWork","cwd":source.home}),
    );
    open_move(&mut native, &destination);
    native.text("RetryWork");
    native.text("\r");
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("Tab moved to destination session"))
    });
    let tabs = destination.request(
        "tab.list",
        json!({"workspace_id":replacement["workspace"]["workspace_id"]}),
    );
    assert_eq!(tabs["tabs"].as_array().unwrap().len(), 2);
    let moved = destination.request("session.snapshot", json!({}));
    let terminal =
        source.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone();
    let current = moved["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["terminal_id"] == terminal)
        .unwrap()["pane_id"]
        .as_str()
        .unwrap();
    destination.request("pane.focus", json!({"pane_id":current}));
    let mut attached = destination.attach();
    while attached.focused_pane_id.as_deref() != Some(current) {
        attached.receive().unwrap();
    }
    let mut input = attached.input().unwrap();
    input.focus_pane_target(current);
    input
        .type_text("printf '%s\\n' \"$retry_marker\" > retry-retained\n")
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::fs::read_to_string(source.home.join("retry-retained"))
            .ok()
            .as_deref()
            == Some("original\n")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "retained shell did not receive attached client input"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
