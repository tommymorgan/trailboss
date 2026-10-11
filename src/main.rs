use crossterm::{
    cursor,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use herdr_fork::{terminal, Client};
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
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            cursor::Show
        );
    }
}
fn main() -> io::Result<()> {
    let raw: Vec<String> = std::env::args().collect();
    let (raw, remote) =
        herdr_runtime::remote::extract_remote_args(&raw).map_err(io::Error::other)?;
    let mut args = herdr_runtime::session::configure_from_args(&raw).map_err(io::Error::other)?;
    let executable = if let Some(index) = args.iter().position(|arg| arg == "--remote-executable") {
        if remote.is_none() {
            return Err(io::Error::other("--remote-executable requires --remote"));
        }
        let path = args
            .get(index + 1)
            .cloned()
            .ok_or_else(|| io::Error::other("missing value for --remote-executable"))?;
        args.drain(index..=index + 1);
        Some(path)
    } else {
        None
    };
    match args.get(1).map(String::as_str) {
        Some("remote-client-bridge") => {
            enable_default_producer()?;
            return herdr_runtime::remote::run_remote_client_bridge(&args[2..]);
        }
        Some("remote-api-bridge") => {
            return herdr_runtime::remote::run_remote_api_bridge(&args[2..])
        }
        _ => {}
    }
    if matches!(
        args.get(1).map(String::as_str),
        Some(
            "api"
                | "status"
                | "server"
                | "session"
                | "workspace"
                | "worktree"
                | "tab"
                | "pane"
                | "agent"
                | "notification"
                | "integration"
                | "plugin"
        )
    ) {
        if let herdr_runtime::cli::CommandOutcome::Handled(code) =
            herdr_runtime::cli::maybe_run(&args)?
        {
            std::process::exit(code);
        }
    }
    if args.get(1).is_some_and(|a| a == "server") {
        return herdr_runtime::run_server();
    }
    if args.len() > 1 {
        return Err(io::Error::other(
            "usage: herdr-fork [SESSION_API_COMMAND | server] [--session NAME] [--remote HOST [--remote-executable PATH]]",
        ));
    }
    let remote = remote
        .map(|remote| {
            herdr_runtime::remote::RemoteConnection::connect(
                remote.target(),
                &herdr_runtime::session::active_name()
                    .unwrap_or_else(|| herdr_runtime::session::DEFAULT_SESSION_NAME.into()),
                executable.as_deref(),
            )
        })
        .transpose()?;
    let path = remote.as_ref().map_or_else(
        || {
            herdr_runtime::session::client_socket_path_for(
                herdr_runtime::session::active_name().as_deref(),
            )
        },
        |connection| connection.socket_path().to_owned(),
    );
    run_client(&path, remote.is_some())
}
fn run_client(path: &std::path::Path, remote: bool) -> io::Result<()> {
    let (cols, rows) = crossterm::terminal::size()?;
    let mut viewport = terminal::Rect::viewport(cols, rows);
    let mut client = match Client::attach(path, cols, rows) {
        Ok(client) => client,
        Err(error)
            if !remote
                && matches!(
                    error.kind(),
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
                if let Ok(client) = Client::attach(path, cols, rows) {
                    break client;
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
        Err(error) => return Err(error),
    };
    // Attaching without a pane is valid: a declaration can offer the first tab.
    while client.boot_id.is_empty() {
        client.receive()?;
    }
    if !remote {
        enable_default_producer()?;
    }
    let mut input = client.input()?;
    client.blocking()?;
    enable_raw_mode()?;
    let _terminal = Terminal;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let mut core = herdr_fork::core::Core::default();
    core.update_session(
        client.workspace_id.clone(),
        client.focused_tab_id.clone(),
        client.focused_pane_id.is_some(),
    );
    let mut surface = client.primary.take();
    let mut declarations = client.declarations.take().unwrap_or_default();
    let mut boot_id = client.boot_id.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut last_focus = (
            client.workspace_id.clone(),
            client.focused_tab_id.clone(),
            client.focused_pane_id.clone(),
        );
        loop {
            if let Err(error) = client.receive() {
                let _ = tx.send(Err(error));
                break;
            }
            let session = (
                client.workspace_id.clone(),
                client.focused_tab_id.clone(),
                client.focused_pane_id.clone(),
            );
            let focus_update = if session != last_focus {
                last_focus.clone_from(&session);
                Some(session)
            } else {
                None
            };
            if client.primary.is_some() || client.declarations.is_some() || focus_update.is_some() {
                let update = (
                    client.primary.take(),
                    client.declarations.take(),
                    focus_update,
                    client.boot_id.clone(),
                );
                if tx.send(Ok(update)).is_err() {
                    break;
                }
            }
        }
    });
    let mut dirty = true;
    let mut pane_size = (cols, rows);
    loop {
        while let Ok(update) = rx.try_recv() {
            let (next_surface, next_declarations, session, next_boot) = update?;
            if let Some(next) = next_surface {
                surface = Some(next);
            }
            if let Some(next) = next_declarations {
                declarations = next;
            }
            if let Some((workspace, tab, pane)) = session {
                input.focus_pane_target(pane.as_deref().unwrap_or_default());
                if surface
                    .as_ref()
                    .is_some_and(|s| Some(s.pane_id.as_str()) != pane.as_deref())
                {
                    surface = None;
                }
                core.update_session(workspace, tab, pane.is_some());
            }
            boot_id = next_boot;
            dirty = true;
        }
        if dirty {
            if let Some(rect) = terminal::surface_rect(&core, &declarations, viewport) {
                let size = (rect.width, rect.height);
                if size != pane_size {
                    input.resize(size.0, size.1)?;
                    pane_size = size;
                }
            }
            terminal::draw_frame(
                &mut io::stdout(),
                surface.as_ref(),
                &declarations,
                &core,
                viewport,
            )?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(16))? {
            continue;
        }
        match event::read()? {
            Event::Paste(text) => {
                input.paste(&mut core, &boot_id, &text)?;
                dirty = true;
            }
            Event::Resize(cols, rows) => {
                viewport = terminal::Rect::viewport(cols, rows);
                dirty = true;
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                if let Some((plugin, action, context)) =
                    terminal::action_at(&core, &declarations, viewport, mouse.column, mouse.row)
                {
                    input.dispatch(&mut core, &boot_id, plugin, action, context)?;
                    dirty = true;
                }
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    input.detach()?;
                    return Ok(());
                }
                let text = match key.code {
                    KeyCode::Char(c)
                        if key.modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii() =>
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
                input.dispatch_text(&mut core, &boot_id, &text)?;
                dirty = true;
            }
            _ => {}
        }
    }
}
fn enable_default_producer() -> io::Result<()> {
    use herdr_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(
        herdr_runtime::session::active_api_socket_path(),
    ));
    let request: Request = serde_json::from_value(
        serde_json::json!({"id":"stock-registry","method":"plugin.list","params":{}}),
    )
    .map_err(io::Error::other)?;
    let reply = api
        .request_value_with_timeout(&request, Duration::from_secs(5))
        .map_err(io::Error::other)?;
    if reply["result"]["plugins"]
        .as_array()
        .is_some_and(|plugins| {
            plugins
                .iter()
                .any(|plugin| plugin["plugin_id"] == "tommymorgan.stock-ui")
        })
    {
        return Ok(());
    }
    let request: Request = serde_json::from_value(serde_json::json!({"id":"stock-producer","method":"plugin.link","params":{"path":concat!(env!("CARGO_MANIFEST_DIR"),"/plugins/stock-ui"),"enabled":true}})).map_err(io::Error::other)?;
    let reply = api
        .request_value_with_timeout(&request, Duration::from_secs(5))
        .map_err(io::Error::other)?;
    if let Some(error) = reply.get("error") {
        return Err(io::Error::other(error.to_string()));
    }
    Ok(())
}
