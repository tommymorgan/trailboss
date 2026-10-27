//! Stable-ID sidebar drags. Endpoint mutations are emitted only on release.
use super::{
    application,
    menus::{self, Action},
    Rect,
};
use crate::{core::Core, sidebar::Declarations};
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use serde_json::{json, Value};
use std::io::{self, Write};
use trailboss_runtime::protocol::{
    widgets::{Context, Node},
    ClientShellSnapshot,
};

#[derive(Debug, PartialEq)]
pub struct MoveRequest {
    pub method: &'static str,
    pub params: Value,
}
#[derive(Default)]
pub struct Release {
    pub movement: Option<MoveRequest>,
    pub click: Option<Action>,
}
struct Press {
    action: Action,
    boot: String,
    start: (u16, u16),
    pointer: (u16, u16),
    viewport: Rect,
    dragging: bool,
    destination: Option<String>,
}
impl Press {
    fn retain_destination(&mut self, workspace: &str) {
        let destination =
            (Some(workspace) != self.action.context.workspace_id.as_deref()).then_some(workspace);
        if self.destination.as_deref() != destination {
            self.destination = destination.map(str::to_owned);
        }
    }
}
#[derive(Default)]
pub struct Drag {
    press: Option<Press>,
    indicator: Option<Rect>,
    pub list_offset: u16,
}

fn workspace<'a>(
    snapshot: &'a ClientShellSnapshot,
    id: &str,
) -> Option<&'a trailboss_runtime::protocol::ClientShellWorkspace> {
    snapshot.workspaces.iter().find(|w| w.workspace_id == id)
}
fn root(snapshot: &ClientShellSnapshot, id: &str) -> bool {
    workspace(snapshot, id)
        .is_some_and(|w| !w.worktree.as_ref().is_some_and(|t| t.is_linked_worktree))
}
fn valid(snapshot: &ClientShellSnapshot, context: &Context) -> bool {
    let Some(space) = context.workspace_id.as_deref() else {
        return false;
    };
    workspace(snapshot, space).is_some()
        && context.tab_id.as_ref().is_none_or(|tab| {
            snapshot
                .tabs
                .iter()
                .any(|t| t.tab_id == *tab && t.workspace_id == space)
        })
}
fn rows<'a>(
    core: &Core,
    declarations: &'a Declarations,
    viewport: Rect,
    list_offset: u16,
    visit: &mut impl FnMut(&'a str, &'a str, &'a Context, Rect, bool),
) {
    let Some(pane) = super::surface_rect(core, declarations, viewport) else {
        return;
    };
    application(
        core,
        declarations,
        viewport,
        list_offset,
        &mut |plugin, node, rect| {
            if rect.width == 0
                || rect.height == 0
                || rect.x >= pane.x
                || rect.x.saturating_add(rect.width) > pane.x
            {
                return;
            }
            if let Node::Row {
                action: Some(action),
                context,
                menu,
                ..
            } = node
            {
                if context.workspace_id.is_some() {
                    visit(plugin, action, context, rect, menu.is_some());
                }
            }
        },
    );
}
fn hit(
    core: &Core,
    declarations: &Declarations,
    viewport: Rect,
    list_offset: u16,
    x: u16,
    y: u16,
) -> Option<Action> {
    let mut result = None;
    rows(
        core,
        declarations,
        viewport,
        list_offset,
        &mut |plugin, action, context, rect, menu| {
            if context.pane_id.is_none()
                && rect.contains(x, y)
                && !(menu && menus::menu_cell(rect) == Some(x))
            {
                result = Some(Action {
                    plugin: plugin.into(),
                    action: action.into(),
                    context: context.clone(),
                });
            }
        },
    );
    result
}

impl Drag {
    pub fn captured(&self) -> bool {
        self.press.is_some()
    }
    pub fn pending(&self) -> bool {
        self.press.as_ref().is_some_and(|p| !p.dragging)
    }
    pub fn indicator(&self) -> Option<Rect> {
        self.indicator
    }
    pub fn cancel(&mut self) -> bool {
        let changed = self.press.take().is_some() || self.indicator.is_some();
        self.indicator = None;
        changed
    }
    pub fn press(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        snapshot: &ClientShellSnapshot,
        x: u16,
        y: u16,
    ) -> bool {
        let list_offset = self.list_offset;
        self.cancel();
        if super::has_modal(core, declarations, viewport) {
            return false;
        }
        let Some(action) = hit(core, declarations, viewport, list_offset, x, y)
            .filter(|a| valid(snapshot, &a.context))
        else {
            return false;
        };
        self.press = Some(Press {
            action,
            boot: snapshot.boot_id.clone(),
            start: (x, y),
            pointer: (x, y),
            viewport,
            dragging: false,
            destination: None,
        });
        true
    }
    /// Reconcile on snapshot/declaration replacement as well as before mouse events.
    pub fn reconcile(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        snapshot: &ClientShellSnapshot,
    ) -> bool {
        let list_offset = self.list_offset;
        let Some(press) = self.press.as_mut() else {
            return false;
        };
        let mut exists = false;
        rows(
            core,
            declarations,
            viewport,
            list_offset,
            &mut |plugin, action, context, _, _| {
                exists |= plugin == press.action.plugin
                    && action == press.action.action
                    && *context == press.action.context;
            },
        );
        if press.boot != snapshot.boot_id
            || press.viewport != viewport
            || !valid(snapshot, &press.action.context)
            || press
                .destination
                .as_deref()
                .is_some_and(|destination| workspace(snapshot, destination).is_none())
            || !exists
            || super::has_modal(core, declarations, viewport)
        {
            return self.cancel();
        }
        let previous = self.indicator;
        if press.dragging {
            self.indicator = target(
                core,
                declarations,
                viewport,
                list_offset,
                snapshot,
                &press.action.context,
                press.pointer.0,
                press.pointer.1,
            )
            .map(|(destination, _, rect)| {
                press.retain_destination(destination);
                rect
            });
            if self.indicator.is_none() {
                return self.cancel();
            }
        }
        previous != self.indicator
    }
    pub fn motion(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        snapshot: &ClientShellSnapshot,
        x: u16,
        y: u16,
    ) -> bool {
        let list_offset = self.list_offset;
        let cancelled = self.reconcile(core, declarations, viewport, snapshot);
        let Some(press) = self.press.as_mut() else {
            return cancelled;
        };
        press.pointer = (x, y);
        if x.abs_diff(press.start.0).max(y.abs_diff(press.start.1)) >= 1 {
            press.dragging = true;
        }
        let previous = self.indicator;
        self.indicator = if press.dragging {
            target(
                core,
                declarations,
                viewport,
                list_offset,
                snapshot,
                &press.action.context,
                x,
                y,
            )
            .map(|(destination, _, rect)| {
                press.retain_destination(destination);
                rect
            })
        } else {
            None
        };
        if press.dragging && self.indicator.is_none() {
            return self.cancel();
        }
        previous != self.indicator || cancelled
    }
    pub fn release(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        snapshot: &ClientShellSnapshot,
        x: u16,
        y: u16,
    ) -> Release {
        let list_offset = self.list_offset;
        self.reconcile(core, declarations, viewport, snapshot);
        let Some(press) = self.press.take() else {
            return Release::default();
        };
        self.indicator = None;
        if !press.dragging && x.abs_diff(press.start.0).max(y.abs_diff(press.start.1)) == 0 {
            let click = hit(core, declarations, viewport, list_offset, x, y).filter(|a| {
                a.plugin == press.action.plugin
                    && a.action == press.action.action
                    && a.context == press.action.context
            });
            return Release {
                click,
                movement: None,
            };
        }
        let movement = target(
            core,
            declarations,
            viewport,
            list_offset,
            snapshot,
            &press.action.context,
            x,
            y,
        )
        .and_then(|(destination, before, _)| {
            if Some(destination) == press.action.context.workspace_id.as_deref() {
                move_request(snapshot, &press.action.context, before)
            } else {
                let tab = press.action.context.tab_id.as_deref()?;
                let mut params = json!({"tab_id":tab,"workspace_id":destination});
                if let Some(before) = before {
                    let index = snapshot
                        .tabs
                        .iter()
                        .filter(|tab| tab.workspace_id == destination)
                        .position(|tab| tab.tab_id == before)?;
                    params["insert_index"] = json!(index);
                }
                Some(MoveRequest {
                    method: "tab.transfer",
                    params,
                })
            }
        });
        Release {
            movement,
            click: None,
        }
    }
    pub fn draw_indicator(&self, out: &mut impl Write) -> io::Result<()> {
        if let Some(rect) = self.indicator {
            queue!(
                out,
                cursor::MoveTo(rect.x, rect.y),
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(Color::Reset),
                SetForegroundColor(Color::Cyan)
            )?;
            for _ in 0..rect.width {
                queue!(out, Print("─"))?;
            }
            queue!(out, SetForegroundColor(Color::Reset))?;
        }
        Ok(())
    }
}

fn cross_space_target<'a>(
    core: &Core,
    declarations: &Declarations,
    viewport: Rect,
    list_offset: u16,
    snapshot: &'a ClientShellSnapshot,
    source: &Context,
    x: u16,
    y: u16,
) -> Option<(&'a str, Option<&'a str>, Rect)> {
    source.tab_id.as_ref()?;
    let mut hovered = false;
    let mut result = None;
    let mut trailing = None;
    rows(
        core,
        declarations,
        viewport,
        list_offset,
        &mut |_, _, context, rect, menu| {
            let contains = rect.contains(x, y);
            hovered |= contains;
            if context.workspace_id == source.workspace_id
                || context.pane_id.is_some()
                || !valid(snapshot, context)
                || (menu && menus::menu_cell(rect) == Some(x))
            {
                return;
            }
            let Some(destination) = context
                .workspace_id
                .as_deref()
                .and_then(|id| workspace(snapshot, id))
            else {
                return;
            };
            let before = context
                .tab_id
                .as_deref()
                .and_then(|id| snapshot.tabs.iter().find(|tab| tab.tab_id == id));
            let line = Rect { height: 1, ..rect };
            if contains {
                result = Some((
                    destination.workspace_id.as_str(),
                    before.map(|tab| tab.tab_id.as_str()),
                    line,
                ));
            }
            if let Some(tab) = before {
                if snapshot
                    .tabs
                    .iter()
                    .rev()
                    .find(|tab| tab.workspace_id == destination.workspace_id)
                    .is_some_and(|last| last.tab_id == tab.tab_id)
                    && y == rect.y.saturating_add(rect.height)
                    && x >= rect.x
                    && x < rect.x.saturating_add(rect.width)
                {
                    trailing = Some((destination.workspace_id.as_str(), None, Rect { y, ..line }));
                }
            }
        },
    );
    result.or_else(|| (!hovered).then_some(trailing).flatten())
}

// Slots are derived from live declaration rows, never labels or plugin names.
fn target<'a>(
    core: &Core,
    declarations: &Declarations,
    viewport: Rect,
    list_offset: u16,
    snapshot: &'a ClientShellSnapshot,
    source: &Context,
    x: u16,
    y: u16,
) -> Option<(&'a str, Option<&'a str>, Rect)> {
    let pane = super::surface_rect(core, declarations, viewport)?;
    if x < viewport.x
        || x >= pane.x
        || y < viewport.y
        || y >= viewport.y.saturating_add(viewport.height)
    {
        return None;
    }
    let space = workspace(snapshot, source.workspace_id.as_deref()?)?
        .workspace_id
        .as_str();
    if source.tab_id.is_none() && !root(snapshot, space) {
        return None;
    }
    if let Some(target) = cross_space_target(
        core,
        declarations,
        viewport,
        list_offset,
        snapshot,
        source,
        x,
        y,
    ) {
        return Some(target);
    }
    let mut best: Option<(u16, Option<&str>, Rect)> = None;
    let mut first = u16::MAX;
    let mut last = 0u16;
    let mut last_context: Option<&Context> = None;
    let mut last_rect = None;
    rows(
        core,
        declarations,
        viewport,
        list_offset,
        &mut |_, _, context, rect, _| {
            let eligible = if source.tab_id.is_some() {
                context.workspace_id.as_deref() == Some(space)
                    && context.tab_id.is_some()
                    && context.pane_id.is_none()
                    && valid(snapshot, context)
            } else {
                // Expanded tabs, panes and linked workspaces occupy the space group.
                // Only root headers below contribute insertion slots.
                valid(snapshot, context)
            };
            if !eligible {
                return;
            }
            first = first.min(rect.y);
            last = last.max(rect.y.saturating_add(rect.height));
            let id = if source.tab_id.is_some() {
                snapshot
                    .tabs
                    .iter()
                    .find(|t| Some(t.tab_id.as_str()) == context.tab_id.as_deref())
                    .map(|t| t.tab_id.as_str())
            } else {
                snapshot
                    .workspaces
                    .iter()
                    .find(|w| {
                        Some(w.workspace_id.as_str()) == context.workspace_id.as_deref()
                            && context.tab_id.is_none()
                            && context.pane_id.is_none()
                            && root(snapshot, &w.workspace_id)
                    })
                    .map(|w| w.workspace_id.as_str())
            };
            if let Some(id) = id {
                if last_rect.is_none_or(|r: Rect| rect.y >= r.y) {
                    last_context = Some(context);
                    last_rect = Some(rect);
                }
                let line = Rect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width.min(pane.x.saturating_sub(rect.x)),
                    height: 1,
                };
                let distance = y.abs_diff(line.y);
                if best.as_ref().is_none_or(|b| distance < b.0) {
                    best = Some((distance, Some(id), line));
                }
            }
        },
    );
    if first == u16::MAX || y < first.saturating_sub(1) || y > last {
        return None;
    }
    if let (Some(context), Some(rect)) = (
        last_context,
        last_rect.filter(|_| last < viewport.y.saturating_add(viewport.height)),
    ) {
        let next = if source.tab_id.is_some() {
            let pos = snapshot
                .tabs
                .iter()
                .position(|t| Some(t.tab_id.as_str()) == context.tab_id.as_deref())?;
            snapshot.tabs[pos + 1..]
                .iter()
                .find(|t| t.workspace_id == space)
                .map(|t| t.tab_id.as_str())
        } else {
            // A last visible root can be collapsed. Skip its entire linked group.
            let id = context.workspace_id.as_deref()?;
            let pos = snapshot
                .workspaces
                .iter()
                .position(|w| w.workspace_id == id)?;
            snapshot.workspaces[pos + 1..]
                .iter()
                .find(|w| root(snapshot, &w.workspace_id))
                .map(|w| w.workspace_id.as_str())
        };
        let line = Rect {
            x: rect.x,
            y: last,
            width: rect.width.min(pane.x.saturating_sub(rect.x)),
            height: 1,
        };
        let distance = y.abs_diff(last);
        if best.as_ref().is_none_or(|b| distance < b.0) {
            best = Some((distance, next, line));
        }
    }
    best.map(|(_, before, line)| (space, before, line))
}

pub fn move_request(
    snapshot: &ClientShellSnapshot,
    source: &Context,
    before: Option<&str>,
) -> Option<MoveRequest> {
    let space = source.workspace_id.as_deref()?;
    if let Some(tab) = source.tab_id.as_deref() {
        let mut source_index = None;
        let mut insert_index = None;
        let mut count = 0;
        for t in snapshot.tabs.iter().filter(|t| t.workspace_id == space) {
            if t.tab_id == tab {
                source_index = Some(count)
            }
            if Some(t.tab_id.as_str()) == before {
                insert_index = Some(count)
            }
            count += 1;
        }
        let source_index = source_index?;
        let index = match before {
            Some(_) => insert_index?,
            None => count,
        };
        if index == source_index || index == source_index + 1 {
            return None;
        }
        return Some(MoveRequest {
            method: "tab.move",
            params: json!({"tab_id":tab,"insert_index":index}),
        });
    }
    let source = workspace(snapshot, space)?;
    if !root(snapshot, space)
        || before == Some(space)
        || before.is_some_and(|id| !root(snapshot, id))
    {
        return None;
    }
    if let Some(tree) = &source.worktree {
        let ids: Vec<&str> = std::iter::once(space)
            .chain(
                snapshot
                    .workspaces
                    .iter()
                    .filter(|w| {
                        w.workspace_id != space
                            && w.worktree.as_ref().is_some_and(|t| t.key == tree.key)
                    })
                    .map(|w| w.workspace_id.as_str()),
            )
            .collect();
        if before.is_some_and(|id| ids.contains(&id)) {
            return None;
        }
        // Check the resulting order without allocating a second workspace collection.
        let insertion = before
            .map(|id| {
                snapshot
                    .workspaces
                    .iter()
                    .filter(|w| !ids.contains(&w.workspace_id.as_str()))
                    .position(|w| w.workspace_id == id)
            })
            .unwrap_or_else(|| Some(snapshot.workspaces.len() - ids.len()))?;
        let mut unchanged = true;
        let mut output = 0;
        for w in snapshot
            .workspaces
            .iter()
            .filter(|w| !ids.contains(&w.workspace_id.as_str()))
        {
            if output == insertion {
                for id in &ids {
                    unchanged &= snapshot.workspaces[output].workspace_id == *id;
                    output += 1;
                }
            }
            unchanged &= snapshot.workspaces[output].workspace_id == w.workspace_id;
            output += 1;
        }
        if output == insertion {
            for id in &ids {
                unchanged &= snapshot.workspaces[output].workspace_id == *id;
                output += 1;
            }
        }
        if unchanged {
            return None;
        }
        Some(MoveRequest {
            method: "workspace.move_block",
            params: json!({"workspace_ids":ids,"before_workspace_id":before}),
        })
    } else {
        let position = snapshot
            .workspaces
            .iter()
            .position(|w| w.workspace_id == space)?;
        let index = match before {
            Some(id) => snapshot
                .workspaces
                .iter()
                .position(|w| w.workspace_id == id)?,
            None => snapshot.workspaces.len(),
        };
        if index == position || index == position + 1 {
            return None;
        }
        Some(MoveRequest {
            method: "workspace.move",
            params: json!({"workspace_id":space,"insert_index":index}),
        })
    }
}
