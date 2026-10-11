#[path = "support/console.rs"]
mod console;
mod support;
use console::Console;
use serde_json::json;
use std::time::{Duration, Instant};
use support::Core;

fn ready(core: &Core) -> Console {
    let mut observer = core.attach();
    while observer.primary.is_none() || observer.focused_pane_id.is_none() {
        observer.receive().unwrap();
    }
    assert_eq!(
        observer.primary.as_ref().unwrap().pane_id,
        observer.focused_pane_id.as_ref().unwrap().as_str()
    );
    drop(observer);
    let mut console = Console::new(core);
    console.wait(|rows| {
        rows.iter()
            .any(|r| r.chars().skip(20).collect::<String>().contains('$'))
    });
    console
}
fn wheel(console: &mut Console, button: u8, x: usize, y: usize) {
    console.text(&format!("\x1b[<{button};{};{}M", x + 1, y + 1));
}
fn pane(rows: &[String]) -> Vec<String> {
    rows.iter().map(|r| r.chars().skip(20).collect()).collect()
}
fn numbered_top(rows: &[String]) -> usize {
    pane(rows)
        .iter()
        .find_map(|r| {
            r.trim()
                .strip_prefix("scroll-line-")
                .and_then(|n| n.parse().ok())
        })
        .unwrap()
}
#[test]
fn executable_wheel_scrolls_primary_shell_up_and_down_using_configured_lines() {
    for lines in [3, 7] {
        let core = Core::new();
        if lines != 3 {
            std::fs::write(
                core.home.join("config.toml"),
                "[ui]\nmouse_scroll_lines = 7\n",
            )
            .unwrap();
        }
        let mut console = ready(&core);
        console
            .text("i=1; while [ $i -le 80 ]; do printf 'scroll-line-%s\\n' $i; i=$((i+1)); done\r");
        let bottom = console.wait(|rows| pane(rows).iter().any(|r| r.trim() == "scroll-line-80"));
        let top = numbered_top(&bottom);
        wheel(&mut console, 64, 35, 8);
        console.wait(|rows| numbered_top(rows) == top - lines);
        wheel(&mut console, 65, 35, 8);
        console.wait(|rows| numbered_top(rows) == top);
    }
}

#[test]
fn executable_modified_keys_edit_real_readline_words_and_navigation() {
    let core = Core::new();
    std::fs::write(
        core.home.join("editor.sh"),
        r#"#!/bin/bash
for i in 1 2 3 4; do
  read -er -p "edit-$i> " value
  printf '\nRESULT-%s:<%s>\n' "$i" "$value"
done
"#,
    )
    .unwrap();
    let mut console = ready(&core);
    console.text("bash --noprofile --norc editor.sh\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("edit-1>")));
    console.text("one two three\x1b[1;5DX\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("RESULT-1:<one two Xthree>")));
    console.wait(|rows| rows.iter().any(|r| r.contains("edit-2>")));
    console.text("one two three\x1b[H\x1b[1;5CX\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("RESULT-2:<oneX two three>")));
    console.wait(|rows| rows.iter().any(|r| r.contains("edit-3>")));
    console.text("one two three\x1b\x7fX\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("RESULT-3:<one two X>")));
    console.wait(|rows| rows.iter().any(|r| r.contains("edit-4>")));
    console.text("abcde\x1b[H\x1b[3~X\x1b[F\x7fY\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("RESULT-4:<XbcdY>")));
}

fn log(core: &Core) -> Vec<String> {
    std::fs::read_to_string(core.home.join("input.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}
fn wait_log(core: &Core, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let events = log(core);
        if events.len() >= count {
            return events;
        }
        assert!(Instant::now() < deadline, "child input missing: {events:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn executable_app_wheel_preserves_relative_coordinates_modifiers_and_capture_boundaries() {
    let core = Core::new();
    std::fs::write(
        core.home.join("mouse.py"),
        r#"import os, sys, tty, re
f = open('input.log', 'w', buffering=1)
tty.setraw(0)
sys.stdout.write('\x1b[?1000h\x1b[?1006h\x1b[2J\x1b[HMOUSE-READY\r\n'); sys.stdout.flush()
buffer = b''
while True:
    buffer += os.read(0, 4096)
    while buffer:
        if buffer.startswith(b'\x1b[<'):
            match = re.match(rb'\x1b\[<\d+;\d+;\d+[Mm]', buffer)
            if not match: break
            event = match.group(); buffer = buffer[len(event):]
        else:
            event = buffer[:1]; buffer = buffer[1:]
        f.write(event.hex() + '\n')
"#,
    )
    .unwrap();
    let mut console = ready(&core);
    console.text("python3 mouse.py\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("MOUSE-READY")));
    wheel(&mut console, 64, 35, 8);
    wheel(&mut console, 65 + 16, 35, 8);
    assert_eq!(
        wait_log(&core, 2),
        ["1b5b3c36343b31363b394d", "1b5b3c38313b31363b394d"]
    );
    // Sidebar coordinates are not child coordinates; no input may leak.
    wheel(&mut console, 64, 10, 8);
    console.text("b");
    assert_eq!(wait_log(&core, 3).last().unwrap(), "62");
    // A resize capture consumes wheel until release, and changes subsequent geometry.
    console.text("\x1b[<0;21;9M\x1b[<32;31;9M");
    console.wait(|rows| {
        rows.iter().any(|r| {
            r.chars()
                .skip(30)
                .collect::<String>()
                .starts_with("MOUSE-READY")
        })
    });
    wheel(&mut console, 64, 35, 8);
    console.text("\x1b[<0;31;9m");
    wheel(&mut console, 64, 35, 8);
    assert_eq!(wait_log(&core, 4).last().unwrap(), "1b5b3c36343b363b394d");
    // Reset split for native menus. Context-menu capture must block wheel outside itself.
    console.text("\x1b[<0;31;9M\x1b[<32;21;9M\x1b[<0;21;9m");
    console.wait(|rows| {
        rows.iter().any(|r| {
            r.chars()
                .skip(20)
                .collect::<String>()
                .starts_with("MOUSE-READY")
        })
    });
    console.text("\x1b[<2;5;3M");
    console.wait(|rows| rows.iter().any(|r| r.contains("New tab")));
    wheel(&mut console, 64, 35, 8);
    console.text("\x1b[27u\x1b[27u");
    console.text("c");
    assert_eq!(wait_log(&core, 5).last().unwrap(), "63");
    assert_eq!(log(&core).len(), 5);
    // Prefix-focused menu navigation captures wheel without opening a popup.
    console.text("\x02\t");
    wheel(&mut console, 64, 35, 8);
    console.text("\x1b[27u");
    console.text("d");
    assert_eq!(wait_log(&core, 6).last().unwrap(), "64");
    assert_eq!(log(&core).len(), 6);
    // Named modal text editing stays local; cancel restores child input.
    console.text("\x1b[<2;5;3M");
    let rows = console.wait(|rows| rows.iter().any(|r| r.contains("Rename “")));
    let y = rows.iter().position(|r| r.contains("Rename “")).unwrap();
    let x = rows[y].chars().position(|c| c == 'R').unwrap();
    console.click(x, y);
    console.wait(|rows| {
        rows.iter()
            .any(|r| r.contains("Rename space") && r.contains('│'))
    });
    wheel(&mut console, 64, 35, 8);
    console.text("local\x7f\x1b[27u");
    console.text("e");
    assert_eq!(wait_log(&core, 7).last().unwrap(), "65");
    assert_eq!(log(&core).len(), 7);
    // An arbitrary plugin modal captures even wheel outside its visible dialog.
    let plugin = core.home.join("arbitrary-modal");
    std::fs::create_dir_all(&plugin).unwrap();
    let declaration = json!({"herdr":"widget_tree","version":1,"zones":[],
    "composition":{"priority":100,"tree":{"type":"column","children":[
        {"type":"surface"},
        {"type":"modal","child":{"type":"column","children":[
            {"type":"label","text":"ARBITRARY-CAPTURE","role":"label"},
            {"type":"input"}
        ]}}
    ]}}});
    std::fs::write(
        plugin.join("producer.ts"),
        format!("console.log(JSON.stringify({declaration})); await Bun.stdin.text();"),
    )
    .unwrap();
    std::fs::write(plugin.join("trailboss-plugin.toml"), "id = \"fixture.input-modal\"\nname = \"Arbitrary input capture\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[ui]]\nid = \"tree\"\nsurface = \"sidebar\"\ncommand = [\"bun\", \"producer.ts\"]\nstream = true\n").unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    console.wait(|rows| rows.iter().any(|r| r.contains("ARBITRARY-CAPTURE")));
    wheel(&mut console, 64, 35, 8);
    console.text("must-not-leak");
    // Let the executable consume the negative events before replacing the declaration.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(log(&core).len(), 7);
    core.request("plugin.disable", json!({"plugin_id":"fixture.input-modal"}));
    console.wait(|rows| rows.iter().all(|r| !r.contains("ARBITRARY-CAPTURE")));
    console.text("f");
    assert_eq!(wait_log(&core, 8).last().unwrap(), "66");
    assert_eq!(log(&core).len(), 8);
    // No-pane state accepts wheel safely without sending to the old target.
    let snapshot = core.request("session.snapshot", json!({}));
    for workspace in snapshot["snapshot"]["workspaces"].as_array().unwrap() {
        core.request(
            "workspace.close",
            json!({"workspace_id":workspace["workspace_id"]}),
        );
    }
    console.wait(|rows| rows.iter().any(|r| r.contains("No pane yet")));
    wheel(&mut console, 64, 35, 8);
    console.text("\x02q");
}

#[test]
fn executable_wheel_uses_child_alternate_scroll_without_mouse_reporting() {
    let core = Core::new();
    std::fs::write(
        core.home.join("alternate.py"),
        r#"import os, sys, tty
tty.setraw(0)
f = open('input.log', 'w', buffering=1)
sys.stdout.write('\x1b[?1049h\x1b[?1007h\x1b[?1hALTERNATE-READY\r\n'); sys.stdout.flush()
while True:
    for byte in os.read(0, 4096): f.write(bytes([byte]).hex() + '\n')
"#,
    )
    .unwrap();
    let mut console = ready(&core);
    console.text("python3 alternate.py\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("ALTERNATE-READY")));
    wheel(&mut console, 64, 35, 8);
    wheel(&mut console, 65, 35, 8);
    console.text("q");
    assert_eq!(wait_log(&core, 7).join(""), "1b4f411b4f4271");
}
