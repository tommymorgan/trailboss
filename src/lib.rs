use std::{io, os::unix::net::UnixStream, path::Path};
pub use trailboss_runtime::core_actions;
use trailboss_runtime::protocol::{
    self, ClientMessage, ClientPaneInputEvent, ClientSurfaceSize, ServerMessage,
};
pub mod core;
pub mod sidebar;
pub mod terminal;
#[derive(Debug)]
pub struct Cell {
    pub symbol: String,
    pub foreground: u32,
    pub background: u32,
    pub modifiers: u16,
    /// Index into the Surface hyperlink table.
    pub hyperlink: Option<u32>,
}
#[derive(Debug)]
pub struct Surface {
    pub pane_id: String,
    pub width: u16,
    pub height: u16,
    pub cells: Vec<Cell>,
    /// Sanitized OSC 8 targets shared with retained cells without copying URI bytes.
    pub hyperlinks: Vec<Option<std::sync::Arc<str>>>,
    pub graphics: std::sync::Arc<protocol::SurfaceGraphicsScene>,
    pub graphics_scope: String,
    pub cursor: Option<(u16, u16)>,
    pub focused: bool,
    pub mouse_reporting: bool,
}
impl Surface {
    pub fn text(&self) -> String {
        let mut text = String::new();
        for (row_index, row) in self.cells.chunks(usize::from(self.width)).enumerate() {
            if row_index > 0 {
                text.push('\n');
            }
            for cell in row {
                text.push_str(&cell.symbol);
            }
        }
        text
    }
}

pub struct Client {
    stream: UnixStream,
    received_messages: u64,
    received_surface_frames: u64,
    last_message_kind: &'static str,
    raw: Option<protocol::PaneSurfaceFrame>,
    pub graphics_messages: Vec<ServerMessage>,
    pub endpoint_response: Option<ServerMessage>,
    pub surface_changed: bool,
    pub workspace_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub boot_id: String,
    pub declarations: Option<Vec<(String, protocol::widgets::Declaration)>>,
    pub primary: Option<Surface>,
    pub snapshot: Option<protocol::ClientShellSnapshot>,
    pub command_ui: Option<protocol::endpoint::EndpointPluginUi>,
    pending_ui: Option<protocol::endpoint::EndpointPluginUi>,
    projection_pair: Option<(String, u64)>,
    argument_capability: bool,
}
fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
impl Client {
    pub fn attach(path: &Path, cols: u16, rows: u16) -> io::Result<Self> {
        Self::attach_graphics(path, cols, rows, (0, 0), false, false)
    }
    pub fn attach_graphics(
        path: &Path,
        cols: u16,
        rows: u16,
        cell_size: (u32, u32),
        exact_cell_size: bool,
        direct_graphics: bool,
    ) -> io::Result<Self> {
        let mut stream = UnixStream::connect(path).inspect_err(|failure| {
            tracing::debug!(
                event = "native_client_attach_failed",
                phase = "socket_connect",
                error_kind = ?failure.kind(),
                raw_os_error = ?failure.raw_os_error(),
                "native client socket connection failed"
            );
        })?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .inspect_err(|failure| {
                tracing::error!(
                    event = "native_client_attach_failed",
                    phase = "read_timeout",
                    error_kind = ?failure.kind(),
                    raw_os_error = ?failure.raw_os_error(),
                    "native client socket setup failed"
                );
            })?;
        use protocol::endpoint::*;
        let hello = EndpointClientHello {
            generation: ENDPOINT_PROTOCOL_GENERATION,
            cell_width_px: cell_size.0,
            cell_height_px: cell_size.1,
            surface_size: ClientSurfaceSize { cols, rows },
            pixel_mouse: exact_cell_size && cfg!(unix),
            direct_graphics,
            endpoint_keybindings: false,
            mouse_capture: false,
            surface_active: true,
            surface_reuse: false,
            surface_delta: false,
            surface_scroll: false,
            snapshot_codecs: vec![SNAPSHOT_CODEC_V1.into()],
            surface_codecs: vec![SURFACE_CODEC_V1.into()],
            input_codecs: vec![INPUT_CODEC_V1.into()],
            blob_codecs: vec![BLOB_CODEC_V1.into()],
        };
        protocol::write_message(
            &mut stream,
            &ClientMessage::EndpointControl {
                kind: ENDPOINT_HELLO_KIND.into(),
                data: serde_json::to_string(&hello).map_err(error)?,
            },
        )
        .inspect_err(|failure| {
            let (failure_kind, io_kind, raw_os_error) = match failure {
                protocol::FramingError::UnexpectedEof => ("unexpected_eof", None, None),
                protocol::FramingError::Io(error) => {
                    ("io", Some(error.kind()), error.raw_os_error())
                }
                protocol::FramingError::Oversized { .. } => ("oversized", None, None),
                protocol::FramingError::Bincode(_) => ("encode", None, None),
            };
            tracing::error!(
                event = "native_client_attach_failed",
                phase = "hello_write",
                failure_kind,
                ?io_kind,
                ?raw_os_error,
                "native client hello write failed (payload omitted)"
            );
        })
        .map_err(error)?;
        Ok(Self {
            stream,
            received_messages: 0,
            received_surface_frames: 0,
            last_message_kind: "none",
            raw: None,
            graphics_messages: Vec::new(),
            endpoint_response: None,
            surface_changed: false,
            workspace_id: None,
            focused_pane_id: None,
            focused_tab_id: None,
            command_ui: None,
            pending_ui: None,
            projection_pair: None,
            argument_capability: false,
            boot_id: String::new(),
            declarations: None,
            primary: None,
            snapshot: None,
        })
    }
    pub fn receive(&mut self) -> io::Result<()> {
        let message: ServerMessage =
            match protocol::read_message(&mut self.stream, protocol::MAX_GRAPHICS_FRAME_SIZE) {
                Ok(message) => message,
                Err(failure) => {
                    let (failure_kind, io_kind, raw_os_error) = match &failure {
                        protocol::FramingError::UnexpectedEof => ("unexpected_eof", None, None),
                        protocol::FramingError::Io(error) => {
                            ("io", Some(error.kind()), error.raw_os_error())
                        }
                        protocol::FramingError::Oversized { .. } => ("oversized", None, None),
                        protocol::FramingError::Bincode(_) => ("decode", None, None),
                    };
                    tracing::error!(
                        event = "native_client_receive_failed",
                        phase = "read_frame",
                        failure_kind,
                        ?io_kind,
                        ?raw_os_error,
                        received_messages = self.received_messages,
                        received_surface_frames = self.received_surface_frames,
                        last_message_kind = self.last_message_kind,
                        "native client frame receive failed (payload omitted)"
                    );
                    return Err(error(failure));
                }
            };
        self.received_messages = self.received_messages.saturating_add(1);
        self.last_message_kind = match &message {
            ServerMessage::Welcome { .. } => "welcome",
            ServerMessage::Terminal(_) => "terminal",
            ServerMessage::Graphics { .. } => "graphics",
            ServerMessage::ServerShutdown { .. } => "server_shutdown",
            ServerMessage::Notify { .. } => "notify",
            ServerMessage::Clipboard { .. } => "clipboard",
            ServerMessage::WindowTitle { .. } => "window_title",
            ServerMessage::ReloadSoundConfig => "reload_sound_config",
            ServerMessage::MouseCapture { .. } => "mouse_capture",
            ServerMessage::TerminalBell { .. } => "terminal_bell",
            ServerMessage::GraphicsFile { .. } => "graphics_file",
            ServerMessage::GraphicsTransmissionRetired { .. } => "graphics_transmission_retired",
            ServerMessage::ClientShellSnapshot(_) => "client_shell_snapshot",
            ServerMessage::PaneSurface(_) => "pane_surface",
            ServerMessage::SemanticNotification(_) => "semantic_notification",
            ServerMessage::ClientShellError { .. } => "client_shell_error",
            ServerMessage::DirectTerminalKeyboardProtocol { .. } => {
                "direct_terminal_keyboard_protocol"
            }
            ServerMessage::ClientShellKeyboardReportAll { .. } => {
                "client_shell_keyboard_report_all"
            }
            ServerMessage::ClientShellEndpointResponseChunk { .. } => {
                "client_shell_endpoint_response_chunk"
            }
            ServerMessage::PaneSurfacePatch(_) => "pane_surface_patch",
            ServerMessage::EndpointControl { kind, .. }
                if kind == protocol::endpoint::PLUGIN_UI_KIND =>
            {
                "endpoint_plugin_ui"
            }
            ServerMessage::EndpointControl { kind, .. }
                if kind == protocol::endpoint::ENDPOINT_SNAPSHOT_KIND =>
            {
                "endpoint_snapshot"
            }
            ServerMessage::EndpointControl { .. } => "endpoint_control",
        };
        if matches!(
            &message,
            ServerMessage::PaneSurface(_) | ServerMessage::PaneSurfacePatch(_)
        ) {
            self.received_surface_frames = self.received_surface_frames.saturating_add(1);
        }
        self.apply_message(message).inspect_err(|failure| {
            tracing::error!(
                event = "native_client_receive_failed",
                phase = "apply_message",
                error_kind = ?failure.kind(),
                raw_os_error = ?failure.raw_os_error(),
                received_messages = self.received_messages,
                received_surface_frames = self.received_surface_frames,
                last_message_kind = self.last_message_kind,
                "native client message processing failed (payload and error text omitted)"
            );
        })
    }
    fn apply_message(&mut self, message: ServerMessage) -> io::Result<()> {
        match message {
            ServerMessage::ClientShellSnapshot(s) => {
                trailboss_runtime::render_prof::trace(
                    "client.snapshot.receive",
                    format_args!("revision={} pane={:?}", s.revision, s.focused_pane_id),
                );
                self.reset_boot(&s.boot_id);
                self.boot_id = s.boot_id;
                self.focused_tab_id = s.focused_tab_id;
                self.workspace_id = s.focused_workspace_id;
                self.focused_pane_id = s.focused_pane_id;
            }
            ServerMessage::PaneSurface(s) => {
                if trailboss_runtime::render_prof::trace_enabled() {
                    trailboss_runtime::render_prof::trace(
                        "client.surface.receive",
                        format_args!(
                            "revision={} surface_revision={} pane={:?}",
                            s.projection_revision,
                            s.surface_revision,
                            s.panes.iter().find(|p| p.focused).map(|p| &p.pane_id)
                        ),
                    );
                }
                self.reset_boot(&s.boot_id);
                self.boot_id.clone_from(&s.boot_id);
                self.raw = Some(s);
                self.project();
            }
            ServerMessage::PaneSurfacePatch(patch) => {
                let s = self
                    .raw
                    .as_mut()
                    .ok_or_else(|| error("patch without Surface"))?;
                if s.boot_id != patch.boot_id
                    || s.projection_revision != patch.projection_revision
                    || s.surface_revision != patch.base_surface_revision
                {
                    return Err(error("Surface revision mismatch"));
                }
                for row in patch.rows {
                    let start =
                        usize::from(row.y) * usize::from(s.frame.width) + usize::from(row.x);
                    for (target, cell) in s.frame.cells[start..start + row.cells.len()]
                        .iter_mut()
                        .zip(row.cells)
                    {
                        *target = cell;
                    }
                }
                for pane in patch.panes {
                    if let Some(target) = s.panes.iter_mut().find(|p| p.pane_id == pane.pane_id) {
                        *target = pane;
                    }
                }
                s.surface_revision = patch.surface_revision;
                s.frame.cursor = patch.cursor;
                self.project();
            }
            ServerMessage::EndpointControl { kind, data }
                if kind == protocol::endpoint::ENDPOINT_WELCOME_KIND =>
            {
                let welcome: protocol::endpoint::EndpointServerWelcome =
                    serde_json::from_str(&data).map_err(error)?;
                self.argument_capability = welcome.capabilities.iter().any(|capability| {
                    capability == protocol::endpoint::PLUGIN_ACTION_ARGUMENTS_CAPABILITY
                });
            }
            ServerMessage::EndpointControl { kind, data }
                if kind == protocol::endpoint::PLUGIN_UI_KIND =>
            {
                let ui: protocol::endpoint::EndpointPluginUi =
                    serde_json::from_str(&data).map_err(error)?;
                trailboss_runtime::render_prof::trace(
                    "client.declarations.receive",
                    format_args!("revision={}", ui.revision),
                );
                self.pending_ui = Some(ui);
            }
            ServerMessage::EndpointControl { kind, data }
                if kind == protocol::endpoint::ENDPOINT_SNAPSHOT_KIND =>
            {
                let s: protocol::ClientShellSnapshot =
                    serde_json::from_str(&data).map_err(error)?;
                trailboss_runtime::render_prof::trace(
                    "client.snapshot.receive",
                    format_args!("revision={} pane={:?}", s.revision, s.focused_pane_id),
                );
                self.reset_boot(&s.boot_id);
                self.boot_id.clone_from(&s.boot_id);
                self.focused_tab_id.clone_from(&s.focused_tab_id);
                self.workspace_id.clone_from(&s.focused_workspace_id);
                self.focused_pane_id.clone_from(&s.focused_pane_id);
                self.projection_pair = Some((s.boot_id.clone(), s.revision));
                self.snapshot = Some(s);
            }
            message @ ServerMessage::ClientShellEndpointResponseChunk { .. }
                if matches!(&message, ServerMessage::ClientShellEndpointResponseChunk { request_id, .. }
                    if request_id.starts_with("tab-cycle-") || request_id.starts_with("tab-transfer-") || request_id.starts_with("widget-action-") || request_id.starts_with("argument-log-")) =>
            {
                self.endpoint_response = Some(message);
            }
            ServerMessage::Welcome { error: Some(e), .. }
            | ServerMessage::ClientShellError { message: e } => return Err(error(e)),
            message @ (ServerMessage::GraphicsFile { .. }
            | ServerMessage::GraphicsTransmissionRetired { .. }) => {
                self.graphics_messages.push(message)
            }
            ServerMessage::ServerShutdown { reason } => {
                return Err(error(reason.as_deref().unwrap_or("core stopped")))
            }
            _ => {}
        }
        if self.pending_ui.as_ref().is_some_and(|ui| {
            self.projection_pair
                .as_ref()
                .is_some_and(|(boot, revision)| *boot == ui.boot_id && *revision == ui.revision)
        }) {
            let mut ui = self.pending_ui.take().expect("paired projection");
            if !self.argument_capability {
                for item in &mut ui.ui {
                    item.command_inputs.clear();
                }
            }
            self.declarations = Some(
                ui.ui
                    .iter()
                    .filter_map(|item| {
                        item.declaration
                            .clone()
                            .map(|tree| (item.plugin_id.clone(), tree))
                    })
                    .collect(),
            );
            self.command_ui = Some(ui);
        }
        Ok(())
    }
    fn reset_boot(&mut self, boot: &str) {
        if !self.boot_id.is_empty() && self.boot_id != boot {
            self.raw = None;
            self.primary = None;
            self.surface_changed = true;
            self.graphics_messages.clear();
            self.command_ui = None;
            self.pending_ui = None;
            self.projection_pair = None;
        }
    }
    fn project(&mut self) {
        let Some(s) = self.raw.as_mut() else {
            return;
        };
        self.surface_changed = true;
        self.primary = None;
        if let Some(pane) = s.panes.iter().find(|p| p.focused) {
            let r = pane.inner_rect;
            let mut cells = Vec::with_capacity(usize::from(r.width) * usize::from(r.height));
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    let c = &s.frame.cells
                        [usize::from(y) * usize::from(s.frame.width) + usize::from(x)];
                    cells.push(Cell {
                        symbol: c.symbol.clone(),
                        foreground: c.fg,
                        background: c.bg,
                        modifiers: c.modifier,
                        hyperlink: c.hyperlink,
                    });
                }
            }
            let hyperlinks = s
                .frame
                .hyperlinks
                .iter()
                .map(|uri| {
                    let value: String = uri.chars().filter(|c| !c.is_control()).collect();
                    (!value.is_empty()).then(|| std::sync::Arc::from(value))
                })
                .collect();
            let graphics = protocol::SurfaceGraphicsScene {
                assets: s
                    .graphics
                    .assets
                    .iter_mut()
                    .map(|asset| protocol::SurfaceGraphicsAsset {
                        key: asset.key.clone(),
                        data: std::mem::take(&mut asset.data),
                    })
                    .collect(),
                placements: s
                    .graphics
                    .placements
                    .iter()
                    .filter_map(|placement| {
                        if !matches!(&placement.asset.source,
                        protocol::SurfaceGraphicsSource::Terminal {
                            target: protocol::SurfaceGraphicsTarget::Pane { pane_id }, ..
                        } if pane_id == &pane.pane_id)
                        {
                            return None;
                        }
                        let mut placement = placement.clone();
                        placement.x = placement.x.saturating_sub(r.x);
                        placement.y = placement.y.saturating_sub(r.y);
                        Some(placement)
                    })
                    .collect(),
                retained_assets: s.graphics.retained_assets.clone(),
            };
            self.primary = Some(Surface {
                pane_id: pane.pane_id.clone(),
                width: r.width,
                height: r.height,
                cells,
                hyperlinks,
                graphics: std::sync::Arc::new(graphics),
                graphics_scope: s.boot_id.clone(),
                cursor: s.frame.cursor.as_ref().and_then(|c| {
                    (c.x >= r.x && c.y >= r.y && c.x < r.x + r.width && c.y < r.y + r.height)
                        .then(|| (c.x - r.x, c.y - r.y))
                }),
                focused: pane.focused,
                mouse_reporting: pane.mouse_reporting,
            });
        }
    }
    pub fn type_text(&mut self, text: &str) -> io::Result<()> {
        let pane_id = self
            .focused_pane_id
            .clone()
            .ok_or_else(|| error("no focused pane"))?;
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellPaneInput {
                pane_id,
                events: vec![ClientPaneInputEvent::TextCommit(text.to_owned())],
            },
        )
        .map_err(error)
    }
    pub fn input(&self) -> io::Result<Input> {
        Ok(Input {
            stream: self.stream.try_clone()?,
            next_action_id: 0,
            pane_id: self.focused_pane_id.clone().unwrap_or_default(),
        })
    }
    pub fn blocking(&self) -> io::Result<()> {
        self.stream.set_read_timeout(None)
    }
}
pub struct Input {
    stream: UnixStream,
    pane_id: String,
    next_action_id: u64,
}
impl Input {
    pub fn dispatch(
        &mut self,
        core: &mut core::Core,
        boot_id: &str,
        plugin: &str,
        action: &str,
        context: &protocol::widgets::Context,
    ) -> io::Result<()> {
        match core.invoke(action, context, None) {
            core::Effect::Consumed => Ok(()),
            core::Effect::Request { method, params } => {
                self.dispatch_request(core, boot_id, method, params)
            }
            core::Effect::Plugin => self
                .invoke(boot_id, plugin, action, context, None)
                .map(|_| ()),
        }
    }
    pub fn dispatch_request(
        &mut self,
        core: &mut core::Core,
        boot_id: &str,
        method: &str,
        mut params: serde_json::Value,
    ) -> io::Result<()> {
        if method != "tab.focus" {
            return self.request(boot_id, method, params);
        }
        let Some(serde_json::Value::String(tab)) = params
            .as_object_mut()
            .and_then(|params| params.remove("tab_id"))
        else {
            return Err(error("tab focus target is missing"));
        };
        if let core::Effect::Request { params, .. } = core.request_tab_focus(tab) {
            self.request_tab_cycle(
                boot_id,
                core.tab_cycle_request_id().expect("focus request identity"),
                params,
            )?;
        }
        Ok(())
    }
    pub fn dispatch_text(
        &mut self,
        core: &mut core::Core,
        boot_id: &str,
        text: &str,
    ) -> io::Result<()> {
        match core.text(text) {
            Some(core::Effect::Request { method, params }) => self.request(boot_id, method, params),
            Some(_) => Ok(()),
            None => self.type_text(text),
        }
    }
    pub fn paste(&mut self, core: &mut core::Core, _boot_id: &str, text: &str) -> io::Result<()> {
        if text.is_empty() && self.paste_image(core)? {
            return Ok(());
        }
        match core.paste(text) {
            Some(_) => Ok(()),
            None => self.type_text(text),
        }
    }
    /// Stage a native clipboard image for the focused pane, if it accepts pane input.
    /// A false result leaves the caller responsible for ordinary key/text dispatch.
    pub fn paste_image(&mut self, core: &core::Core) -> io::Result<bool> {
        if !core.visible(protocol::widgets::Condition::HasPane)
            || core.editing()
            || self.pane_id.is_empty()
        {
            return Ok(false);
        }
        trailboss_runtime::client::paste_clipboard_image(
            &mut self.stream,
            protocol::ClientClipboardImageTarget::Pane(self.pane_id.clone()),
        )
    }
    pub fn focus_pane_target(&mut self, pane_id: &str) {
        if self.pane_id != pane_id {
            self.pane_id.clear();
            self.pane_id.push_str(pane_id);
        }
    }
    pub fn invoke(
        &mut self,
        boot_id: &str,
        plugin_id: &str,
        action: &str,
        context: &protocol::widgets::Context,
        arguments: Option<&str>,
    ) -> io::Result<u64> {
        self.next_action_id += 1;
        let mut request = serde_json::json!({ "id": format!("widget-action-{}", self.next_action_id), "method": "plugin.action.invoke", "params": {
            "plugin_id": plugin_id, "action_id": action, "context": {
                "workspace_id": context.workspace_id, "tab_id": context.tab_id, "pane_id": context.pane_id, "invocation_source": "plugin_ui"
            }
        }});
        if let Some(arguments) = arguments {
            request["params"]["arguments"] = serde_json::json!(arguments);
        }
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellEndpointRequest {
                boot_id: boot_id.to_owned(),
                request: request.to_string(),
            },
        )
        .map_err(error)?;
        Ok(self.next_action_id)
    }
    pub fn request(
        &mut self,
        boot_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> io::Result<()> {
        self.next_action_id += 1;
        let request = serde_json::json!({"id":format!("core-action-{}",self.next_action_id),"method":method,"params":params});
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellEndpointRequest {
                boot_id: boot_id.to_owned(),
                request: request.to_string(),
            },
        )
        .map_err(error)
    }
    pub fn request_argument_log(
        &mut self,
        boot_id: &str,
        params: serde_json::Value,
    ) -> io::Result<u64> {
        self.next_action_id += 1;
        let id = self.next_action_id;
        let request = serde_json::json!({
            "id":format!("argument-log-{id}"), "method":"plugin.log.list", "params":params,
        });
        self.send(&ClientMessage::ClientShellEndpointRequest {
            boot_id: boot_id.to_owned(),
            request: request.to_string(),
        })?;
        Ok(id)
    }
    pub fn request_tab_transfer(
        &mut self,
        boot_id: &str,
        params: serde_json::Value,
    ) -> io::Result<u64> {
        self.next_action_id += 1;
        let id = self.next_action_id;
        let request = serde_json::json!({
            "id": format!("tab-transfer-{id}"),
            "method": "tab.transfer",
            "params": params,
        });
        self.send(&ClientMessage::ClientShellEndpointRequest {
            boot_id: boot_id.to_owned(),
            request: request.to_string(),
        })?;
        Ok(id)
    }
    pub fn request_tab_cycle(
        &mut self,
        boot_id: &str,
        request_id: u64,
        params: serde_json::Value,
    ) -> io::Result<()> {
        let request = serde_json::json!({
            "id": format!("tab-cycle-{request_id}"),
            "method": "tab.focus",
            "params": params,
        });
        self.send(&ClientMessage::ClientShellEndpointRequest {
            boot_id: boot_id.to_owned(),
            request: request.to_string(),
        })
    }
    pub fn dispatch_key(
        &mut self,
        core: &mut core::Core,
        boot_id: &str,
        key: crossterm::event::KeyEvent,
    ) -> io::Result<()> {
        use crossterm::event::{KeyCode, KeyModifiers};
        if core.editing() {
            // These are dialog editing commands, never terminal escape encodings.
            let text = match key.code {
                KeyCode::Char(c) if (key.modifiers - KeyModifiers::SHIFT).is_empty() => {
                    c.to_string()
                }
                KeyCode::Enter if key.modifiers.is_empty() => "\r".into(),
                KeyCode::Esc if key.modifiers.is_empty() => "\x1b".into(),
                KeyCode::Backspace if key.modifiers.is_empty() => "\x7f".into(),
                _ => return Ok(()),
            };
            return self.dispatch_text(core, boot_id, &text);
        }
        if !core.visible(protocol::widgets::Condition::HasPane) {
            return Ok(());
        }
        match ClientPaneInputEvent::from_crossterm_key(key) {
            Some(event) => self.pane_event(event),
            None => Ok(()),
        }
    }
    pub fn wheel(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        rect: terminal::Rect,
        lines: usize,
    ) -> io::Result<()> {
        use crossterm::event::MouseEventKind;
        if !matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) || !rect.contains(mouse.column, mouse.row)
        {
            return Ok(());
        }
        let column = mouse.column - rect.x;
        let row = mouse.row - rect.y;
        let Some(kind) = protocol::ClientMouseKind::from_crossterm(mouse.kind) else {
            return Ok(());
        };
        self.pane_event(ClientPaneInputEvent::Mouse {
            kind,
            position: protocol::ClientMousePosition::Cell { column, row },
            geometry: None,
            modifiers: mouse.modifiers.bits(),
            lines: lines.min(u16::MAX as usize) as u16,
        })
    }
    pub fn mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        rect: terminal::Rect,
    ) -> io::Result<()> {
        if !rect.contains(mouse.column, mouse.row) {
            return Ok(());
        }
        let Some(kind) = protocol::ClientMouseKind::from_crossterm(mouse.kind) else {
            return Ok(());
        };
        self.pane_event(ClientPaneInputEvent::Mouse {
            kind,
            position: protocol::ClientMousePosition::Cell {
                column: mouse.column - rect.x,
                row: mouse.row - rect.y,
            },
            geometry: None,
            modifiers: mouse.modifiers.bits(),
            lines: 1,
        })
    }
    pub fn type_text(&mut self, text: &str) -> io::Result<()> {
        self.pane_event(ClientPaneInputEvent::TextCommit(text.to_owned()))
    }
    fn pane_event(&mut self, event: ClientPaneInputEvent) -> io::Result<()> {
        if self.pane_id.is_empty() {
            return Ok(());
        }
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellPaneInput {
                pane_id: self.pane_id.clone(),
                events: vec![event],
            },
        )
        .map_err(error)
    }
    pub fn send(&mut self, message: &ClientMessage) -> io::Result<()> {
        protocol::write_message(&mut self.stream, message).map_err(error)
    }
    pub fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        self.resize_graphics(cols, rows, (0, 0), false)
    }
    pub fn resize_graphics(
        &mut self,
        cols: u16,
        rows: u16,
        cell_size: (u32, u32),
        exact_cell_size: bool,
    ) -> io::Result<()> {
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellResize {
                cell_width_px: cell_size.0,
                cell_height_px: cell_size.1,
                surface_size: ClientSurfaceSize { cols, rows },
                pixel_mouse: exact_cell_size && cfg!(unix),
            },
        )
        .map_err(error)
    }
    pub fn detach(&mut self) -> io::Result<()> {
        protocol::write_message(&mut self.stream, &ClientMessage::Detach).map_err(error)?;
        self.stream.shutdown(std::net::Shutdown::Both)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn editing_and_cancel_emit_no_pane_input_until_the_next_ordinary_text() {
        for action in ["host:new", "host:rename-space"] {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            let mut input = Input {
                stream,
                pane_id: "pane".into(),
                next_action_id: 0,
            };
            let mut core = core::Core::default();
            core.update_session(Some("workspace".into()), Some("tab".into()), true);
            input
                .dispatch(
                    &mut core,
                    "boot",
                    "plugin",
                    action,
                    &protocol::widgets::Context {
                        workspace_id: Some("workspace".into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            input.dispatch_text(&mut core, "boot", "typed").unwrap();
            input.paste(&mut core, "boot", "pasted").unwrap();
            input.paste(&mut core, "boot", "\r").unwrap();
            input.paste(&mut core, "boot", "").unwrap();
            input.dispatch_text(&mut core, "boot", "\x1b").unwrap();
            peer.set_nonblocking(true).unwrap();
            let mut byte = [0];
            assert_eq!(
                peer.read(&mut byte).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            peer.set_nonblocking(false).unwrap();
            peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            input.dispatch_text(&mut core, "boot", "ordinary").unwrap();
            let message: ClientMessage =
                protocol::read_message(&mut peer, protocol::MAX_FRAME_SIZE).unwrap();
            let ClientMessage::ClientShellPaneInput { pane_id, events } = message else {
                panic!("ordinary text was not pane input");
            };
            assert_eq!(pane_id, "pane");
            assert!(
                matches!(events.as_slice(), [ClientPaneInputEvent::TextCommit(text)] if text == "ordinary")
            );
        }
    }
    #[test]
    fn terminal_keys_preserve_all_modifiers_kinds_and_target_on_the_native_wire() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut input = Input {
            stream,
            pane_id: "old".into(),
            next_action_id: 0,
        };
        input.focus_pane_target("live");
        let mut core = core::Core::default();
        core.update_session(Some("space".into()), Some("tab".into()), true);
        for code in [
            KeyCode::Char('界'),
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::BackTab,
            KeyCode::Backspace,
            KeyCode::F(12),
        ] {
            for modifiers in [
                KeyModifiers::NONE,
                KeyModifiers::SHIFT,
                KeyModifiers::ALT,
                KeyModifiers::CONTROL,
                KeyModifiers::SUPER,
                KeyModifiers::HYPER,
                KeyModifiers::META,
                KeyModifiers::all(),
            ] {
                for kind in [KeyEventKind::Press, KeyEventKind::Repeat] {
                    let key = KeyEvent::new_with_kind(code, modifiers, kind);
                    input.dispatch_key(&mut core, "boot", key).unwrap();
                    let message: ClientMessage =
                        protocol::read_message(&mut peer, protocol::MAX_FRAME_SIZE).unwrap();
                    let ClientMessage::ClientShellPaneInput { pane_id, events } = message else {
                        panic!("key was not semantic pane input")
                    };
                    assert_eq!(pane_id, "live");
                    assert_eq!(
                        events,
                        vec![ClientPaneInputEvent::from_crossterm_key(key).unwrap()]
                    );
                    assert!(
                        matches!(&events[0], ClientPaneInputEvent::Key { modifiers: actual, .. } if *actual == modifiers.bits())
                    );
                }
            }
        }
    }

    #[test]
    fn legacy_core_declarations_keep_visible_rows_and_original_action_targets() {
        let (mut client, mut peer) = hyperlink_client();
        let message = ServerMessage::EndpointControl {
            kind: protocol::endpoint::PLUGIN_UI_KIND.into(),
            data: serde_json::json!({
                "boot_id": "existing-core", "revision": 7,
                "ui": [{
                    "plugin_id": "existing.tree", "ui_id": "tree", "title": "",
                    "declaration": {
                        "herdr": "widget_tree", "version": 1,
                        "zones": [{"zone": "list", "tree": {
                            "type": "column", "children": [{
                                "type": "row", "action": "focus-tab",
                                "context": {"tab_id": "existing-tab"},
                                "children": [{"type": "label", "text": "Existing pane", "role": "label"}]
                            }]
                        }}]
                    }
                }]
            }).to_string(),
        };
        protocol::write_message(&mut peer, &message).unwrap();
        client.receive().unwrap();
        let declarations = client.declarations.as_ref().unwrap();
        let (plugin, action, context) = sidebar::action_at(declarations, 0).unwrap();
        assert_eq!(plugin, "existing.tree");
        assert_eq!(action, "focus-tab");
        assert_eq!(context.tab_id.as_deref(), Some("existing-tab"));
        let mut output = Vec::new();
        sidebar::draw(
            &mut output,
            declarations,
            terminal::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 4,
            },
            None,
        )
        .unwrap();
        assert!(String::from_utf8(output).unwrap().contains("Existing pane"));
    }

    fn hyperlink_client() -> (Client, UnixStream) {
        let (stream, peer) = UnixStream::pair().unwrap();
        (
            Client {
                stream,
                received_messages: 0,
                received_surface_frames: 0,
                last_message_kind: "none",
                raw: None,
                graphics_messages: Vec::new(),
                endpoint_response: None,
                surface_changed: false,
                workspace_id: None,
                focused_pane_id: None,
                focused_tab_id: None,
                boot_id: String::new(),
                declarations: None,
                primary: None,
                snapshot: None,
                command_ui: None,
                pending_ui: None,
                projection_pair: None,
                argument_capability: false,
            },
            peer,
        )
    }
    fn linked_native_frame(uri: &str) -> protocol::PaneSurfaceFrame {
        let inner = protocol::SurfaceRect {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        };
        let cells = "ABCDEFGH"
            .chars()
            .map(|ch| protocol::CellData {
                symbol: ch.to_string(),
                fg: 0,
                bg: 0,
                modifier: 0,
                skip: false,
                hyperlink: Some(0),
            })
            .collect();
        protocol::PaneSurfaceFrame {
            boot_id: "private-test".into(),
            projection_revision: 1,
            surface_revision: 1,
            frame: protocol::FrameData {
                width: 4,
                height: 2,
                cells,
                cursor: None,
                hyperlinks: vec![uri.into()],
                graphics: Vec::new(),
            },
            panes: vec![protocol::PaneSurfacePane {
                pane_id: "private-pane".into(),
                content_revision: 1,
                rect: inner,
                inner_rect: inner,
                scrollbar_rect: None,
                scroll: None,
                focused: true,
                mouse_reporting: false,
                sgr_pixel_mouse: false,
                alternate_screen_active: false,
                pixel_width: 0,
                pixel_height: 0,
            }],
            splits: Vec::new(),
            popup: None,
            graphics: Default::default(),
        }
    }
    fn hyperlink_declarations() -> sidebar::Declarations {
        vec![(
            "fixture".into(),
            serde_json::from_value(serde_json::json!({
                "herdr":"widget_tree", "version":1, "zones":[],
                "composition":{"priority":1,"tree":{"type":"column","children":[
                    {"type":"label","text":"Chrome","role":"label"},
                    {"type":"flex","weight":1,"child":{"type":"surface"}}
                ]}}
            }))
            .unwrap(),
        )]
    }
    fn paint_projected(
        client: &Client,
        renderer: &mut terminal::FrameRenderer,
        screen: &mut ghostty_vt::Terminal,
        rect: terminal::Rect,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        renderer
            .draw_frame(
                &mut bytes,
                client.primary.as_ref(),
                &hyperlink_declarations(),
                &core::Core::default(),
                rect,
            )
            .unwrap();
        screen.write(&bytes);
        bytes
    }
    #[test]
    fn native_targets_strip_controls_and_empty_or_invalid_targets_render_unlinked() {
        let (mut client, mut peer) = hyperlink_client();
        let mut native = linked_native_frame("https://example.test/a\x1b\x07\n\r\u{009c}b");
        native.frame.hyperlinks.push("\x1b\x07\n".into());
        native.frame.cells[1].hyperlink = Some(1);
        native.frame.cells[2].hyperlink = Some(99);
        protocol::write_message(&mut peer, &ServerMessage::PaneSurface(native)).unwrap();
        client.receive().unwrap();
        let mut renderer = terminal::FrameRenderer::default();
        let mut screen = ghostty_vt::Terminal::new(6, 4, 0).unwrap();
        let bytes = paint_projected(
            &client,
            &mut renderer,
            &mut screen,
            terminal::Rect::viewport(6, 4),
        );
        assert_eq!(
            screen.viewport_hyperlink_uri(0, 1).unwrap().as_deref(),
            Some("https://example.test/ab")
        );
        assert_eq!(screen.viewport_hyperlink_uri(1, 1).unwrap(), None);
        assert_eq!(screen.viewport_hyperlink_uri(2, 1).unwrap(), None);
        assert!(!bytes.contains(&7));
        screen.write(b"\x1b[4;1Hz");
        assert_eq!(screen.viewport_hyperlink_uri(0, 3).unwrap(), None);
    }
    #[test]
    fn native_surface_rows_survive_viewport_resize_without_local_reflow() {
        let (mut client, mut peer) = hyperlink_client();
        protocol::write_message(
            &mut peer,
            &ServerMessage::PaneSurface(linked_native_frame("https://example.test")),
        )
        .unwrap();
        client.receive().unwrap();
        let mut renderer = terminal::FrameRenderer::default();
        let mut screen = ghostty_vt::Terminal::new(6, 4, 0).unwrap();
        for width in [6, 3, 6] {
            screen.resize(width, 4, 0, 0).unwrap();
            paint_projected(
                &client,
                &mut renderer,
                &mut screen,
                terminal::Rect::viewport(width, 4),
            );
            assert_eq!(
                screen.read_text_viewport((0, 1), (0, 1), false).unwrap(),
                "A"
            );
            assert_eq!(
                screen.read_text_viewport((0, 2), (0, 2), false).unwrap(),
                "E"
            );
            assert_eq!(
                screen.read_text_viewport((2, 1), (2, 1), false).unwrap(),
                "C"
            );
            assert_eq!(
                screen.read_text_viewport((2, 2), (2, 2), false).unwrap(),
                "G"
            );
            assert_eq!(
                screen.viewport_hyperlink_uri(0, 2).unwrap().as_deref(),
                Some("https://example.test")
            );
        }
    }
}
