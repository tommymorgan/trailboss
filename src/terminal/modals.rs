//! Out-of-flow public modal declarations. No action-specific rendering.
use super::menus::{Action, Response};
use super::{extent, leaf_row, paint_input, paint_leader, paint_text, walk, Rect};
use crate::{core::Core, sidebar::Declarations};
use crossterm::{
    cursor,
    event::{KeyCode, KeyEvent},
    queue,
    style::{Color, Print, SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};
use trailboss_runtime::protocol::widgets::Node;

#[derive(Clone, Copy)]
pub(super) struct Dialog<'a> {
    plugin: &'a str,
    tree: &'a Node,
    pub rect: Rect,
}
fn find<'a>(
    core: &Core,
    declarations: &'a Declarations,
    plugin: &'a str,
    node: &'a Node,
    depth: usize,
) -> Option<(&'a str, &'a Node)> {
    if depth > 16 {
        return None;
    }
    let node = core.resolve(node)?;
    match node {
        Node::Modal { child } => Some((plugin, child)),
        Node::Use { name } => {
            let (owner, tree) = core.fragment(declarations, name)?;
            find(core, declarations, owner, tree, depth + 1)
        }
        Node::Flex { child, .. } => find(core, declarations, plugin, child, depth + 1),
        Node::Zone { zone, fallback } => {
            let mut supplied = false;
            for (owner, declaration) in declarations {
                for placement in &declaration.zones {
                    if &placement.zone == zone {
                        supplied = true;
                        if let Some(found) =
                            find(core, declarations, owner, &placement.tree, depth + 1)
                        {
                            return Some(found);
                        }
                    }
                }
            }
            if supplied {
                None
            } else {
                fallback
                    .as_ref()
                    .and_then(|tree| find(core, declarations, plugin, tree, depth + 1))
            }
        }
        _ => core.flow(node).and_then(|(_, children)| {
            children
                .iter()
                .find_map(|child| find(core, declarations, plugin, child, depth + 1))
        }),
    }
}
pub(super) fn dialog<'a>(
    core: &Core,
    declarations: &'a Declarations,
    viewport: Rect,
) -> Option<Dialog<'a>> {
    let (plugin, root) = core.composition(declarations)?;
    let (plugin, tree) = find(core, declarations, plugin, root, 0)?;
    Some(from_tree(core, declarations, plugin, tree, viewport))
}

pub(super) fn from_tree<'a>(
    core: &Core,
    declarations: &Declarations,
    plugin: &'a str,
    tree: &'a Node,
    viewport: Rect,
) -> Dialog<'a> {
    let width = 48.min(viewport.width);
    let height = extent(core, declarations, tree, false)
        .0
        .saturating_add(2)
        .min(viewport.height);
    Dialog {
        plugin,
        tree,
        rect: Rect {
            x: viewport.x + (viewport.width - width) / 2,
            y: viewport.y + (viewport.height - height) / 2,
            width,
            height,
        },
    }
}
fn content(rect: Rect) -> Rect {
    Rect {
        x: rect.x + u16::from(rect.width > 0),
        y: rect.y + u16::from(rect.height > 0),
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    }
}
pub(super) fn visit<'a>(
    core: &Core,
    declarations: &'a Declarations,
    dialog: Dialog<'a>,
    callback: &mut impl FnMut(&'a str, &'a Node, Rect),
) {
    walk(
        core,
        declarations,
        dialog.plugin,
        dialog.tree,
        content(dialog.rect),
        0,
        0,
        callback,
    );
}
pub(super) fn actionable(node: &Node) -> bool {
    matches!(
        node,
        Node::Input
            | Node::Row {
                action: Some(_),
                ..
            }
    )
}
fn action(plugin: &str, node: &Node) -> Option<Action> {
    if let Node::Row {
        action: Some(action),
        context,
        ..
    } = node
    {
        Some(Action {
            plugin: plugin.into(),
            action: action.clone(),
            context: context.clone(),
        })
    } else {
        None
    }
}
pub(super) fn click(
    core: &Core,
    declarations: &Declarations,
    dialog: Dialog<'_>,
    x: u16,
    y: u16,
    focus: &mut usize,
) -> Response {
    let mut chosen = None;
    let mut index = 0;
    visit(core, declarations, dialog, &mut |plugin, node, rect| {
        if actionable(node) {
            if rect.contains(x, y) {
                *focus = index;
                chosen = action(plugin, node);
            }
            index += 1;
        }
    });
    chosen.map_or(Response::Consumed, Response::Invoke)
}
pub(super) fn key(
    core: &Core,
    declarations: &Declarations,
    dialog: Dialog<'_>,
    key: KeyEvent,
    focus: &mut usize,
) -> Response {
    let mut count = 0;
    let mut chosen = None;
    visit(core, declarations, dialog, &mut |plugin, node, _| {
        if actionable(node) {
            if count == *focus {
                chosen = action(plugin, node);
            }
            count += 1;
        }
    });
    match key.code {
        KeyCode::Tab | KeyCode::BackTab if count > 0 => {
            *focus = (*focus
                + if key.code == KeyCode::Tab {
                    1
                } else {
                    count - 1
                })
                % count;
            Response::Consumed
        }
        KeyCode::Enter if chosen.is_some() => Response::Invoke(chosen.unwrap()),
        KeyCode::Char(' ') if chosen.is_some() => Response::Invoke(chosen.unwrap()),
        KeyCode::Esc | KeyCode::Enter => Response::Pass,
        KeyCode::Char(_) | KeyCode::Backspace if *focus == 0 => Response::Pass,
        _ => Response::Consumed,
    }
}
pub(super) fn draw(
    out: &mut impl Write,
    core: &Core,
    declarations: &Declarations,
    dialog: Dialog<'_>,
    focus: usize,
) -> io::Result<()> {
    let rect = dialog.rect;
    for y in 0..rect.height {
        queue!(
            out,
            cursor::MoveTo(rect.x, rect.y + y),
            SetForegroundColor(Color::DarkMagenta),
            SetBackgroundColor(Color::Reset)
        )?;
        for x in 0..rect.width {
            queue!(
                out,
                Print(if y == 0 || y + 1 == rect.height {
                    if x == 0 || x + 1 == rect.width {
                        '+'
                    } else {
                        '─'
                    }
                } else if x == 0 || x + 1 == rect.width {
                    '│'
                } else {
                    ' '
                })
            )?;
        }
    }
    let mut result = Ok(());
    let mut index = 0;
    visit(core, declarations, dialog, &mut |_, node, rect| {
        if result.is_err() {
            return;
        }
        let selected = actionable(node) && index == focus;
        if actionable(node) {
            index += 1;
        }
        result = match node {
            Node::Row { .. } if leaf_row(node) => crate::sidebar::draw_row(
                out,
                node,
                Rect {
                    x: rect.x + u16::from(rect.width > 0),
                    width: rect.width.saturating_sub(1),
                    ..rect
                },
                None,
                false,
            ),
            Node::Label { text, role, tone } => paint_text(out, text, role, *tone, rect),
            Node::Input => paint_input(out, &core.input_text, rect),
            Node::Leader { tone } => paint_leader(out, *tone, rect),
            _ => Ok(()),
        };
        if result.is_ok() && selected && matches!(node, Node::Row { .. }) && rect.width > 0 {
            result = queue!(
                out,
                cursor::MoveTo(rect.x, rect.y),
                SetForegroundColor(Color::Cyan),
                Print('›'),
                SetForegroundColor(Color::Reset)
            );
        }
    });
    result
}
