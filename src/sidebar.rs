//! Terminal-only widget layout; shared declarations contain no cell geometry.
use crate::terminal::Rect;
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};
use trailboss_runtime::protocol::widgets::{Context, Declaration, Node, Tone};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub type Declarations = Vec<(String, Declaration)>;

pub fn rows(declarations: &Declarations) -> impl Iterator<Item = (&str, &Node)> {
    declarations.iter().flat_map(|(plugin, declaration)| {
        declaration.zones.iter().flat_map(move |placement| {
            let children = match &placement.tree {
                Node::Column { children } => children.as_slice(),
                _ => &[],
            };
            children.iter().map(move |row| (plugin.as_str(), row))
        })
    })
}
pub fn action_at(declarations: &Declarations, row: usize) -> Option<(&str, &str, &Context)> {
    let (
        plugin,
        Node::Row {
            action: Some(action),
            context,
            ..
        },
    ) = rows(declarations).nth(row)?
    else {
        return None;
    };
    Some((plugin, action, context))
}
pub(crate) fn role_color(role: &str) -> Color {
    match role {
        "muted" | "rule" | "leader" | "status-idle" => Color::DarkGrey,
        "status-working" => Color::Yellow,
        "status-blocked" => Color::Red,
        "status-waiting" => Color::Blue,
        "status-done" => Color::Green,
        _ => Color::Reset,
    }
}
pub(crate) fn segment_colors(role: &str, tone: Option<Tone>, background: Color) -> (Color, Color) {
    match tone {
        Some(Tone::BgLime) => (role_color(role), Color::Green),
        Some(Tone::FgCyan) => (Color::Cyan, background),
        None => (role_color(role), background),
    }
}
fn width(node: &Node) -> usize {
    match node {
        Node::Label { text, .. } => text.width(),
        _ => 0,
    }
}
pub fn draw(
    out: &mut impl Write,
    declarations: &Declarations,
    rect: Rect,
    focused_tab: Option<&str>,
) -> io::Result<()> {
    let preview_rows = declarations.iter().flat_map(|(_, declaration)| {
        declaration.zones.iter().flat_map(|placement| {
            let themed = starts_panel(&placement.tree, declarations);
            let children = match &placement.tree {
                Node::Column { children } => children.as_slice(),
                _ => &[],
            };
            children.iter().map(move |node| (node, themed))
        })
    });
    for (y, (node, themed)) in preview_rows.take(usize::from(rect.height)).enumerate() {
        draw_row(
            out,
            node,
            Rect {
                y: rect.y + y as u16,
                ..rect
            },
            focused_tab,
            themed,
        )?;
    }
    out.flush()
}
// CSS panel and translucent selection resolved to opaque terminal RGB cells.
pub(crate) const PANEL: Color = Color::Rgb {
    r: 13,
    g: 10,
    b: 16,
};
const SELECTED: Color = Color::Rgb {
    r: 26,
    g: 38,
    b: 61,
};
const EDGE: Color = Color::Rgb {
    r: 77,
    g: 127,
    b: 184,
};
fn themed_color(role: &str, focused: bool) -> Color {
    match role {
        "muted" | "title" => Color::Rgb {
            r: 111,
            g: 104,
            b: 117,
        },
        "rule" | "leader" => Color::Rgb {
            r: 100,
            g: 94,
            b: 106,
        },
        "status-idle" => Color::Rgb {
            r: 74,
            g: 69,
            b: 80,
        },
        "status-done" => Color::Rgb {
            r: 123,
            g: 184,
            b: 121,
        },
        "status-working" => Color::Rgb {
            r: 226,
            g: 162,
            b: 58,
        },
        "status-blocked" => Color::Rgb {
            r: 208,
            g: 90,
            b: 85,
        },
        "status-waiting" => Color::Rgb {
            r: 111,
            g: 143,
            b: 192,
        },
        _ if focused => Color::Rgb {
            r: 242,
            g: 238,
            b: 245,
        },
        _ => Color::Rgb {
            r: 185,
            g: 179,
            b: 189,
        },
    }
}
fn spaces(out: &mut impl Write, count: usize) -> io::Result<()> {
    for _ in 0..count {
        queue!(out, Print(' '))?;
    }
    Ok(())
}
pub(crate) fn prefix(out: &mut impl Write, text: &str, budget: usize) -> io::Result<usize> {
    let mut used = 0;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if used + width > budget || (matches!(ch, '…' | '⋯') && used + width >= budget) {
            break;
        }
        queue!(out, Print(ch))?;
        used += width;
    }
    Ok(used)
}
pub(crate) fn clipped(out: &mut impl Write, text: &str, limit: usize) -> io::Result<usize> {
    let width = text.width();
    let truncated = width > limit || (text.ends_with(['…', '⋯']) && width == limit);
    if !truncated {
        return prefix(out, text, limit);
    }
    overflow(out, text, limit)
}
fn overflow(out: &mut impl Write, text: &str, limit: usize) -> io::Result<usize> {
    if limit < 2 {
        return prefix(out, text, limit);
    }
    let used = prefix(out, text, limit - 2)?;
    queue!(out, Print("… "))?;
    Ok(used + 2)
}
pub(crate) fn draw_title(
    out: &mut impl Write,
    text: &str,
    tone: Option<Tone>,
    rect: Rect,
) -> io::Result<()> {
    let (fg, bg) = match tone {
        Some(tone) => segment_colors("title", Some(tone), PANEL),
        None => (themed_color("title", false), PANEL),
    };
    queue!(
        out,
        cursor::MoveTo(rect.x, rect.y),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(fg),
        SetBackgroundColor(bg)
    )?;
    let width = usize::from(rect.width);
    let left = width.saturating_sub(text.width()) / 2;
    spaces(out, left)?;
    let used = clipped(out, text, width - left)?;
    spaces(out, width - left - used)?;
    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(Color::Reset),
        SetForegroundColor(Color::Reset)
    )
}
fn draw_themed_row(
    out: &mut impl Write,
    children: &[Node],
    context: &Context,
    rect: Rect,
    focused_tab: Option<&str>,
) -> io::Result<()> {
    let section = matches!(children.first(), Some(Node::Label { role, .. }) if role == "section");
    let tab_row =
        matches!(children.first(), Some(Node::Label { role, .. }) if role.starts_with("status-"));
    let padded = section || tab_row || context.pane_id.is_some();
    let focused = tab_row
        && !section
        && context.pane_id.is_none()
        && context
            .tab_id
            .as_deref()
            .is_some_and(|id| Some(id) == focused_tab);
    let bg = if focused { SELECTED } else { PANEL };
    queue!(
        out,
        cursor::MoveTo(rect.x, rect.y),
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(bg)
    )?;
    let width = usize::from(rect.width);
    let padding = if section {
        1
    } else if padded {
        3
    } else {
        0
    };
    let left = padding.min(width);
    if focused && left > 0 {
        queue!(out, SetForegroundColor(EDGE), Print('▏'))?;
        spaces(out, left - 1)?;
    } else {
        spaces(out, left)?;
    }
    let right = usize::from(padded && width > left);
    let mut remaining = width.saturating_sub(left + right);
    let leaders = children
        .iter()
        .filter(|node| matches!(node, Node::Leader { .. }))
        .count();
    let tail: usize = children.iter().skip(1).map(width_of).sum();
    let rule = section && remaining > tail;
    let gaps = usize::from(section && remaining >= tail + leaders * 2 + 2) * leaders * 2;
    let fixed: usize = children.iter().map(width_of).sum::<usize>() + gaps;
    let name_limit = if section {
        remaining.saturating_sub(tail + gaps + usize::from(rule && leaders > 0))
    } else {
        remaining
    };
    let name_width = children.first().map(width_of).unwrap_or(0);
    let fixed = if section {
        fixed - name_width + name_width.min(name_limit)
    } else {
        fixed
    };
    let elastic = remaining.saturating_sub(fixed);
    let count_width: usize = if section {
        children
            .iter()
            .filter(|node| matches!(node, Node::Label { role, .. } if role == "muted"))
            .map(width_of)
            .sum()
    } else {
        0
    };
    let mut status_width: usize = if section {
        children
            .iter()
            .filter(|node| matches!(node, Node::Label { role, .. } if role.starts_with("status-")))
            .map(width_of)
            .sum()
    } else {
        0
    };
    let mut cluster_clipped = false;
    let mut leader_index = 0;
    for (index, child) in children.iter().enumerate() {
        match child {
            Node::Label { text, role, tone } => {
                let (fg, background) = match tone {
                    Some(tone) => segment_colors(role, Some(*tone), bg),
                    None => (themed_color(role, focused), bg),
                };
                queue!(
                    out,
                    SetForegroundColor(fg),
                    SetBackgroundColor(background),
                    SetAttribute(if role == "section" || (focused && role == "label") {
                        Attribute::Bold
                    } else {
                        Attribute::NormalIntensity
                    })
                )?;
                if section && role.starts_with("status-") {
                    if cluster_clipped {
                        continue;
                    }
                    let available = remaining.saturating_sub(count_width);
                    if status_width > available && available <= text.width() + 1 {
                        let used = overflow(out, text, available)?;
                        remaining -= used;
                        cluster_clipped = true;
                        continue;
                    }
                    status_width = status_width.saturating_sub(text.width());
                }
                let limit = if section && index == 0 {
                    name_limit.min(remaining)
                } else {
                    remaining
                };
                remaining -= clipped(out, text, limit)?;
            }
            Node::Leader { tone } => {
                let before = usize::from(gaps > 0 && remaining > 0);
                spaces(out, before)?;
                remaining -= before;
                let count = (elastic / leaders + usize::from(leader_index < elastic % leaders))
                    .min(remaining.saturating_sub(usize::from(gaps > 0)));
                let (fg, background) = match tone {
                    Some(tone) => segment_colors("leader", Some(*tone), bg),
                    None => (themed_color("leader", false), bg),
                };
                queue!(
                    out,
                    SetForegroundColor(fg),
                    SetBackgroundColor(background),
                    SetAttribute(Attribute::NormalIntensity)
                )?;
                for _ in 0..count {
                    queue!(out, Print(if section { '─' } else { '·' }))?;
                }
                remaining -= count;
                let after = usize::from(gaps > 0 && remaining > 0);
                spaces(out, after)?;
                remaining -= after;
                leader_index += 1;
            }
            _ => {}
        }
    }
    queue!(
        out,
        SetBackgroundColor(bg),
        SetForegroundColor(themed_color("label", focused)),
        SetAttribute(Attribute::NormalIntensity)
    )?;
    spaces(out, remaining + right)?;
    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(Color::Reset),
        SetForegroundColor(Color::Reset)
    )
}
fn width_of(node: &Node) -> usize {
    width(node)
}
pub(crate) fn starts_panel(node: &Node, declarations: &Declarations) -> bool {
    fn first(node: &Node, declarations: &Declarations, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        let next = |node: &Node| first(node, declarations, depth + 1);
        match node {
            Node::Label { role, .. } => role == "title" || role == "section",
            Node::Row { children, .. } | Node::Column { children } => {
                children.first().is_some_and(next)
            }
            Node::Flex { child, .. } => next(child),
            Node::Zone { zone, fallback } => {
                let placement = declarations
                    .iter()
                    .flat_map(|(_, declaration)| &declaration.zones)
                    .find(|placement| &placement.zone == zone);
                match placement {
                    Some(placement) => next(&placement.tree),
                    None => fallback.as_deref().is_some_and(next),
                }
            }
            Node::List { entries } => entries.first().is_some_and(next),
            _ => false,
        }
    }
    first(node, declarations, 0)
}
pub(crate) fn draw_row(
    out: &mut impl Write,
    node: &Node,
    rect: Rect,
    focused_tab: Option<&str>,
    themed: bool,
) -> io::Result<()> {
    let Node::Row {
        children, context, ..
    } = node
    else {
        return Ok(());
    };
    if let [Node::Label { text, role, tone }] = children.as_slice() {
        if role == "title" {
            return draw_title(out, text, *tone, rect);
        }
    }
    if themed || matches!(children.first(), Some(Node::Label { role, .. }) if role == "section") {
        return draw_themed_row(out, children, context, rect, focused_tab);
    }
    let focused = context.pane_id.is_none()
        && context
            .tab_id
            .as_deref()
            .is_some_and(|id| Some(id) == focused_tab);
    queue!(
        out,
        cursor::MoveTo(rect.x, rect.y),
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(if focused {
            Color::DarkBlue
        } else {
            Color::Reset
        })
    )?;
    if focused {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    let fixed: usize = children.iter().map(width).sum();
    let leaders = children
        .iter()
        .filter(|node| matches!(node, Node::Leader { .. }))
        .count();
    let elastic = usize::from(rect.width).saturating_sub(fixed);
    let mut remaining = usize::from(rect.width);
    let mut leader_index = 0;
    for child in children {
        match child {
            Node::Label { text, role, tone } => {
                let (fg, bg) = segment_colors(
                    role,
                    *tone,
                    if focused {
                        Color::DarkBlue
                    } else {
                        Color::Reset
                    },
                );
                queue!(out, SetForegroundColor(fg), SetBackgroundColor(bg))?;
                remaining -= prefix(out, text, remaining)?;
            }
            Node::Leader { tone } => {
                let count = (elastic / leaders + usize::from(leader_index < elastic % leaders))
                    .min(remaining);
                let (fg, bg) = segment_colors(
                    "leader",
                    *tone,
                    if focused {
                        Color::DarkBlue
                    } else {
                        Color::Reset
                    },
                );
                queue!(out, SetForegroundColor(fg), SetBackgroundColor(bg))?;
                for _ in 0..count {
                    queue!(out, Print('·'))?;
                }
                remaining -= count;
                leader_index += 1;
            }
            _ => {}
        }
    }
    queue!(
        out,
        SetForegroundColor(Color::Reset),
        SetBackgroundColor(if focused {
            Color::DarkBlue
        } else {
            Color::Reset
        })
    )?;
    for _ in 0..remaining {
        queue!(out, Print(' '))?;
    }

    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(Color::Reset),
        SetForegroundColor(Color::Reset)
    )?;
    Ok(())
}
