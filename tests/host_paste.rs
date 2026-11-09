mod support;

#[path = "support/console.rs"]
mod console;

use serde_json::json;
use std::time::{Duration, Instant};

const PASTE: &str = "line one\nline two\n";

#[test]
fn host_paste_reaches_a_bracketed_paste_pane_as_one_bracketed_block() {
    check_host_paste(true, b"\x1b[200~line one\nline two\n\x1b[201~");
}

#[test]
fn host_paste_reaches_a_plain_pane_as_plain_text() {
    check_host_paste(false, PASTE.as_bytes());
}

fn check_host_paste(bracketed: bool, expected: &[u8]) {
    let core = support::Core::new();
    core.request(
        "workspace.create",
        json!({"label":"Paste workspace", "cwd":core.home, "focus":true}),
    );
    // The raw pane witness records the exact bytes the pane application reads,
    // after optionally enabling bracketed paste (DECSET 2004) like an editor would.
    let enable = if bracketed { "\\x1b[?2004h" } else { "" };
    let script = format!(
        "import os, sys, tty\ntty.setraw(0)\nf = open('pasted', 'ab', buffering=0)\nsys.stdout.write('{enable}PASTE_READY\\n')\nsys.stdout.flush()\nwhile True:\n data = os.read(0, 4096)\n if not data: break\n f.write(data)\n",
    );
    std::fs::write(core.home.join("witness.py"), script).unwrap();
    let mut setup = core.attach();
    let deadline = Instant::now() + Duration::from_secs(8);
    while setup.primary.is_none() {
        setup.receive().unwrap();
        assert!(Instant::now() < deadline, "setup pane did not appear");
    }
    setup
        .input()
        .unwrap()
        .type_text("python3 witness.py\n")
        .unwrap();
    while setup.primary.as_ref().is_none_or(|surface| {
        !surface
            .text()
            .lines()
            .any(|line| line.trim_end().ends_with("PASTE_READY"))
    }) {
        setup.receive().unwrap();
        assert!(Instant::now() < deadline, "raw paste witness did not start");
    }
    drop(setup);

    let mut native = console::Console::new(&core);
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.trim_end().ends_with("PASTE_READY"))
    });
    // The host terminal delivers one bracketed paste because the native client
    // enabled bracketed paste on its outer terminal.
    native.text(&format!("\x1b[200~{PASTE}\x1b[201~"));

    let path = core.home.join("pasted");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let bytes = std::fs::read(&path).unwrap_or_default();
        if bytes.len() >= expected.len() {
            assert_eq!(
                String::from_utf8_lossy(&bytes),
                String::from_utf8_lossy(expected),
                "pane received the host paste in the wrong form"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pane did not receive the host paste: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        std::thread::sleep(Duration::from_millis(10));
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
}
