use herdr_runtime::protocol::{
    self, ClientMessage, ClientPaneInputEvent, ClientSurfaceSize, ServerMessage,
};
use std::{io, os::unix::net::UnixStream, path::Path};
pub mod core;
pub mod sidebar;
pub mod terminal;
#[derive(Debug)]
pub struct Cell {
    pub symbol: String,
    pub foreground: u32,
    pub background: u32,
    pub modifiers: u16,
}
#[derive(Debug)]
pub struct Surface {
    pub pane_id: String,
    pub width: u16,
    pub height: u16,
    pub cells: Vec<Cell>,
    pub cursor: Option<(u16, u16)>,
    pub focused: bool,
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
    raw: Option<protocol::PaneSurfaceFrame>,
    pub workspace_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub boot_id: String,
    pub declarations: Option<Vec<(String, protocol::widgets::Declaration)>>,
    pub primary: Option<Surface>,
}
fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
impl Client {
    pub fn attach(path: &Path, cols: u16, rows: u16) -> io::Result<Self> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        use protocol::endpoint::*;
        let hello = EndpointClientHello {
            generation: ENDPOINT_PROTOCOL_GENERATION,
            cell_width_px: 0,
            cell_height_px: 0,
            surface_size: ClientSurfaceSize { cols, rows },
            pixel_mouse: false,
            direct_graphics: false,
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
        .map_err(error)?;
        Ok(Self {
            stream,
            raw: None,
            workspace_id: None,
            focused_pane_id: None,
            focused_tab_id: None,
            boot_id: String::new(),
            declarations: None,
            primary: None,
        })
    }
    pub fn receive(&mut self) -> io::Result<()> {
        let message: ServerMessage =
            protocol::read_message(&mut self.stream, protocol::MAX_FRAME_SIZE).map_err(error)?;
        match message {
            ServerMessage::ClientShellSnapshot(s) => {
                self.boot_id = s.boot_id;
                self.focused_tab_id = s.focused_tab_id;
                self.workspace_id = s.focused_workspace_id;
                self.focused_pane_id = s.focused_pane_id;
            }
            ServerMessage::PaneSurface(s) => {
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
                if kind == protocol::endpoint::PLUGIN_UI_KIND =>
            {
                let ui: protocol::endpoint::EndpointPluginUi =
                    serde_json::from_str(&data).map_err(error)?;
                self.declarations = Some(
                    ui.ui
                        .into_iter()
                        .filter_map(|item| item.declaration.map(|tree| (item.plugin_id, tree)))
                        .collect(),
                );
            }
            ServerMessage::EndpointControl { kind, data }
                if kind == protocol::endpoint::ENDPOINT_SNAPSHOT_KIND =>
            {
                let s: protocol::ClientShellSnapshot =
                    serde_json::from_str(&data).map_err(error)?;
                self.boot_id = s.boot_id;
                self.focused_tab_id = s.focused_tab_id;
                self.workspace_id = s.focused_workspace_id;
                self.focused_pane_id = s.focused_pane_id;
            }
            ServerMessage::Welcome { error: Some(e), .. }
            | ServerMessage::ClientShellError { message: e } => return Err(error(e)),
            ServerMessage::ServerShutdown { .. } => return Err(error("core stopped")),
            _ => {}
        }
        Ok(())
    }
    fn project(&mut self) {
        let Some(s) = self.raw.as_ref() else {
            return;
        };
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
                    });
                }
            }
            self.primary = Some(Surface {
                pane_id: pane.pane_id.clone(),
                width: r.width,
                height: r.height,
                cells,
                cursor: s.frame.cursor.as_ref().and_then(|c| {
                    (c.x >= r.x && c.y >= r.y && c.x < r.x + r.width && c.y < r.y + r.height)
                        .then(|| (c.x - r.x, c.y - r.y))
                }),
                focused: pane.focused,
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
        match core.invoke(action, context) {
            core::Effect::Consumed => Ok(()),
            core::Effect::Request { method, params } => self.request(boot_id, method, params),
            core::Effect::Plugin => self.invoke(boot_id, plugin, action, context),
        }
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
    pub fn paste(&mut self, core: &mut core::Core, boot_id: &str, text: &str) -> io::Result<()> {
        if text.is_empty()
            && core.visible(protocol::widgets::Condition::HasPane)
            && !core.visible(protocol::widgets::Condition::NewTab)
            && !core.visible(protocol::widgets::Condition::Menu)
            && !self.pane_id.is_empty()
            && herdr_runtime::client::paste_clipboard_image(
                &mut self.stream,
                protocol::ClientClipboardImageTarget::Pane(self.pane_id.clone()),
            )?
        {
            return Ok(());
        }
        self.dispatch_text(core, boot_id, text)
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
    ) -> io::Result<()> {
        self.next_action_id += 1;
        let request = serde_json::json!({ "id": format!("widget-action-{}", self.next_action_id), "method": "plugin.action.invoke", "params": {
            "plugin_id": plugin_id, "action_id": action, "context": {
                "workspace_id": context.workspace_id, "tab_id": context.tab_id, "pane_id": context.pane_id, "invocation_source": "plugin_ui"
            }
        }});
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellEndpointRequest {
                boot_id: boot_id.to_owned(),
                request: request.to_string(),
            },
        )
        .map_err(error)
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
    pub fn type_text(&mut self, text: &str) -> io::Result<()> {
        if self.pane_id.is_empty() {
            return Ok(());
        }
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellPaneInput {
                pane_id: self.pane_id.clone(),
                events: vec![ClientPaneInputEvent::TextCommit(text.to_owned())],
            },
        )
        .map_err(error)
    }
    pub fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        protocol::write_message(
            &mut self.stream,
            &ClientMessage::ClientShellResize {
                cell_width_px: 0,
                cell_height_px: 0,
                surface_size: ClientSurfaceSize { cols, rows },
                pixel_mouse: false,
            },
        )
        .map_err(error)
    }
    pub fn detach(&mut self) -> io::Result<()> {
        protocol::write_message(&mut self.stream, &ClientMessage::Detach).map_err(error)?;
        self.stream.shutdown(std::net::Shutdown::Both)
    }
}
