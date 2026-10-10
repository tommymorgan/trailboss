use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use herdr_fork::{
    terminal::{self, Layout},
    Client,
};
use std::{
    io,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
struct Terminal;
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
    }
}
fn main() -> io::Result<()> {
    let raw: Vec<String> = std::env::args().collect();
    let args = herdr_runtime::session::configure_from_args(&raw).map_err(io::Error::other)?;
    if args.get(1).is_some_and(|a| a == "server") {
        return herdr_runtime::run_server();
    }
    if args.len() > 1 {
        return Err(io::Error::other(
            "usage: herdr-fork [server] [--session NAME]",
        ));
    }
    let (cols, rows) = crossterm::terminal::size()?;
    let mut layout = Layout::new(cols, rows)?;
    let path = herdr_runtime::session::client_socket_path_for(
        herdr_runtime::session::active_name().as_deref(),
    );
    let mut client = match Client::attach(&path, layout.working.width, layout.working.height) {
        Ok(c) => c,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            let mut command = Command::new(std::env::current_exe()?);
            command.arg("server");
            if let Some(name) = herdr_runtime::session::active_name() {
                command.args(["--session", &name]);
            }
            let mut child = command
                .env("HERDR_STARTUP_CWD", std::env::current_dir()?)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Ok(c) = Client::attach(&path, layout.working.width, layout.working.height) {
                    break c;
                }
                if let Some(status) = child.try_wait()? {
                    return Err(io::Error::other(format!("core exited: {status}")));
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::other("core startup timed out"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Err(e) => return Err(e),
    };
    while client.primary.is_none() || client.focused_pane_id.is_none() {
        client.receive()?;
    }
    let mut input = client.input()?;
    client.blocking()?;
    enable_raw_mode()?;
    let _terminal = Terminal;
    execute!(io::stdout(), EnterAlternateScreen)?;
    if let Some(surface) = client.primary.take() {
        terminal::draw(&mut io::stdout(), &surface, &layout)?;
    }
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || loop {
        let result = client.receive();
        if let Err(e) = result {
            let _ = tx.send(Err(e));
            break;
        }
        if let Some(s) = client.primary.take() {
            if tx.send(Ok(s)).is_err() {
                break;
            }
        }
    });
    loop {
        while let Ok(surface) = rx.try_recv() {
            terminal::draw(&mut io::stdout(), &surface?, &layout)?;
        }
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Resize(c, r) => {
                    layout = Layout::new(c, r)?;
                    input.resize(layout.working.width, layout.working.height)?;
                }
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    if k.code == KeyCode::Char('q') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        input.detach()?;
                        return Ok(());
                    }
                    let text = match k.code {
                        KeyCode::Char(c)
                            if k.modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii() =>
                        {
                            String::from((c as u8 & 0x1f) as char)
                        }
                        KeyCode::Char(c) => c.to_string(),
                        KeyCode::Enter => "\r".into(),
                        KeyCode::Backspace => "\x7f".into(),
                        KeyCode::Tab => "\t".into(),
                        KeyCode::Esc => "\x1b".into(),
                        KeyCode::Up => "\x1b[A".into(),
                        KeyCode::Down => "\x1b[B".into(),
                        KeyCode::Right => "\x1b[C".into(),
                        KeyCode::Left => "\x1b[D".into(),
                        _ => continue,
                    };
                    input.type_text(&text)?;
                }
                _ => {}
            }
        }
    }
}
