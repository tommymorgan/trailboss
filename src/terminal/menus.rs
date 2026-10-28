//! Native interaction and popup geometry for public row menus.
use super::{application, extent, walk, Rect};
use crate::{core::Core, sidebar::Declarations};
use crossterm::{
    cursor,
    event::{KeyCode, KeyEvent, KeyModifiers},
    queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};
use trailboss_runtime::protocol::widgets::{Context, Node};

#[derive(Clone)]
struct Origin {
    plugin: String,
    context: Context,
}
impl Origin {
    fn matches(&self, plugin: &str, node: &Node) -> bool {
        matches!(node, Node::Row { context, .. } if self.plugin == plugin && self.context == *context)
    }
}
struct MenuLevel {
    plugin: String,
    context: Context,
    tree: Node,
}
enum PopupChoice {
    Invoke(Action),
    Open(MenuLevel),
}
enum PopupSource {
    Row(Node),
    Command(Box<super::palette::Handoff>),
}
struct Popup {
    origin: Origin,
    source: PopupSource,
    selected: usize,
    path: Vec<MenuLevel>,
}
impl Popup {
    fn tree(&self) -> &Node {
        match &self.source {
            PopupSource::Row(tree) => tree,
            PopupSource::Command(handoff) => &handoff.tree,
        }
    }
    fn content(&self) -> (&str, &Node) {
        self.path
            .last()
            .map_or((&self.origin.plugin, self.tree()), |level| {
                (&level.plugin, &level.tree)
            })
    }
    fn context(&self) -> &Context {
        self.path
            .iter()
            .rev()
            .find(|level| level.context != Context::default())
            .map_or(&self.origin.context, |level| &level.context)
    }
    fn refresh(&mut self, core: &Core, declarations: &Declarations, viewport: Rect) -> bool {
        if let PopupSource::Command(handoff) = &mut self.source {
            if !handoff.refresh(core, declarations, viewport) {
                return false;
            }
        }
        self.valid(core, declarations, viewport)
    }
    fn valid(&self, core: &Core, declarations: &Declarations, viewport: Rect) -> bool {
        if matches!(&self.source, PopupSource::Command(handoff) if !handoff.valid(core, declarations, viewport))
        {
            return false;
        }
        let mut plugin = self.origin.plugin.as_str();
        let mut tree = self.tree();
        for level in &self.path {
            let mut found = false;
            walk(
                core,
                declarations,
                plugin,
                tree,
                viewport,
                0,
                0,
                &mut |owner, node, _| {
                    found |= matches!(node, Node::Row { context, menu: Some(menu), .. }
                    if owner == level.plugin && *context == level.context && **menu == level.tree);
                },
            );
            if !found {
                return false;
            }
            plugin = &level.plugin;
            tree = &level.tree;
        }
        true
    }
    fn choice(&self, plugin: &str, node: &Node) -> Option<PopupChoice> {
        if let Some(action) = action(plugin, node, Some(self.context())) {
            return Some(PopupChoice::Invoke(action));
        }
        if let Node::Row {
            menu: Some(menu),
            context,
            ..
        } = node
        {
            return Some(PopupChoice::Open(MenuLevel {
                plugin: plugin.into(),
                context: context.clone(),
                tree: (**menu).clone(),
            }));
        }
        None
    }
    fn activate(&mut self, choice: PopupChoice) -> (Response, bool) {
        match choice {
            PopupChoice::Invoke(action) => (Response::Invoke(action), true),
            PopupChoice::Open(level) => {
                if self.path.len() < 16 {
                    self.path.push(level);
                    self.selected = 0;
                }
                (Response::Consumed, false)
            }
        }
    }
}
fn selectable(node: &Node) -> bool {
    matches!(node, Node::Row { action, menu, .. } if action.is_some() || menu.is_some())
}
pub struct Action {
    pub plugin: String,
    pub action: String,
    pub context: Context,
}
pub enum Response {
    Pass,
    Consumed,
    Invoke(Action),
    Prepare(Action),
    Arguments(Action, Option<String>),
}
#[derive(Default)]
pub struct Controls {
    pointer: Option<(u16, u16)>,
    focus: Option<Origin>,
    popup: Option<Popup>,
    palette: Option<super::palette::Palette>,
    interaction: Option<super::palette::Handoff>,
    pub list_offset: u16,
    pub(super) dialog_focus: usize,
}
fn triggers<'a>(
    core: &Core,
    declarations: &'a Declarations,
    viewport: Rect,
    list_offset: u16,
    visit: &mut impl FnMut(&'a str, &'a Node, Rect),
) {
    application(
        core,
        declarations,
        viewport,
        list_offset,
        &mut |plugin, node, rect| {
            if matches!(node, Node::Row { menu: Some(tree), .. } if extent(core, declarations, tree, false).0 > 0)
            {
                visit(plugin, node, rect);
            }
        },
    );
}
fn action(plugin: &str, node: &Node, inherited: Option<&Context>) -> Option<Action> {
    match node {
        Node::Row {
            action: Some(action),
            context,
            ..
        } => Some(Action {
            plugin: plugin.into(),
            action: action.clone(),
            context: if *context == Context::default() {
                inherited.unwrap_or(context)
            } else {
                context
            }
            .clone(),
        }),
        _ => None,
    }
}
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}
pub(crate) fn menu_cell(rect: Rect) -> Option<u16> {
    (rect.width >= 3).then(|| rect.x + rect.width - 2)
}
impl Controls {
    pub(super) fn interaction_dialog<'a>(
        &'a self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
    ) -> Option<super::modals::Dialog<'a>> {
        self.interaction.as_ref().map(|interaction| {
            super::modals::from_tree(
                core,
                declarations,
                &interaction.plugin,
                &interaction.tree,
                viewport,
            )
        })
    }
    fn invalidate_interaction(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
    ) -> Option<Response> {
        if self
            .interaction
            .as_mut()
            .is_some_and(|interaction| !interaction.refresh(core, declarations, viewport))
        {
            let interaction = self.interaction.take().expect("invalid interaction");
            self.dialog_focus = 0;
            Some(Response::Invoke(Action {
                plugin: interaction.plugin,
                action: "host:cancel".into(),
                context: Context::default(),
            }))
        } else {
            None
        }
    }
    pub fn revealed(
        &self,
        core: &Core,
        declarations: &Declarations,
        plugin: &str,
        node: &Node,
        rect: Rect,
    ) -> bool {
        menu_cell(rect).is_some()
            && (self.pointer.is_some_and(|(x, y)| rect.contains(x, y))
                || self.focus.as_ref().is_some_and(|f| f.matches(plugin, node)))
            && matches!(node, Node::Row {menu:Some(tree),..} if extent(core,declarations,tree,false).0>0)
    }
    pub fn modal(&self) -> bool {
        self.palette.is_some() || self.popup.is_some() || self.interaction.is_some()
    }
    pub fn captures_input(&self) -> bool {
        self.modal() || self.focus.is_some()
    }
    pub fn open_palette(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        context: Option<&Context>,
    ) -> bool {
        let Some(palette) = super::palette::Palette::open(core, declarations, context) else {
            return false;
        };
        self.reset();
        self.palette = Some(palette);
        true
    }
    pub fn paste(&mut self, text: &str) -> bool {
        self.palette
            .as_mut()
            .is_some_and(|palette| palette.paste(text))
    }
    pub fn scroll_list(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        x: u16,
        y: u16,
        up: bool,
        lines: u16,
    ) -> bool {
        let Some((rect, maximum)) = super::list_viewport(core, declarations, viewport) else {
            return false;
        };
        if !rect.contains(x, y) {
            return false;
        }
        let current = self.list_offset.min(maximum);
        self.list_offset = if up {
            current.saturating_sub(lines)
        } else {
            current.saturating_add(lines).min(maximum)
        };
        self.focus = None;
        true
    }
    pub fn wheel(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        up: bool,
    ) -> Response {
        if let Some(palette) = &mut self.palette {
            return palette.wheel(core, declarations, viewport, up);
        }
        if self.modal() {
            return self.key(
                core,
                declarations,
                viewport,
                KeyEvent::new(
                    if up { KeyCode::Up } else { KeyCode::Down },
                    KeyModifiers::NONE,
                ),
            );
        }
        Response::Pass
    }
    pub fn reset(&mut self) {
        self.palette = None;
        self.popup = None;
        self.focus = None;
        self.interaction = None;
    }
    fn accept_handoff(&mut self, handoff: super::palette::Handoff) {
        if handoff.input {
            self.interaction = Some(handoff);
            self.dialog_focus = 0;
        } else {
            self.focus = None;
            self.popup = Some(Popup {
                origin: Origin {
                    plugin: handoff.plugin.clone(),
                    context: handoff.context.clone(),
                },
                source: PopupSource::Command(Box::new(handoff)),
                selected: 0,
                path: Vec::new(),
            });
        }
    }
    fn open(&mut self, plugin: &str, node: &Node) {
        if let Node::Row {
            menu: Some(tree),
            context,
            ..
        } = node
        {
            let origin = Origin {
                plugin: plugin.into(),
                context: context.clone(),
            };
            self.focus = Some(origin.clone());
            self.popup = Some(Popup {
                origin,
                source: PopupSource::Row(*tree.clone()),
                selected: 0,
                path: Vec::new(),
            });
        }
    }
    pub fn reconcile(&mut self, core: &Core, declarations: &Declarations, viewport: Rect) {
        self.list_offset = self.list_offset.min(
            super::list_viewport(core, declarations, viewport).map_or(0, |(_, maximum)| maximum),
        );
        if !core.editing() {
            self.interaction = None;
        }
        if let Some(interaction) = &mut self.interaction {
            interaction.refresh(core, declarations, viewport);
            self.focus = None;
            self.popup = None;
            return;
        }
        if let Some(palette) = &mut self.palette {
            palette.reconcile(core, declarations);
            palette.layout(viewport);
            return;
        }
        if self
            .popup
            .as_ref()
            .is_some_and(|popup| matches!(&popup.source, PopupSource::Command(_)))
        {
            if self
                .popup
                .as_mut()
                .is_some_and(|popup| !popup.refresh(core, declarations, viewport))
            {
                self.popup = None;
            }
            return;
        }
        if super::modals::dialog(core, declarations, viewport).is_some() {
            self.focus = None;
            self.popup = None;
            return;
        }
        self.dialog_focus = 0;
        if self.focus.is_none() && self.popup.is_none() {
            return;
        }
        let mut focus_exists = false;
        let mut popup_exists = false;
        triggers(
            core,
            declarations,
            viewport,
            self.list_offset,
            &mut |plugin, node, _| {
                focus_exists |= self.focus.as_ref().is_some_and(|f| f.matches(plugin, node));
                popup_exists |= self.popup.as_ref().is_some_and(|p| {
                    p.origin.matches(plugin, node)
                        && matches!(node, Node::Row { menu: Some(tree), .. } if tree.as_ref() == p.tree())
                });
            },
        );
        if !focus_exists {
            self.focus = None;
        }
        if !popup_exists
            || self
                .popup
                .as_ref()
                .is_some_and(|p| !p.valid(core, declarations, viewport))
        {
            self.popup = None;
        }
    }
    pub fn focus_next(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        reverse: bool,
    ) {
        if super::modals::dialog(core, declarations, viewport).is_some() {
            return;
        }
        let mut count = 0;
        let mut current = None;
        triggers(
            core,
            declarations,
            viewport,
            self.list_offset,
            &mut |plugin, node, _| {
                if self.focus.as_ref().is_some_and(|f| f.matches(plugin, node)) {
                    current = Some(count);
                }
                count += 1;
            },
        );
        if count == 0 {
            self.focus = None;
            return;
        }
        let next = match (current, reverse) {
            (Some(i), true) => (i + count - 1) % count,
            (Some(i), false) => (i + 1) % count,
            (None, true) => count - 1,
            (None, false) => 0,
        };
        let mut index = 0;
        let mut focus = None;
        triggers(
            core,
            declarations,
            viewport,
            self.list_offset,
            &mut |plugin, node, _| {
                if index == next {
                    if let Node::Row { context, .. } = node {
                        focus = Some(Origin {
                            plugin: plugin.into(),
                            context: context.clone(),
                        });
                    }
                }
                index += 1;
            },
        );
        self.focus = focus;
    }
    pub fn hover(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        x: u16,
        y: u16,
    ) -> bool {
        if let Some(palette) = &mut self.palette {
            return palette.hover(core, declarations, viewport, x, y);
        }
        if super::modals::dialog(core, declarations, viewport).is_some() {
            return false;
        }
        let mut old = None;
        let mut new = None;
        triggers(
            core,
            declarations,
            viewport,
            self.list_offset,
            &mut |_, _, rect| {
                if self.pointer.is_some_and(|(x, y)| rect.contains(x, y)) {
                    old = Some(rect);
                }
                if rect.contains(x, y) {
                    new = Some(rect);
                }
            },
        );
        self.pointer = Some((x, y));
        let geometry = self.popup_rect(core, declarations, viewport);
        let mut changed = old != new;
        if let (Some(popup), Some(rect)) = (&mut self.popup, geometry) {
            let mut index = 0;
            let mut selected = popup.selected;
            walk(
                core,
                declarations,
                popup.content().0,
                popup.content().1,
                interior(rect),
                0,
                0,
                &mut |_, node, area| {
                    if selectable(node) {
                        if area.contains(x, y) && selected != index {
                            selected = index;
                            changed = true;
                        }
                        index += 1;
                    }
                },
            );
            popup.selected = selected;
        }
        changed
    }
    pub fn click(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        x: u16,
        y: u16,
        context_menu: bool,
    ) -> Response {
        if let Some(palette) = &mut self.palette {
            if context_menu {
                return Response::Consumed;
            }
            let (response, close) = palette.click(core, declarations, viewport, x, y);
            let handoff = palette.take_handoff();
            if close {
                self.palette = None;
            }
            if let Some(handoff) = handoff {
                self.accept_handoff(handoff);
            }
            return response;
        }
        if let Some(response) = self.invalidate_interaction(core, declarations, viewport) {
            return response;
        }
        if let Some(interaction) = &self.interaction {
            if context_menu {
                return Response::Consumed;
            }
            let dialog = super::modals::from_tree(
                core,
                declarations,
                &interaction.plugin,
                &interaction.tree,
                viewport,
            );
            return super::modals::click(core, declarations, dialog, x, y, &mut self.dialog_focus);
        }
        if let Some(dialog) = super::modals::dialog(core, declarations, viewport) {
            if context_menu {
                return Response::Consumed;
            }
            return super::modals::click(core, declarations, dialog, x, y, &mut self.dialog_focus);
        }
        if self.popup.is_some() {
            let mut chosen = None;
            if let (Some(popup), Some(rect)) =
                (&self.popup, self.popup_rect(core, declarations, viewport))
            {
                walk(
                    core,
                    declarations,
                    popup.content().0,
                    popup.content().1,
                    interior(rect),
                    0,
                    0,
                    &mut |plugin, node, area| {
                        if area.contains(x, y) && selectable(node) {
                            chosen = popup.choice(plugin, node);
                        }
                    },
                );
            }
            let (response, close) = chosen.map_or((Response::Consumed, true), |choice| {
                self.popup.as_mut().expect("open popup").activate(choice)
            });
            if close {
                self.popup = None;
                self.focus = None;
            }
            return response;
        }
        let mut target = None;
        triggers(
            core,
            declarations,
            viewport,
            self.list_offset,
            &mut |plugin, node, rect| {
                if rect.contains(x, y) && (context_menu || menu_cell(rect) == Some(x)) {
                    target = Some((plugin, node));
                }
            },
        );
        if let Some((plugin, node)) = target {
            self.open(plugin, node);
            Response::Consumed
        } else {
            self.focus = None;
            Response::Pass
        }
    }
    pub fn key(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        key: KeyEvent,
    ) -> Response {
        if let Some(palette) = &mut self.palette {
            let (response, close) = palette.key(core, declarations, viewport, key);
            let handoff = palette.take_handoff();
            if close {
                self.palette = None;
            }
            if let Some(handoff) = handoff {
                self.accept_handoff(handoff);
            }
            return response;
        }
        if let Some(response) = self.invalidate_interaction(core, declarations, viewport) {
            return response;
        }
        if let Some(interaction) = &self.interaction {
            if interaction.arguments {
                if core.argument_finished || !core.argument_editing {
                    self.interaction = None;
                    return Response::Consumed;
                }
                return match key.code {
                    KeyCode::Esc => {
                        self.interaction = None;
                        Response::Invoke(Action {
                            plugin: "trailboss.core".into(),
                            action: "host:cancel".into(),
                            context: Context::default(),
                        })
                    }
                    KeyCode::Enter if core.argument_pending.is_none() => Response::Arguments(
                        Action {
                            plugin: interaction.plugin.clone(),
                            action: interaction.selected_action().to_owned(),
                            context: interaction.context.clone(),
                        },
                        Some(core.input_text.clone()),
                    ),
                    KeyCode::Char(_) | KeyCode::Backspace if core.argument_pending.is_none() => {
                        Response::Pass
                    }
                    _ => Response::Consumed,
                };
            }
            let dialog = super::modals::from_tree(
                core,
                declarations,
                &interaction.plugin,
                &interaction.tree,
                viewport,
            );
            return super::modals::key(core, declarations, dialog, key, &mut self.dialog_focus);
        }
        if let Some(dialog) = super::modals::dialog(core, declarations, viewport) {
            return super::modals::key(core, declarations, dialog, key, &mut self.dialog_focus);
        }
        self.dialog_focus = 0;
        let visible = self
            .popup_rect(core, declarations, viewport)
            .map(interior)
            .unwrap_or(Rect::viewport(0, 0));
        if let Some(popup) = &mut self.popup {
            let mut count = 0;
            let mut selected = None;
            walk(
                core,
                declarations,
                popup.content().0,
                popup.content().1,
                visible,
                0,
                0,
                &mut |plugin, node, _| {
                    if selectable(node) {
                        if count == popup.selected {
                            selected = Some((plugin, node));
                        }
                        count += 1;
                    }
                },
            );
            match key.code {
                KeyCode::Esc => self.popup = None,
                KeyCode::Up | KeyCode::BackTab => {
                    if count > 0 {
                        popup.selected = (popup.selected + count - 1) % count;
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if count > 0 {
                        popup.selected = (popup.selected + 1) % count;
                    }
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let choice = selected.and_then(|(plugin, node)| popup.choice(plugin, node));
                    let (response, close) =
                        choice.map_or((Response::Consumed, true), |choice| popup.activate(choice));
                    if close {
                        self.popup = None;
                        self.focus = None;
                    }
                    return response;
                }
                _ => {}
            }
            return Response::Consumed;
        }
        if self.focus.is_none() {
            return Response::Pass;
        }
        match key.code {
            KeyCode::Esc => self.focus = None,
            KeyCode::Tab | KeyCode::Down => self.focus_next(core, declarations, viewport, false),
            KeyCode::BackTab | KeyCode::Up => self.focus_next(core, declarations, viewport, true),
            KeyCode::F(10) if key.modifiers.contains(KeyModifiers::SHIFT) => {
                let mut target = None;
                triggers(
                    core,
                    declarations,
                    viewport,
                    self.list_offset,
                    &mut |plugin, node, _| {
                        if self.focus.as_ref().is_some_and(|f| f.matches(plugin, node)) {
                            target = Some((plugin, node));
                        }
                    },
                );
                if let Some((plugin, node)) = target {
                    self.open(plugin, node);
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let mut target = None;
                triggers(
                    core,
                    declarations,
                    viewport,
                    self.list_offset,
                    &mut |plugin, node, _| {
                        if self.focus.as_ref().is_some_and(|f| f.matches(plugin, node)) {
                            target = Some((plugin, node));
                        }
                    },
                );
                if let Some((plugin, node)) = target {
                    if let Some(a) = action(plugin, node, None) {
                        return Response::Invoke(a);
                    }
                    self.open(plugin, node);
                }
            }
            _ => {}
        }
        Response::Consumed
    }
    pub(super) fn popup_rect(
        &self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
    ) -> Option<Rect> {
        if let Some(palette) = &self.palette {
            return Some(palette.rect(viewport));
        }
        let popup = self.popup.as_ref()?;
        if matches!(&popup.source, PopupSource::Command(_)) {
            return centered_popup(core, declarations, popup.content().1, viewport);
        }
        let (width, height) = popup_size(core, declarations, popup.content().1, viewport)?;
        let mut anchor = None;
        triggers(
            core,
            declarations,
            viewport,
            self.list_offset,
            &mut |plugin, node, rect| {
                if popup.origin.matches(plugin, node) {
                    anchor = Some(rect);
                }
            },
        );
        let anchor = anchor?;
        let x =
            (anchor.x + anchor.width.saturating_sub(2)).min(viewport.x + viewport.width - width);
        let below = anchor.y.saturating_add(1);
        let y = if below + height <= viewport.y + viewport.height {
            below
        } else {
            anchor.y.saturating_sub(height).max(viewport.y)
        };
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }
    pub(super) fn draw_popup(
        &self,
        out: &mut impl Write,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
    ) -> io::Result<Option<Rect>> {
        if let Some(palette) = &self.palette {
            return palette.draw(out, core, declarations, viewport);
        }
        let Some(rect) = self.popup_rect(core, declarations, viewport) else {
            return Ok(None);
        };
        let popup = self.popup.as_ref().expect("popup geometry requires popup");
        let bg = Color::Rgb {
            r: 33,
            g: 25,
            b: 38,
        };
        let edge = Color::Rgb {
            r: 73,
            g: 54,
            b: 78,
        };
        for y in rect.y..rect.y + rect.height {
            queue!(
                out,
                cursor::MoveTo(rect.x, y),
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(bg),
                SetForegroundColor(edge)
            )?;
            for x in 0..rect.width {
                let ch = if y == rect.y || y == rect.y + rect.height - 1 {
                    if x == 0 || x == rect.width - 1 {
                        '+'
                    } else {
                        '─'
                    }
                } else if x == 0 || x == rect.width - 1 {
                    '│'
                } else {
                    ' '
                };
                queue!(out, Print(ch))?;
            }
        }
        let mut result: io::Result<()> = Ok(());
        let mut index = 0;
        walk(
            core,
            declarations,
            popup.content().0,
            popup.content().1,
            interior(rect),
            0,
            0,
            &mut |_, node, area| {
                if result.is_err() {
                    return;
                }
                let (children, active) = match node {
                    Node::Row { children, .. } => (children.as_slice(), selectable(node)),
                    Node::Label { .. } => (std::slice::from_ref(node), false),
                    _ => return,
                };
                let selected = active && index == popup.selected;
                if active {
                    index += 1;
                }
                let row_bg = if selected {
                    Color::Rgb {
                        r: 58,
                        g: 43,
                        b: 67,
                    }
                } else {
                    bg
                };
                result = (|| {
                    queue!(
                        out,
                        cursor::MoveTo(area.x, area.y),
                        SetBackgroundColor(row_bg)
                    )?;
                    for _ in 0..area.width {
                        queue!(out, Print(' '))?;
                    }
                    queue!(out, cursor::MoveTo(area.x, area.y))?;
                    let mut used = 0;
                    for child in children {
                        if let Node::Label { text, role, .. } = child {
                            let fg = if role == "danger" {
                                Color::Rgb {
                                    r: 223,
                                    g: 140,
                                    b: 136,
                                }
                            } else if role == "muted" {
                                Color::Rgb {
                                    r: 147,
                                    g: 133,
                                    b: 159,
                                }
                            } else {
                                Color::Rgb {
                                    r: 185,
                                    g: 179,
                                    b: 189,
                                }
                            };
                            queue!(out, SetForegroundColor(fg))?;
                            for ch in text.chars() {
                                let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                                if used + width > usize::from(area.width) {
                                    break;
                                }
                                queue!(out, Print(ch))?;
                                used += width;
                            }
                        }
                    }
                    Ok(())
                })();
            },
        );
        result?;
        queue!(
            out,
            SetAttribute(Attribute::Reset),
            SetBackgroundColor(Color::Reset),
            SetForegroundColor(Color::Reset)
        )?;
        Ok(Some(rect))
    }
}
fn popup_size(
    core: &Core,
    declarations: &Declarations,
    tree: &Node,
    viewport: Rect,
) -> Option<(u16, u16)> {
    if viewport.width < 3 || viewport.height < 3 {
        return None;
    }
    Some((
        extent(core, declarations, tree, true)
            .0
            .saturating_add(4)
            .max(8)
            .min(viewport.width),
        extent(core, declarations, tree, false)
            .0
            .saturating_add(2)
            .min(viewport.height),
    ))
}
fn centered_popup(
    core: &Core,
    declarations: &Declarations,
    tree: &Node,
    viewport: Rect,
) -> Option<Rect> {
    let (width, height) = popup_size(core, declarations, tree, viewport)?;
    Some(Rect {
        x: viewport.x + (viewport.width - width) / 2,
        y: viewport.y + (viewport.height - height) / 2,
        width,
        height,
    })
}

pub(super) fn visit_interaction_choices(
    core: &Core,
    declarations: &Declarations,
    content: (&str, &Node),
    viewport: Rect,
    input: bool,
    visit: &mut dyn FnMut(usize, &str, &Node, bool) -> bool,
) -> bool {
    fn level(
        core: &Core,
        declarations: &Declarations,
        content: (&str, &Node),
        viewport: Rect,
        input: bool,
        depth: usize,
        visit: &mut dyn FnMut(usize, &str, &Node, bool) -> bool,
    ) -> bool {
        if depth > 32 {
            return false;
        }
        let mut valid = true;
        let mut inspect = |owner, node: &Node, _| {
            if !valid {
                return;
            }
            let focusable = if input {
                super::modals::actionable(node)
            } else {
                selectable(node)
            };
            valid = visit(depth, owner, node, focusable);
            if !input {
                if let Node::Row {
                    menu: Some(menu), ..
                } = node
                {
                    valid = valid
                        && level(
                            core,
                            declarations,
                            (owner, menu),
                            viewport,
                            false,
                            depth + 1,
                            visit,
                        );
                }
            }
        };
        if input {
            let dialog =
                super::modals::from_tree(core, declarations, content.0, content.1, viewport);
            super::modals::visit(core, declarations, dialog, &mut inspect);
        } else {
            let Some(rect) = centered_popup(core, declarations, content.1, viewport) else {
                return false;
            };
            walk(
                core,
                declarations,
                content.0,
                content.1,
                interior(rect),
                0,
                0,
                &mut inspect,
            );
        }
        valid
    }
    level(core, declarations, content, viewport, input, 0, visit)
}

fn interior(rect: Rect) -> Rect {
    Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    }
}
pub(super) fn intersect(a: Option<Rect>, b: Option<Rect>) -> bool {
    a.zip(b).is_some_and(|(a, b)| overlaps(a, b))
}
