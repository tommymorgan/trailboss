use crossterm::{
    cursor,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use trailboss::{terminal, Client};
mod selection;
use std::{
    io::{self, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
const NORMAL_POINTER: &str = "\x1b]22;default\x1b\\";
struct Terminal;
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            crossterm::style::Print(NORMAL_POINTER),
            PopKeyboardEnhancementFlags,
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
        trailboss_runtime::remote::extract_remote_args(&raw).map_err(io::Error::other)?;
    let mut args =
        trailboss_runtime::session::configure_from_args(&raw).map_err(io::Error::other)?;
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
            return trailboss_runtime::remote::run_remote_client_bridge(
                &args[2..],
                enable_default_producer,
            );
        }
        Some("remote-api-bridge") => {
            return trailboss_runtime::remote::run_remote_api_bridge(&args[2..])
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
        if let trailboss_runtime::cli::CommandOutcome::Handled(code) =
            trailboss_runtime::cli::maybe_run(&args)?
        {
            std::process::exit(code);
        }
    }
    if args.get(1).is_some_and(|a| a == "server") {
        return trailboss_runtime::run_server();
    }
    if args.len() > 1 {
        return Err(io::Error::other(
            "usage: trailboss [SESSION_API_COMMAND | server] [--session NAME] [--remote HOST [--remote-executable PATH]]",
        ));
    }
    trailboss_runtime::client::initialize_native_diagnostics();
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(location) = info.location() {
            tracing::error!(
                event = "native_client_panic",
                file = location.file(),
                line = location.line(),
                column = location.column(),
                backtrace = %std::backtrace::Backtrace::force_capture(),
                "native client panicked (payload omitted)"
            );
        } else {
            tracing::error!(
                event = "native_client_panic",
                backtrace = %std::backtrace::Backtrace::force_capture(),
                "native client panicked (location unavailable; payload omitted)"
            );
        }
        previous_hook(info);
    }));
    tracing::info!(
        event = "native_client_start",
        pid = std::process::id(),
        session = ?trailboss_runtime::session::active_name(),
        transport = if remote.is_some() { "ssh" } else { "unix" },
        "native client starting"
    );
    let result = run_native_client(remote, executable.as_deref());
    match &result {
        Ok(()) => tracing::info!(event = "native_client_end", outcome = "detached", "native client ended"),
        Err(error) => tracing::error!(
            event = "native_client_end",
            outcome = "failure",
            error_kind = ?error.kind(),
            raw_os_error = ?error.raw_os_error(),
            "native client failed (error text omitted)"
        ),
    }
    result
}
fn run_native_client(
    remote: Option<trailboss_runtime::remote::RemoteLaunch>,
    executable: Option<&str>,
) -> io::Result<()> {
    tracing::info!(event = "native_client_transport", phase = "discovery", "selecting client transport");
    let remote = remote
        .map(|remote| {
            trailboss_runtime::remote::RemoteConnection::connect(
                remote.target(),
                &trailboss_runtime::session::active_name()
                    .unwrap_or_else(|| trailboss_runtime::session::DEFAULT_SESSION_NAME.into()),
                executable,
            )
        })
        .transpose()
        .inspect_err(|error| {
            tracing::error!(
                event = "native_client_transport_failed",
                phase = "discovery",
                error_kind = ?error.kind(),
                raw_os_error = ?error.raw_os_error(),
                "remote discovery or bridge connection failed"
            );
        })?;
    let path = remote.as_ref().map_or_else(
        || {
            trailboss_runtime::session::client_socket_path_for(
                trailboss_runtime::session::active_name().as_deref(),
            )
        },
        |connection| connection.socket_path().to_owned(),
    );
    tracing::info!(
        event = "native_client_transport",
        phase = "ready",
        transport = if remote.is_some() { "ssh" } else { "unix" },
        "client transport selected"
    );
    run_client(&path, remote.is_some())
}
fn run_client(path: &std::path::Path, remote: bool) -> io::Result<()> {
    let mut phase = "graphics_host";
    let result = run_client_inner(path, remote, &mut phase);
    if let Err(error) = &result {
        tracing::error!(
            event = "native_client_loop_failed",
            phase,
            error_kind = ?error.kind(),
            raw_os_error = ?error.raw_os_error(),
            "native client attach/render loop failed (error text omitted)"
        );
    }
    result
}
fn run_client_inner(
    path: &std::path::Path,
    remote: bool,
    phase: &mut &'static str,
) -> io::Result<()> {
    let defaults = trailboss_runtime::config::Config::load().config;
    let graphics_enabled = defaults.kitty_graphics_enabled();
    let mut host = trailboss_runtime::client::graphics_transport::Host::new(graphics_enabled)?;
    let (cols, rows, cell_width, cell_height, exact_cell_size) = host.geometry();
    let direct_graphics = host.direct_allowed(remote);
    let mut viewport = terminal::Rect::viewport(cols, rows);
    *phase = "attach";
    tracing::info!(event = "native_client_attach", phase = "start", cols, rows, direct_graphics, "attaching native client");
    let mut client = match Client::attach_graphics(
        path,
        cols,
        rows,
        (cell_width, cell_height),
        exact_cell_size,
        direct_graphics,
    ) {
        Ok(client) => client,
        Err(error)
            if !remote
                && matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
        {
            *phase = "core_startup";
            tracing::info!(event = "native_client_attach", phase = "core_startup", "starting local core after unavailable socket");
            let mut command = Command::new(std::env::current_exe()?);
            command.arg("server");
            if let Some(name) = trailboss_runtime::session::active_name() {
                command.args(["--session", &name]);
            }
            let mut child = command
                .env("TRAILBOSS_STARTUP_CWD", std::env::current_dir()?)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Ok(client) = Client::attach_graphics(
                    path,
                    cols,
                    rows,
                    (cell_width, cell_height),
                    exact_cell_size,
                    direct_graphics,
                ) {
                    break client;
                }
                if let Some(status) = child.try_wait()? {
                    tracing::error!(event = "native_client_attach_failed", phase = "core_startup", %status, "local core exited before attach");
                    return Err(io::Error::other(format!("core exited: {status}")));
                }
                if Instant::now() >= deadline {
                    tracing::error!(event = "native_client_attach_failed", phase = "core_startup", timeout_seconds = 15, "local core attach deadline elapsed");
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::other("core startup timed out"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Err(error) => return Err(error),
    };
    *phase = "initial_snapshot";
    tracing::info!(event = "native_client_attach", phase = "hello_sent", "waiting for initial session snapshot");
    // Attaching without a pane is valid: a declaration can offer the first tab.
    while client.boot_id.is_empty() {
        client.receive()?;
    }
    tracing::info!(event = "native_client_attach", phase = "snapshot_ready", "initial session snapshot received");
    *phase = "producer";
    if !remote {
        enable_default_producer()?;
    }
    *phase = "input";
    let mut input = client.input()?;
    client.blocking()?;
    *phase = "terminal_setup";
    enable_raw_mode()?;
    let _terminal = Terminal;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        PushKeyboardEnhancementFlags(host.keyboard_enhancement_flags())
    )?;
    let mut core = trailboss::core::Core::default();
    core.update_session(
        client.workspace_id.clone(),
        client.focused_tab_id.clone(),
        client.focused_pane_id.is_some(),
    );
    let mut surface = client.primary.take();
    let mut declarations = client.declarations.take().unwrap_or_default();
    let mut boot_id = client.boot_id.clone();
    let mut snapshot = client.snapshot.take();
    let mut graphics_messages = std::mem::take(&mut client.graphics_messages);
    let (tx, rx) = mpsc::sync_channel(1);
    tracing::info!(event = "native_client_receiver_start", "starting native receive worker");
    std::thread::spawn(move || {
        let mut last_focus = (
            client.workspace_id.clone(),
            client.focused_tab_id.clone(),
            client.focused_pane_id.clone(),
        );
        loop {
            if let Err(error) = client.receive() {
                tracing::error!(event = "native_client_receiver_end", reason = "receive_failure", "native receive worker failed");
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
            if client.surface_changed
                || client.declarations.is_some()
                || focus_update.is_some()
                || client.snapshot.is_some()
                || !client.graphics_messages.is_empty()
            {
                let update = (
                    client.primary.take(),
                    client.declarations.take(),
                    focus_update,
                    client.boot_id.clone(),
                    client.snapshot.take(),
                    std::mem::take(&mut client.graphics_messages),
                    std::mem::take(&mut client.surface_changed),
                );
                trailboss_runtime::render_prof::trace(
                    "client.update.send_start",
                    format_args!("pane={:?}", client.focused_pane_id),
                );
                if tx.send(Ok(update)).is_err() {
                    tracing::info!(event = "native_client_receiver_end", reason = "consumer_closed", "native receive worker stopped after consumer closed");
                    break;
                }
                trailboss_runtime::render_prof::trace(
                    "client.update.send_end",
                    format_args!("pane={:?}", client.focused_pane_id),
                );
            }
        }
    });
    let mut dirty = true;
    host.query_geometry(io::stdout())?;
    let prefix_keys = defaults.prefix_keys();
    let detach_keys = defaults.keybinds().detach;
    let mouse_scroll_lines = defaults.ui.mouse_scroll_lines();
    let mut pane_size = (cols, rows);
    let mut selection: Option<selection::PaneSelection> = None;
    let mut last_pane_click: Option<Instant> = None;
    // Retain one output buffer: cell commands must not become thousands of
    // individual PTY writes before the completed frame is flushed.
    let mut output = io::BufWriter::with_capacity(128 * 1024, io::stdout());
    let mut renderer = terminal::FrameRenderer::default();
    renderer.graphics = trailboss_runtime::client::graphics::Renderer::from_environment();
    renderer
        .graphics
        .configure(graphics_enabled, (cell_width, cell_height));
    let mut pending_direct = std::collections::HashMap::new();
    let mut retired_direct = std::collections::HashSet::new();
    renderer.copy_feedback_position = defaults.ui.toast.clipboard.position;
    let mut sidebar_resize = terminal::split::SidebarResize::default();
    // Keep ownership through button-up even when an invalid drop cancels its source.
    let mut reorder_gesture = false;
    let mut pointer = None;
    let mut resize_cursor = None;
    let mut pointer_dirty = true;
    // Match the existing endpoint health probe cadence. Quiet native sessions
    // must keep the remote bridge alive without input or redraws.
    let health_ping = trailboss_runtime::protocol::ClientMessage::EndpointControl {
        kind: trailboss_runtime::protocol::endpoint::HEALTH_PING_KIND.into(),
        data: String::new(),
    };
    let mut next_health_ping = Instant::now() + Duration::from_secs(5);
    *phase = "render_loop";
    tracing::info!(event = "native_client_render_start", "native render loop started");
    loop {
        let now = Instant::now();
        dirty |= core.tick_copy_feedback(now);
        if core.detach_requested {
            *phase = "detach";
            tracing::info!(event = "native_client_detach", source = "menu", phase = "requested", "native client detach requested");
            renderer.graphics.cleanup(&mut output)?;
            input.detach()?;
            tracing::info!(event = "native_client_detach", source = "menu", phase = "complete", "native client detached");
            return Ok(());
        }
        if now >= next_health_ping {
            input.send(&health_ping)?;
            next_health_ping = now + Duration::from_secs(5);
        }
        while let Ok(update) = rx.try_recv() {
            let (
                next_surface,
                next_declarations,
                session,
                next_boot,
                next_snapshot,
                next_graphics,
                surface_changed,
            ) = update?;
            if boot_id != next_boot {
                core.clear_tab_history();
                core.prefix_active = false;
            }
            graphics_messages.extend(next_graphics);
            trailboss_runtime::render_prof::trace(
                "client.update.apply",
                format_args!("pane={:?}", next_surface.as_ref().map(|s| &s.pane_id)),
            );
            if surface_changed {
                if surface.as_ref().map(|s| &s.pane_id) != next_surface.as_ref().map(|s| &s.pane_id)
                {
                    selection = None;
                }
                surface = next_surface;
            }
            if let Some(next) = next_declarations {
                declarations = next;
            }
            if let Some(next) = next_snapshot {
                snapshot = Some(next);
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
            if boot_id != next_boot {
                renderer.graphics.cleanup(&mut output)?;
                host.reset();
                pending_direct.clear();
                retired_direct.clear();
                selection = None;
                core.clear_copy_feedback();
                renderer.controls.list_offset = 0;
                renderer.controls.reset();
                renderer.reorder.cancel();
                reorder_gesture = false;
            }
            boot_id = next_boot;
            dirty = true;
        }
        if let Some(surface) = &surface {
            renderer
                .graphics
                .set_scene(&surface.graphics_scope, surface.graphics.clone());
        }
        for message in graphics_messages.drain(..) {
            use trailboss_runtime::protocol::{ClientMessage, ServerMessage};
            match message {
                ServerMessage::GraphicsTransmissionRetired {
                    transfer_id,
                    image_id,
                } => {
                    if retired_direct.len() < 4096 {
                        retired_direct.insert((transfer_id, image_id));
                    }
                    host.retire(transfer_id);
                    pending_direct.remove(&(transfer_id, image_id));
                    renderer.graphics.retire_direct_image(image_id);
                    dirty = true;
                }
                message @ ServerMessage::GraphicsFile { .. } => {
                    let ServerMessage::GraphicsFile {
                        transfer_id,
                        image_id,
                        ref surface_asset,
                        ..
                    } = message
                    else {
                        unreachable!()
                    };
                    if retired_direct.contains(&(transfer_id, image_id)) {
                        continue;
                    }
                    let sent = host.start_file(
                        &message,
                        direct_graphics && retired_direct.len() < 4096 && surface.is_some(),
                        |asset, image| renderer.graphics.accepts_direct_asset(asset, image),
                        &mut output,
                    )?;
                    if sent {
                        if let Some(asset) = surface_asset {
                            pending_direct.insert((transfer_id, image_id), asset.clone());
                        }
                        input.send(&ClientMessage::GraphicsTransmissionStarted {
                            transfer_id,
                            image_id,
                        })?;
                    } else {
                        input.send(&ClientMessage::GraphicsTransmissionResult {
                            transfer_id,
                            image_id,
                            success: false,
                        })?;
                    }
                }
                _ => {}
            }
        }
        if dirty {
            sidebar_resize.apply(&core, &mut declarations, viewport);
            renderer.reorder.list_offset = renderer.controls.list_offset;
            if let Some(snapshot) = &snapshot {
                renderer
                    .reorder
                    .reconcile(&core, &declarations, viewport, snapshot);
            }
            if renderer.controls.modal() {
                renderer.reorder.cancel();
            }
        }
        if selection.as_ref().is_some_and(|selected| {
            surface
                .as_ref()
                .zip(terminal::surface_rect(&core, &declarations, viewport))
                .is_none_or(|(surface, rect)| !selected.matches(surface, rect))
        }) {
            selection = None;
            dirty = true;
        }
        if dirty {
            pointer_dirty = true;
            if let Some(rect) = terminal::surface_rect(&core, &declarations, viewport) {
                let size = (rect.width, rect.height);
                if size != pane_size {
                    let geometry = host.geometry();
                    input.resize_graphics(size.0, size.1, (geometry.2, geometry.3), geometry.4)?;
                    pane_size = size;
                }
            }
            if let (Some(selected), Some(surface)) = (&selection, &mut surface) {
                selected.decorate(surface);
            }
            trailboss_runtime::render_prof::trace(
                "client.render.start",
                format_args!("pane={:?}", surface.as_ref().map(|s| &s.pane_id)),
            );
            let drawn = renderer.draw_frame(
                &mut output,
                surface.as_ref(),
                &declarations,
                &core,
                viewport,
            );
            // Restore the server-provided styles even if terminal output failed.
            if let (Some(selected), Some(surface)) = (&selection, &mut surface) {
                selected.decorate(surface);
            }
            drawn?;
            trailboss_runtime::render_prof::trace(
                "client.render.end",
                format_args!("pane={:?}", surface.as_ref().map(|s| &s.pane_id)),
            );
            dirty = false;
        }
        if pointer_dirty {
            pointer_dirty = false;
            let next_cursor = !renderer.controls.modal()
                && !reorder_gesture
                && sidebar_resize.resize_cursor(&core, &declarations, viewport, pointer);
            if Some(next_cursor) != resize_cursor {
                // OSC 22 changes the mouse pointer, not the terminal text cursor.
                output.write_all(if next_cursor {
                    b"\x1b]22;ew-resize\x1b\\"
                } else {
                    NORMAL_POINTER.as_bytes()
                })?;
                output.flush()?;
                resize_cursor = Some(next_cursor);
            }
        }
        let geometry_changed = host.refresh_geometry()?;
        if geometry_changed {
            let geometry = host.geometry();
            renderer
                .graphics
                .configure(graphics_enabled, (geometry.2, geometry.3));
            input.resize_graphics(
                pane_size.0,
                pane_size.1,
                (geometry.2, geometry.3),
                geometry.4,
            )?;
            // Grid changes are queued as native Resize events, preserving gesture
            // cancellation in the existing resize handler.
            dirty = true;
        }
        let event = host.poll(Duration::from_millis(16))?;
        while let Some(mut reply) = host.take_reply() {
            if let trailboss_runtime::protocol::ClientMessage::GraphicsTransmissionResult {
                transfer_id,
                image_id,
                success,
            } = &mut reply
            {
                if let Some(asset) = pending_direct.remove(&(*transfer_id, *image_id)) {
                    *success &= renderer.graphics.trust_direct_asset(&asset, *image_id);
                    if *success {
                        if let Err(error) = renderer.draw_frame(
                            &mut output,
                            surface.as_ref(),
                            &declarations,
                            &core,
                            viewport,
                        ) {
                            renderer.graphics.retire_direct_image(*image_id);
                            *success = false;
                            input.send(&reply)?;
                            return Err(error);
                        }
                    } else {
                        renderer.graphics.retire_direct_image(*image_id);
                    }
                } else {
                    *success = false;
                }
            }
            input.send(&reply)?;
        }
        let Some(event) = event else {
            continue;
        };
        pointer_dirty = true;
        match &event {
            Event::Mouse(mouse) => pointer = Some((mouse.column, mouse.row)),
            Event::Resize(..) | Event::Key(_) | Event::Paste(_) => pointer = None,
            _ => {}
        }
        let ends_tab_cycle = match &event {
            Event::Paste(_) => true,
            Event::Mouse(mouse) => mouse.kind != MouseEventKind::Moved,
            _ => false,
        };
        if ends_tab_cycle && core.finish_tab_cycle() {
            core.prefix_active = false;
            dirty = true;
        }
        match event {
            Event::Paste(text) => {
                renderer.reorder.cancel();
                reorder_gesture = false;
                if renderer.controls.paste(&text) {
                    dirty = true;
                    continue;
                }
                if renderer.controls.captures_input() && !core.editing() {
                    continue;
                }
                selection = None;
                input.paste(&mut core, &boot_id, &text)?;
                dirty = true;
            }
            Event::Resize(cols, rows) => {
                sidebar_resize.finish();
                renderer.reorder.cancel();
                reorder_gesture = false;
                if renderer.controls.modal() && core.editing() {
                    core.invoke("host:cancel", &Default::default(), None);
                }
                renderer.controls.reset();
                selection = None;
                viewport = terminal::Rect::viewport(cols, rows);
                dirty = true;
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) =>
            {
                let response = renderer.controls.wheel(
                    &core,
                    &declarations,
                    viewport,
                    mouse.kind == MouseEventKind::ScrollUp,
                );
                if apply_menu_response(
                    response,
                    &mut input,
                    &mut core,
                    &boot_id,
                    snapshot.as_ref(),
                )? {
                    dirty = true;
                    continue;
                }
                if !sidebar_resize.captures_input()
                    && !reorder_gesture
                    && !terminal::has_modal(&core, &declarations, viewport)
                    && renderer.controls.scroll_list(
                        &core,
                        &declarations,
                        viewport,
                        mouse.column,
                        mouse.row,
                        mouse.kind == MouseEventKind::ScrollUp,
                        mouse_scroll_lines.min(u16::MAX as usize) as u16,
                    )
                {
                    dirty = true;
                    continue;
                }
                if renderer.controls.captures_input()
                    || sidebar_resize.captures_input()
                    || reorder_gesture
                    || terminal::has_modal(&core, &declarations, viewport)
                    || !core.visible(trailboss_runtime::protocol::widgets::Condition::HasPane)
                {
                    continue;
                }
                if let Some((_, rect)) =
                    surface
                        .as_ref()
                        .zip(terminal::surface_rect(&core, &declarations, viewport))
                {
                    if rect.contains(mouse.column, mouse.row) {
                        selection = None;
                        input.wheel(mouse, rect, mouse_scroll_lines)?;
                        dirty = true;
                    }
                }
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::Down(MouseButton::Left | MouseButton::Right)
                ) =>
            {
                let previous_selection = selection.take();
                let previous_click = last_pane_click.take();
                let response = renderer.controls.click(
                    &core,
                    &declarations,
                    viewport,
                    mouse.column,
                    mouse.row,
                    mouse.kind == MouseEventKind::Down(MouseButton::Right),
                );
                if apply_menu_response(
                    response,
                    &mut input,
                    &mut core,
                    &boot_id,
                    snapshot.as_ref(),
                )? {
                    dirty = true;
                    renderer.reorder.cancel();
                    reorder_gesture = false;
                    continue;
                }
                if mouse.kind == MouseEventKind::Down(MouseButton::Right) {
                    continue;
                }
                if sidebar_resize.begin(&core, &declarations, viewport, mouse.column, mouse.row) {
                    renderer.controls.reset();
                    renderer.reorder.cancel();
                    reorder_gesture = false;
                    dirty = true;
                    continue;
                }
                if let Some(snapshot) = &snapshot {
                    if renderer.reorder.press(
                        &core,
                        &declarations,
                        viewport,
                        snapshot,
                        mouse.column,
                        mouse.row,
                    ) {
                        reorder_gesture = true;
                        dirty = true;
                        continue;
                    }
                }
                if let Some((plugin, action, context)) = terminal::action_at(
                    &core,
                    &declarations,
                    viewport,
                    renderer.controls.list_offset,
                    mouse.column,
                    mouse.row,
                ) {
                    input.dispatch(&mut core, &boot_id, plugin, action, context)?;
                } else if let Some((surface, rect)) =
                    surface
                        .as_ref()
                        .zip(terminal::surface_rect(&core, &declarations, viewport))
                {
                    if surface.mouse_reporting && mouse.modifiers.is_empty() {
                        input.mouse(mouse, rect)?;
                        continue;
                    }
                    let now = Instant::now();
                    let double_click = previous_click
                        .is_some_and(|at| now.duration_since(at) <= Duration::from_millis(350))
                        && previous_selection.as_ref().is_some_and(|selected| {
                            selected.was_click_at(surface, rect, mouse.column, mouse.row)
                        });
                    selection = if double_click {
                        selection::PaneSelection::start_word(surface, rect, mouse.column, mouse.row)
                    } else {
                        selection::PaneSelection::start(surface, rect, mouse.column, mouse.row)
                    };
                    if !double_click {
                        last_pane_click = Some(now);
                    }
                }
                dirty = true;
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Moved => {
                if let Some(rect) = terminal::surface_rect(&core, &declarations, viewport) {
                    if surface
                        .as_ref()
                        .is_some_and(|surface| surface.mouse_reporting)
                    {
                        input.mouse(mouse, rect)?;
                    }
                }
                dirty |= renderer.controls.hover(
                    &core,
                    &declarations,
                    viewport,
                    mouse.column,
                    mouse.row,
                );
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Drag(MouseButton::Left) => {
                if sidebar_resize.drag_to(mouse.column, viewport) {
                    dirty = true;
                    continue;
                }
                if reorder_gesture {
                    if let Some(snapshot) = &snapshot {
                        dirty |= renderer.reorder.motion(
                            &core,
                            &declarations,
                            viewport,
                            snapshot,
                            mouse.column,
                            mouse.row,
                        );
                    }
                    continue;
                }
                if let Some(rect) = terminal::surface_rect(&core, &declarations, viewport) {
                    if surface
                        .as_ref()
                        .is_some_and(|surface| surface.mouse_reporting)
                        && mouse.modifiers.is_empty()
                    {
                        input.mouse(mouse, rect)?;
                        continue;
                    }
                }
                if let Some(selected) = &mut selection {
                    selected.drag(mouse.column, mouse.row);
                    dirty = true;
                }
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Up(MouseButton::Left) => {
                if sidebar_resize.drag_to(mouse.column, viewport) {
                    sidebar_resize.finish();
                    dirty = true;
                    continue;
                }
                if reorder_gesture {
                    reorder_gesture = false;
                    if let Some(snapshot) = &snapshot {
                        let released = renderer.reorder.release(
                            &core,
                            &declarations,
                            viewport,
                            snapshot,
                            mouse.column,
                            mouse.row,
                        );
                        if let Some(movement) = released.movement {
                            input.request(&boot_id, movement.method, movement.params)?;
                        }
                        if let Some(action) = released.click {
                            input.dispatch(
                                &mut core,
                                &boot_id,
                                &action.plugin,
                                &action.action,
                                &action.context,
                            )?;
                        }
                    } else {
                        renderer.reorder.cancel();
                    }
                    dirty = true;
                    continue;
                }
                if let Some(rect) = terminal::surface_rect(&core, &declarations, viewport) {
                    if surface
                        .as_ref()
                        .is_some_and(|surface| surface.mouse_reporting)
                        && mouse.modifiers.is_empty()
                    {
                        input.mouse(mouse, rect)?;
                        continue;
                    }
                }
                if let (Some(mut selected), Some(surface)) = (selection.take(), &surface) {
                    selected.drag(mouse.column, mouse.row);
                    if let Some(text) = selected.finish(surface) {
                        trailboss_runtime::client::write_osc52_bytes(text.as_bytes());
                        if defaults.ui.toast.clipboard.enabled {
                            core.copy_dispatched(Instant::now());
                        }
                    }
                    if selected.retain_after_release() {
                        selection = Some(selected);
                    }
                    dirty = true;
                }
            }
            Event::Mouse(mouse) => {
                if !renderer.controls.captures_input()
                    && mouse.modifiers.is_empty()
                    && !sidebar_resize.captures_input()
                    && !reorder_gesture
                    && !terminal::has_modal(&core, &declarations, viewport)
                    && surface
                        .as_ref()
                        .is_some_and(|surface| surface.mouse_reporting)
                {
                    if let Some(rect) = terminal::surface_rect(&core, &declarations, viewport) {
                        input.mouse(mouse, rect)?;
                    }
                }
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                sidebar_resize.finish();
                renderer.reorder.cancel();
                reorder_gesture = false;
                selection.take();
                let combo = (key.code, key.modifiers);
                let is_prefix = prefix_keys.contains(&combo);
                if let Some(effect) = core.tab_cycle_key(key, is_prefix, snapshot.as_ref()) {
                    core.prefix_active = false;
                    if let trailboss::core::Effect::Request { method, params } = effect {
                        input.request(&boot_id, method, params)?;
                    }
                    dirty = true;
                    continue;
                }
                dirty = true;
                if core.prefix_active {
                    core.prefix_active = false;
                    if detach_keys.bindings.iter().any(|binding| {
                        binding.trigger.is_prefix() && binding.trigger.combo() == combo
                    }) {
                        *phase = "detach";
                        tracing::info!(event = "native_client_detach", source = "keybinding", phase = "requested", "native client detach requested");
                        renderer.graphics.cleanup(&mut output)?;
                        input.detach()?;
                        tracing::info!(event = "native_client_detach", source = "keybinding", phase = "complete", "native client detached");
                        return Ok(());
                    }
                    if key.code == KeyCode::Char('l') && key.modifiers.is_empty() {
                        if let Some(snapshot) = &snapshot {
                            if let Some(tab) = core.cycle_tab(snapshot) {
                                input.request(
                                    &boot_id,
                                    "tab.focus",
                                    serde_json::json!({"tab_id": tab}),
                                )?;
                            }
                        }
                        dirty = true;
                        continue;
                    }
                    if key.code == KeyCode::Char('p') && key.modifiers.is_empty() {
                        if core.editing() {
                            core.invoke("host:cancel", &Default::default(), None);
                        }
                        renderer.controls.open_palette(&core, &declarations, None);
                        dirty = true;
                        continue;
                    }
                    if key.code == KeyCode::Tab && key.modifiers.is_empty() {
                        renderer.controls.reset();
                        renderer
                            .controls
                            .focus_next(&core, &declarations, viewport, false);
                        dirty = true;
                        continue;
                    }
                    // Trailboss sends a literal prefix when pressed twice; other
                    // unmatched prefix commands (including Escape) are consumed.
                    if !is_prefix {
                        continue;
                    }
                } else if is_prefix {
                    core.prefix_active = true;
                    continue;
                }
                if key.code == KeyCode::F(1) && key.modifiers.is_empty() {
                    if core.editing() {
                        core.invoke("host:cancel", &Default::default(), None);
                    }
                    renderer.controls.open_palette(&core, &declarations, None);
                    dirty = true;
                    continue;
                }
                let response = renderer.controls.key(&core, &declarations, viewport, key);
                if apply_menu_response(
                    response,
                    &mut input,
                    &mut core,
                    &boot_id,
                    snapshot.as_ref(),
                )? {
                    dirty = true;
                    continue;
                }
                if remote
                    && key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Char('v')
                    && key.modifiers == KeyModifiers::CONTROL
                    && input.paste_image(&core)?
                {
                    dirty = true;
                    continue;
                }
                if terminal::has_modal(&core, &declarations, viewport) && !core.editing() {
                    continue;
                }
                input.dispatch_key(&mut core, &boot_id, key)?;
                dirty = true;
            }
            _ => {}
        }
    }
}
fn apply_menu_response(
    response: terminal::menus::Response,
    input: &mut trailboss::Input,
    core: &mut trailboss::core::Core,
    boot_id: &str,
    snapshot: Option<&trailboss_runtime::protocol::ClientShellSnapshot>,
) -> io::Result<bool> {
    match response {
        terminal::menus::Response::Pass => Ok(false),
        terminal::menus::Response::Consumed => Ok(true),
        terminal::menus::Response::Prepare(action) => {
            match core.prepare_palette(&action.action, &action.context) {
                trailboss::core::Effect::Consumed => {}
                trailboss::core::Effect::Request { method, params } => {
                    input.request(boot_id, method, params)?
                }
                trailboss::core::Effect::Plugin => {
                    input.invoke(boot_id, &action.plugin, &action.action, &action.context)?
                }
            }
            Ok(true)
        }
        terminal::menus::Response::Invoke(action) => {
            match core.invoke(&action.action, &action.context, snapshot) {
                trailboss::core::Effect::Consumed => {}
                trailboss::core::Effect::Request { method, params } => {
                    input.request(boot_id, method, params)?;
                }
                trailboss::core::Effect::Plugin => {
                    input.invoke(boot_id, &action.plugin, &action.action, &action.context)?;
                }
            }
            Ok(true)
        }
    }
}
fn enable_default_producer() -> io::Result<()> {
    use trailboss_runtime::{
        api_client::{ApiClient, ConnectionTarget},
        api_schema::Request,
    };
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(
        trailboss_runtime::session::active_api_socket_path(),
    ));
    let request: Request = serde_json::from_value(
        serde_json::json!({"id":"stock-registry","method":"plugin.list","params":{}}),
    )
    .map_err(io::Error::other)?;
    let reply = api
        .request_value_with_timeout(&request, Duration::from_secs(5))
        .map_err(io::Error::other)?;
    if let Some(error) = reply.get("error") {
        return Err(io::Error::other(error.to_string()));
    }
    let packaged = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("executable has no parent directory"))?
        .join("plugins");
    let producers = if packaged.join("stock-ui/trailboss-plugin.toml").is_file() {
        packaged
    } else {
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/plugins"))
    };
    for (directory, plugin_id) in [
        ("stock-ui", "tommymorgan.stock-ui"),
        ("agent-tree", "tommymorgan.agent-tree"),
        ("sidebar-menus", "tommymorgan.sidebar-menus"),
    ] {
        let existing = reply["result"]["plugins"].as_array().and_then(|plugins| {
            plugins
                .iter()
                .find(|plugin| plugin["plugin_id"] == plugin_id)
        });
        if directory == "agent-tree" && existing.is_none() {
            continue;
        }
        let producer = std::fs::canonicalize(producers.join(directory))?;
        if existing.is_some_and(|plugin| {
            plugin["plugin_root"]
                .as_str()
                .is_some_and(|root| std::path::Path::new(root) == producer)
        }) {
            continue;
        }
        let enabled = existing
            .and_then(|plugin| plugin["enabled"].as_bool())
            .unwrap_or(true);
        let request: Request = serde_json::from_value(serde_json::json!({
            "id":"bundled-producer", "method":"plugin.link",
            "params":{"path":producer,"enabled":enabled}
        }))
        .map_err(io::Error::other)?;
        let linked = api
            .request_value_with_timeout(&request, Duration::from_secs(5))
            .map_err(io::Error::other)?;
        if let Some(error) = linked.get("error") {
            return Err(io::Error::other(error.to_string()));
        }
    }
    Ok(())
}
