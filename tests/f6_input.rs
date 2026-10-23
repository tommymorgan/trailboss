mod support;

#[path = "support/console.rs"]
mod console;

use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn adjacent_f6_typing_and_f6_delivers_typing_once_to_preview_then_selects_previous_tab() {
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
    // One outer-PTY write: do not wait for a preview acknowledgement between
    // F6, ordinary typing, and the next F6. Typing must finish B's visit before
    // the new burst starts from B's committed MRU order and selects C.
    native.text("\x1b[17~y\x1b[17~");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let focused =
            core.request("session.snapshot", json!({}))["snapshot"]["focused_tab_id"].clone();
        let bytes = std::fs::read(core.home.join("input-B")).unwrap_or_default();
        if focused == third["tab"]["tab_id"] && bytes == b"y" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "adjacent F6/typing/F6 lost or misrouted input: focus={focused}, B={bytes:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.trim_end().ends_with("F6_READY_C"))
    });
    assert_eq!(std::fs::read(core.home.join("input-A")).unwrap(), b"");
    assert_eq!(std::fs::read(core.home.join("input-B")).unwrap(), b"y");
    assert_eq!(std::fs::read(core.home.join("input-C")).unwrap(), b"");

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
