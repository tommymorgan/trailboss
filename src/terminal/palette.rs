//! Native QuickPick interaction for public fragments; producer identity travels with each row.
use super::{
    menus::{Action, Response},
    Rect,
};
use crate::{core::Core, sidebar::Declarations};
use crossterm::{
    cursor,
    event::{KeyCode, KeyEvent, KeyModifiers},
    queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};
use trailboss_runtime::{
    core_actions::COMMAND_FRAGMENT_PREFIX,
    protocol::widgets::{Context, Node},
};
use unicode_width::UnicodeWidthChar;

fn draw_input(out: &mut impl Write, rect: Rect, text: &str) -> io::Result<()> {
    if rect.height < 2 || rect.width < 3 {
        return Ok(());
    }
    let width = usize::from(rect.width - 2);
    queue!(out, cursor::MoveTo(rect.x + 1, rect.y + 1))?;
    if width < 3 {
        queue!(out, Print("▏"))?;
        return Ok(());
    }
    let capacity = width - 3;
    // Input is append/backspace-only: keep the end and its caret visible.
    let mut start = text.len();
    let mut cells = 0;
    for (index, ch) in text.char_indices().rev() {
        let next = ch.width().unwrap_or(0);
        if cells + next > capacity {
            break;
        }
        cells += next;
        start = index;
    }
    queue!(out, Print("["))?;
    let printed = crate::sidebar::prefix(out, &text[start..], capacity)?;
    queue!(out, Print("▏"))?;
    let remaining = capacity.saturating_sub(printed);
    let placeholder = if text.is_empty() {
        queue!(out, SetForegroundColor(Color::DarkGrey))?;
        crate::sidebar::clipped(out, " Type text…", remaining)?
    } else {
        0
    };
    for _ in placeholder..remaining {
        queue!(out, Print(' '))?;
    }
    queue!(out, SetForegroundColor(Color::White), Print("]"))?;
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    plugin: String,
    action: Option<String>,
    context: Context,
    command_locator: Option<String>,
    anonymous_label: String,
}
struct Entry<'a> {
    plugin: &'a str,
    node: &'a Node,
    command_locator: Option<&'a str>,
    label: String,
    group: Option<&'a Node>,
}
impl Entry<'_> {
    fn selectable(&self) -> bool {
        matches!(self.node, Node::Row { .. })
    }
    fn matches(&self, identity: &Identity) -> bool {
        let Node::Row {
            action, context, ..
        } = self.node
        else {
            return false;
        };
        self.plugin == identity.plugin
            && *action == identity.action
            && *context == identity.context
            && self.command_locator == identity.command_locator.as_deref()
            && (identity.anonymous_label.is_empty() || self.label == identity.anonymous_label)
    }
    fn identity(&self) -> Identity {
        let Node::Row {
            action, context, ..
        } = self.node
        else {
            unreachable!()
        };
        Identity {
            plugin: self.plugin.into(),
            action: action.clone(),
            context: context.clone(),
            command_locator: self.command_locator.map(str::to_owned),
            anonymous_label: if self.command_locator.is_none()
                && action.is_none()
                && *context == Context::default()
            {
                self.label.clone()
            } else {
                String::new()
            },
        }
    }
}
#[derive(Default)]
struct View<'a> {
    entries: Vec<Entry<'a>>,
    labels: Vec<&'a str>,
    input: bool,
}
fn label(node: &Node, text: &mut String) {
    match node {
        Node::Label { text: value, .. } => {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(value);
        }
        Node::Row { children, .. } | Node::Column { children } => {
            for child in children {
                label(child, text);
            }
        }
        _ => {}
    }
}
fn collect<'a>(
    core: &Core,
    declarations: &'a Declarations,
    plugin: &'a str,
    node: &'a Node,
    command_locator: Option<&'a str>,
    depth: usize,
    view: &mut View<'a>,
) {
    if depth > 32 {
        return;
    }
    let Some(node) = core.resolve(node) else {
        return;
    };
    match node {
        Node::Use { name } => {
            if let Some((provider, tree)) = core.fragment(declarations, name) {
                // Only Core-issued command Uses gain private identity. Ordinary public
                // Use aliases retain their existing owner/action/context/label matching.
                let command_locator = if name.starts_with(COMMAND_FRAGMENT_PREFIX) {
                    Some(name.as_str())
                } else {
                    command_locator
                };
                collect(
                    core,
                    declarations,
                    provider,
                    tree,
                    command_locator,
                    depth + 1,
                    view,
                );
            }
        }
        Node::Row {
            action,
            menu,
            children,
            ..
        } => {
            if action.is_some() || menu.is_some() {
                let mut text = String::new();
                label(node, &mut text);
                view.entries.push(Entry {
                    plugin,
                    node,
                    command_locator,
                    label: text,
                    group: None,
                });
            } else {
                for child in children {
                    collect(
                        core,
                        declarations,
                        plugin,
                        child,
                        command_locator,
                        depth + 1,
                        view,
                    );
                }
            }
        }
        Node::Column { children } | Node::List { entries: children } => {
            let start = view.entries.len();
            let heading = children
                .first()
                .filter(|node| matches!(node, Node::Label { role, .. } if role == "heading"));
            for child in children {
                collect(
                    core,
                    declarations,
                    plugin,
                    child,
                    command_locator,
                    depth + 1,
                    view,
                );
            }
            if let Some(heading) = heading {
                for entry in &mut view.entries[start..] {
                    entry.group = Some(heading);
                }
            }
        }
        Node::Flex { child, .. } | Node::Modal { child } => collect(
            core,
            declarations,
            plugin,
            child,
            command_locator,
            depth + 1,
            view,
        ),
        Node::Input => view.input = true,
        Node::Label { text, role, .. } if role == "heading" => view.entries.push(Entry {
            plugin,
            node,
            command_locator,
            label: text.clone(),
            group: None,
        }),
        Node::Label { text, .. } => view.labels.push(text),
        _ => {}
    }
}
fn score(text: &str, query: &str) -> Option<(usize, usize)> {
    let text = text.to_lowercase();
    let query = query.to_lowercase();
    if query.is_empty() {
        return Some((0, 0));
    }
    if text.starts_with(&query) {
        return Some((0, text.chars().count()));
    }
    if let Some(pos) = text.find(&query) {
        return Some((1, pos));
    }
    let mut chars = text.chars().enumerate();
    let mut first = None;
    let mut last = 0;
    for wanted in query.chars() {
        let (index, _) = chars.find(|(_, ch)| *ch == wanted)?;
        first.get_or_insert(index);
        last = index;
    }
    Some((2, last - first.unwrap_or(0)))
}
pub(super) struct Palette {
    path: Vec<Identity>,
    crumbs: Vec<String>,
    query: String,
    selected: usize,
    scroll: usize,
    selected_id: Option<Identity>,
    provider: String,
    stale: bool,
    pending_cancel: bool,
    editing: bool,
}
impl Palette {
    pub fn open(
        core: &Core,
        declarations: &Declarations,
        _context: Option<&Context>,
    ) -> Option<Self> {
        let (provider, _) = core.fragment(declarations, "command-palette")?;
        Some(Self {
            path: Vec::new(),
            crumbs: Vec::new(),
            query: String::new(),
            selected: 0,
            scroll: 0,
            selected_id: None,
            provider: provider.into(),
            stale: false,
            pending_cancel: false,
            editing: false,
        })
    }
    fn view<'a>(&self, core: &Core, declarations: &'a Declarations) -> Option<View<'a>> {
        let (mut plugin, mut tree) = core.fragment(declarations, "command-palette")?;
        if plugin != self.provider {
            return None;
        }
        let mut command_locator = None;
        for identity in &self.path {
            let mut view = View::default();
            collect(
                core,
                declarations,
                plugin,
                tree,
                command_locator,
                0,
                &mut view,
            );
            let mut matches = view.entries.iter().filter(|entry| entry.matches(identity));
            let entry = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            let Node::Row {
                menu: Some(menu), ..
            } = entry.node
            else {
                return None;
            };
            plugin = entry.plugin;
            tree = menu;
            command_locator = entry.command_locator;
        }
        let mut view = View::default();
        collect(
            core,
            declarations,
            plugin,
            tree,
            command_locator,
            0,
            &mut view,
        );
        if !view.input {
            let grouped = view.entries.iter().any(|entry| entry.group.is_some());
            if grouped {
                // Traverse backwards so each heading knows whether its group retained a tab.
                view.entries.reverse();
                let mut matched_group = None;
                view.entries.retain(|entry| {
                    if entry.selectable() {
                        let matches = score(&entry.label, &self.query).is_some();
                        if matches {
                            matched_group = entry.group;
                        }
                        matches
                    } else {
                        self.query.is_empty()
                            || matched_group.is_some_and(|group| std::ptr::eq(group, entry.node))
                    }
                });
                view.entries.reverse();
            } else {
                view.entries
                    .retain(|entry| score(&entry.label, &self.query).is_some());
                view.entries
                    .sort_by_cached_key(|entry| score(&entry.label, &self.query));
            }
        }
        Some(view)
    }
    pub fn reconcile(&mut self, core: &Core, declarations: &Declarations) {
        self.editing = core.editing();
        if self.view(core, declarations).is_none() {
            self.pending_cancel |= core.editing();
            while !self.path.is_empty() && self.view(core, declarations).is_none() {
                self.path.pop();
                self.crumbs.pop();
            }
            if let Some((provider, _)) = core.fragment(declarations, "command-palette") {
                self.provider = provider.into();
            }
            self.query.clear();
            self.selected_id = None;
            self.selected = 0;
            self.scroll = 0;
            self.stale = true;
        }
        if let Some(view) = self.view(core, declarations) {
            if let Some(id) = &self.selected_id {
                if let Some(index) = view.entries.iter().position(|entry| entry.matches(id)) {
                    self.selected = index;
                } else {
                    self.selected = 0;
                    self.stale = true;
                }
            }
            self.selected = self.selected.min(view.entries.len().saturating_sub(1));
            if !view
                .entries
                .get(self.selected)
                .is_some_and(Entry::selectable)
            {
                self.selected = view
                    .entries
                    .iter()
                    .enumerate()
                    .skip(self.selected)
                    .find(|(_, entry)| entry.selectable())
                    .or_else(|| {
                        view.entries
                            .iter()
                            .enumerate()
                            .rev()
                            .find(|(_, entry)| entry.selectable())
                    })
                    .map_or(0, |(index, _)| index);
            }
            self.selected_id = view
                .entries
                .get(self.selected)
                .filter(|entry| entry.selectable())
                .map(Entry::identity);
        }
    }
    pub fn rect(&self, viewport: Rect) -> Rect {
        let width = viewport.width.min(76);
        let height = viewport.height.min(15);
        Rect {
            x: viewport.x + (viewport.width - width) / 2,
            y: viewport.y + (viewport.height - height).min(2),
            width,
            height,
        }
    }
    pub fn layout(&mut self, viewport: Rect) {
        self.ensure_scroll(self.visible_rows(viewport));
    }
    fn visible_rows(&self, viewport: Rect) -> usize {
        usize::from(self.rect(viewport).height.saturating_sub(4)).max(1)
    }
    fn ensure_scroll(&mut self, rows: usize) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
        if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
    }
    fn cancel(&self) -> Response {
        Response::Invoke(Action {
            plugin: self.provider.clone(),
            action: "host:cancel".into(),
            context: Context::default(),
        })
    }
    fn back(&mut self, core: &Core) -> (Response, bool) {
        let editing = core.editing();
        let close = self.path.is_empty();
        self.path.pop();
        self.crumbs.pop();
        self.query.clear();
        self.selected = 0;
        self.scroll = 0;
        self.selected_id = None;
        (
            if editing {
                self.cancel()
            } else {
                Response::Consumed
            },
            close,
        )
    }
    fn choose(&mut self, core: &Core, declarations: &Declarations) -> (Response, bool) {
        if self.pending_cancel {
            self.pending_cancel = false;
            return (self.cancel(), false);
        }
        if self.stale {
            self.stale = false;
            return (Response::Consumed, false);
        }
        let Some(view) = self.view(core, declarations) else {
            return (Response::Consumed, false);
        };
        let Some(entry) = view.entries.get(self.selected) else {
            return (Response::Consumed, false);
        };
        let Node::Row {
            action,
            context,
            menu,
            ..
        } = entry.node
        else {
            return (Response::Consumed, false);
        };
        if view.input
            && core.input_text.trim().is_empty()
            && action.as_deref() != Some("host:cancel")
        {
            return (Response::Consumed, false);
        }
        let mut child_view = View::default();
        if let Some(menu) = menu {
            collect(
                core,
                declarations,
                entry.plugin,
                menu,
                entry.command_locator,
                0,
                &mut child_view,
            );
        }
        let prepares_input = child_view.input;
        let response = action
            .as_ref()
            .map(|action| {
                let action = Action {
                    plugin: entry.plugin.into(),
                    action: action.clone(),
                    context: if *context == Context::default() {
                        self.path
                            .iter()
                            .rev()
                            .find(|ancestor| ancestor.context != Context::default())
                            .map_or(context, |ancestor| &ancestor.context)
                    } else {
                        context
                    }
                    .clone(),
                };
                if prepares_input {
                    Response::Prepare(action)
                } else {
                    Response::Invoke(action)
                }
            })
            .unwrap_or(Response::Consumed);
        if menu.is_some() {
            self.editing = action.is_some() && prepares_input;
            self.path.push(entry.identity());
            self.crumbs.push(entry.label.clone());
            self.query.clear();
            self.selected = 0;
            self.scroll = 0;
            self.selected_id = None;
            (response, false)
        } else {
            (response, true)
        }
    }
    pub fn editing(&self) -> bool {
        self.editing
    }
    pub fn paste(&mut self, text: &str, editing: bool) -> bool {
        if editing {
            return false;
        }
        self.query
            .extend(text.chars().filter(|ch| !ch.is_control()));
        self.selected = 0;
        self.scroll = 0;
        self.selected_id = None;
        true
    }
    pub fn key(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        key: KeyEvent,
    ) -> (Response, bool) {
        self.reconcile(core, declarations);
        self.layout(viewport);
        if self.pending_cancel {
            self.pending_cancel = false;
            return (self.cancel(), false);
        }
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::ALT))
            || (key.code == KeyCode::Backspace && self.query.is_empty() && !core.editing())
        {
            return self.back(core);
        }
        if key.code == KeyCode::Enter {
            return self.choose(core, declarations);
        }
        let count = self
            .view(core, declarations)
            .map_or(0, |view| view.entries.len());
        let rows = self.visible_rows(viewport);
        match key.code {
            KeyCode::Up | KeyCode::BackTab | KeyCode::PageUp => {
                let target = self
                    .selected
                    .saturating_sub(if key.code == KeyCode::PageUp { rows } else { 1 });
                if let Some(view) = self.view(core, declarations) {
                    if let Some(index) = (0..=target)
                        .rev()
                        .find(|&index| view.entries.get(index).is_some_and(Entry::selectable))
                    {
                        self.selected = index;
                    }
                }
            }
            KeyCode::Down | KeyCode::Tab | KeyCode::PageDown => {
                let target = (self.selected
                    + if key.code == KeyCode::PageDown {
                        rows
                    } else {
                        1
                    })
                .min(count.saturating_sub(1));
                if let Some(view) = self.view(core, declarations) {
                    if let Some(index) =
                        (target..count).find(|&index| view.entries[index].selectable())
                    {
                        self.selected = index;
                    }
                }
            }
            KeyCode::Home if !core.editing() => self.selected = 0,
            KeyCode::End if !core.editing() => self.selected = count.saturating_sub(1),
            _ if core.editing() => return (Response::Pass, false),
            KeyCode::Char(ch)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                self.query.push(ch);
                self.selected = 0;
                self.scroll = 0;
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
                self.scroll = 0;
            }
            _ => {}
        }
        self.stale = false;
        self.selected_id = None;
        self.reconcile(core, declarations);
        self.ensure_scroll(rows);
        (Response::Consumed, false)
    }
    pub fn hover(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        x: u16,
        y: u16,
    ) -> bool {
        self.layout(viewport);
        let rect = self.rect(viewport);
        let start = rect.y.saturating_add(3);
        if rect.contains(x, y) && y >= start && y < rect.y + rect.height.saturating_sub(1) {
            let index = self.scroll + usize::from(y - start);
            if self
                .view(core, declarations)
                .is_some_and(|view| view.entries.get(index).is_some_and(Entry::selectable))
                && self.selected != index
            {
                self.selected = index;
                self.selected_id = None;
                self.stale = false;
                return true;
            }
        }
        false
    }
    pub fn click(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        x: u16,
        y: u16,
    ) -> (Response, bool) {
        self.reconcile(core, declarations);
        self.layout(viewport);
        if !self.rect(viewport).contains(x, y) {
            return (
                if core.editing() {
                    self.cancel()
                } else {
                    Response::Consumed
                },
                true,
            );
        }
        let start = self.rect(viewport).y.saturating_add(3);
        if y >= start && y < self.rect(viewport).y + self.rect(viewport).height.saturating_sub(1) {
            let index = self.scroll + usize::from(y - start);
            if self
                .view(core, declarations)
                .is_some_and(|view| view.entries.get(index).is_some_and(Entry::selectable))
            {
                self.selected = index;
                self.selected_id = None;
                return self.choose(core, declarations);
            }
        }
        (Response::Consumed, false)
    }
    pub fn wheel(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        up: bool,
    ) -> Response {
        self.key(
            core,
            declarations,
            viewport,
            KeyEvent::new(
                if up { KeyCode::Up } else { KeyCode::Down },
                KeyModifiers::NONE,
            ),
        )
        .0
    }
    pub fn draw(
        &self,
        out: &mut impl Write,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
    ) -> io::Result<Option<Rect>> {
        let rect = self.rect(viewport);
        if rect.width == 0 || rect.height == 0 {
            return Ok(Some(rect));
        }
        let bg = Color::Rgb {
            r: 30,
            g: 30,
            b: 36,
        };
        let fg = Color::Rgb {
            r: 220,
            g: 220,
            b: 225,
        };
        for y in rect.y..rect.y + rect.height {
            queue!(
                out,
                cursor::MoveTo(rect.x, y),
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(bg),
                SetForegroundColor(fg)
            )?;
            for _ in 0..rect.width {
                queue!(out, Print(' '))?;
            }
        }
        let mut line =
            |offset: u16, text: &str, selected: bool, tone: Option<Color>| -> io::Result<()> {
                if offset >= rect.height {
                    return Ok(());
                }
                queue!(
                    out,
                    cursor::MoveTo(rect.x, rect.y + offset),
                    SetBackgroundColor(if selected {
                        Color::Rgb {
                            r: 45,
                            g: 70,
                            b: 100,
                        }
                    } else {
                        bg
                    }),
                    SetForegroundColor(tone.unwrap_or(fg))
                )?;
                for _ in 0..rect.width {
                    queue!(out, Print(' '))?;
                }
                queue!(
                    out,
                    cursor::MoveTo(rect.x + u16::from(rect.width > 1), rect.y + offset)
                )?;
                crate::sidebar::clipped(out, text, usize::from(rect.width.saturating_sub(2)))?;
                Ok(())
            };
        line(
            0,
            &format!(
                "Commands{}",
                if self.crumbs.is_empty() {
                    String::new()
                } else {
                    format!(" › {}", self.crumbs.join(" › "))
                }
            ),
            false,
            None,
        )?;
        let view = self.view(core, declarations).unwrap_or_default();
        line(
            1,
            if view.input { "" } else { &self.query },
            view.input,
            None,
        )?;
        line(2, &view.labels.join(" "), false, Some(Color::DarkGrey))?;
        let rows = usize::from(rect.height.saturating_sub(4));
        let scroll = if self.selected < self.scroll {
            self.selected
        } else if self.selected >= self.scroll + rows.max(1) {
            self.selected + 1 - rows.max(1)
        } else {
            self.scroll
        };
        if view.entries.is_empty() {
            line(3, "No matches", false, Some(Color::DarkGrey))?;
        }
        for (offset, entry) in view.entries.iter().skip(scroll).take(rows).enumerate() {
            let tone = if let Node::Row { children, .. } = entry.node {
                children.iter().find_map(|child| match child {
                    Node::Label { role, .. } if role == "danger" => Some(Color::Rgb {
                        r: 223,
                        g: 140,
                        b: 136,
                    }),
                    Node::Label { role, .. } if role == "muted" => Some(Color::DarkGrey),
                    _ => None,
                })
            } else {
                Some(Color::DarkGrey)
            };
            line(
                3 + offset as u16,
                &entry.label,
                scroll + offset == self.selected && entry.selectable(),
                tone,
            )?;
        }
        if rect.height > 3 {
            let footer = if view.input {
                format!(
                    "Type text · Enter {} · Esc {} · ↑↓ actions",
                    view.entries
                        .get(self.selected)
                        .filter(|entry| entry.selectable())
                        .map_or("select", |entry| entry.label.as_str()),
                    if core.editing() { "cancel" } else { "back" }
                )
            } else {
                format!(
                    "{} / {}  · Enter select · Esc back",
                    view.entries
                        .iter()
                        .take(self.selected + 1)
                        .filter(|entry| entry.selectable())
                        .count(),
                    view.entries
                        .iter()
                        .filter(|entry| entry.selectable())
                        .count()
                )
            };
            line(rect.height - 1, &footer, false, Some(Color::DarkGrey))?;
        }
        if view.input {
            queue!(
                out,
                SetBackgroundColor(Color::Rgb {
                    r: 45,
                    g: 70,
                    b: 100,
                }),
                SetForegroundColor(Color::White)
            )?;
            draw_input(out, rect, &core.input_text)?;
        }
        queue!(
            out,
            SetAttribute(Attribute::Reset),
            SetBackgroundColor(Color::Reset),
            SetForegroundColor(Color::Reset)
        )?;
        Ok(Some(rect))
    }
}
