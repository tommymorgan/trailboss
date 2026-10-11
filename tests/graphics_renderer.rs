use std::{
    io::{self, Write},
    sync::Arc,
};
use trailboss::{
    core::Core,
    sidebar::Declarations,
    terminal::{FrameRenderer, Rect},
    Cell, Surface,
};
use trailboss_runtime::{client::graphics::Renderer, protocol::*};

fn scene() -> Arc<SurfaceGraphicsScene> {
    let key = SurfaceGraphicsAssetKey {
        source: SurfaceGraphicsSource::Terminal {
            target: SurfaceGraphicsTarget::Pane {
                pane_id: "pane".into(),
            },
            image_id: 1,
        },
        image_width: 8,
        image_height: 4,
        format: SurfaceGraphicsFormat::Rgba,
        data_len: 128,
        data_fingerprint: 1,
    };
    Arc::new(SurfaceGraphicsScene {
        assets: vec![SurfaceGraphicsAsset {
            key: key.clone(),
            data: [255, 0, 0, 255].repeat(32),
        }],
        placements: vec![SurfaceGraphicsPlacement {
            asset: key,
            logical_placement_id: 1,
            x: 0,
            y: 0,
            cols: 4,
            rows: 2,
            source_x: 0,
            source_y: 0,
            source_width: 8,
            source_height: 4,
            x_offset: 0,
            y_offset: 0,
            z: 1,
            scrollback_offset: 0,
        }],
        retained_assets: Vec::new(),
    })
}
fn screen() -> ghostty_vt::Terminal {
    let mut terminal = ghostty_vt::Terminal::new(20, 8, 0).unwrap();
    terminal.enable_kitty_graphics().unwrap();
    terminal.resize(20, 8, 2, 2).unwrap();
    terminal
}
fn output(
    renderer: &mut Renderer,
    terminal: &mut ghostty_vt::Terminal,
    origin: Option<(u16, u16)>,
    clip: (u16, u16),
    covers: &[(u16, u16, u16, u16)],
) {
    let mut bytes = Vec::new();
    renderer
        .write_frame(
            &mut bytes,
            b"\x1b[?2026h\x1b[5;9H\x1b[?2026l",
            origin,
            clip,
            covers,
        )
        .unwrap();
    terminal.write(&bytes);
}
#[test]
fn original_engine_preserves_cursor_and_pixels_across_metadata_only_patches() {
    let mut renderer = Renderer::default();
    renderer.configure(true, (2, 2));
    let original = scene();
    renderer.set_scene("boot-a/connection", Arc::clone(&original));
    let mut terminal = screen();
    output(&mut renderer, &mut terminal, Some((3, 1)), (10, 6), &[]);
    let first = terminal.kitty_image_placements().unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(
        (first[0].render.viewport_col, first[0].render.viewport_row),
        (3, 1)
    );
    assert_eq!(first[0].data, original.assets[0].data);
    assert_eq!(terminal.cursor_y().unwrap(), 4);
    let mut metadata = SurfaceGraphicsScene {
        assets: Vec::new(),
        placements: original.placements.clone(),
        retained_assets: Vec::new(),
    };
    metadata.placements[0].x = 1;
    renderer.set_scene("boot-a/connection", Arc::new(metadata));
    output(&mut renderer, &mut terminal, Some((3, 1)), (10, 6), &[]);
    let next = terminal.kitty_image_placements().unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].image_id, first[0].image_id);
    assert_eq!(next[0].render.viewport_col, 4);
    assert_eq!(next[0].data, first[0].data);
}
#[test]
fn overlays_and_shrinking_surface_crop_images_without_losing_assets() {
    let mut renderer = Renderer::default();
    renderer.configure(true, (2, 2));
    renderer.set_scene("scope", scene());
    let mut terminal = screen();
    output(
        &mut renderer,
        &mut terminal,
        Some((3, 1)),
        (2, 2),
        &[(3, 1, 1, 1)],
    );
    let placements = terminal.kitty_image_placements().unwrap();
    assert!(!placements.is_empty());
    for p in &placements {
        assert!(p.render.viewport_col >= 3 && p.render.viewport_row >= 1);
        assert!(p.render.viewport_col + p.render.grid_cols as i32 <= 5);
        assert!(p.render.viewport_row + p.render.grid_rows as i32 <= 3);
        assert!(
            !(p.render.viewport_col <= 3
                && 3 < p.render.viewport_col + p.render.grid_cols as i32
                && p.render.viewport_row <= 1
                && 1 < p.render.viewport_row + p.render.grid_rows as i32)
        );
    }
    output(&mut renderer, &mut terminal, Some((3, 1)), (10, 6), &[]);
    let restored = terminal.kitty_image_placements().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].render.grid_cols, 4);
    assert_eq!(restored[0].render.grid_rows, 2);
}
#[test]
fn disabled_missing_surface_and_scope_replacement_remove_old_host_images() {
    let mut renderer = Renderer::default();
    renderer.configure(true, (2, 2));
    renderer.set_scene("old", scene());
    let mut terminal = screen();
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    let old = terminal.kitty_image_placements().unwrap()[0].image_id;
    output(&mut renderer, &mut terminal, None, (0, 0), &[]);
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    renderer.configure(false, (2, 2));
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
    renderer.configure(true, (2, 2));
    renderer.set_scene("new", scene());
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    assert_eq!(
        terminal.kitty_image_fingerprints(&[old]).unwrap(),
        vec![None]
    );
    let mut cleanup = Vec::new();
    renderer.cleanup(&mut cleanup).unwrap();
    terminal.write(&cleanup);
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
}
#[test]
fn declaration_origin_and_copy_toast_hide_then_restore_graphics_beside_chrome() {
    let declarations: Declarations = vec![("test".into(),serde_json::from_value(serde_json::json!({
        "herdr":"widget_tree","version":1,"zones":[],"composition":{"priority":1,"tree":{
            "type":"row","children":[{"type":"label","text":"SIDE","role":"label"},{"type":"surface"}]}
        }
    })).unwrap())];
    let pane = Surface {
        pane_id: "pane".into(),
        width: 16,
        height: 8,
        cells: (0..128)
            .map(|_| Cell {
                symbol: " ".into(),
                foreground: 0,
                background: 0,
                modifiers: 0,
                hyperlink: None,
            })
            .collect(),
        hyperlinks: Vec::new(),
        cursor: Some((1, 1)),
        focused: true,
        mouse_reporting: false,
        graphics: scene(),
        graphics_scope: "scope".into(),
    };
    let mut core = Core::default();
    let viewport = Rect::viewport(20, 8);
    let origin = trailboss::terminal::surface_rect(&core, &declarations, viewport).unwrap();
    assert_eq!(
        origin,
        Rect {
            x: 4,
            y: 0,
            width: 16,
            height: 8
        }
    );
    let mut renderer = FrameRenderer::default();
    renderer.graphics.configure(true, (2, 2));
    let mut bytes = Vec::new();
    renderer
        .draw_frame(&mut bytes, Some(&pane), &declarations, &core, viewport)
        .unwrap();
    let mut terminal = screen();
    terminal.write(&bytes);
    let placements = terminal.kitty_image_placements().unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(
        (
            placements[0].render.viewport_col,
            placements[0].render.viewport_row
        ),
        (i32::from(origin.x), i32::from(origin.y))
    );
    assert_eq!(terminal.cursor_y().unwrap(), origin.y + 1);
    renderer.copy_feedback_position = trailboss_runtime::config::ToastClipboardPosition::TopLeft;
    core.copy_dispatched(std::time::Instant::now());
    bytes.clear();
    renderer
        .draw_frame(&mut bytes, Some(&pane), &declarations, &core, viewport)
        .unwrap();
    terminal.write(&bytes);
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
    core.clear_copy_feedback();
    bytes.clear();
    renderer
        .draw_frame(&mut bytes, Some(&pane), &declarations, &core, viewport)
        .unwrap();
    terminal.write(&bytes);
    let restored = terminal.kitty_image_placements().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].data, placements[0].data);
}
struct Broken;
impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("output unavailable"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("output unavailable"))
    }
}
#[test]
fn failed_cleanup_retries_deletion_before_detach() {
    let mut renderer = Renderer::default();
    renderer.configure(true, (2, 2));
    renderer.set_scene("scope", scene());
    let mut terminal = screen();
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    let id = terminal.kitty_image_placements().unwrap()[0].image_id;
    assert!(renderer.cleanup(&mut Broken).is_err());
    let mut bytes = Vec::new();
    renderer.cleanup(&mut bytes).unwrap();
    terminal.write(&bytes);
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
    assert_eq!(
        terminal.kitty_image_fingerprints(&[id]).unwrap(),
        vec![None]
    );
}

#[test]
fn unselected_pane_pixels_survive_until_a_metadata_only_focus_change() {
    let original = scene();
    let mut renderer = Renderer::default();
    renderer.configure(true, (2, 2));
    renderer.set_scene(
        "scope",
        Arc::new(SurfaceGraphicsScene {
            assets: original.assets.clone(),
            placements: Vec::new(),
            retained_assets: vec![original.placements[0].asset.clone()],
        }),
    );
    let mut terminal = screen();
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
    renderer.set_scene(
        "scope",
        Arc::new(SurfaceGraphicsScene {
            assets: Vec::new(),
            placements: original.placements.clone(),
            retained_assets: Vec::new(),
        }),
    );
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    let restored = terminal.kitty_image_placements().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].data, original.assets[0].data);
}

#[test]
fn failed_output_replays_cached_pixels_even_after_payload_free_patch() {
    let original = scene();
    let mut renderer = Renderer::default();
    renderer.configure(true, (2, 2));
    renderer.set_scene("scope", Arc::clone(&original));
    let mut terminal = screen();
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    renderer.set_scene(
        "scope",
        Arc::new(SurfaceGraphicsScene {
            assets: Vec::new(),
            placements: original.placements.clone(),
            retained_assets: Vec::new(),
        }),
    );
    assert!(renderer
        .write_frame(&mut Broken, b"", Some((0, 0)), (20, 8), &[])
        .is_err());
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    let restored = terminal.kitty_image_placements().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].data, original.assets[0].data);
}

#[test]
fn failed_scope_replacement_still_deletes_previous_boot_images_on_retry() {
    let mut renderer = Renderer::default();
    renderer.configure(true, (2, 2));
    renderer.set_scene("old-boot", scene());
    let mut terminal = screen();
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    let old = terminal.kitty_image_placements().unwrap()[0].image_id;
    renderer.set_scene("new-boot", scene());
    assert!(renderer
        .write_frame(&mut Broken, b"", Some((0, 0)), (20, 8), &[])
        .is_err());
    output(&mut renderer, &mut terminal, Some((0, 0)), (20, 8), &[]);
    assert_eq!(
        terminal.kitty_image_fingerprints(&[old]).unwrap(),
        vec![None]
    );
    assert_eq!(terminal.kitty_image_placements().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn resizing_replays_direct_acked_images_without_inline_pixels() {
    let original = scene();
    let mut terminal = screen();
    // The real client uses the alternate screen, whose resize does not reflow
    // the primary scrollback and discard image pins when narrowing the grid.
    let mut setup = Vec::new();
    crossterm::execute!(&mut setup, crossterm::terminal::EnterAlternateScreen).unwrap();
    terminal.write(&setup);
    let mut upload = Renderer::default();
    upload.configure(true, (2, 2));
    upload.set_scene("scope", Arc::clone(&original));
    let mut upload_bytes = Vec::new();
    upload
        .write_frame(&mut upload_bytes, b"", Some((0, 0)), (20, 8), &[])
        .unwrap();
    // Seed only the original engine upload, matching GraphicsFile a=t: the
    // host has pixels but no placement until the ACK is committed by a frame.
    let upload_start = upload_bytes
        .windows(3)
        .position(|p| p == b"\x1b_G")
        .unwrap()
        + 3;
    let header_end = upload_bytes[upload_start..]
        .iter()
        .position(|b| *b == b';')
        .unwrap()
        + upload_start;
    let id = std::str::from_utf8(&upload_bytes[upload_start..header_end])
        .unwrap()
        .split(',')
        .find_map(|part| part.strip_prefix("i="))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let placement_start = upload_bytes
        .windows(b"\x1b_Ga=p,".len())
        .position(|p| p == b"\x1b_Ga=p,")
        .unwrap();
    let metadata = Arc::new(SurfaceGraphicsScene {
        assets: Vec::new(),
        placements: original.placements.clone(),
        retained_assets: Vec::new(),
    });
    let pane = Surface {
        pane_id: "pane".into(),
        width: 20,
        height: 8,
        cells: (0..160)
            .map(|_| Cell {
                symbol: " ".into(),
                foreground: 0,
                background: 0,
                modifiers: 0,
                hyperlink: None,
            })
            .collect(),
        hyperlinks: Vec::new(),
        cursor: Some((1, 1)),
        focused: true,
        mouse_reporting: false,
        graphics: Arc::clone(&metadata),
        graphics_scope: "scope".into(),
    };
    let declarations: Declarations = vec![(
        "test".into(),
        serde_json::from_value(serde_json::json!({
            "herdr":"widget_tree", "version":1, "zones":[],
            "composition":{"priority":1,"tree":{"type":"surface"}}
        }))
        .unwrap(),
    )];
    let core = Core::default();
    let mut renderer = FrameRenderer::default();
    renderer.graphics.configure(true, (2, 2));
    renderer.graphics.set_scene("scope", metadata);
    // The metadata frame consumes the old-scope cleanup before direct upload,
    // just as the live client advertises the scene before receiving GraphicsFile.
    let mut bytes = Vec::new();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&pane),
            &declarations,
            &core,
            Rect::viewport(20, 8),
        )
        .unwrap();
    terminal.write(&bytes);
    terminal.write(&upload_bytes[..placement_start]);
    terminal.write(b"\x1b8");
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
    assert!(renderer
        .graphics
        .trust_direct_asset(&original.placements[0].asset, id));
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&pane),
            &declarations,
            &core,
            Rect::viewport(20, 8),
        )
        .unwrap();
    terminal.write(&bytes);
    assert_eq!(
        terminal.kitty_image_placements().unwrap().len(),
        1,
        "initial direct-only replay"
    );
    let fingerprint = terminal.kitty_image_fingerprints(&[id]).unwrap();
    assert!(fingerprint[0].is_some());
    for (width, height) in [(3, 6), (20, 8)] {
        terminal.resize(width, height, 2, 2).unwrap();
        assert_eq!(
            terminal.kitty_image_fingerprints(&[id]).unwrap(),
            fingerprint,
            "host pixel storage after resize to {width}x{height}, before renderer output"
        );
        bytes.clear();
        renderer
            .draw_frame(
                &mut bytes,
                Some(&pane),
                &declarations,
                &core,
                Rect::viewport(width, height),
            )
            .unwrap();
        terminal.write(&bytes);
        let placed = terminal.kitty_image_placements().unwrap();
        assert_eq!(
            placed.len(),
            1,
            "direct-only replay after resize to {width}x{height}"
        );
        assert_eq!(placed[0].image_id, id);
        assert_eq!(placed[0].data, original.assets[0].data);
        assert!(placed[0].render.grid_cols <= u32::from(width));
        assert_eq!(placed[0].render.grid_rows, 2);
    }
}
