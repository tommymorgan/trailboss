//! Narrow host geometry and native reply adapter for renderer-neutral clients.
use crate::protocol::{
    ClientMessage, ServerMessage, SurfaceGraphicsAssetKey, SurfaceGraphicsFormat,
    SurfaceGraphicsSource,
};
use std::{
    collections::VecDeque,
    io::{self, Read},
    os::fd::AsRawFd,
    sync::atomic::AtomicU64,
};
pub type Geometry = (u16, u16, u32, u32, bool);
pub struct Host {
    enabled: bool,
    reported: AtomicU64,
    geometry: Geometry,
    next_geometry_poll: std::time::Instant,
    framer: crate::raw_input::RawInputByteFramer,
    filter: super::direct_graphics::InputFilter,
    matcher: super::direct_graphics::ResponseMatcher,
    events: VecDeque<crossterm::event::Event>,
    replies: VecDeque<ClientMessage>,
}
impl Host {
    pub fn new(enabled: bool) -> io::Result<Self> {
        Ok(Self {
            enabled,
            reported: AtomicU64::new(0),
            geometry: super::terminal_geometry::initial_terminal_geometry(enabled, enabled)?,
            next_geometry_poll: std::time::Instant::now(),
            framer: crate::raw_input::RawInputByteFramer::for_host_input(),
            filter: Default::default(),
            matcher: Default::default(),
            events: Default::default(),
            replies: Default::default(),
        })
    }
    pub fn keyboard_enhancement_flags(&self) -> crossterm::event::KeyboardEnhancementFlags {
        crate::input::ime_compatible_keyboard_enhancement_flags()
    }
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }
    pub fn direct_allowed(&self, remote: bool) -> bool {
        self.enabled
            && super::handshake::direct_graphics_capability(
                !remote,
                self.geometry.4,
                (self.geometry.2, self.geometry.3),
                super::handshake::direct_graphics_profile_allowed(),
            )
    }
    pub fn query_geometry(&mut self, writer: impl io::Write) -> io::Result<()> {
        if super::terminal_geometry::host_cell_size_query_required(self.enabled) {
            self.framer.host_cell_size_query_sent();
            super::terminal_geometry::write_host_cell_size_query(writer)?;
        }
        Ok(())
    }
    pub fn refresh_geometry(&mut self) -> io::Result<bool> {
        let now = std::time::Instant::now();
        if now < self.next_geometry_poll {
            return Ok(false);
        }
        self.next_geometry_poll = now + std::time::Duration::from_millis(100);
        let next = super::terminal_geometry::current_terminal_geometry(
            self.enabled,
            self.enabled,
            &self.reported,
            Some((self.geometry.2, self.geometry.3)),
        )?;
        let changed = next != self.geometry;
        if (next.0, next.1) != (self.geometry.0, self.geometry.1) {
            self.events
                .push_back(crossterm::event::Event::Resize(next.0, next.1));
        }
        self.geometry = next;
        Ok(changed)
    }
    pub fn take_reply(&mut self) -> Option<ClientMessage> {
        self.replies.pop_front()
    }
    pub fn retire(&mut self, transfer_id: u64) {
        self.matcher.retire(transfer_id);
    }
    pub fn reset(&mut self) {
        self.matcher = Default::default();
        self.filter = Default::default();
        self.replies.clear();
    }
    pub fn start_file(
        &mut self,
        message: &ServerMessage,
        allowed: bool,
        accepts: impl FnOnce(&SurfaceGraphicsAssetKey, u32) -> bool,
        mut writer: impl io::Write,
    ) -> io::Result<bool> {
        let ServerMessage::GraphicsFile {
            path,
            expected_len,
            image_id,
            transfer_id,
            leading,
            control,
            surface_asset,
        } = message
        else {
            return Ok(false);
        };
        let native = surface_asset
            .as_ref()
            .is_some_and(|asset| matches!(asset.source, SurfaceGraphicsSource::Terminal { .. }));
        let valid = allowed
            && surface_asset
                .as_ref()
                .is_some_and(|asset| accepts(asset, *image_id))
            && (!native
                || surface_asset.as_ref().is_some_and(|asset| {
                    leading.is_empty()
                        && asset.format == SurfaceGraphicsFormat::Rgba
                        && *expected_len == asset.data_len
                        && *control
                            == format!(
                                "a=t,f=32,s={},v={},i={},q=0",
                                asset.image_width, asset.image_height, image_id
                            )
                }))
            && usize::try_from(*expected_len).ok().is_some_and(|len| {
                (crate::pane_graphics_files::validate_direct_source(
                    std::path::Path::new(path),
                    len,
                )
                .is_ok()
                    || (native
                        && crate::pane_graphics_files::validate_native_source(
                            std::path::Path::new(path),
                            len,
                        )
                        .is_ok()))
                    && super::direct_graphics::valid_control(control, *image_id, len)
            })
            && self.matcher.arm(*transfer_id, *image_id);
        if !valid {
            return Ok(false);
        }
        let mut bytes = Vec::new();
        crate::kitty_graphics::encode_kitty_regular_file(&mut bytes, leading, control, path);
        if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
            self.matcher.retire(*transfer_id);
            return Err(error);
        }
        self.matcher.start(*transfer_id);
        Ok(true)
    }
    pub fn poll(
        &mut self,
        timeout: std::time::Duration,
    ) -> io::Result<Option<crossterm::event::Event>> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        if crate::platform::poll_fd_readable(
            reader.as_raw_fd(),
            timeout.as_millis().min(i32::MAX as u128) as i32,
        )? {
            let mut scratch = [0; 4096];
            let n = reader.read(&mut scratch)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "terminal input closed",
                ));
            }
            let (chunks, responses) = self.filter.push(&scratch[..n], &mut self.matcher);
            for response in responses {
                self.replies
                    .push_back(ClientMessage::GraphicsTransmissionResult {
                        transfer_id: response.transfer_id,
                        image_id: response.image_id,
                        success: response.success,
                    });
            }
            for chunk in chunks {
                let framed = self.framer.push(&chunk);
                self.decode(framed);
            }
        } else {
            if let Some(bytes) = self.filter.flush_if_inactive(&mut self.matcher) {
                let chunks = self.framer.push(&bytes);
                self.decode(chunks);
            }
            let chunks = self.framer.flush_timeout();
            self.decode(chunks);
        }
        Ok(self.events.pop_front())
    }
    fn decode(&mut self, chunks: Vec<Vec<u8>>) {
        use crate::raw_input::RawInputEvent;
        for chunk in chunks {
            for event in crate::raw_input::parse_framed_input(&chunk) {
                let event = match event {
                    RawInputEvent::Key(key) => {
                        crossterm::event::Event::Key(crossterm::event::KeyEvent::new_with_kind(
                            key.code,
                            key.modifiers,
                            key.kind,
                        ))
                    }
                    RawInputEvent::Paste(text) => crossterm::event::Event::Paste(text),
                    RawInputEvent::Text(text) => crossterm::event::Event::Paste(text.into_string()),
                    RawInputEvent::Mouse(mouse) => crossterm::event::Event::Mouse(mouse),
                    RawInputEvent::HostCellSizeReport {
                        width_px,
                        height_px,
                    } => {
                        super::terminal_geometry::store_reported_cell_size(
                            &self.reported,
                            width_px,
                            height_px,
                        );
                        continue;
                    }
                    RawInputEvent::OuterFocusGained => crossterm::event::Event::FocusGained,
                    RawInputEvent::OuterFocusLost => crossterm::event::Event::FocusLost,
                    _ => continue,
                };
                self.events.push_back(event);
            }
        }
    }
}
