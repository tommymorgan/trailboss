use trailboss::{
    core::Core,
    terminal::{self, Rect},
    Cell, Surface,
};

fn surface(text: &str) -> Surface {
    Surface {
        pane_id: "isolated-renderer".into(),
        width: text.len() as u16,
        height: 1,
        cells: text
            .chars()
            .map(|ch| Cell {
                symbol: ch.to_string(),
                foreground: 0,
                background: 0,
                modifiers: 0,
                hyperlink: None,
            })
            .collect(),
        hyperlinks: Vec::new(),
        graphics: Default::default(),
        graphics_scope: String::new(),
        cursor: Some((0, 0)),
        focused: true,
        mouse_reporting: false,
    }
}

fn composition(tree: serde_json::Value) -> trailboss::sidebar::Declarations {
    crossterm::style::force_color_output(true);
    vec![(
        "renderer-fixture".into(),
        serde_json::from_value(serde_json::json!({
            "herdr":"widget_tree", "version":1, "zones":[],
            "composition":{"priority":1,"tree":tree}
        }))
        .unwrap(),
    )]
}

fn sidebar_style(screen: &ghostty_vt::Terminal, x: u16, y: u16) -> ghostty_vt::CellStyle {
    let mut state = ghostty_vt::RenderState::new().unwrap();
    state.update(screen).unwrap();
    let mut storage = ghostty_vt::RowIterator::new().unwrap();
    let mut rows = state.populate_row_iterator(&mut storage).unwrap();
    for _ in 0..=y {
        assert!(rows.next());
    }
    let mut storage = ghostty_vt::RowCells::new().unwrap();
    let mut cells = rows.populate_cells(&mut storage).unwrap();
    cells.select(x).unwrap();
    cells.style().unwrap()
}
fn sidebar_rgb(r: u8, g: u8, b: u8) -> Option<ghostty_vt::CellColor> {
    Some(ghostty_vt::CellColor::Rgb(ghostty_vt::RgbColor { r, g, b }))
}
fn sidebar_text(screen: &ghostty_vt::Terminal) -> Vec<String> {
    screen
        .screen_text_rows()
        .unwrap()
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .flat_map(|cell| cell.graphemes.iter())
                .filter_map(|cp| char::from_u32(*cp))
                .collect()
        })
        .collect()
}
#[test]
fn sectioned_sidebar_keeps_palette_hierarchy_and_selected_tab_inside_its_cells() {
    use serde_json::json;
    let mut core = Core::default();
    core.focused_tab_id = Some("chosen".into());
    let declarations = composition(json!({"type":"row","children":[
        {"type":"flex","weight":1,"child":{"type":"column","children":[
            {"type":"label","text":"host · app","role":"title"},
            {"type":"row","context":{"workspace_id":"w"},"children":[
                {"type":"label","text":"infilla","role":"section"},{"type":"leader"},
                {"type":"label","text":"●","role":"status-working"},{"type":"label","text":"●","role":"status-blocked"}]},
            {"type":"row","context":{"workspace_id":"w","tab_id":"chosen"},"children":[
                {"type":"label","text":"○ ","role":"status-idle"},{"type":"label","text":"plugins","role":"label"}]},
            {"type":"row","context":{"workspace_id":"w","tab_id":"chosen","pane_id":"p"},"children":[
                {"type":"label","text":"  ○ ","role":"status-idle"},{"type":"label","text":"shell","role":"label"}]},
            {"type":"row","context":{"workspace_id":"r"},"children":[
                {"type":"label","text":"very-long-workspace-name","role":"section"},{"type":"leader"},
                {"type":"label","text":"●","role":"status-waiting"},{"type":"label","text":" 2 hidden","role":"muted"}]}
        ]}},
        {"type":"flex","weight":3,"child":{"type":"surface"}}
    ]}));
    let mut renderer = terminal::FrameRenderer::default();
    let mut bytes = Vec::new();
    let mut screen = ghostty_vt::Terminal::new(80, 6, 0).unwrap();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("ANCHOR")),
            &declarations,
            &core,
            Rect::viewport(80, 6),
        )
        .unwrap();
    screen.write(&bytes);
    let rows = sidebar_text(&screen);
    assert!(rows[0].contains("host · app"));
    assert!(rows[1].contains("infilla") && rows[1].contains('─'));
    assert!(rows[2].starts_with('▏') && rows[2].contains("plugins"));
    assert!(rows[3].contains("shell") && !rows[3].starts_with('▏'));
    assert!(rows[4]
        .chars()
        .take(20)
        .collect::<String>()
        .ends_with("● 2 hidden "));
    assert!(rows[4].contains("…"));
    assert_eq!(
        rows[0].chars().skip(20).take(6).collect::<String>(),
        "ANCHOR"
    );
    let luminance = |color| match color {
        Some(ghostty_vt::CellColor::Rgb(rgb)) => {
            2126 * u32::from(rgb.r) + 7152 * u32::from(rgb.g) + 722 * u32::from(rgb.b)
        }
        _ => panic!("expected resolved sidebar RGB"),
    };
    let rule_x = rows[1].chars().position(|ch| ch == '─').unwrap() as u16;
    let rule = luminance(sidebar_style(&screen, rule_x, 1).fg_color);
    let label = luminance(sidebar_style(&screen, 1, 1).fg_color);
    let panel = luminance(sidebar_style(&screen, rule_x, 1).bg_color);
    assert!(rule > 330_000 && rule > panel * 4 && rule < label);
    assert!(sidebar_style(&screen, 1, 1).bold);
    assert_eq!(
        sidebar_style(&screen, 19, 2).bg_color,
        sidebar_rgb(26, 38, 61)
    );
    assert_eq!(
        sidebar_style(&screen, 5, 3).bg_color,
        sidebar_rgb(13, 10, 16)
    );
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("ANCHOR")),
            &declarations,
            &core,
            Rect::viewport(80, 6),
        )
        .unwrap();
    assert!(!bytes.windows(6).any(|bytes| bytes == b"ANCHOR"));
}

#[test]
fn themed_sidebar_gaps_and_footer_stay_opaque_without_repainting_adjacent_pane() {
    use serde_json::json;
    let declarations = composition(json!({"type":"row","children":[
        {"type":"flex","weight":1,"child":{"type":"column","children":[
            {"type":"row","children":[{"type":"label","text":"host","role":"title"}]},
            {"type":"row","children":[{"type":"label","text":"","role":"muted"}]},
            {"type":"row","action":"host:new-space","children":[{"type":"label","text":"+ New space","role":"label"}]}
        ]}},
        {"type":"flex","weight":3,"child":{"type":"surface"}}
    ]}));
    let core = Core::default();
    let mut renderer = terminal::FrameRenderer::default();
    let mut bytes = Vec::new();
    let mut screen = ghostty_vt::Terminal::new(80, 6, 0).unwrap();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("PANE")),
            &declarations,
            &core,
            Rect::viewport(80, 6),
        )
        .unwrap();
    screen.write(&bytes);
    for (x, y) in [(0, 1), (19, 5), (1, 2)] {
        assert_eq!(
            sidebar_style(&screen, x, y).bg_color,
            sidebar_rgb(13, 10, 16)
        );
    }
    assert_eq!(
        sidebar_style(&screen, 1, 2).fg_color,
        sidebar_rgb(185, 179, 189)
    );
    assert_eq!(sidebar_style(&screen, 20, 0).bg_color, None);
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("PANE")),
            &declarations,
            &core,
            Rect::viewport(80, 6),
        )
        .unwrap();
    assert!(!bytes
        .windows(3)
        .any(|bytes| bytes == b"38;" || bytes == b"48;"));
}
#[test]
fn updating_one_cell_never_blanks_unchanged_pane_content() {
    let mut core = Core::default();
    core.has_pane = true;
    let declarations = composition(serde_json::json!({"type":"surface"}));
    let mut screen = ghostty_vt::Terminal::new(8, 2, 0).unwrap();
    let mut bytes = Vec::new();
    let mut renderer = terminal::FrameRenderer::default();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("AAAA")),
            &declarations,
            &core,
            Rect::viewport(8, 2),
        )
        .unwrap();
    screen.write(&bytes);
    assert_eq!(
        screen.screen_text_rows().unwrap()[0].cells[0].graphemes,
        vec![65]
    );
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("ABAA")),
            &declarations,
            &core,
            Rect::viewport(8, 2),
        )
        .unwrap();
    for byte in bytes {
        screen.write(&[byte]);
        let row = &screen.screen_text_rows().unwrap()[0];
        assert!(
            row.cells[0].graphemes == vec![65] && row.cells[2].graphemes == vec![65],
            "unchanged pane cells vanished during frame: {row:?}"
        );
    }
}

#[path = "support/console.rs"]
mod console;
mod support;

#[test]
fn full_screen_pty_burst_preserves_stable_content_between_updates() {
    use std::time::{Duration, Instant};
    let core = support::Core::new();
    let mut client = core.attach();
    while client.focused_pane_id.is_none() {
        client.receive().unwrap();
    }
    let binary = std::env::var_os("TRAILBOSS_RENDERER_BASELINE_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(support::executable);
    let mut command = portable_pty::CommandBuilder::new(binary);
    command.args(["--session", &core.name]);
    for key in support::HOMES {
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
    command.env_remove("NO_COLOR");
    let mut console = console::Console::from_command(command);
    console.text(concat!(r#"printf '\033[2J\033[H'; y=1; while [ $y -le 20 ]; do printf '\033[%s;1HAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' "$y"; y=$((y+1)); done; printf '\033[21;1HRENDER-STABLE-ANCHOR'; sleep 0.3; i=0; while [ $i -lt 30 ]; do printf '\033[1;1H%02d' "$i"; i=$((i+1)); sleep 0.03; done; printf '\033[2;1HRENDER-BURST-DONE'"#, "\r"));
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut initial = Vec::new();
    loop {
        let bytes = console
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| panic!("initial burst: {error}; rows={:?}", console.rows()));
        console.screen.write(&bytes);
        initial.extend_from_slice(&bytes);
        if console
            .rows()
            .iter()
            .any(|row| row.contains("RENDER-STABLE-ANCHOR"))
        {
            break;
        }
    }
    let anchor_row = console
        .rows()
        .iter()
        .position(|row| row.contains("RENDER-STABLE-ANCHOR"))
        .unwrap();
    let mut update = Vec::new();
    let mut lost_samples = 0;
    let mut samples = 0;
    loop {
        let bytes = console
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        update.extend_from_slice(&bytes);
        for chunk in bytes.chunks(64) {
            console.screen.write(chunk);
            samples += 1;
            if !console.rows()[anchor_row].contains("RENDER-STABLE-ANCHOR") {
                lost_samples += 1;
            }
        }
        if console
            .rows()
            .iter()
            .any(|row| row.contains("RENDER-BURST-DONE"))
        {
            break;
        }
    }
    console.wait(|rows| rows.iter().any(|row| row.contains("RENDER-BURST-DONE")));
    let clears = update
        .windows(4)
        .filter(|bytes| *bytes == b"\x1b[2J")
        .count();
    eprintln!("PTY burst: {} bytes, {samples} partial-frame samples, {lost_samples} missing anchors, {clears} whole-screen clears", update.len());
    if let Some(root) = std::env::var_os("TRAILBOSS_RENDERER_EVIDENCE_DIR") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("initial.ansi"), initial).unwrap();
        std::fs::write(root.join("update.ansi"), update).unwrap();
    }
    assert_eq!(
        clears, 0,
        "repainting erased the visible screen before reconstructing it"
    );
    assert_eq!(
        lost_samples, 0,
        "PTY frame tearing blanked unchanged content"
    );
    assert!(
        samples >= 20,
        "burst did not exercise repeated real PTY output"
    );
}

#[test]
fn removed_surface_and_shortened_labels_leave_no_stale_cells() {
    let core = Core::default();
    let mut renderer = terminal::FrameRenderer::default();
    let mut bytes = Vec::new();
    let mut screen = ghostty_vt::Terminal::new(8, 3, 0).unwrap();
    for text in ["LONGTEXT", "X"] {
        let declarations =
            composition(serde_json::json!({"type":"label","text":text,"role":"label"}));
        bytes.clear();
        renderer
            .draw_frame(&mut bytes, None, &declarations, &core, Rect::viewport(8, 3))
            .unwrap();
        screen.write(&bytes);
    }
    assert_eq!(
        screen.screen_text_rows().unwrap()[0].cells[0].graphemes,
        vec![88]
    );
    assert!(screen.screen_text_rows().unwrap()[0].cells[1..]
        .iter()
        .all(|cell| cell.graphemes == vec![32]));
    let declarations = composition(serde_json::json!({"type":"surface"}));
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("PANE")),
            &declarations,
            &core,
            Rect::viewport(8, 3),
        )
        .unwrap();
    screen.write(&bytes);
    bytes.clear();
    renderer
        .draw_frame(&mut bytes, None, &declarations, &core, Rect::viewport(8, 3))
        .unwrap();
    screen.write(&bytes);
    assert!(screen
        .screen_text_rows()
        .unwrap()
        .iter()
        .flat_map(|row| &row.cells)
        .all(|cell| cell.graphemes == vec![32]));
}

#[test]
fn style_only_updates_restore_exact_pane_styles_and_cursor_visibility() {
    crossterm::style::force_color_output(true);
    let core = Core::default();
    let declarations = composition(serde_json::json!({"type":"surface"}));
    let mut renderer = terminal::FrameRenderer::default();
    let mut bytes = Vec::new();
    let mut screen = ghostty_vt::Terminal::new(8, 3, 0).unwrap();
    let mut pane = surface("X");
    pane.cells[0].foreground = 0x01000001;
    pane.cells[0].background = 0x01000002;
    pane.cells[0].modifiers = 1 | 2 | 4 | 8 | 16 | 64 | 256;
    for styled in [true, false] {
        if !styled {
            pane.cells[0].foreground = 0;
            pane.cells[0].background = 0;
            pane.cells[0].modifiers = 0;
        }
        pane.cursor = Some((3, 1));
        bytes.clear();
        renderer
            .draw_frame(
                &mut bytes,
                Some(&pane),
                &declarations,
                &core,
                Rect::viewport(8, 3),
            )
            .unwrap();
        screen.write(&bytes);
        let mut state = ghostty_vt::RenderState::new().unwrap();
        state.update(&screen).unwrap();
        let cursor = state.cursor().unwrap();
        assert!(cursor.visible);
        let cursor = cursor.viewport.unwrap();
        assert_eq!((cursor.x, cursor.y), (3, 1));
        let mut storage = ghostty_vt::RowIterator::new().unwrap();
        let mut rows = state.populate_row_iterator(&mut storage).unwrap();
        assert!(rows.next());
        let mut storage = ghostty_vt::RowCells::new().unwrap();
        let mut cells = rows.populate_cells(&mut storage).unwrap();
        cells.select(0).unwrap();
        let style = cells.style().unwrap();
        assert_eq!(
            style.fg_color,
            styled.then_some(ghostty_vt::CellColor::Palette(1)),
            "{style:?}"
        );
        assert_eq!(
            style.bg_color,
            styled.then_some(ghostty_vt::CellColor::Palette(2))
        );
        assert_eq!(
            (
                style.bold,
                style.faint,
                style.italic,
                style.underlined,
                style.blink,
                style.inverse,
                style.strikethrough
            ),
            (styled, styled, styled, styled, styled, styled, styled)
        );
    }
    pane.cursor = None;
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&pane),
            &declarations,
            &core,
            Rect::viewport(8, 3),
        )
        .unwrap();
    screen.write(&bytes);
    let mut state = ghostty_vt::RenderState::new().unwrap();
    state.update(&screen).unwrap();
    assert!(!state.cursor().unwrap().visible);
}

#[test]
fn wide_cells_and_geometry_redraws_remove_the_previous_layout() {
    let core = Core::default();
    let declarations = composition(serde_json::json!({"type":"surface"}));
    let mut renderer = terminal::FrameRenderer::default();
    let mut bytes = Vec::new();
    let mut screen = ghostty_vt::Terminal::new(8, 3, 0).unwrap();
    let mut pane = surface("ABC");
    pane.cells[0].symbol = "界".into();
    pane.cells[1].symbol.clear();
    pane.cells[2].symbol = "e\u{301}".into();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&pane),
            &declarations,
            &core,
            Rect::viewport(8, 3),
        )
        .unwrap();
    screen.write(&bytes);
    let rows = screen.screen_text_rows().unwrap();
    assert_eq!(rows[0].cells[0].graphemes, vec!['界' as u32]);
    assert_eq!(rows[0].cells[2].graphemes, vec![101, 0x301]);
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("ABC")),
            &declarations,
            &core,
            Rect::viewport(8, 3),
        )
        .unwrap();
    screen.write(&bytes);
    let rows = screen.screen_text_rows().unwrap();
    assert_eq!(rows[0].cells[0].graphemes, vec![65]);
    assert_eq!(rows[0].cells[1].graphemes, vec![66]);
    assert_eq!(rows[0].cells[2].graphemes, vec![67]);
    bytes.clear();
    renderer
        .draw_frame(
            &mut bytes,
            Some(&surface("Z")),
            &declarations,
            &core,
            Rect {
                x: 1,
                y: 1,
                width: 4,
                height: 1,
            },
        )
        .unwrap();
    screen.write(&bytes);
    let rows = screen.screen_text_rows().unwrap();
    assert!(rows[0]
        .cells
        .iter()
        .all(|cell| cell.graphemes.is_empty() || cell.graphemes == [32]));
    assert_eq!(rows[1].cells[1].graphemes, vec![90]);
}

#[test]
fn very_narrow_sections_keep_status_and_hidden_count_before_decorative_rule() {
    use serde_json::json;
    let declarations = composition(
        json!({"type":"row","context":{"workspace_id":"w"},"children":[
            {"type":"label","text":"界界界workspace","role":"section"}, {"type":"leader"},
            {"type":"label","text":"●","role":"status-waiting"}, {"type":"label","text":" 2 hidden","role":"muted"}
        ]}),
    );
    let mut screen = ghostty_vt::Terminal::new(12, 2, 0).unwrap();
    let mut bytes = Vec::new();
    terminal::FrameRenderer::default()
        .draw_frame(
            &mut bytes,
            None,
            &declarations,
            &Core::default(),
            Rect::viewport(12, 2),
        )
        .unwrap();
    screen.write(&bytes);
    assert_eq!(sidebar_text(&screen)[0], " ● 2 hidden ");
}

#[test]
fn arbitrary_workspace_rows_and_empty_flows_do_not_opt_into_sidebar_theme() {
    use serde_json::json;
    let declarations = composition(json!({"type":"column","children":[
        {"type":"row","context":{"workspace_id":"w","tab_id":"t"},"children":[{"type":"label","text":"Build","role":"label"}]},
        {"type":"row","children":[]},
        {"type":"row","children":[{"type":"label","text":"END","role":"label","tone":"fg-cyan"}]}
    ]}));
    let mut core = Core::default();
    core.focused_tab_id = Some("t".into());
    let mut screen = ghostty_vt::Terminal::new(8, 3, 0).unwrap();
    let mut bytes = Vec::new();
    terminal::FrameRenderer::default()
        .draw_frame(&mut bytes, None, &declarations, &core, Rect::viewport(8, 3))
        .unwrap();
    screen.write(&bytes);
    assert_eq!(sidebar_text(&screen)[0], "Build   ");
    assert_eq!(sidebar_text(&screen)[1], "END     ");
    assert_eq!(
        sidebar_style(&screen, 0, 0).bg_color,
        Some(ghostty_vt::CellColor::Palette(4))
    );
    assert_eq!(
        sidebar_style(&screen, 0, 1).fg_color,
        Some(ghostty_vt::CellColor::Palette(14))
    );
}

#[test]
fn executable_pty_shows_sectioned_sidebar_while_shell_and_cursor_remain_live() {
    let core = support::Core::new();
    let mut console = console::Console::new(&core);
    let rows = console.wait(|rows| rows.iter().any(|row| row.starts_with('▏')));
    let selected = rows.iter().position(|row| row.starts_with('▏')).unwrap();
    assert_eq!(
        sidebar_style(&console.screen, 0, selected as u16).fg_color,
        sidebar_rgb(77, 127, 184)
    );
    assert_eq!(
        sidebar_style(&console.screen, 19, selected as u16).bg_color,
        sidebar_rgb(26, 38, 61)
    );
    assert_eq!(rows.iter().filter(|row| row.starts_with('▏')).count(), 1);
    let section = rows
        .iter()
        .position(|row| row.chars().take(20).any(|ch| ch == '─'))
        .unwrap();
    let rule_x = rows[section].chars().position(|ch| ch == '─').unwrap() as u16;
    let brightness = |color| match color {
        Some(ghostty_vt::CellColor::Rgb(rgb)) => {
            u32::from(rgb.r) + u32::from(rgb.g) + u32::from(rgb.b)
        }
        _ => panic!("native sidebar RGB missing"),
    };
    let rule = brightness(sidebar_style(&console.screen, rule_x, section as u16).fg_color);
    assert!(
        rule > 90 && rule < brightness(sidebar_style(&console.screen, 1, section as u16).fg_color)
    );
    for (row, menu_text) in [
        (23, "New space"),
        (section, "New tab in"),
        (selected, "Rename"),
    ] {
        console.text(&format!("\x1b[<35;1;{}M", row + 1));
        let frame = console.wait(|rows| rows[row].chars().nth(18) == Some('⋯'));
        assert_eq!(frame[row].chars().nth(19), Some(' '));
        console.click(18, row);
        console.wait(|rows| rows.iter().any(|r| r.contains(menu_text)));
        console.text("\x1b");
        console.wait(|rows| rows.iter().all(|r| !r.contains(menu_text)));
        console.click(0, 10);
        console.text("\x1b[<35;70;20M");
        console.wait(|rows| rows.iter().all(|r| !r.contains('⋯')));
    }
    console.text("printf 'SIDEBAR-%s\\n' PTY-LIVE\r");
    let rows = console.wait(|rows| {
        rows.iter()
            .any(|row| row.chars().skip(20).collect::<String>().trim() == "SIDEBAR-PTY-LIVE")
    });
    assert!(rows.iter().any(|row| row.starts_with('▏')));
    let mut state = ghostty_vt::RenderState::new().unwrap();
    state.update(&console.screen).unwrap();
    let cursor = state.cursor().unwrap();
    assert!(cursor.visible);
    assert!(cursor.viewport.unwrap().x >= 20);
    eprintln!("Variant C executable PTY rows:\n{}", rows.join("\n"));
    if let Some(root) = std::env::var_os("TRAILBOSS_RENDERER_EVIDENCE_DIR") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("variant-c-pty.txt"), rows.join("\n")).unwrap();
    }
}

#[test]
fn oversized_status_clusters_show_ellipsis_without_losing_hidden_count() {
    use serde_json::json;
    let mut children = vec![
        json!({"type":"label","text":"workspace","role":"section"}),
        json!({"type":"leader"}),
    ];
    children.extend((0..10).map(|_| json!({"type":"label","text":"●","role":"status-working"})));
    children.push(json!({"type":"label","text":" 2 hidden","role":"muted"}));
    let declarations = composition(json!({"type":"row","children":children}));
    for width in 1..=24 {
        let mut bytes = Vec::new();
        let mut screen = ghostty_vt::Terminal::new(width, 2, 0).unwrap();
        terminal::FrameRenderer::default()
            .draw_frame(
                &mut bytes,
                None,
                &declarations,
                &Core::default(),
                Rect::viewport(width, 2),
            )
            .unwrap();
        screen.write(&bytes);
        let row = sidebar_text(&screen)[0].clone();
        assert!(row.ends_with(' '));
        if width >= 11 {
            assert!(row.contains("2 hidden"), "hidden count lost at {width}");
        }
        for (x, ch) in row.chars().enumerate() {
            if ch == '…' {
                assert_eq!(row.chars().nth(x + 1), Some(' '));
            }
        }
        if width == 16 {
            assert!(row.contains("… "));
        }
    }
}

#[test]
fn inherited_panel_color_does_not_change_generic_right_aligned_label_geometry() {
    use serde_json::json;
    let declarations = composition(json!({"type":"column","children":[
        {"type":"label","text":"host","role":"title"},
        {"type":"row","context":{"tab_id":"t"},"children":[{"type":"label","text":"           after new","role":"label"}]}
    ]}));
    let mut core = Core::default();
    core.focused_tab_id = Some("t".into());
    let mut bytes = Vec::new();
    let mut screen = ghostty_vt::Terminal::new(20, 3, 0).unwrap();
    terminal::FrameRenderer::default()
        .draw_frame(
            &mut bytes,
            None,
            &declarations,
            &core,
            Rect::viewport(20, 3),
        )
        .unwrap();
    screen.write(&bytes);
    assert_eq!(sidebar_text(&screen)[1], "           after new");
    assert_eq!(
        sidebar_style(&screen, 11, 1).fg_color,
        sidebar_rgb(185, 179, 189)
    );
    assert_eq!(
        sidebar_style(&screen, 19, 1).bg_color,
        sidebar_rgb(13, 10, 16)
    );
}

#[test]
fn direct_sidebar_preview_and_frame_renderer_agree_on_sectioned_row_styles() {
    use serde_json::json;
    crossterm::style::force_color_output(true);
    let tree = json!({"type":"column","children":[
        {"type":"row","children":[{"type":"label","text":"host","role":"title"}]},
        {"type":"row","context":{"workspace_id":"w","tab_id":"t"},"children":[
            {"type":"label","text":"○ ","role":"status-idle"},{"type":"label","text":"chosen","role":"label"}]}
    ]});
    let direct = vec![(
        "fixture".into(),
        serde_json::from_value(
            json!({"herdr":"widget_tree","version":1,"zones":[{"zone":"list","tree":tree}]}),
        )
        .unwrap(),
    )];
    let framed = composition(tree);
    let mut core = Core::default();
    core.focused_tab_id = Some("t".into());
    let mut preview = ghostty_vt::Terminal::new(20, 2, 0).unwrap();
    let mut frame = ghostty_vt::Terminal::new(20, 2, 0).unwrap();
    let mut bytes = Vec::new();
    trailboss::sidebar::draw(&mut bytes, &direct, Rect::viewport(20, 2), Some("t")).unwrap();
    preview.write(&bytes);
    bytes.clear();
    terminal::FrameRenderer::default()
        .draw_frame(&mut bytes, None, &framed, &core, Rect::viewport(20, 2))
        .unwrap();
    frame.write(&bytes);
    assert_eq!(sidebar_text(&preview), sidebar_text(&frame));
    for y in 0..2 {
        for x in 0..20 {
            assert_eq!(sidebar_style(&preview, x, y), sidebar_style(&frame, x, y));
        }
    }
}

#[test]
fn every_clipped_label_keeps_an_ellipsis_margin_without_crossing_its_cells() {
    use serde_json::json;
    for text in [
        "abcdefghijk",
        "界界界界",
        "e\u{301}e\u{301}abcdefgh",
        "abc…",
        "",
        "ok",
    ] {
        for width in 1..=10 {
            for role in ["label", "title", "section"] {
                let declarations = composition(json!({"type":"row","children":[
                    {"type":"flex","weight":1,"child":{"type":"column","children":[
                        {"type":"row","children":[{"type":"label","text":text,"role":role}]}]}},
                    {"type":"flex","weight":1,"child":{"type":"surface"}}
                ]}));
                let mut screen = ghostty_vt::Terminal::new(width * 2, 2, 0).unwrap();
                let mut bytes = Vec::new();
                terminal::FrameRenderer::default()
                    .draw_frame(
                        &mut bytes,
                        Some(&surface("ANCHOR")),
                        &declarations,
                        &Core::default(),
                        Rect::viewport(width * 2, 2),
                    )
                    .unwrap();
                screen.write(&bytes);
                let cells = &screen.screen_text_rows().unwrap()[0].cells;
                for x in 0..usize::from(width) {
                    if cells[x].graphemes.contains(&('…' as u32))
                        || cells[x].graphemes.contains(&('⋯' as u32))
                    {
                        assert!(
                            x + 1 < usize::from(width),
                            "ellipsis at boundary: {text:?} {role} {width}"
                        );
                        assert!(
                            cells[x + 1].graphemes.is_empty() || cells[x + 1].graphemes == vec![32]
                        );
                    }
                }
                assert_eq!(cells[usize::from(width)].graphemes, vec![65]);
                if text.is_empty() || (text == "ok" && width >= 4) {
                    assert!(!cells[..usize::from(width)]
                        .iter()
                        .any(|c| c.graphemes.contains(&('…' as u32))));
                }
            }
        }
    }
}

#[test]
fn public_surface_targets_are_sanitized_and_uri_identity_not_table_index_drives_repaint() {
    let mut pane = surface("AB");
    pane.hyperlinks = vec![Some("https://example.test/a\x1b\x07\nb".into())];
    pane.cells[0].hyperlink = Some(0);
    let declarations = composition(serde_json::json!({"type":"surface"}));
    let mut renderer = terminal::FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(6, 3, 0).unwrap();
    let core = Core::default();
    let mut bytes = Vec::new();
    let mut paint = |pane: &Surface, bytes: &mut Vec<u8>| {
        bytes.clear();
        renderer
            .draw_frame(
                bytes,
                Some(pane),
                &declarations,
                &core,
                Rect::viewport(6, 3),
            )
            .unwrap();
        screen.write(bytes);
        assert_eq!(
            screen.viewport_hyperlink_uri(0, 0).unwrap().as_deref(),
            Some("https://example.test/ab")
        );
        assert_eq!(screen.viewport_hyperlink_uri(1, 0).unwrap(), None);
    };
    paint(&pane, &mut bytes);
    assert!(!bytes.contains(&7));
    // Sanitization-only and index-only changes are visually identical, so no pane redraw.
    pane.hyperlinks[0] = Some("https://example.test/ab".into());
    paint(&pane, &mut bytes);
    assert!(!bytes.contains(&b'A'));
    pane.hyperlinks.insert(0, Some("unreferenced".into()));
    pane.cells[0].hyperlink = Some(1);
    paint(&pane, &mut bytes);
    assert!(!bytes.contains(&b'A'));
}

#[test]
fn interrupted_linked_glyph_closes_osc8_and_the_next_frame_repaints_its_baseline() {
    struct FailGlyph {
        bytes: Vec<u8>,
        failed: bool,
    }
    impl std::io::Write for FailGlyph {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if !self.failed {
                if let Some(glyph) = bytes.iter().position(|byte| *byte == b'A') {
                    if glyph != 0 {
                        self.bytes.extend_from_slice(&bytes[..glyph]);
                        return Ok(glyph);
                    }
                    self.failed = true;
                    return Err(std::io::Error::other("owned output fixture"));
                }
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut pane = surface("A");
    pane.hyperlinks = vec![Some("https://example.test/full".into())];
    pane.cells[0].hyperlink = Some(0);
    let declarations = composition(serde_json::json!({"type":"surface"}));
    let core = Core::default();
    let rect = Rect::viewport(6, 3);
    let mut renderer = terminal::FrameRenderer::default();
    let mut failed = FailGlyph {
        bytes: Vec::new(),
        failed: false,
    };
    assert!(renderer
        .draw_frame(&mut failed, Some(&pane), &declarations, &core, rect)
        .is_err());
    assert!(failed.failed);
    assert!(failed.bytes.ends_with(b"\x1b]8;;\x1b\\"));
    let mut screen = ghostty_vt::Terminal::new(6, 3, 0).unwrap();
    screen.write(&failed.bytes);
    screen.write(b"\x1b[2;1Hplain");
    assert_eq!(screen.viewport_hyperlink_uri(0, 1).unwrap(), None);
    let mut recovered = Vec::new();
    renderer
        .draw_frame(&mut recovered, Some(&pane), &declarations, &core, rect)
        .unwrap();
    screen.write(&recovered);
    assert_eq!(
        screen.viewport_hyperlink_uri(0, 0).unwrap().as_deref(),
        Some("https://example.test/full")
    );
    assert_eq!(screen.viewport_hyperlink_uri(0, 1).unwrap(), None);
}

#[test]
fn repeated_copy_feedback_expires_on_idle_tick_and_restores_retained_pane_and_chrome() {
    use std::time::{Duration, Instant};
    let mut core = Core::default();
    core.has_pane = true;
    let declarations = composition(serde_json::json!({"type":"column","children":[
        {"type":"flex","weight":1,"child":{"type":"surface"}},
        {"type":"label","text":"Retained bottom chrome","role":"label"}
    ]}));
    let mut pane = surface("A");
    pane.width = 40;
    pane.height = 5;
    pane.cells = surface(&"A".repeat(200)).cells;
    let mut renderer = terminal::FrameRenderer::default();
    let mut screen = ghostty_vt::Terminal::new(40, 6, 0).unwrap();
    let paint =
        |renderer: &mut terminal::FrameRenderer, screen: &mut ghostty_vt::Terminal, core: &Core| {
            let mut bytes = Vec::new();
            renderer
                .draw_frame(
                    &mut bytes,
                    Some(&pane),
                    &declarations,
                    core,
                    Rect::viewport(40, 6),
                )
                .unwrap();
            screen.write(&bytes);
        };
    paint(&mut renderer, &mut screen, &core);
    let baseline = sidebar_text(&screen);
    let now = Instant::now();
    core.copy_dispatched(now);
    paint(&mut renderer, &mut screen, &core);
    assert_ne!(sidebar_text(&screen), baseline);
    assert!(!renderer.controls.modal());
    assert!(core.text("typed while visible").is_none());
    core.copy_dispatched(now + Duration::from_secs(1));
    assert!(!core.tick_copy_feedback(now + Duration::from_secs(2)));
    paint(&mut renderer, &mut screen, &core);
    assert_ne!(sidebar_text(&screen), baseline);
    assert!(core.tick_copy_feedback(now + Duration::from_secs(3)));
    paint(&mut renderer, &mut screen, &core);
    assert_eq!(sidebar_text(&screen), baseline);
    assert!(!core.tick_copy_feedback(now + Duration::from_secs(4)));
}

#[test]
fn copy_feedback_clips_after_resize_and_restores_each_configured_edge() {
    use std::time::{Duration, Instant};
    use trailboss_runtime::config::ToastClipboardPosition::*;
    for position in [
        TopLeft,
        TopCenter,
        TopRight,
        BottomLeft,
        BottomCenter,
        BottomRight,
    ] {
        let mut core = Core::default();
        let declarations = composition(serde_json::json!({"type":"surface"}));
        let mut renderer = terminal::FrameRenderer::default();
        renderer.copy_feedback_position = position;
        let now = Instant::now();
        core.copy_dispatched(now);
        renderer
            .draw_frame(
                &mut Vec::new(),
                None,
                &declarations,
                &core,
                Rect::viewport(80, 24),
            )
            .unwrap();
        for (width, height) in [(1, 1), (2, 2), (8, 3)] {
            let mut screen = ghostty_vt::Terminal::new(width, height, 0).unwrap();
            let mut bytes = Vec::new();
            renderer
                .draw_frame(
                    &mut bytes,
                    None,
                    &declarations,
                    &core,
                    Rect::viewport(width, height),
                )
                .unwrap();
            screen.write(&bytes);
            assert!(sidebar_text(&screen)
                .iter()
                .any(|row| !row.trim().is_empty()));
            core.clear_copy_feedback();
            bytes.clear();
            renderer
                .draw_frame(
                    &mut bytes,
                    None,
                    &declarations,
                    &core,
                    Rect::viewport(width, height),
                )
                .unwrap();
            screen.write(&bytes);
            assert!(sidebar_text(&screen)
                .iter()
                .all(|row| row.trim().is_empty()));
            core.copy_dispatched(now);
        }
        assert!(core.tick_copy_feedback(now + Duration::from_secs(2)));
    }
}

#[test]
fn tab_cycle_hint_stays_beside_copy_feedback_and_dismissal_restores_the_frame() {
    let snapshot = serde_json::from_value(serde_json::json!({
        "boot_id":"test","revision":1,"update_install_command":"","latest_release_notes_available":false,
        "integration_updates_available":false,"worktree_directory":"","focused_tab_id":"b",
        "tab_bar_right":[],"tab_bar_right_separator":"","agent_order":[],"workspaces":[],
        "tabs":[{"tab_id":"a","workspace_id":"w","number":1,"label":"A","custom_label":false,"zoomed":false,"focused":false,"agent_status":"idle"},
        {"tab_id":"b","workspace_id":"w","number":2,"label":"B","custom_label":false,"zoomed":false,"focused":true,"agent_status":"idle"}],
        "panes":[],"agents":[],"commands":[]
    })).unwrap();
    use trailboss_runtime::config::ToastClipboardPosition::*;
    for position in [
        TopLeft,
        TopCenter,
        TopRight,
        BottomLeft,
        BottomCenter,
        BottomRight,
    ] {
        let mut core = Core::default();
        core.update_session(Some("w".into()), Some("a".into()), true);
        core.update_session(Some("w".into()), Some("b".into()), true);
        let declarations = composition(serde_json::json!({"type":"column","children":[
        {"type":"flex","weight":1,"child":{"type":"surface"}},
        {"type":"label","text":"Retained bottom chrome","role":"label"}]}));
        let mut pane = surface(&"A".repeat(80 * 9));
        pane.width = 80;
        pane.height = 9;
        let mut renderer = terminal::FrameRenderer::default();
        renderer.copy_feedback_position = position;
        let mut screen = ghostty_vt::Terminal::new(80, 10, 0).unwrap();
        let paint = |renderer: &mut terminal::FrameRenderer,
                     screen: &mut ghostty_vt::Terminal,
                     core: &Core| {
            let mut bytes = Vec::new();
            renderer
                .draw_frame(
                    &mut bytes,
                    Some(&pane),
                    &declarations,
                    core,
                    Rect::viewport(80, 10),
                )
                .unwrap();
            screen.write(&bytes);
        };
        paint(&mut renderer, &mut screen, &core);
        let baseline = sidebar_text(&screen);
        core.copy_dispatched(std::time::Instant::now());
        paint(&mut renderer, &mut screen, &core);
        let copy_only = sidebar_text(&screen);
        assert_eq!(core.cycle_tab(&snapshot), Some("a"));
        paint(&mut renderer, &mut screen, &core);
        let both = sidebar_text(&screen);
        assert!(both.iter().any(|row| row.contains("MRU: L next")));
        assert!(both.iter().any(|row| row.contains("Enter/Esc done")));
        let copy_y = copy_only
            .iter()
            .position(|row| row.contains("copied to clipboard"))
            .unwrap();
        for y in copy_y - 1..=copy_y + 1 {
            assert_eq!(both[y], copy_only[y]);
        }
        assert!(!renderer.controls.modal());
        core.finish_tab_cycle();
        paint(&mut renderer, &mut screen, &core);
        assert_eq!(sidebar_text(&screen), copy_only);
        core.clear_copy_feedback();
        paint(&mut renderer, &mut screen, &core);
        assert_eq!(sidebar_text(&screen), baseline);
    }
}
