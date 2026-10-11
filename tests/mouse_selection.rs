#[path = "support/console.rs"]
mod console;
mod support;
use console::Console;
use std::{
    io::Write,
    time::{Duration, Instant},
};
use support::{Core, HOMES};

fn client(core: &Core) -> Console {
    let mut command = portable_pty::CommandBuilder::new(support::executable());
    command.args(["--session", &core.name]);
    for key in HOMES {
        command.env(key, &core.home);
    }
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
        command.env_remove(key);
    }
    command.env("TRAILBOSS_CONFIG_PATH", core.home.join("config.toml"));
    command.env("TERM", "xterm-256color");
    command.env("SHELL", "/bin/sh");
    // Observe OSC52 without touching any human desktop clipboard.
    command.env("SSH_TTY", "/dev/selection-proof");
    command.env_remove("NO_COLOR");
    Console::from_command(command)
}

fn mouse(console: &mut Console, button: u8, x: usize, y: usize, end: char) {
    console
        .writer
        .write_all(format!("\x1b[<{button};{};{}{end}", x + 1, y + 1).as_bytes())
        .unwrap();
    console.writer.flush().unwrap();
}
fn inverse(console: &mut Console, x: u16, y: u16) -> bool {
    let mut state = ghostty_vt::RenderState::new().unwrap();
    state.update(&console.screen).unwrap();
    let mut storage = ghostty_vt::RowIterator::new().unwrap();
    let mut rows = state.populate_row_iterator(&mut storage).unwrap();
    for _ in 0..=y {
        assert!(rows.next());
    }
    let mut cells = ghostty_vt::RowCells::new().unwrap();
    let mut row = rows.populate_cells(&mut cells).unwrap();
    row.select(x).unwrap();
    row.style().unwrap().inverse
}
#[test]
fn mouse_drag_highlights_only_pane_text_and_copies_on_release() {
    let core = Core::new();
    let mut observer = core.attach();
    while observer.primary.is_none() {
        observer.receive().unwrap();
    }
    let pane_id = observer.primary.take().unwrap().pane_id;
    drop(observer);
    let mut console = client(&core);
    console.wait(|rows| rows.iter().any(|r| r.contains("$")));
    console.text("printf '\\033[2J\\033[H select-%s\\n' me\r");
    let rows = console.wait(|rows| {
        rows.iter().any(|r| {
            r.chars()
                .skip(21)
                .collect::<String>()
                .starts_with("select-me")
        })
    });
    let y = rows
        .iter()
        .position(|r| {
            r.chars()
                .skip(21)
                .collect::<String>()
                .starts_with("select-me")
        })
        .unwrap();
    mouse(&mut console, 0, 21, y, 'M');
    mouse(&mut console, 32, 29, y, 'M');
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if inverse(&mut console, 21, y as u16) && inverse(&mut console, 29, y as u16) {
            break;
        }
        let bytes = console
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("dragged pane text did not become highlighted");
        console.screen.write(&bytes);
    }
    assert!(
        !inverse(&mut console, 19, y as u16),
        "selection leaked into sidebar"
    );
    assert!(
        !inverse(&mut console, 30, y as u16),
        "selection extended past dragged text"
    );
    mouse(&mut console, 0, 29, y, 'm');
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut output = Vec::new();
    loop {
        // Independent known OSC52 encoding of exactly "select-me".
        if output
            .windows(b"\x1b]52;c;c2VsZWN0LW1l\x07".len())
            .any(|w| w == b"\x1b]52;c;c2VsZWN0LW1l\x07")
        {
            break;
        }
        output.extend(
            console
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("release did not copy the selected text to the client clipboard"),
        );
    }
    console.text("printf '\\033[2J\\033[Husable-%s\\n' after-selection\r");
    console.wait(|rows| rows.iter().any(|r| r.contains("usable-after-selection")));
    let mut observer = core.attach();
    while observer.primary.is_none() {
        observer.receive().unwrap();
    }
    assert_eq!(
        observer.primary.unwrap().pane_id,
        pane_id,
        "selection changed the focused pane"
    );
}
