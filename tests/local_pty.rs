use herdr_fork::{Client, Zone};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Session {
    child: Child,
    home: PathBuf,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
fn given_named_local_session() -> Session {
    let home = std::env::temp_dir().join(format!(
        "hf-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_herdr-fork"))
        .args(["server", "--session", "spike"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_STATE_HOME", &home)
        .env("XDG_DATA_HOME", &home)
        .env("HERDR_STARTUP_CWD", &home)
        .env("SHELL", "/bin/sh")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_CONFIG_PATH")
        .env_remove("HERDR_CLIENT_SOCKET_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    Session { child, home }
}
fn when_attach_and_type(session: &mut Session) -> Client {
    let socket = session
        .home
        .join("herdr-fork/sessions/spike/herdr-client.sock");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut client = loop {
        if let Ok(c) = Client::attach(&socket, 80, 24) {
            break c;
        }
        assert!(
            Instant::now() < deadline,
            "core socket did not start: {:?}",
            session.child.try_wait()
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    while client.primary.is_none() || client.focused_pane_id.is_none() {
        client.receive().unwrap();
    }
    client.type_text("printf spike-ready\r").unwrap();
    client
}
fn then_live_primary_surface_and_reserved_zones(client: &mut Client) {
    assert!(
        client.workspace_id.is_some(),
        "default workspace not focused"
    );
    let surface = client.primary.as_ref().unwrap();
    assert!(
        surface.focused,
        "primary working Surface must retain core PTY focus"
    );
    assert_eq!(Some(&surface.pane_id), client.focused_pane_id.as_ref());
    assert_eq!(
        client.reserved_zones,
        [Zone::List, Zone::Footer, Zone::FooterRight]
    );
    wait_for(client, "spike-ready");
    client.type_text("printf 'further-%s\\n' input\r").unwrap();
    wait_for(client, "further-input");
    client
        .type_text("printf '\\033[2;5;9mstyled-%s\\033[0m\\n' live\r")
        .unwrap();
    wait_for(client, "styled-live");
    let layout = herdr_fork::terminal::Layout::new(100, 25).unwrap();
    let mut output = Vec::new();
    herdr_fork::terminal::draw(&mut output, client.primary.as_ref().unwrap(), &layout).unwrap();
    let mut screen = ghostty_vt::Terminal::new(100, 25, 0).unwrap();
    screen.write(&output);
    assert_rendered_shell_style(&screen);
    let rows = screen.screen_text_rows().unwrap();
    let text: String = rows
        .iter()
        .flat_map(|r| r.cells.iter())
        .flat_map(|c| c.graphemes.iter())
        .filter_map(|n| char::from_u32(*n))
        .collect();
    assert!(
        text.contains("further-input"),
        "renderer must show real subsequent output"
    );
    for zone in [layout.list, layout.footer, layout.footer_right] {
        assert!(zone.width > 0 && zone.height > 0);
        for y in zone.y..zone.y + zone.height {
            for x in zone.x..zone.x + zone.width {
                assert!(
                    rows[usize::from(y)].cells[usize::from(x)]
                        .graphemes
                        .iter()
                        .all(|c| *c == 32),
                    "reserved Zone must remain empty"
                );
            }
        }
    }
}
#[test]
fn use_a_local_pty_through_the_rust_client() {
    let mut session = given_named_local_session();
    let mut client = when_attach_and_type(&mut session);
    then_live_primary_surface_and_reserved_zones(&mut client);
}
#[test]
fn living_session_feature() {
    let mut session = None;
    let mut client = None;
    let mut steps = 0;
    for line in include_str!("../features/session.feature")
        .lines()
        .map(str::trim)
    {
        if line.starts_with("Given ") {
            assert_eq!(steps, 0);
            assert_eq!(
                line,
                "Given a named local session is started by the vendored core with one shell pane"
            );
            session = Some(given_named_local_session());
        } else if line.starts_with("When ") {
            assert_eq!(steps, 1);
            assert_eq!(line, "When I attach the Rust client with the default workspace focused and type \"printf spike-ready\" into its primary working pane");
            client = Some(when_attach_and_type(session.as_mut().unwrap()));
        } else if line.starts_with("Then ") {
            assert_eq!(steps, 2);
            assert_eq!(line,"Then the client’s primary focused working Surface is the live PTY-backed pane, it shows \"spike-ready\" from that PTY, and it accepts further shell input while plugin UI remains in reserved Zones");
            then_live_primary_surface_and_reserved_zones(client.as_mut().unwrap());
        } else {
            assert!(
                line.is_empty() || line.starts_with("Feature: ") || line.starts_with("Scenario: "),
                "unbound living step: {line}"
            );
            continue;
        }
        steps += 1;
    }
    assert_eq!(steps, 3);
}
fn wait_for(client: &mut Client, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !client
        .primary
        .as_ref()
        .is_some_and(|s| s.text().lines().any(|line| line.starts_with(text)))
    {
        assert!(Instant::now() < deadline, "live PTY output missing: {text}");
        client
            .receive()
            .unwrap_or_else(|e| panic!("live PTY output missing: {text}: {e}"));
    }
}
struct Console(Box<dyn portable_pty::Child + Send + Sync>);
impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn interactive_binary_uses_the_real_primary_pty_and_empty_zones() {
    use std::io::{Read, Write};
    let mut session = given_named_local_session();
    let client = when_attach_and_type(&mut session);
    drop(client);
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: 25,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_herdr-fork"));
    command.args(["--session", "spike"]);
    for name in ["HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME"] {
        command.env(name, &session.home);
    }
    command.env("TERM", "xterm-256color");
    for name in [
        "HERDR_ENV",
        "HERDR_SOCKET_PATH",
        "HERDR_CLIENT_SOCKET_PATH",
        "HERDR_CONFIG_PATH",
        "HERDR_SESSION",
        "HERDR_PANE_ID",
    ] {
        command.env_remove(name);
    }
    let mut console = Console(pair.slave.spawn_command(command).unwrap());
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reading = std::thread::spawn(move || {
        let mut bytes = [0; 16384];
        while let Ok(n) = reader.read(&mut bytes) {
            if n == 0 || tx.send(bytes[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut screen = ghostty_vt::Terminal::new(100, 25, 0).unwrap();
    let wait_screen = |screen: &mut ghostty_vt::Terminal, expected: &str| {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let rows = screen.screen_text_rows().unwrap();
            if rows.iter().any(|r| {
                r.cells[20..]
                    .iter()
                    .flat_map(|c| c.graphemes.iter())
                    .filter_map(|n| char::from_u32(*n))
                    .collect::<String>()
                    .starts_with(expected)
            }) {
                break;
            }
            let bytes = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|e| panic!("interactive screen missing {expected}: {e}"));
            screen.write(&bytes);
        }
    };
    wait_screen(&mut screen, "spike-ready");
    writer.write_all(b"printf 'console-%s\\n' live\r").unwrap();
    writer.flush().unwrap();
    wait_screen(&mut screen, "console-live");
    writer
        .write_all(b"printf '\\033[2;5;9mstyled-%s\\033[0m\\n' live\r")
        .unwrap();
    writer.flush().unwrap();
    wait_screen(&mut screen, "styled-live");
    assert_rendered_shell_style(&screen);
    let rows = screen.screen_text_rows().unwrap();
    for (y, row) in rows.iter().enumerate() {
        let reserved = if y == 24 {
            &row.cells[..]
        } else {
            &row.cells[..20]
        };
        assert!(
            reserved
                .iter()
                .flat_map(|c| c.graphemes.iter())
                .all(|c| *c == 32),
            "console painted a reserved Zone"
        );
    }
    writer.write_all(b"\x11").unwrap();
    writer.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = console.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "client did not detach");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(writer);
    drop(pair.master);
    drop(console);
    drop(rx);
    reading.join().unwrap();
}
fn assert_rendered_shell_style(screen: &ghostty_vt::Terminal) {
    let rows = screen.screen_text_rows().unwrap();
    let y = rows
        .iter()
        .position(|row| {
            row.cells[20..]
                .iter()
                .flat_map(|c| c.graphemes.iter())
                .filter_map(|n| char::from_u32(*n))
                .collect::<String>()
                .starts_with("styled-live")
        })
        .expect("styled shell output visible");
    let mut state = ghostty_vt::RenderState::new().unwrap();
    state.update(screen).unwrap();
    let mut row_storage = ghostty_vt::RowIterator::new().unwrap();
    let mut row_iter = state.populate_row_iterator(&mut row_storage).unwrap();
    for _ in 0..=y {
        assert!(row_iter.next());
    }
    let mut cell_storage = ghostty_vt::RowCells::new().unwrap();
    let mut cell_iter = row_iter.populate_cells(&mut cell_storage).unwrap();
    cell_iter.select(20).unwrap();
    let style = cell_iter.style().unwrap();
    assert!(style.faint, "renderer lost shell DIM attribute");
    assert!(style.blink, "renderer lost shell SLOW_BLINK attribute");
    assert!(
        style.strikethrough,
        "renderer lost shell CROSSED_OUT attribute"
    );
}
