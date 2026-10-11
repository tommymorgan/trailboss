//! Renderer-neutral boundary around the original retained Kitty scene engine.
use crate::kitty_graphics::{
    surface::{ClientState, Occlusion, Visibility},
    GraphicsOperation, GraphicsOutput, HostCellSize,
};
#[cfg(unix)]
use crate::protocol::SurfaceGraphicsAssetKey;
use crate::protocol::SurfaceGraphicsScene;
use std::io::{self, Write};
use std::sync::Arc;

#[derive(Default)]
pub struct Renderer {
    state: ClientState,
    scene: Option<Arc<SurfaceGraphicsScene>>,
    files: super::image_files::FileTransport,
    enabled: bool,
    cell_size: HostCellSize,
    cleanup_on_drop: bool,
    failed_ids: Vec<u32>,
}
impl Renderer {
    pub fn from_environment() -> Self {
        let mut renderer = Self::default();
        renderer.files = super::image_files::FileTransport::from_environment();
        renderer.cleanup_on_drop = true;
        renderer
    }
    pub fn configure(&mut self, enabled: bool, cell_size: (u32, u32)) {
        self.enabled = enabled;
        self.cell_size = HostCellSize {
            width_px: cell_size.0,
            height_px: cell_size.1,
        };
    }
    pub fn set_scene(&mut self, scope: &str, scene: Arc<SurfaceGraphicsScene>) {
        let changed_scope = self.state.scope() != scope;
        self.state.set_scope(scope);
        if changed_scope
            || self
                .scene
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, &scene))
        {
            self.state.set_shared_scene(&scene);
            self.scene = Some(scene);
        }
    }
    pub fn invalidate_output(&mut self) {
        self.state.invalidate_output();
    }
    #[cfg(unix)]
    pub fn accepts_direct_asset(&self, key: &SurfaceGraphicsAssetKey, image_id: u32) -> bool {
        self.state.accepts_direct_asset(key, image_id)
    }
    #[cfg(unix)]
    pub fn trust_direct_asset(&mut self, key: &SurfaceGraphicsAssetKey, image_id: u32) -> bool {
        self.state.trust_direct_asset(key, image_id)
    }
    #[cfg(unix)]
    pub fn retire_direct_image(&mut self, image_id: u32) {
        self.state.retire_direct_image(image_id);
    }
    pub fn write_frame(
        &mut self,
        writer: &mut impl Write,
        encoded: &[u8],
        origin: Option<(u16, u16)>,
        clip: (u16, u16),
        covers: &[(u16, u16, u16, u16)],
    ) -> io::Result<()> {
        let mut occlusion = Occlusion::default();
        for &(x, y, width, height) in covers {
            occlusion.cover(ratatui::layout::Rect::new(x, y, width, height));
        }
        let (x, y) = origin.unwrap_or_default();
        // Crop the old pane scene immediately when declaration geometry shrinks,
        // while waiting for the endpoint's resized scene.
        let right = x.saturating_add(clip.0);
        let bottom = y.saturating_add(clip.1);
        occlusion.cover(ratatui::layout::Rect::new(
            right,
            y,
            u16::MAX - right,
            u16::MAX - y,
        ));
        occlusion.cover(ratatui::layout::Rect::new(
            x,
            bottom,
            u16::MAX - x,
            u16::MAX - bottom,
        ));
        let visibility = if self.enabled && origin.is_some() {
            Visibility::Main
        } else {
            Visibility::Hidden
        };
        if matches!(visibility, Visibility::Hidden) {
            self.state.invalidate_output();
        }
        let mut graphics =
            self.state
                .encode_output(visibility, (x, y), None, self.cell_size, &occlusion);
        if !self.failed_ids.is_empty() {
            let mut recovery = Vec::new();
            for &id in &self.failed_ids {
                crate::kitty_graphics::encode_delete_image(&mut recovery, id);
            }
            let mut recovered = GraphicsOutput::from_bytes(recovery);
            recovered.extend(graphics);
            graphics = recovered;
        }
        let result = super::frame_output::write_composed_frame(
            &mut *writer,
            encoded,
            &graphics,
            &mut self.files,
        )
        .and_then(|()| writer.flush());
        if result.is_err() {
            self.state.invalidate_output();
            for operation in &graphics.operations {
                match operation {
                    GraphicsOperation::Bytes(bytes) => self
                        .failed_ids
                        .extend(super::frame_output::kitty_graphics_image_ids(bytes)),
                    GraphicsOperation::Upload { control, .. } => {
                        self.failed_ids
                            .extend(control.split(',').filter_map(|part| {
                                part.strip_prefix("i=")
                                    .and_then(|id| id.parse::<u32>().ok())
                            }));
                    }
                }
            }
            self.failed_ids.sort_unstable();
            self.failed_ids.dedup();
        } else {
            self.failed_ids.clear();
        }
        result
    }
    pub fn cleanup(&mut self, writer: &mut impl Write) -> io::Result<()> {
        let checkpoint = self.state.clone();
        self.state.set_scope("");
        let mut cleanup = self.state.take_pending_cleanup();
        for &id in &self.failed_ids {
            crate::kitty_graphics::encode_delete_image(&mut cleanup, id);
        }
        let result =
            super::frame_output::write_encoded_frame_with_graphics(&mut *writer, b"", &cleanup)
                .and_then(|()| writer.flush());
        if result.is_err() {
            self.state = checkpoint;
            self.state.invalidate_output();
        } else {
            self.scene = None;
            self.failed_ids.clear();
        }
        result
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = self.cleanup(&mut io::stdout().lock());
        }
    }
}
