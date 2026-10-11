//! Terminal-only widget layout; shared declarations contain no cell geometry.
use crate::terminal::Rect;
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use herdr_runtime::protocol::widgets::{Context, Declaration, Node, Tone};
use std::io::{self, Write};
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
        "status-waiting" => Color::Cyan,
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
    for (y, (_, node)) in rows(declarations)
        .take(usize::from(rect.height))
        .enumerate()
    {
        draw_row(
            out,
            node,
            Rect {
                y: rect.y + y as u16,
                ..rect
            },
            focused_tab,
        )?;
    }
    out.flush()
}
pub(crate) fn draw_row(
    out: &mut impl Write,
    node: &Node,
    rect: Rect,
    focused_tab: Option<&str>,
) -> io::Result<()> {
    let Node::Row {
        children, context, ..
    } = node
    else {
        return Ok(());
    };
    let focused = context
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
                for ch in text.chars() {
                    let width = ch.width().unwrap_or(0);
                    if width > remaining {
                        break;
                    }
                    queue!(out, Print(ch))?;
                    remaining -= width;
                }
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
