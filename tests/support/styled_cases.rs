use serde_json::{json, Value};

pub fn declaration() -> Value {
    let segments = |prefix: &str| {
        json!({"type":"row","children":[
            {"type":"label","text":prefix,"role":"label","tone":"bg-lime"},
            {"type":"label","text":"m","role":"muted"},
            {"type":"leader","tone":"fg-cyan"},
            {"type":"label","text":"!","role":"status-working"}
        ]})
    };
    json!({"herdr":"widget_tree","version":1,"zones":[
        {"zone":"list","tree":{"type":"list","entries":[segments("L"),
            {"type":"row","context":{"tab_id":"padding-focus"},"children":[{"type":"label","text":"P","role":"label","tone":"bg-lime"}]},
            {"type":"row","children":[{"type":"label","text":format!("V{}", "界".repeat(50)),"role":"label"},{"type":"leader","tone":"bg-lime"}]}]}},
        {"zone":"footer","tree":{"type":"column","children":[segments("F"), {"type":"label","text":"must-be-clipped","role":"label"}]}},
        {"zone":"footer_right","tree":segments("R")}
    ]})
}
pub fn assert_screen(screen: &ghostty_vt::Terminal, width: u16, _height: u16, _focused: bool) {
    let rows = screen.screen_text_rows().unwrap();
    let text: Vec<String> = rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| {
                    cell.graphemes
                        .first()
                        .and_then(|cp| char::from_u32(*cp))
                        .unwrap_or(' ')
                })
                .collect()
        })
        .collect();
    let sidebar = usize::from(width / 4);
    for prefix in ['L', 'F', 'R'] {
        let y = text
            .iter()
            .position(|row| row.starts_with(prefix))
            .expect("declared zone was not painted");
        let expected = format!("{prefix}m{}!", "·".repeat(sidebar.saturating_sub(3)));
        assert_eq!(
            text[y].chars().take(sidebar).collect::<String>(),
            expected,
            "zone {prefix}: {text:?}"
        );
        assert!(
            !text[y]
                .chars()
                .skip(sidebar)
                .any(|ch| ch == '·' || ch == '!'),
            "zone crossed Surface boundary"
        );
        let mut state = ghostty_vt::RenderState::new().unwrap();
        state.update(screen).unwrap();
        // Assert the decoded cell styles, not the bytes emitted by the renderer.
        let mut storage = ghostty_vt::RowIterator::new().unwrap();
        let mut iter = state.populate_row_iterator(&mut storage).unwrap();
        for _ in 0..=y {
            assert!(iter.next());
        }
        let mut cells = ghostty_vt::RowCells::new().unwrap();
        let mut cells = iter.populate_cells(&mut cells).unwrap();
        for x in [0, 2] {
            cells.select(x as u16).unwrap();
            let style = cells.style().unwrap();
            if x == 0 {
                assert_eq!(
                    style.bg_color,
                    Some(ghostty_vt::CellColor::Palette(10)),
                    "explicit bg-lime tone was lost"
                );
            } else {
                assert_eq!(
                    style.fg_color,
                    Some(ghostty_vt::CellColor::Palette(14)),
                    "explicit fg-cyan tone was lost"
                );
            }
        }
    }
}
