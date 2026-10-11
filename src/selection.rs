//! Pane-local mouse selection for the spike binary.

use trailboss::{terminal::Rect, Surface};
use unicode_width::UnicodeWidthStr;

/// Inclusive pane-cell range; decoration is reversible without copying the grid.
pub struct PaneSelection {
    pane_id: String,
    rect: Rect,
    width: u16,
    height: u16,
    anchor: (u16, u16),
    cursor: (u16, u16),
    dragged: bool,
    finished: bool,
    word: bool,
}

impl PaneSelection {
    pub fn start(surface: &Surface, rect: Rect, x: u16, y: u16) -> Option<Self> {
        let (width, height) = visible_size(surface, rect)?;
        let col = x.checked_sub(rect.x)?;
        let row = y.checked_sub(rect.y)?;
        if col >= width || row >= height {
            return None;
        }
        Some(Self {
            pane_id: surface.pane_id.clone(),
            rect,
            width: surface.width,
            height: surface.height,
            anchor: (row, col),
            cursor: (row, col),
            dragged: false,
            finished: false,
            word: false,
        })
    }
    pub fn start_word(surface: &Surface, rect: Rect, x: u16, y: u16) -> Option<Self> {
        let mut selected = Self::start(surface, rect, x, y)?;
        let (left, right) = word_bounds(surface, rect, selected.anchor.0, selected.anchor.1)?;
        selected.anchor.1 = left;
        selected.cursor.1 = right;
        selected.dragged = true;
        selected.word = true;
        Some(selected)
    }

    pub fn was_click_at(&self, surface: &Surface, rect: Rect, x: u16, y: u16) -> bool {
        !self.dragged
            && self.matches(surface, rect)
            && x.abs_diff(rect.x + self.anchor.1) <= 1
            && y.abs_diff(rect.y + self.anchor.0) <= 1
    }

    pub fn retain_after_release(&self) -> bool {
        self.word || !self.dragged
    }

    pub fn drag(&mut self, x: u16, y: u16) {
        if self.finished || self.word {
            return;
        }
        self.cursor = (
            y.saturating_sub(self.rect.y)
                .min(self.height.min(self.rect.height) - 1),
            x.saturating_sub(self.rect.x)
                .min(self.width.min(self.rect.width) - 1),
        );
        // Trailboss activates an outward drag even when clamping retains the anchor.
        self.dragged |= x != self.rect.x + self.anchor.1 || y != self.rect.y + self.anchor.0;
    }

    pub fn matches(&self, surface: &Surface, rect: Rect) -> bool {
        self.pane_id == surface.pane_id
            && self.rect == rect
            && self.width == surface.width
            && self.height == surface.height
            && visible_size(surface, rect).is_some()
    }

    pub fn finish(&mut self, surface: &Surface) -> Option<String> {
        if self.finished || !self.dragged || !self.matches(surface, self.rect) {
            return None;
        }
        self.finished = true;
        let mut text = String::new();
        let (start, end) = self.ordered();
        for row in start.0..=end.0 {
            let row_start = text.len();
            let (left, right) = self.columns(row);
            let base = usize::from(row) * usize::from(self.width);
            let mut col = 0usize;
            // Scan from the row origin so a selected continuation includes its glyph
            // and never contributes a duplicate space from the covered cell.
            while col <= usize::from(right) {
                let symbol = &surface.cells[base + col].symbol;
                let span = symbol.width().max(1).min(usize::from(self.width) - col);
                if col + span > usize::from(left) {
                    text.push_str(symbol);
                }
                col += span;
            }
            text.truncate(row_start + text[row_start..].trim_end().len());
            if row != end.0 {
                text.push('\n');
            }
        }
        Some(text)
    }

    /// Call again on the same grid and range after painting to restore its styles.
    pub fn decorate(&self, surface: &mut Surface) {
        if !self.dragged || !self.matches(surface, self.rect) {
            return;
        }
        let (start, end) = self.ordered();
        for row in start.0..=end.0 {
            let (left, right) = self.columns(row);
            let base = usize::from(row) * usize::from(self.width);
            for col in left..=right {
                surface.cells[base + usize::from(col)].modifiers ^= 64;
            }
        }
    }

    fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    fn columns(&self, row: u16) -> (u16, u16) {
        let (start, end) = self.ordered();
        (
            if row == start.0 { start.1 } else { 0 },
            if row == end.0 {
                end.1
            } else {
                self.width.min(self.rect.width) - 1
            },
        )
    }
}

// Walk the displayed graphemes, not a copied text row: covered wide cells are
// skipped and combining/ZWJ sequences retain their actual terminal geometry.
fn word_bounds(surface: &Surface, rect: Rect, row: u16, clicked: u16) -> Option<(u16, u16)> {
    use trailboss_runtime::word_selection::{
        is_leading_token_wrapper, is_trailing_token_wrapper, is_word_separator,
    };
    let width = surface.width.min(rect.width);
    let base = usize::from(row) * usize::from(surface.width);
    let mut start = None;
    let mut end = None;
    let mut col = 0;
    while col < width {
        let symbol = &surface.cells[base + usize::from(col)].symbol;
        let ch = symbol.chars().next().unwrap_or(' ');
        let span = symbol.width().max(1).min(usize::from(width - col)) as u16;
        if is_word_separator(ch) {
            if let (Some(left), Some(right)) = (start, end) {
                if left <= clicked && clicked <= right {
                    return Some((left, right));
                }
            }
            start = None;
            end = None;
        } else {
            if start.is_none() && !is_leading_token_wrapper(ch) {
                start = Some(col);
            }
            if start.is_some() && !is_trailing_token_wrapper(ch) {
                end = Some(col + span - 1);
            }
        }
        col += span;
    }
    start
        .zip(end)
        .filter(|&(left, right)| left <= clicked && clicked <= right)
}

fn visible_size(surface: &Surface, rect: Rect) -> Option<(u16, u16)> {
    let width = surface.width.min(rect.width);
    let height = surface.height.min(rect.height);
    if width == 0
        || height == 0
        || surface.cells.len() != usize::from(surface.width) * usize::from(surface.height)
    {
        return None;
    }
    rect.x.checked_add(rect.width - 1)?;
    rect.y.checked_add(rect.height - 1)?;
    Some((width, height))
}
#[cfg(test)]
mod tests {
    use super::PaneSelection;
    use trailboss::{terminal::Rect, Cell, Surface};

    fn surface(width: u16, symbols: &[&str]) -> Surface {
        Surface {
            pane_id: "pane-a".into(),
            width,
            height: (symbols.len() / usize::from(width)) as u16,
            cells: symbols
                .iter()
                .map(|symbol| Cell {
                    symbol: (*symbol).into(),
                    foreground: 2,
                    background: 3,
                    modifiers: 1,
                    hyperlink: None,
                })
                .collect(),
            hyperlinks: Vec::new(),
            graphics: Default::default(),
            graphics_scope: String::new(),
            cursor: None,
            focused: true,
            mouse_reporting: false,
        }
    }

    #[test]
    fn forward_drag_copies_inclusive_cells_and_restores_original_styles() {
        let mut pane = surface(5, &["a", "b", "c", "d", "e"]);
        pane.cells[2].modifiers = 65;
        let rect = Rect {
            x: 10,
            y: 3,
            width: 5,
            height: 1,
        };
        let mut selection = PaneSelection::start(&pane, rect, 11, 3).unwrap();
        selection.drag(13, 3);
        selection.decorate(&mut pane);
        assert_eq!(
            pane.cells
                .iter()
                .map(|cell| cell.modifiers)
                .collect::<Vec<_>>(),
            [1, 65, 1, 65, 1]
        );
        assert_eq!(pane.cells[2].foreground, 2);
        assert_eq!(pane.cells[2].background, 3);
        selection.decorate(&mut pane);
        assert_eq!(
            pane.cells
                .iter()
                .map(|cell| cell.modifiers)
                .collect::<Vec<_>>(),
            [1, 1, 65, 1, 1]
        );
        assert_eq!(selection.finish(&pane).as_deref(), Some("bcd"));
    }

    #[test]
    fn reverse_multiline_drag_trims_each_row_without_removing_blank_lines() {
        let mut pane = surface(
            4,
            &["a", "b", " ", " ", " ", " ", " ", " ", "c", "d", "e", "f"],
        );
        let mut selection = PaneSelection::start(&pane, Rect::viewport(4, 3), 1, 2).unwrap();
        selection.drag(1, 0);
        selection.decorate(&mut pane);
        assert_eq!(
            pane.cells
                .iter()
                .map(|cell| cell.modifiers)
                .collect::<Vec<_>>(),
            [1, 65, 65, 65, 65, 65, 65, 65, 65, 65, 1, 1]
        );
        selection.decorate(&mut pane);
        assert_eq!(selection.finish(&pane).as_deref(), Some("b\n\ncd"));
    }

    #[test]
    fn unicode_drag_copies_whole_graphemes_once_when_starting_on_a_wide_tail() {
        for tail in ["", " "] {
            let pane = surface(7, &["界", tail, "e\u{301}", " ", "👨‍👩‍👧", tail, "!"]);
            let mut selection = PaneSelection::start(&pane, Rect::viewport(7, 1), 1, 0).unwrap();
            selection.drag(6, 0);
            assert_eq!(selection.finish(&pane).as_deref(), Some("界e\u{301} 👨‍👩‍👧!"));
        }
    }

    #[test]
    fn plain_click_and_same_cell_motion_neither_highlight_nor_copy() {
        let mut pane = surface(2, &["a", "b"]);
        let mut selection = PaneSelection::start(&pane, Rect::viewport(2, 1), 0, 0).unwrap();
        selection.decorate(&mut pane);
        assert_eq!(selection.finish(&pane), None);
        selection.drag(0, 0);
        selection.decorate(&mut pane);
        assert_eq!(pane.cells[0].modifiers, 1);
        assert_eq!(pane.cells[1].modifiers, 1);
        assert_eq!(selection.finish(&pane), None);
    }

    #[test]
    fn drag_clips_to_displayed_pane_edges_without_copying_hidden_columns() {
        let mut pane = surface(5, &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let rect = Rect {
            x: 20,
            y: 5,
            width: 3,
            height: 2,
        };
        for (x, y) in [(19, 5), (20, 4), (23, 5), (20, 7)] {
            assert!(PaneSelection::start(&pane, rect, x, y).is_none());
        }
        let mut forward = PaneSelection::start(&pane, rect, 21, 5).unwrap();
        forward.drag(u16::MAX, u16::MAX);
        forward.decorate(&mut pane);
        assert_eq!(
            pane.cells
                .iter()
                .map(|cell| cell.modifiers)
                .collect::<Vec<_>>(),
            [1, 65, 65, 1, 1, 65, 65, 65, 1, 1]
        );
        forward.decorate(&mut pane);
        assert_eq!(forward.finish(&pane).as_deref(), Some("bc\nfgh"));
        let mut reverse = PaneSelection::start(&pane, rect, 21, 6).unwrap();
        reverse.drag(0, 0);
        assert_eq!(reverse.finish(&pane).as_deref(), Some("abc\nfg"));
    }

    #[test]
    fn stale_pane_geometry_and_inconsistent_grids_reject_copy_and_decoration() {
        let rect = Rect::viewport(2, 1);
        let pane = surface(2, &["a", "b"]);
        let mut selection = PaneSelection::start(&pane, rect, 0, 0).unwrap();
        selection.drag(1, 0);
        assert!(selection.matches(&pane, rect));
        assert!(!selection.matches(&pane, Rect { x: 1, ..rect }));
        let mut other = surface(2, &["x", "y"]);
        other.pane_id = "pane-b".into();
        assert!(!selection.matches(&other, rect));
        assert_eq!(selection.finish(&other), None);
        selection.decorate(&mut other);
        assert_eq!(other.cells[0].modifiers, 1);
        assert_eq!(other.cells[1].modifiers, 1);
        other.pane_id = "pane-a".into();
        other.width = 1;
        other.height = 2;
        assert!(!selection.matches(&other, rect));
        assert_eq!(selection.finish(&other), None);
        other.width = 2;
        other.height = 1;
        other.cells.pop();
        assert!(!selection.matches(&other, rect));
        assert!(PaneSelection::start(&other, rect, 0, 0).is_none());
        assert_eq!(selection.finish(&other), None);
        selection.decorate(&mut other);
        assert_eq!(other.cells[0].modifiers, 1);
    }

    #[test]
    fn zero_and_overflowing_geometry_reject_selection_without_panicking() {
        let pane = surface(2, &["a", "b"]);
        for rect in [
            Rect::viewport(0, 1),
            Rect::viewport(2, 0),
            Rect {
                x: u16::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
            Rect {
                x: 0,
                y: u16::MAX,
                width: 2,
                height: 2,
            },
        ] {
            assert!(PaneSelection::start(&pane, rect, rect.x, rect.y).is_none());
        }
    }

    #[test]
    fn drag_returning_to_anchor_copies_once_and_retains_final_highlight() {
        let mut pane = surface(2, &["a", "b"]);
        let mut selection = PaneSelection::start(&pane, Rect::viewport(2, 1), 0, 0).unwrap();
        selection.drag(1, 0);
        selection.drag(0, 0);
        assert_eq!(selection.finish(&pane).as_deref(), Some("a"));
        assert_eq!(selection.finish(&pane), None);
        selection.drag(1, 0);
        selection.decorate(&mut pane);
        assert_eq!(pane.cells[0].modifiers, 65);
        assert_eq!(pane.cells[1].modifiers, 1);
        selection.decorate(&mut pane);
        assert_eq!(pane.cells[0].modifiers, 1);
    }

    #[test]
    fn outward_drag_from_a_pane_corner_copies_the_clamped_anchor_cell() {
        let mut pane = surface(2, &["a", "b"]);
        let rect = Rect {
            x: 10,
            y: 5,
            width: 2,
            height: 1,
        };
        let mut selection = PaneSelection::start(&pane, rect, 10, 5).unwrap();
        selection.drag(9, 4);
        selection.decorate(&mut pane);
        assert_eq!(pane.cells[0].modifiers, 65);
        assert_eq!(pane.cells[1].modifiers, 1);
        selection.decorate(&mut pane);
        assert_eq!(selection.finish(&pane).as_deref(), Some("a"));
    }
    #[test]
    fn word_click_copies_the_whole_token_once_and_retains_its_highlight() {
        let mut pane = surface(9, &[" ", "'", "h", "e", "l", "l", "o", "'", ","]);
        let rect = Rect::viewport(9, 1);
        let mut selected = PaneSelection::start_word(&pane, rect, 4, 0).unwrap();
        selected.drag(4, 0);
        assert_eq!(selected.finish(&pane).as_deref(), Some("hello"));
        assert_eq!(selected.finish(&pane), None);
        assert!(selected.retain_after_release());
        selected.decorate(&mut pane);
        for (col, cell) in pane.cells.iter().enumerate() {
            assert_eq!(cell.modifiers, if (2..=6).contains(&col) { 65 } else { 1 });
        }
        selected.decorate(&mut pane);
        assert!(pane.cells.iter().all(|cell| cell.modifiers == 1));
        for col in [0, 1, 7, 8] {
            assert!(PaneSelection::start_word(&pane, rect, col, 0).is_none());
        }
    }

    #[test]
    fn word_click_on_a_wide_tail_preserves_combining_and_zwj_graphemes() {
        let pane = surface(8, &["界", "", "e\u{301}", "👨‍👩‍👧", "", " ", "x", "!"]);
        let rect = Rect::viewport(8, 1);
        for col in 0..=4 {
            let mut selected = PaneSelection::start_word(&pane, rect, col, 0).unwrap();
            assert_eq!(selected.finish(&pane).as_deref(), Some("界e\u{301}👨‍👩‍👧"));
        }
        let mut selected = PaneSelection::start_word(&pane, rect, 6, 0).unwrap();
        assert_eq!(selected.finish(&pane).as_deref(), Some("x"));
    }

    #[test]
    fn word_click_respects_visible_edges_and_does_not_reuse_a_drag_or_other_pane() {
        let pane = surface(6, &["a", "b", "c", "d", "e", "f"]);
        let rect = Rect::viewport(3, 1);
        let mut selected = PaneSelection::start_word(&pane, rect, 1, 0).unwrap();
        assert_eq!(selected.finish(&pane).as_deref(), Some("abc"));
        assert!(PaneSelection::start_word(&pane, rect, 3, 0).is_none());
        let mut click = PaneSelection::start(&pane, rect, 1, 0).unwrap();
        assert!(click.was_click_at(&pane, rect, 2, 0));
        let mut other = surface(6, &["a", "b", "c", "d", "e", "f"]);
        other.pane_id = "other".into();
        assert!(!click.was_click_at(&other, rect, 1, 0));
        click.drag(2, 0);
        assert!(!click.was_click_at(&pane, rect, 1, 0));
        assert!(!click.retain_after_release());
    }
}
