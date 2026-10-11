//! Terminal adapter: declarations select every visible node and its placement.
use crate::{core::Core, sidebar::Declarations, Surface};
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use herdr_runtime::protocol::widgets::{Context, Node};
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
    fn contains(self, x: u16, y: u16) -> bool {
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
    for y in 0..surface.height.min(rect.height) {
        for x in 0..surface.width.min(rect.width) {
            let c = &surface.cells[usize::from(y) * usize::from(surface.width) + usize::from(x)];
            queue!(
                out,
                cursor::MoveTo(rect.x + x, rect.y + y),
                SetAttribute(Attribute::Reset),
                SetForegroundColor(color(c.foreground)),
                SetBackgroundColor(color(c.background))
            )?;
            for (mask, attribute) in [
                (1, Attribute::Bold),
                (4, Attribute::Italic),
                (8, Attribute::Underlined),
                (2, Attribute::Dim),
                (16, Attribute::SlowBlink),
                (256, Attribute::CrossedOut),
            ] {
                if c.modifiers & mask != 0 {
                    queue!(out, SetAttribute(attribute))?;
                }
            }
            queue!(out, Print(&c.symbol))?;
        }
    }
    Ok(())
}
fn leaf_row(node: &Node) -> bool {
    matches!(node, Node::Row { children, .. } if children.iter().all(|n| matches!(n, Node::Label { .. } | Node::Leader)))
}
fn extent(core: &Core, node: &Node, horizontal: bool) -> (u16, u16) {
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
        Node::TabName => {
            if horizontal {
                (0, 1)
            } else {
                (1, 0)
            }
        }
        Node::Column { children } | Node::Row { children, .. } => {
            let parallel = matches!(node, Node::Row { .. }) == horizontal;
            let mut fixed = 0u16;
            for child in children {
                let (size, weight) = extent(core, child, horizontal);
                if weight > 0 {
                    return (0, 1);
                }
                fixed = if parallel {
                    fixed.saturating_add(size)
                } else {
                    fixed.max(size)
                };
            }
            (fixed.max(u16::from(!horizontal)), 0)
        }
        Node::Surface | Node::Zone { .. } | Node::Leader => (0, 1),
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
    visit: &mut impl FnMut(&'a str, &'a Node, Rect),
) {
    if depth > 16 || rect.width == 0 || rect.height == 0 {
        return;
    }
    let Some(node) = core.resolve(node) else {
        return;
    };
    visit(plugin, node, rect);
    match node {
        Node::Flex { child, .. } => walk(core, declarations, plugin, child, rect, depth + 1, visit),
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
                .map(|(_, tree)| extent(core, tree, false))
                .fold((0u64, 0u64), |(fixed, weights), (size, weight)| {
                    (fixed + u64::from(size), weights + u64::from(weight))
                });
            let flexible = u64::from(rect.height).saturating_sub(fixed);
            let mut found = false;
            let mut offset = 0;
            let mut cumulative = 0;
            for (id, tree) in placements {
                found = true;
                let (size, weight) = extent(core, tree, false);
                let before = cumulative;
                cumulative += u64::from(weight);
                let allocated = (flexible * cumulative).checked_div(weights).unwrap_or(0)
                    - (flexible * before).checked_div(weights).unwrap_or(0);
                let height = u64::from(size)
                    .saturating_add(allocated)
                    .min(u64::from(rect.height - offset)) as u16;
                let placement = Rect {
                    y: rect.y + offset,
                    height,
                    ..rect
                };
                walk(core, declarations, id, tree, placement, depth + 1, visit);
                offset += height;
            }
            if !found {
                if let Some(child) = fallback {
                    walk(core, declarations, plugin, child, rect, depth + 1, visit);
                }
            }
        }
        Node::Row { children, .. } | Node::Column { children } if !leaf_row(node) => {
            let horizontal = matches!(node, Node::Row { .. });
            let available = if horizontal { rect.width } else { rect.height };
            let (fixed, weights) = children
                .iter()
                .map(|child| extent(core, child, horizontal))
                .fold((0u64, 0u64), |(f, w), (a, b)| {
                    (f + u64::from(a), w + u64::from(b))
                });
            let flexible = u64::from(available).saturating_sub(fixed);
            let mut offset = 0;
            let mut cumulative = 0;
            for child in children {
                let (size, weight) = extent(core, child, horizontal);
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
                    visit,
                );
                offset += size;
            }
        }
        _ => {}
    }
}
fn application<'a>(
    core: &Core,
    declarations: &'a Declarations,
    rect: Rect,
    visit: &mut impl FnMut(&'a str, &'a Node, Rect),
) {
    if let Some((plugin, root)) = core.composition(declarations) {
        walk(core, declarations, plugin, root, rect, 0, visit);
    }
}
pub fn surface_rect(core: &Core, declarations: &Declarations, rect: Rect) -> Option<Rect> {
    let mut target = None;
    application(core, declarations, rect, &mut |_, node, rect| {
        if matches!(node, Node::Surface) {
            target = Some(rect);
        }
    });
    target
}
pub fn action_at<'a>(
    core: &Core,
    declarations: &'a Declarations,
    rect: Rect,
    x: u16,
    y: u16,
) -> Option<(&'a str, &'a str, &'a Context)> {
    let mut target = None;
    application(core, declarations, rect, &mut |plugin, node, rect| {
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
    });
    target
}
pub fn draw_frame(
    out: &mut impl Write,
    surface: Option<&Surface>,
    declarations: &Declarations,
    core: &Core,
    rect: Rect,
) -> io::Result<()> {
    queue!(
        out,
        cursor::Hide,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset),
        SetBackgroundColor(Color::Reset),
        Clear(ClearType::All)
    )?;
    let mut result = Ok(());
    let mut pane_cursor = None;
    application(core, declarations, rect, &mut |_, node, rect| {
        if result.is_err() {
            return;
        }
        result = match node {
            Node::Surface => surface.map_or(Ok(()), |surface| {
                pane_cursor = surface
                    .cursor
                    .filter(|(x, y)| *x < rect.width && *y < rect.height)
                    .map(|(x, y)| (rect.x + x, rect.y + y));
                draw(out, surface, rect)
            }),
            Node::Row { .. } if leaf_row(node) => {
                crate::sidebar::draw_row(out, node, rect, core.focused_tab_id.as_deref())
            }
            Node::Label { text, role } => paint_text(out, text, role, rect),
            Node::TabName => paint_text(out, &core.tab_name, "label", rect),
            _ => Ok(()),
        };
    });
    result?;
    if let Some((x, y)) = pane_cursor {
        queue!(out, cursor::MoveTo(x, y), cursor::Show)?;
    }
    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset),
        SetBackgroundColor(Color::Reset)
    )?;
    out.flush()
}
fn paint_text(out: &mut impl Write, text: &str, role: &str, rect: Rect) -> io::Result<()> {
    queue!(
        out,
        cursor::MoveTo(rect.x, rect.y),
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(Color::Reset),
        SetForegroundColor(crate::sidebar::role_color(role))
    )?;
    let mut width = 0;
    for ch in text.chars() {
        width += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width > usize::from(rect.width) {
            break;
        }
        queue!(out, Print(ch))?;
    }
    Ok(())
}
