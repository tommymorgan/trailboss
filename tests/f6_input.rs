mod support;

#[path = "support/console.rs"]
mod console;

use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn adjacent_f6_typing_and_f6_delivers_typing_once_to_preview_then_selects_previous_tab() {
    check_adjacent_input("\x1b[17~y\x1b[17~", "B", "C");
}

#[test]
fn reverse_and_mixed_direction_bursts_preserve_adjacent_typing() {
    for (keys, typed_tab, final_tab) in [
        ("\x1b[17;2~y\x1b[17~", "A", "C"),
        ("\x1b[17~\x1b[17;2~y\x1b[17;2~", "C", "A"),
    ] {
        check_adjacent_input(keys, typed_tab, final_tab);
    }
}

fn check_adjacent_input(keys: &str, typed_tab: &str, final_tab: &str) {
    let core = support::Core::new();
    let first = core.request(
        "workspace.create",
        json!({"label":"F6 first workspace", "cwd":core.home, "focus":true}),
    );
    let second = core.request(
        "tab.create",
        json!({"workspace_id":first["workspace"]["workspace_id"], "label":"B", "cwd":core.home, "focus":false}),
    );
    let third = core.request(
        "workspace.create",
        json!({"label":"F6 other workspace", "cwd":core.home, "focus":false}),
    );
    let tabs = [&first, &second, &third];
    let mut setup = core.attach();
    for (label, created) in ["A", "B", "C"].into_iter().zip(tabs) {
        core.request(
            "tab.rename",
            json!({"tab_id":created["tab"]["tab_id"], "label":label}),
        );
        // The raw pane witness records bytes, not shell echo or submitted lines.
        // Each witness and its output live entirely inside this Core's owned HOME.
        let script = format!(
            "import os, tty\ntty.setraw(0)\nf = open({:?}, 'ab', buffering=0)\nprint('F6_READY_{}', flush=True)\nwhile True:\n data = os.read(0, 4096)\n if not data: break\n f.write(data)\n",
            format!("input-{label}"), label,
        );
        std::fs::write(core.home.join(format!("witness-{label}.py")), script).unwrap();
        core.request("tab.focus", json!({"tab_id":created["tab"]["tab_id"]}));
        let deadline = Instant::now() + Duration::from_secs(8);
        while setup.focused_tab_id.as_deref() != created["tab"]["tab_id"].as_str()
            || setup.primary.as_ref().map(|pane| pane.pane_id.as_str())
                != created["root_pane"]["pane_id"].as_str()
        {
            setup.receive().unwrap();
            assert!(
                Instant::now() < deadline,
                "setup focus did not reach {label}"
            );
        }
        setup
            .input()
            .unwrap()
            .type_text(&format!("python3 witness-{label}.py\n"))
            .unwrap();
    }
    drop(setup);
    core.request("tab.focus", json!({"tab_id":first["tab"]["tab_id"]}));
    let mut native = console::Console::new(&core);
    // API focus arranges visits only. Screen observations prove that this actual
    // native client ingested A, B, C before the unmodified F6 bytes under test.
    for (label, created) in ["A", "B", "C"].into_iter().zip(tabs) {
        core.request("tab.focus", json!({"tab_id":created["tab"]["tab_id"]}));
        let marker = format!("F6_READY_{label}");
        native.wait(|rows| rows.iter().any(|row| row.trim_end().ends_with(&marker)));
        assert_eq!(
            core.request("session.snapshot", json!({}))["snapshot"]["focused_tab_id"],
            created["tab"]["tab_id"],
        );
    }
    // One outer-PTY write keeps direction changes and typing adjacent while
    // focus replies may still be outstanding.
    native.text(keys);
    let final_id = tabs[["A", "B", "C"]
        .iter()
        .position(|label| *label == final_tab)
        .unwrap()]["tab"]["tab_id"]
        .clone();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let focused =
            core.request("session.snapshot", json!({}))["snapshot"]["focused_tab_id"].clone();
        let bytes = std::fs::read(core.home.join(format!("input-{typed_tab}"))).unwrap_or_default();
        if focused == final_id && bytes == b"y" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "adjacent directional F6 lost or misrouted input: focus={focused}, {typed_tab}={bytes:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let marker = format!("F6_READY_{final_tab}");
    native.wait(|rows| rows.iter().any(|row| row.trim_end().ends_with(&marker)));
    for label in ["A", "B", "C"] {
        let expected = if label == typed_tab {
            b"y".as_slice()
        } else {
            b"".as_slice()
        };
        assert_eq!(
            std::fs::read(core.home.join(format!("input-{label}"))).unwrap(),
            expected,
            "input delivered to the wrong tab: {label}"
        );
    }

    native.text("\x02q");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = native.child.try_wait().unwrap() {
            assert!(status.success(), "normal native detach failed: {status:?}");
            break;
        }
        assert!(Instant::now() < deadline, "normal native detach timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Console and Core guards also tear down only these owned resources on panic.
}
