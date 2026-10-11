//! Terminal adapter: declarations select every visible node and its placement.
use crate::{core::Core, sidebar::Declarations, Surface};
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use trailboss_runtime::config::ToastClipboardPosition;
use trailboss_runtime::protocol::widgets::{Context, Node, Tone, Zone};
pub mod menus;
mod modals;
mod palette;
pub mod reorder;
pub mod split;
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}
impl Rect {
    pub fn viewport(width: u16, height: u16) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
    pub fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
fn color(v: u32) -> Color {
    match v >> 24 {
        1 => Color::AnsiValue(v as u8),
        2 => Color::Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        },
        _ => match v {
            0 => Color::Reset,
            n => Color::AnsiValue((n - 1) as u8),
        },
    }
}
/// Low-level Surface painting. Only the declaration walker calls this in the application.
pub fn draw(out: &mut impl Write, surface: &Surface, rect: Rect) -> io::Result<()> {
    out.write_all(CLOSE_HYPERLINK)?;
    let mut hyperlinks = Vec::new();
    refresh_hyperlinks(&mut hyperlinks, Some(surface));
    for y in 0..surface.height.min(rect.height) {
        for x in 0..surface.width.min(rect.width) {
            let c = &surface.cells[usize::from(y) * usize::from(surface.width) + usize::from(x)];
            paint_cell(
                out,
                c,
                cell_hyperlink(&hyperlinks, c).map(AsRef::as_ref),
                rect.x + x,
                rect.y + y,
            )?;
        }
    }
    Ok(())
}
fn leaf_row(node: &Node) -> bool {
    matches!(node, Node::Row { children, .. } if !children.is_empty() && children.iter().all(|n| matches!(n, Node::Label { .. } | Node::Leader { .. })))
}
fn extent(core: &Core, declarations: &Declarations, node: &Node, horizontal: bool) -> (u16, u16) {
    extent_at(core, declarations, node, horizontal, 0)
}
fn extent_at(
    core: &Core,
    declarations: &Declarations,
    node: &Node,
    horizontal: bool,
    depth: usize,
) -> (u16, u16) {
    if depth > 16 {
        return (0, 0);
    }
    let Some(node) = core.resolve(node) else {
        return (0, 0);
    };
    match node {
        Node::Label { text, .. } => (
            if horizontal {
                text.width().min(usize::from(u16::MAX)) as u16
            } else {
                1
            },
            0,
        ),
        Node::Row { .. } if !horizontal && leaf_row(node) => (1, 0),
        Node::Flex { weight, child } => {
            if core.resolve(child).is_some() {
                (0, *weight)
            } else {
                (0, 0)
            }
        }
        Node::Input => {
            if horizontal {
                (0, 1)
            } else {
                (1, 0)
            }
        }
        Node::Column { .. } | Node::Row { .. } | Node::List { .. } => {
            let (flow_horizontal, children) = core.flow(node).expect("flow node");
            let parallel = flow_horizontal == horizontal;
            let mut fixed = 0u16;
            for child in children {
                let (size, weight) =
                    if matches!(core.resolve(child), Some(Node::Spacer { .. })) && !parallel {
                        (0, 0)
                    } else {
                        extent_at(core, declarations, child, horizontal, depth + 1)
                    };
                if weight > 0 {
                    return (0, 1);
                }
                fixed = if parallel {
                    fixed.saturating_add(size)
                } else {
                    fixed.max(size)
                };
            }
            (fixed, 0)
        }
        Node::Spacer { size } => (*size, 0),
        Node::Leader { .. } => {
            if horizontal {
                (0, 1)
            } else {
                (1, 0)
            }
        }
        Node::Zone { zone, fallback }
            if !horizontal && matches!(zone, Zone::Footer | Zone::FooterRight) =>
        {
            let present = fallback.is_some()
                || declarations
                    .iter()
                    .any(|(_, d)| d.zones.iter().any(|p| &p.zone == zone));
            (u16::from(present), 0)
        }
        Node::Surface | Node::Zone { .. } => (0, 1),
        Node::Use { name } => core
            .fragment(declarations, name)
            .map_or((0, 0), |(_, tree)| {
                extent_at(core, declarations, tree, horizontal, depth + 1)
            }),
        Node::Modal { .. } => (0, 0),
        Node::When { .. } => unreachable!("Core resolves conditions"),
    }
}
fn walk<'a>(
    core: &Core,
    declarations: &'a Declarations,
    plugin: &'a str,
    node: &'a Node,
    rect: Rect,
    depth: usize,
    list_offset: u16,
    visit: &mut dyn FnMut(&'a str, &'a Node, Rect),
) {
    if depth > 16 || rect.width == 0 || rect.height == 0 {
        return;
    }
    let Some(node) = core.resolve(node) else {
        return;
    };
    visit(plugin, node, rect);
    match node {
        Node::Use { name } => {
            if let Some((owner, tree)) = core.fragment(declarations, name) {
                walk(
                    core,
                    declarations,
                    owner,
                    tree,
                    rect,
                    depth + 1,
                    list_offset,
                    visit,
                );
            }
        }
        Node::Flex { child, .. } => walk(
            core,
            declarations,
            plugin,
            child,
            rect,
            depth + 1,
            list_offset,
            visit,
        ),
        Node::Zone { zone, fallback } => {
            let placements = declarations.iter().flat_map(|(id, declaration)| {
                declaration
                    .zones
                    .iter()
                    .filter(|placement| &placement.zone == zone)
                    .map(move |placement| (id, &placement.tree))
            });
            let (fixed, weights) = placements
                .clone()
                .map(|(_, tree)| extent(core, declarations, tree, false))
                .fold((0u64, 0u64), |(fixed, weights), (size, weight)| {
                    (fixed + u64::from(size), weights + u64::from(weight))
                });
            let clip = rect;
            let scrolling = *zone == Zone::List;
            let natural = if placements.clone().next().is_none() {
                fallback.as_ref().map_or(0, |child| {
                    u64::from(extent(core, declarations, child, false).0)
                })
            } else {
                fixed
            };
            let rect = if scrolling {
                Rect {
                    y: 0,
                    height: natural.max(u64::from(clip.height)).min(u64::from(u16::MAX)) as u16,
                    ..rect
                }
            } else {
                rect
            };
            let scroll = list_offset.min(rect.height.saturating_sub(clip.height));
            let mut projected = |owner, child, area: Rect| {
                if !scrolling {
                    visit(owner, child, area);
                    return;
                }
                let top = u32::from(area.y).max(u32::from(scroll));
                let bottom = (u32::from(area.y) + u32::from(area.height))
                    .min(u32::from(scroll) + u32::from(clip.height));
                if top < bottom {
                    visit(
                        owner,
                        child,
                        Rect {
                            y: clip.y + (top - u32::from(scroll)) as u16,
                            height: (bottom - top) as u16,
                            ..area
                        },
                    );
                }
            };
            let flexible = u64::from(rect.height).saturating_sub(fixed);
            let mut found = false;
            let mut offset = 0;
            let mut cumulative = 0;
            for (id, tree) in placements {
                found = true;
                let (size, weight) = extent(core, declarations, tree, false);
                let before = cumulative;
                cumulative += u64::from(weight);
                let allocated = (flexible * cumulative).checked_div(weights).unwrap_or(0)
                    - (flexible * before).checked_div(weights).unwrap_or(0);
                let height = u64::from(size)
                    .saturating_add(allocated)
                    .min(u64::from(rect.height - offset)) as u16;
                let (inline, elastic) = extent(core, declarations, tree, true);
                let width = if *zone == Zone::FooterRight && elastic == 0 {
                    inline.min(rect.width)
                } else {
                    rect.width
                };
                let placement = Rect {
                    x: rect.x
                        + if *zone == Zone::FooterRight {
                            rect.width - width
                        } else {
                            0
                        },
                    y: rect.y + offset,
                    width,
                    height,
                };
                walk(
                    core,
                    declarations,
                    id,
                    tree,
                    placement,
                    depth + 1,
                    0,
                    &mut projected,
                );
                offset += height;
            }
            if !found {
                if let Some(child) = fallback {
                    walk(
                        core,
                        declarations,
                        plugin,
                        child,
                        rect,
                        depth + 1,
                        0,
                        &mut projected,
                    );
                }
            }
        }
        Node::Row { .. } | Node::Column { .. } | Node::List { .. } if !leaf_row(node) => {
            let (horizontal, children) = core.flow(node).expect("flow node");
            let available = if horizontal { rect.width } else { rect.height };
            let (fixed, weights) = children
                .iter()
                .map(|child| extent(core, declarations, child, horizontal))
                .fold((0u64, 0u64), |(f, w), (a, b)| {
                    (f + u64::from(a), w + u64::from(b))
                });
            let flexible = u64::from(available).saturating_sub(fixed);
            let mut offset = 0;
            let mut cumulative = 0;
            for child in children {
                let (size, weight) = extent(core, declarations, child, horizontal);
                let before = cumulative;
                cumulative += u64::from(weight);
                let allocated = (flexible * cumulative).checked_div(weights).unwrap_or(0)
                    - (flexible * before).checked_div(weights).unwrap_or(0);
                let size = u64::from(size)
                    .saturating_add(allocated)
                    .min(u64::from(available - offset)) as u16;
                let child_rect = if horizontal {
                    Rect {
                        x: rect.x + offset,
                        width: size,
                        ..rect
                    }
                } else {
                    Rect {
                        y: rect.y + offset,
                        height: size,
                        ..rect
                    }
                };
                walk(
                    core,
                    declarations,
                    plugin,
                    child,
                    child_rect,
                    depth + 1,
                    list_offset,
                    visit,
                );
                offset += size;
            }
        }
        _ => {}
    }
}
fn list_viewport(core: &Core, declarations: &Declarations, viewport: Rect) -> Option<(Rect, u16)> {
    let mut target = None;
    application(core, declarations, viewport, 0, &mut |_, node, rect| {
        if let Node::Zone {
            zone: Zone::List,
            fallback,
        } = node
        {
            let mut found = false;
            let mut height = 0u16;
            for (_, declaration) in declarations {
                for placement in &declaration.zones {
                    if placement.zone == Zone::List {
                        found = true;
                        height = height
                            .saturating_add(extent(core, declarations, &placement.tree, false).0);
                    }
                }
            }
            if !found {
                height = fallback
                    .as_ref()
                    .map_or(0, |child| extent(core, declarations, child, false).0);
            }
            target = Some((rect, height.saturating_sub(rect.height)));
        }
    });
    target
}
fn application<'a>(
    core: &Core,
    declarations: &'a Declarations,
    rect: Rect,
    list_offset: u16,
    visit: &mut impl FnMut(&'a str, &'a Node, Rect),
) {
    if let Some((plugin, root)) = core.composition(declarations) {
        walk(
            core,
            declarations,
            plugin,
            root,
            rect,
            0,
            list_offset,
            visit,
        );
    }
}
pub fn surface_rect(core: &Core, declarations: &Declarations, rect: Rect) -> Option<Rect> {
    let mut target = None;
    application(core, declarations, rect, 0, &mut |_, node, rect| {
        if matches!(node, Node::Surface) {
            target = Some(rect);
        }
    });
    target
}
pub fn has_modal(core: &Core, declarations: &Declarations, rect: Rect) -> bool {
    modals::dialog(core, declarations, rect).is_some()
}
pub fn action_at<'a>(
    core: &Core,
    declarations: &'a Declarations,
    rect: Rect,
    list_offset: u16,
    x: u16,
    y: u16,
) -> Option<(&'a str, &'a str, &'a Context)> {
    let mut target = None;
    application(
        core,
        declarations,
        rect,
        list_offset,
        &mut |plugin, node, rect| {
            if rect.contains(x, y) {
                if let Node::Row {
                    action: Some(action),
                    context,
                    ..
                } = node
                {
                    target = Some((plugin, action.as_str(), context));
                }
            }
        },
    );
    target
}
#[derive(Default)]
struct RetainedCell {
    symbol: String,
    foreground: u32,
    background: u32,
    modifiers: u16,
    hyperlink: Option<std::sync::Arc<str>>,
    valid: bool,
}
#[derive(Clone, Copy, Default)]
enum Paint {
    #[default]
    Blank,
    Panel,
    Surface(usize),
    Chrome,
}
#[derive(Default)]
struct Chrome {
    rect: Option<Rect>,
    bytes: Vec<u8>,
}
/// Retains the visible frame, not the server surface (which selection temporarily decorates).
/// Unchanged pane cells never leave the physical terminal, even on terminals without
/// synchronized-output support. Buffers are reused; only changed glyphs/chrome are copied.
#[derive(Default)]
pub struct FrameRenderer {
    pub controls: menus::Controls,
    pub copy_feedback_position: ToastClipboardPosition,
    copy_feedback_rect: Option<Rect>,
    navigation_hint_rect: Option<Rect>,
    menu_rects: Vec<Rect>,
    scrollbar_rect: Option<Rect>,
    overlay_rect: Option<Rect>,
    pub reorder: reorder::Drag,
    reorder_rect: Option<Rect>,
    rect: Option<Rect>,
    cells: Vec<RetainedCell>,
    paint: Vec<Paint>,
    chrome: Vec<Chrome>,
    scratch: Vec<u8>,
    hyperlinks: Vec<Hyperlink>,
    pub graphics: trailboss_runtime::client::graphics::Renderer,
    frame_bytes: Vec<u8>,
}
impl FrameRenderer {
    pub fn draw_frame(
        &mut self,
        out: &mut impl Write,
        surface: Option<&Surface>,
        declarations: &Declarations,
        core: &Core,
        rect: Rect,
    ) -> io::Result<()> {
        let mut encoded = std::mem::take(&mut self.frame_bytes);
        encoded.clear();
        encoded.extend_from_slice(b"\x1b[?2026h");
        let result = self
            .paint_frame(&mut encoded, surface, declarations, core, rect)
            .and_then(|()| {
                encoded.extend_from_slice(b"\x1b[?2026l");
                if let Some(surface) = surface {
                    self.graphics.set_scene(
                        &surface.graphics_scope,
                        std::sync::Arc::clone(&surface.graphics),
                    );
                }
                let pane = surface.and_then(|s| {
                    surface_rect(core, declarations, rect).map(|r| Rect {
                        width: r.width.min(s.width),
                        height: r.height.min(s.height),
                        ..r
                    })
                });
                let mut covers = [(0, 0, 0, 0); 4];
                let mut count = 0;
                for area in [
                    self.overlay_rect,
                    self.reorder_rect,
                    self.copy_feedback_rect,
                    self.navigation_hint_rect,
                ]
                .into_iter()
                .flatten()
                {
                    covers[count] = (area.x, area.y, area.width, area.height);
                    count += 1;
                }
                self.graphics.write_frame(
                    out,
                    &encoded,
                    pane.map(|r| (r.x, r.y)),
                    pane.map_or((0, 0), |r| (r.width, r.height)),
                    &covers[..count],
                )
            });
        self.frame_bytes = encoded;
        if result.is_err() {
            // A partially written frame is not a usable diff baseline.
            self.rect = None;
            // Try to terminate even an OSC 8 opener interrupted by an output error.
            let _ = out.write_all(b"\x1b\\\x1b[?2026l");
            self.graphics.invalidate_output();
            let _ = out.write_all(CLOSE_HYPERLINK);
        }
        result
    }
    pub fn cleanup(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.graphics.cleanup(out)
    }
    fn paint_frame(
        &mut self,
        out: &mut impl Write,
        surface: Option<&Surface>,
        declarations: &Declarations,
        core: &Core,
        rect: Rect,
    ) -> io::Result<()> {
        out.write_all(CLOSE_HYPERLINK)?;
        refresh_hyperlinks(&mut self.hyperlinks, surface);
        self.controls.reconcile(core, declarations, rect);
        let dialog = (!self.controls.modal())
            .then(|| modals::dialog(core, declarations, rect))
            .flatten();
        let scrollbar = list_viewport(core, declarations, rect)
            .filter(|(area, maximum)| area.width >= 3 && area.height > 0 && *maximum > 0);
        let rail = scrollbar.map(|(area, _)| Rect {
            x: area.x + area.width - 1,
            y: area.y,
            width: 1,
            height: area.height,
        });
        if rail != self.scrollbar_rect {
            for retained in &mut self.chrome {
                if menus::intersect(retained.rect, self.scrollbar_rect) {
                    retained.rect = None;
                }
            }
        }
        let indicator = self.reorder.indicator();
        if self.reorder_rect != indicator {
            for retained in &mut self.chrome {
                if menus::intersect(retained.rect, self.reorder_rect)
                    || menus::intersect(retained.rect, indicator)
                {
                    retained.rect = None;
                }
            }
        }
        let overlay = dialog
            .map(|d| d.rect)
            .or_else(|| self.controls.popup_rect(core, declarations, rect));
        if self.overlay_rect != overlay {
            for retained in &mut self.chrome {
                if menus::intersect(retained.rect, self.overlay_rect) {
                    retained.rect = None;
                }
            }
        }
        // Feedback is painted last. Redraw any retained chrome it previously covered,
        // including menus; pane cells under it are invalidated after painting below.
        for retained in &mut self.chrome {
            if menus::intersect(retained.rect, self.copy_feedback_rect)
                || menus::intersect(retained.rect, self.navigation_hint_rect)
                || self
                    .menu_rects
                    .iter()
                    .any(|area| menus::intersect(retained.rect, Some(*area)))
            {
                retained.rect = None;
            }
        }
        self.menu_rects.clear();
        let resized = self.rect != Some(rect);
        let mut overlay_damaged = false;
        let mut scrollbar_damaged = false;
        let size = usize::from(rect.width) * usize::from(rect.height);
        if self.cells.len() < size {
            self.cells.resize_with(size, RetainedCell::default);
            self.paint.resize(size, Paint::Blank);
        }
        self.paint[..size].fill(Paint::Blank);
        if resized {
            // Match the original blitter: overwrite text on resize, never CSI 2 J.
            // Kitty erase-display also removes ACKed images whose only pixels
            // may now live on the host, so placement replay alone cannot recover.
            if let Some(previous) = self.rect.filter(|old| old.x != rect.x || old.y != rect.y) {
                for y in previous.y..previous.y.saturating_add(previous.height) {
                    for x in previous.x..previous.x.saturating_add(previous.width) {
                        if !rect.contains(x, y) {
                            paint_cell(out, &BLANK, None, x, y)?;
                        }
                    }
                }
            }
            for cell in &mut self.cells {
                cell.valid = false;
            }
        }
        let mut pane_cursor = None;
        let mut chrome_count = 0;
        let mut result = Ok(());
        queue!(out, cursor::Hide)?;
        application(
            core,
            declarations,
            rect,
            self.controls.list_offset,
            &mut |plugin, node, area| {
                if result.is_err() {
                    return;
                }
                if !leaf_row(node)
                    && self
                        .controls
                        .revealed(core, declarations, plugin, node, area)
                {
                    let x = menus::menu_cell(area).expect("revealed menu cell");
                    self.menu_rects.push(Rect {
                        x: x - 1,
                        y: area.y,
                        width: 3,
                        height: 1,
                    });
                }
                if let Node::Column { children } = node {
                    if children
                        .first()
                        .is_some_and(|node| crate::sidebar::starts_panel(node, declarations))
                    {
                        for y in 0..area.height {
                            let start = usize::from(area.y + y - rect.y) * usize::from(rect.width)
                                + usize::from(area.x - rect.x);
                            self.paint[start..start + usize::from(area.width)].fill(Paint::Panel);
                        }
                    }
                }
                let themed = area.width > 0
                    && area.height > 0
                    && matches!(
                        self.paint[usize::from(area.y - rect.y) * usize::from(rect.width)
                            + usize::from(area.x - rect.x)],
                        Paint::Panel
                    );
                if matches!(node, Node::Surface) {
                    if let Some(surface) = surface {
                        pane_cursor = surface
                            .cursor
                            .filter(|(x, y)| *x < area.width && *y < area.height)
                            .map(|(x, y)| (area.x + x, area.y + y));
                        for y in 0..surface.height.min(area.height) {
                            for x in 0..surface.width.min(area.width) {
                                let index = usize::from(area.y + y - rect.y)
                                    * usize::from(rect.width)
                                    + usize::from(area.x + x - rect.x);
                                self.paint[index] = Paint::Surface(
                                    usize::from(y) * usize::from(surface.width) + usize::from(x),
                                );
                            }
                        }
                    }
                    return;
                }
                if !matches!(node, Node::Label { .. } | Node::Leader { .. } | Node::Input)
                    && !leaf_row(node)
                {
                    return;
                }
                self.scratch.clear();
                result = (|| {
                    queue!(
                        self.scratch,
                        SetAttribute(Attribute::Reset),
                        SetForegroundColor(Color::Reset),
                        SetBackgroundColor(Color::Reset)
                    )?;
                    match node {
                        Node::Row { .. } => {
                            let reveal =
                                self.controls
                                    .revealed(core, declarations, plugin, node, area);
                            let menu_x = reveal.then(|| menus::menu_cell(area)).flatten();
                            let content = Rect {
                                width: menu_x.map_or(area.width, |x| x - area.x - 1),
                                ..area
                            };
                            crate::sidebar::draw_row(
                                &mut self.scratch,
                                node,
                                content,
                                core.focused_tab_id.as_deref(),
                                themed,
                            )?;
                            if let Some(x) = menu_x {
                                queue!(
                                    self.scratch,
                                    cursor::MoveTo(x - 1, area.y),
                                    SetBackgroundColor(crate::sidebar::PANEL),
                                    SetForegroundColor(Color::Rgb {
                                        r: 111,
                                        g: 143,
                                        b: 192
                                    }),
                                    Print(" ⋯ "),
                                    SetBackgroundColor(Color::Reset),
                                    SetForegroundColor(Color::Reset)
                                )?;
                            }
                            Ok(())
                        }
                        Node::Label { text, role, tone } => {
                            paint_text(&mut self.scratch, text, role, *tone, area)
                        }
                        Node::Leader { tone } => paint_leader(&mut self.scratch, *tone, area),
                        Node::Input => paint_input(&mut self.scratch, &core.input_text, area),
                        _ => Ok(()),
                    }
                })();
                if self.chrome.len() <= chrome_count {
                    self.chrome.push(Chrome::default());
                }
                let retained = &mut self.chrome[chrome_count];
                let changed = retained.bytes != self.scratch;
                if result.is_ok() && (resized || retained.rect != Some(area) || changed) {
                    overlay_damaged |= menus::intersect(Some(area), overlay);
                    scrollbar_damaged |= menus::intersect(Some(area), rail);
                    result = out.write_all(&self.scratch);
                    if result.is_ok() {
                        if changed {
                            retained.bytes.clone_from(&self.scratch);
                        }
                        retained.rect = Some(area);
                    }
                }
                chrome_count += 1;
                // Text/leader widgets only paint their first row; unused layout rows stay blank.
                if area.height > 0 {
                    for x in 0..area.width {
                        let index = usize::from(area.y - rect.y) * usize::from(rect.width)
                            + usize::from(area.x + x - rect.x);
                        self.paint[index] = Paint::Chrome;
                        self.cells[index].valid = false;
                    }
                }
            },
        );
        result?;
        // Container rows paint through their children. Decorate them afterwards,
        // using the same clipped projection as hover and hit testing.
        for decoration in &self.menu_rects {
            queue!(
                out,
                cursor::MoveTo(decoration.x, decoration.y),
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(crate::sidebar::PANEL),
                SetForegroundColor(Color::Rgb {
                    r: 111,
                    g: 143,
                    b: 192
                }),
                Print(" ⋯ "),
                SetBackgroundColor(Color::Reset),
                SetForegroundColor(Color::Reset)
            )?;
            overlay_damaged |= menus::intersect(Some(*decoration), overlay);
            scrollbar_damaged |= menus::intersect(Some(*decoration), rail);
            for column in decoration.x..decoration.x + decoration.width {
                let index = usize::from(decoration.y - rect.y) * usize::from(rect.width)
                    + usize::from(column - rect.x);
                self.paint[index] = Paint::Chrome;
                self.cells[index].valid = false;
            }
        }
        self.scratch.clear();
        if let (Some((area, maximum)), Some(rail)) = (scrollbar, rail) {
            let height = u32::from(area.height);
            let maximum = u32::from(maximum);
            let thumb = (height * height / (height + maximum)).max(1);
            let offset = u32::from(self.controls.list_offset).min(maximum);
            let start = (offset * (height - thumb) + maximum / 2) / maximum;
            queue!(
                self.scratch,
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(crate::sidebar::PANEL)
            )?;
            for y in 0..area.height {
                let in_thumb = (start..start + thumb).contains(&u32::from(y));
                queue!(
                    self.scratch,
                    cursor::MoveTo(rail.x, rail.y + y),
                    SetForegroundColor(if in_thumb {
                        Color::Rgb {
                            r: 111,
                            g: 143,
                            b: 192,
                        }
                    } else {
                        Color::DarkGrey
                    }),
                    Print(if in_thumb { "┃" } else { "│" })
                )?;
            }
            if self.chrome.len() <= chrome_count {
                self.chrome.push(Chrome::default());
            }
            let retained = &mut self.chrome[chrome_count];
            if resized
                || scrollbar_damaged
                || retained.rect != Some(rail)
                || retained.bytes != self.scratch
            {
                out.write_all(&self.scratch)?;
                retained.bytes.clone_from(&self.scratch);
                retained.rect = Some(rail);
                overlay_damaged |= menus::intersect(Some(rail), overlay);
            }
            chrome_count += 1;
            for y in rail.y..rail.y + rail.height {
                let index = usize::from(y - rect.y) * usize::from(rect.width)
                    + usize::from(rail.x - rect.x);
                self.paint[index] = Paint::Chrome;
                self.cells[index].valid = false;
            }
        }
        self.scrollbar_rect = rail;
        self.scratch.clear();
        if let Some(area) = if let Some(dialog) = dialog {
            modals::draw(
                &mut self.scratch,
                core,
                declarations,
                dialog,
                self.controls.dialog_focus,
            )?;
            Some(dialog.rect)
        } else {
            self.controls
                .draw_popup(&mut self.scratch, core, declarations, rect)?
        } {
            if self.chrome.len() <= chrome_count {
                self.chrome.push(Chrome::default());
            }
            let retained = &mut self.chrome[chrome_count];
            if resized
                || overlay_damaged
                || retained.rect != Some(area)
                || retained.bytes != self.scratch
            {
                out.write_all(&self.scratch)?;
                retained.bytes.clone_from(&self.scratch);
                retained.rect = Some(area);
            }
            chrome_count += 1;
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    let index =
                        usize::from(y - rect.y) * usize::from(rect.width) + usize::from(x - rect.x);
                    self.paint[index] = Paint::Chrome;
                    self.cells[index].valid = false;
                }
            }
        }
        self.overlay_rect = overlay;
        for retained in self.chrome.iter_mut().skip(chrome_count) {
            retained.rect = None;
        }
        for (index, paint) in self.paint[..size].iter().enumerate() {
            let cell = match paint {
                Paint::Chrome => continue,
                Paint::Surface(index) => {
                    &surface.expect("surface paint requires a surface").cells[*index]
                }
                Paint::Panel => &PANEL_BLANK,
                Paint::Blank => &BLANK,
            };
            let hyperlink = match paint {
                Paint::Surface(_) => cell_hyperlink(&self.hyperlinks, cell),
                _ => None,
            };
            let retained = &mut self.cells[index];
            let same_link = same_hyperlink(retained.hyperlink.as_ref(), hyperlink);
            if retained.valid
                && retained.symbol == cell.symbol
                && retained.foreground == cell.foreground
                && retained.background == cell.background
                && retained.modifiers == cell.modifiers
                && same_link
            {
                continue;
            }
            paint_cell(
                out,
                cell,
                hyperlink.map(AsRef::as_ref),
                rect.x + (index % usize::from(rect.width)) as u16,
                rect.y + (index / usize::from(rect.width)) as u16,
            )?;
            if retained.symbol != cell.symbol {
                retained.symbol.clone_from(&cell.symbol);
            }
            retained.foreground = cell.foreground;
            retained.background = cell.background;
            retained.modifiers = cell.modifiers;
            if !same_link {
                retained.hyperlink = hyperlink.cloned();
            }
            retained.valid = true;
        }
        self.reorder.draw_indicator(out)?;
        if let Some(area) = indicator {
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    if rect.contains(x, y) {
                        self.cells[usize::from(y - rect.y) * usize::from(rect.width)
                            + usize::from(x - rect.x)]
                        .valid = false;
                    }
                }
            }
        }
        self.reorder_rect = indicator;
        self.copy_feedback_rect = core
            .copy_feedback()
            .and_then(|message| copy_feedback_rect(rect, message, self.copy_feedback_position));
        if let Some(area) = self.copy_feedback_rect {
            draw_copy_feedback(out, area, core.copy_feedback().expect("visible feedback"))?;
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    self.cells[usize::from(y - rect.y) * usize::from(rect.width)
                        + usize::from(x - rect.x)]
                    .valid = false;
                }
            }
        }
        let hint_viewport = self.copy_feedback_rect.map_or(rect, |copy| {
            let top = matches!(
                self.copy_feedback_position,
                ToastClipboardPosition::TopLeft
                    | ToastClipboardPosition::TopCenter
                    | ToastClipboardPosition::TopRight
            );
            Rect {
                y: rect.y + if top { copy.height } else { 0 },
                height: rect.height.saturating_sub(copy.height),
                ..rect
            }
        });
        let navigation_hint = if core.prefix_active {
            Some("Ctrl-B · waiting for key")
        } else {
            core.tab_cycle_hint()
        };
        self.navigation_hint_rect = navigation_hint.and_then(|message| {
            copy_feedback_rect(hint_viewport, message, self.copy_feedback_position)
        });
        if let Some(area) = self.navigation_hint_rect {
            draw_copy_feedback(out, area, navigation_hint.expect("visible navigation hint"))?;
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    self.cells[usize::from(y - rect.y) * usize::from(rect.width)
                        + usize::from(x - rect.x)]
                    .valid = false;
                }
            }
        }
        if let Some((x, y)) = pane_cursor.filter(|&(x, y)| {
            !self.controls.modal()
                && dialog.is_none()
                && !self
                    .copy_feedback_rect
                    .is_some_and(|area| area.contains(x, y))
                && !self
                    .navigation_hint_rect
                    .is_some_and(|area| area.contains(x, y))
        }) {
            queue!(out, cursor::MoveTo(x, y), cursor::Show)?;
        }
        queue!(
            out,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Reset),
            SetBackgroundColor(Color::Reset)
        )?;
        out.flush()?;
        self.rect = Some(rect);
        Ok(())
    }
}
fn copy_feedback_rect(
    viewport: Rect,
    message: &str,
    position: ToastClipboardPosition,
) -> Option<Rect> {
    if viewport.width == 0 || viewport.height == 0 {
        return None;
    }
    let width = (message.width() as u16)
        .saturating_add(4)
        .min(viewport.width);
    let height = 3.min(viewport.height);
    let x = viewport.x
        + match position {
            ToastClipboardPosition::TopLeft | ToastClipboardPosition::BottomLeft => 0,
            ToastClipboardPosition::TopCenter | ToastClipboardPosition::BottomCenter => {
                (viewport.width - width) / 2
            }
            ToastClipboardPosition::TopRight | ToastClipboardPosition::BottomRight => {
                viewport.width - width
            }
        };
    let y = viewport.y
        + match position {
            ToastClipboardPosition::TopLeft
            | ToastClipboardPosition::TopCenter
            | ToastClipboardPosition::TopRight => 0,
            ToastClipboardPosition::BottomLeft
            | ToastClipboardPosition::BottomCenter
            | ToastClipboardPosition::BottomRight => viewport.height - height,
        };
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}
fn draw_copy_feedback(out: &mut impl Write, area: Rect, message: &str) -> io::Result<()> {
    // Original Herdr clipboard card: green border/dot, panel background, bold text.
    let green = Color::Rgb {
        r: 166,
        g: 227,
        b: 161,
    };
    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(Color::Rgb {
            r: 24,
            g: 24,
            b: 37
        }),
        SetForegroundColor(green)
    )?;
    for y in 0..area.height {
        queue!(out, cursor::MoveTo(area.x, area.y + y))?;
        for x in 0..area.width {
            let symbol = match (x, y) {
                (0, 0) => '┌',
                (x, 0) if x + 1 == area.width => '┐',
                (0, y) if y + 1 == area.height => '└',
                (x, y) if x + 1 == area.width && y + 1 == area.height => '┘',
                (_, y) if y == 0 || y + 1 == area.height => '─',
                (x, _) if x == 0 || x + 1 == area.width => '│',
                _ => ' ',
            };
            queue!(out, Print(symbol))?;
        }
    }
    if area.height > 2 && area.width > 2 {
        queue!(out, cursor::MoveTo(area.x + 1, area.y + 1), Print("●"))?;
        if area.width > 3 {
            queue!(
                out,
                Print(" "),
                SetForegroundColor(Color::Rgb {
                    r: 205,
                    g: 214,
                    b: 244
                }),
                SetAttribute(Attribute::Bold)
            )?;
            crate::sidebar::prefix(out, message, usize::from(area.width.saturating_sub(4)))?;
        }
    }
    Ok(())
}
static BLANK: std::sync::LazyLock<crate::Cell> = std::sync::LazyLock::new(|| crate::Cell {
    symbol: " ".into(),
    foreground: 0,
    background: 0,
    modifiers: 0,
    hyperlink: None,
});
static PANEL_BLANK: std::sync::LazyLock<crate::Cell> = std::sync::LazyLock::new(|| crate::Cell {
    symbol: " ".into(),
    foreground: 0,
    background: 0x020d0a10,
    modifiers: 0,
    hyperlink: None,
});
const CLOSE_HYPERLINK: &[u8] = b"\x1b]8;;\x1b\\";
type Hyperlink = (Option<std::sync::Arc<str>>, Option<std::sync::Arc<str>>);
fn same_hyperlink(a: Option<&std::sync::Arc<str>>, b: Option<&std::sync::Arc<str>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b) || a == b,
        _ => false,
    }
}
fn refresh_hyperlinks(table: &mut Vec<Hyperlink>, surface: Option<&Surface>) {
    let uris = surface.map_or(&[][..], |surface| surface.hyperlinks.as_slice());
    table.resize_with(uris.len(), || (None, None));
    for ((previous, sanitized), uri) in table.iter_mut().zip(uris) {
        if same_hyperlink(previous.as_ref(), uri.as_ref()) {
            continue;
        }
        *sanitized = uri.as_ref().and_then(|uri| {
            if uri.chars().any(char::is_control) {
                let clean: String = uri.chars().filter(|ch| !ch.is_control()).collect();
                (!clean.is_empty()).then(|| std::sync::Arc::<str>::from(clean))
            } else {
                (!uri.is_empty()).then(|| uri.clone())
            }
        });
        previous.clone_from(uri);
    }
}
fn cell_hyperlink<'a>(
    table: &'a [Hyperlink],
    cell: &crate::Cell,
) -> Option<&'a std::sync::Arc<str>> {
    table.get(cell.hyperlink? as usize)?.1.as_ref()
}
fn paint_cell(
    out: &mut impl Write,
    cell: &crate::Cell,
    hyperlink: Option<&str>,
    x: u16,
    y: u16,
) -> io::Result<()> {
    queue!(
        out,
        cursor::MoveTo(x, y),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(color(cell.foreground)),
        SetBackgroundColor(color(cell.background))
    )?;
    for (mask, attribute) in [
        (1, Attribute::Bold),
        (4, Attribute::Italic),
        (8, Attribute::Underlined),
        (2, Attribute::Dim),
        (16, Attribute::SlowBlink),
        (64, Attribute::Reverse),
        (256, Attribute::CrossedOut),
    ] {
        if cell.modifiers & mask != 0 {
            queue!(out, SetAttribute(attribute))?;
        }
    }
    if let Some(uri) = hyperlink {
        let result = (|| {
            write!(out, "\x1b]8;;{uri}\x1b\\")?;
            queue!(out, Print(&cell.symbol))
        })();
        let close = out.write_all(CLOSE_HYPERLINK);
        result.and(close)
    } else {
        queue!(out, Print(&cell.symbol))
    }
}
fn paint_input(out: &mut impl Write, text: &str, rect: Rect) -> io::Result<()> {
    if rect.width == 0 {
        return Ok(());
    }
    let budget = usize::from(rect.width - 1);
    let mut start = text.len();
    let mut width = 0;
    for (index, ch) in text.char_indices().rev() {
        let next = width + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if next > budget {
            break;
        }
        start = index;
        width = next;
    }
    let visible = if text.is_empty() {
        " type name"
    } else {
        &text[start..]
    };
    paint_text(out, visible, "label", None, rect)?;
    paint_text(
        out,
        "▏",
        "label",
        None,
        Rect {
            x: rect.x + width as u16,
            width: 1,
            ..rect
        },
    )
}
fn paint_text(
    out: &mut impl Write,
    text: &str,
    role: &str,
    tone: Option<Tone>,
    rect: Rect,
) -> io::Result<()> {
    if role == "title" {
        return crate::sidebar::draw_title(out, text, tone, rect);
    }
    let (fg, bg) = crate::sidebar::segment_colors(role, tone, Color::Reset);
    queue!(
        out,
        cursor::MoveTo(rect.x, rect.y),
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(bg),
        SetForegroundColor(fg)
    )?;
    let width = crate::sidebar::prefix(out, text, usize::from(rect.width))?;
    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset),
        SetBackgroundColor(Color::Reset)
    )?;
    for _ in width..usize::from(rect.width) {
        queue!(out, Print(' '))?;
    }
    Ok(())
}
fn paint_leader(out: &mut impl Write, tone: Option<Tone>, rect: Rect) -> io::Result<()> {
    let (fg, bg) = crate::sidebar::segment_colors("leader", tone, Color::Reset);
    queue!(
        out,
        cursor::MoveTo(rect.x, rect.y),
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(bg),
        SetForegroundColor(fg)
    )?;
    for _ in 0..rect.width {
        queue!(out, Print('·'))?;
    }
    Ok(())
}
