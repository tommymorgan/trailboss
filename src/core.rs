//! Shared Core semantics and declaration selection. No renderer types or geometry.
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use trailboss_runtime::protocol::widgets::{Condition, Context, Declaration, Node};

#[derive(Default, Debug, PartialEq, Eq)]
enum Interaction {
    #[default]
    None,
    NewTab {
        workspace_id: Option<String>,
    },
    RenameSpace {
        workspace_id: String,
    },
    RenameTab {
        tab_id: String,
    },
    Palette(PaletteInteraction),
}
#[derive(Debug, PartialEq, Eq)]
enum PaletteInteraction {
    NewSpace,
    NewTab { workspace_id: String },
    RenameSpace { workspace_id: String },
    RenameTab { tab_id: String },
}
#[derive(Default, Debug)]
pub struct Core {
    interaction: Interaction,
    pub input_text: String,
    pub detach_requested: bool,
    pub workspace_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub has_pane: bool,
    pub prefix_active: bool,
    recent_tabs: Vec<String>,
    tab_cycle_order: Vec<String>,
    tab_cycle_cursor: Option<usize>,
    finished_cycle_target: Option<String>,
    copy_feedback_deadline: Option<Instant>,
}
pub enum Effect {
    Consumed,
    Request { method: &'static str, params: Value },
    Plugin,
}
impl Core {
    /// Acknowledges a dispatched copy request without retaining its payload.
    pub fn copy_dispatched(&mut self, now: Instant) {
        self.copy_feedback_deadline = Some(now + Duration::from_secs(2));
    }
    pub fn copy_feedback(&self) -> Option<&'static str> {
        self.copy_feedback_deadline.map(|_| "copied to clipboard")
    }
    /// Returns whether an idle client must repaint to remove the acknowledgement.
    pub fn tick_copy_feedback(&mut self, now: Instant) -> bool {
        if self
            .copy_feedback_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.clear_copy_feedback();
            return true;
        }
        false
    }
    pub fn clear_copy_feedback(&mut self) {
        self.copy_feedback_deadline = None;
    }
    /// Public fragment precedence is priority, then plugin ID; no provider is privileged.
    pub fn fragment<'a>(
        &self,
        declarations: &'a [(String, Declaration)],
        name: &str,
    ) -> Option<(&'a str, &'a Node)> {
        declarations
            .iter()
            .flat_map(|(plugin, d)| d.fragments.iter().map(move |f| (plugin, f)))
            .filter(|(_, f)| f.name == name)
            .max_by_key(|(plugin, f)| (f.priority, *plugin))
            .map(|(plugin, f)| (plugin.as_str(), &f.tree))
    }
    pub fn update_session(
        &mut self,
        workspace: Option<String>,
        tab: Option<String>,
        has_pane: bool,
    ) {
        // Preview confirmations must not reorder the frozen traversal or MRU history.
        let pending = self.finished_cycle_target.as_ref();
        if self.tab_cycle_cursor.is_none()
            && pending.is_none_or(|target| Some(target) == tab.as_ref())
        {
            self.finished_cycle_target = None;
            if let Some(current) = &tab {
                if let Some(index) = self.recent_tabs.iter().position(|id| id == current) {
                    self.recent_tabs[..=index].rotate_right(1);
                } else {
                    self.recent_tabs.insert(0, current.clone());
                }
            }
        }
        self.workspace_id = workspace;
        self.focused_tab_id = tab;
        self.has_pane = has_pane;
    }
    pub fn clear_tab_history(&mut self) {
        self.recent_tabs.clear();
        self.tab_cycle_order.clear();
        self.tab_cycle_cursor = None;
        self.finished_cycle_target = None;
    }
    /// A one-shot previous visit, excluding the current snapshot focus and closed tabs.
    pub fn previous_tab(
        &self,
        snapshot: &trailboss_runtime::protocol::ClientShellSnapshot,
    ) -> Option<&str> {
        self.recent_tabs
            .iter()
            .find(|id| {
                snapshot.focused_tab_id.as_ref() != Some(*id)
                    && snapshot.tabs.iter().any(|tab| &tab.tab_id == *id)
            })
            .map(String::as_str)
    }
    /// Freeze MRU once; subsequent steps only advance an index, including confirmations.
    pub fn cycle_tab(
        &mut self,
        snapshot: &trailboss_runtime::protocol::ClientShellSnapshot,
    ) -> Option<&str> {
        if self.tab_cycle_cursor.is_none() {
            self.tab_cycle_order.clone_from(&self.recent_tabs);
            self.tab_cycle_order
                .retain(|id| snapshot.tabs.iter().any(|tab| &tab.tab_id == id));
            if self.tab_cycle_order.len() < 2 {
                self.tab_cycle_order.clear();
                return None;
            }
            let current = self
                .finished_cycle_target
                .as_ref()
                .or(self.focused_tab_id.as_ref());
            self.tab_cycle_cursor = Some(
                self.tab_cycle_order
                    .iter()
                    .position(|id| Some(id) == current)
                    .unwrap_or(0),
            );
            self.finished_cycle_target = None;
        }
        let cursor = self.tab_cycle_cursor.unwrap();
        for step in 1..=self.tab_cycle_order.len() {
            let next = (cursor + step) % self.tab_cycle_order.len();
            if snapshot
                .tabs
                .iter()
                .any(|tab| tab.tab_id == self.tab_cycle_order[next])
            {
                self.tab_cycle_cursor = Some(next);
                return Some(&self.tab_cycle_order[next]);
            }
        }
        self.tab_cycle_cursor = None;
        self.tab_cycle_order.clear();
        self.finished_cycle_target = None;
        None
    }
    /// Commit only the final preview to MRU; pending earlier confirmations stay previews.
    pub fn finish_tab_cycle(&mut self) -> bool {
        let Some(cursor) = self.tab_cycle_cursor.take() else {
            return false;
        };
        let selected = self.tab_cycle_order.swap_remove(cursor);
        if let Some(index) = self.recent_tabs.iter().position(|id| id == &selected) {
            self.recent_tabs[..=index].rotate_right(1);
        }
        self.finished_cycle_target =
            (self.focused_tab_id.as_ref() != Some(&selected)).then_some(selected);
        self.tab_cycle_order.clear();
        true
    }
    /// Cycle keys consume only navigation and explicit finish; other input passes through.
    pub fn tab_cycle_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        is_prefix: bool,
        snapshot: Option<&trailboss_runtime::protocol::ClientShellSnapshot>,
    ) -> Option<Effect> {
        use crossterm::event::KeyCode;
        if self.tab_cycle_cursor.is_none() || is_prefix {
            return None;
        }
        if key.modifiers.is_empty() && key.code == KeyCode::Char('l') {
            return Some(
                snapshot
                    .and_then(|snapshot| self.cycle_tab(snapshot))
                    .map_or(Effect::Consumed, |tab| Effect::Request {
                        method: "tab.focus",
                        params: json!({"tab_id":tab}),
                    }),
            );
        }
        self.finish_tab_cycle();
        (key.modifiers.is_empty() && matches!(key.code, KeyCode::Enter | KeyCode::Esc))
            .then_some(Effect::Consumed)
    }
    pub fn tab_cycle_hint(&self) -> Option<&'static str> {
        self.tab_cycle_cursor
            .map(|_| "MRU: L next · Enter/Esc done")
    }
    pub fn editing(&self) -> bool {
        self.interaction != Interaction::None
    }
    /// Prepare a palette name step without activating stock declaration forms.
    pub fn prepare_palette(&mut self, action: &str, context: &Context) -> Effect {
        let interaction = match action {
            "host:new-space" => Some(PaletteInteraction::NewSpace),
            "host:new-tab" => context
                .workspace_id
                .as_ref()
                .map(|id| PaletteInteraction::NewTab {
                    workspace_id: id.clone(),
                }),
            "host:rename-space" => {
                context
                    .workspace_id
                    .as_ref()
                    .map(|id| PaletteInteraction::RenameSpace {
                        workspace_id: id.clone(),
                    })
            }
            "host:rename-tab" => context
                .tab_id
                .as_ref()
                .map(|id| PaletteInteraction::RenameTab { tab_id: id.clone() }),
            _ => return self.invoke(action, context, None),
        };
        if let Some(interaction) = interaction {
            self.interaction = Interaction::Palette(interaction);
            self.input_text.clear();
        }
        Effect::Consumed
    }
    /// Untargeted host:focus selects the previous live visit in the current snapshot.
    pub fn invoke(
        &mut self,
        action: &str,
        context: &Context,
        snapshot: Option<&trailboss_runtime::protocol::ClientShellSnapshot>,
    ) -> Effect {
        if action == "host:focus" && context.tab_id.is_none() {
            return snapshot
                .and_then(|snapshot| self.previous_tab(snapshot))
                .map_or(Effect::Consumed, |tab| Effect::Request {
                    method: "tab.focus",
                    params: json!({"tab_id":tab}),
                });
        }
        match action {
            "host:new-space" => {
                self.interaction = Interaction::None;
                Effect::Request {
                    method: "workspace.create",
                    params: json!({"label":"New space","focus":true}),
                }
            }
            "host:new" => {
                self.interaction = Interaction::NewTab {
                    workspace_id: self.workspace_id.clone(),
                };
                self.input_text.clear();
                Effect::Consumed
            }
            "host:new-tab" => context
                .workspace_id
                .as_ref()
                .map_or(Effect::Consumed, |workspace| Effect::Request {
                    method: "tab.create",
                    params: json!({"workspace_id":workspace,"label":"New tab","focus":true}),
                }),
            "host:rename-space" => {
                if let Some(workspace_id) = &context.workspace_id {
                    self.interaction = Interaction::RenameSpace {
                        workspace_id: workspace_id.clone(),
                    };
                    self.input_text.clear();
                }
                Effect::Consumed
            }
            "host:rename-tab" => {
                if let Some(tab_id) = &context.tab_id {
                    self.interaction = Interaction::RenameTab {
                        tab_id: tab_id.clone(),
                    };
                    self.input_text.clear();
                }
                Effect::Consumed
            }
            "host:close-tab" => {
                context
                    .tab_id
                    .as_ref()
                    .map_or(Effect::Consumed, |tab| Effect::Request {
                        method: "tab.close",
                        params: json!({"tab_id":tab}),
                    })
            }
            "host:restart-idle-agents" => {
                self.interaction = Interaction::None;
                Effect::Request {
                    method: "agent.restart_idle",
                    params: json!({}),
                }
            }
            "host:detach" => {
                self.detach_requested = true;
                Effect::Consumed
            }
            "host:cancel" => {
                self.interaction = Interaction::None;
                self.input_text.clear();
                Effect::Consumed
            }
            "host:create" => {
                if matches!(
                    self.interaction,
                    Interaction::RenameSpace { .. }
                        | Interaction::RenameTab { .. }
                        | Interaction::Palette(_)
                ) && self.input_text.trim().is_empty()
                {
                    return Effect::Consumed;
                }
                match std::mem::take(&mut self.interaction) {
                    Interaction::NewTab { workspace_id } => {
                        let label = if self.input_text.is_empty() {
                            "New tab"
                        } else {
                            &self.input_text
                        };
                        if let Some(workspace) = workspace_id {
                            Effect::Request {
                                method: "tab.create",
                                params: json!({"workspace_id":workspace,"label":label,"focus":true}),
                            }
                        } else {
                            Effect::Request {
                                method: "workspace.create",
                                params: json!({"label":label,"focus":true}),
                            }
                        }
                    }
                    Interaction::RenameSpace { workspace_id } => Effect::Request {
                        method: "workspace.rename",
                        params: json!({"workspace_id":workspace_id,"label":self.input_text}),
                    },
                    Interaction::RenameTab { tab_id } => Effect::Request {
                        method: "tab.rename",
                        params: json!({"tab_id":tab_id,"label":self.input_text}),
                    },
                    Interaction::Palette(interaction) => {
                        let (method, params) = match interaction {
                            PaletteInteraction::NewSpace => (
                                "workspace.create",
                                json!({"label":self.input_text,"focus":true}),
                            ),
                            PaletteInteraction::NewTab { workspace_id } => (
                                "tab.create",
                                json!({"workspace_id":workspace_id,"label":self.input_text,"focus":true}),
                            ),
                            PaletteInteraction::RenameSpace { workspace_id } => (
                                "workspace.rename",
                                json!({"workspace_id":workspace_id,"label":self.input_text}),
                            ),
                            PaletteInteraction::RenameTab { tab_id } => (
                                "tab.rename",
                                json!({"tab_id":tab_id,"label":self.input_text}),
                            ),
                        };
                        Effect::Request { method, params }
                    }
                    Interaction::None => Effect::Consumed,
                }
            }
            "host:focus" => {
                context
                    .tab_id
                    .as_ref()
                    .map_or(Effect::Consumed, |tab| Effect::Request {
                        method: "tab.focus",
                        params: json!({"tab_id":tab}),
                    })
            }
            _ => Effect::Plugin,
        }
    }
    /// Returns None only when text belongs to the focused PTY.
    pub fn text(&mut self, text: &str) -> Option<Effect> {
        if self.interaction == Interaction::None {
            return (!self.has_pane).then_some(Effect::Consumed);
        }
        if text == "\x1b" {
            return Some(self.invoke("host:cancel", &Context::default(), None));
        }
        if text == "\r" {
            return Some(self.invoke("host:create", &Context::default(), None));
        }
        if text == "\x7f" || text == "\x08" {
            self.input_text.pop();
        } else {
            self.append_input(text);
        }
        Some(Effect::Consumed)
    }
    /// Pasted control bytes must not submit, cancel, or erase an edit.
    pub fn paste(&mut self, text: &str) -> Option<Effect> {
        if self.interaction == Interaction::None {
            return (!self.has_pane).then_some(Effect::Consumed);
        }
        self.append_input(text);
        Some(Effect::Consumed)
    }
    fn append_input(&mut self, text: &str) {
        if !text.chars().any(char::is_control) {
            let remaining = 80usize.saturating_sub(self.input_text.chars().count());
            self.input_text.extend(text.chars().take(remaining));
        }
    }
    pub fn visible(&self, condition: Condition) -> bool {
        match condition {
            Condition::NoPane => !self.has_pane,
            Condition::HasPane => self.has_pane,
            Condition::NewTab => matches!(self.interaction, Interaction::NewTab { .. }),
            Condition::RenameSpace => matches!(self.interaction, Interaction::RenameSpace { .. }),
            Condition::RenameTab => matches!(self.interaction, Interaction::RenameTab { .. }),
        }
    }
    /// Structural semantics shared by every renderer: rows are horizontal;
    /// columns and ordered lists are vertical. Empty flows consume no space.
    pub fn flow<'a>(&self, node: &'a Node) -> Option<(bool, &'a [Node])> {
        match self.resolve(node)? {
            Node::Row { children, .. } => Some((true, children)),
            Node::Column { children } => Some((false, children)),
            Node::List { entries } => Some((false, entries)),
            _ => None,
        }
    }
    pub fn resolve<'a>(&self, node: &'a Node) -> Option<&'a Node> {
        match node {
            Node::When { condition, child } => self
                .visible(*condition)
                .then(|| self.resolve(child))
                .flatten(),
            _ => Some(node),
        }
    }
    pub fn composition<'a>(
        &self,
        declarations: &'a [(String, Declaration)],
    ) -> Option<(&'a str, &'a Node)> {
        declarations
            .iter()
            .filter_map(|(id, d)| d.composition.as_ref().map(|c| (id.as_str(), c)))
            .max_by_key(|(id, c)| (c.priority, *id))
            .map(|(id, c)| (id, &c.tree))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_names_require_nonblank_input_and_keep_chosen_targets_across_focus_updates() {
        for (action, method, target) in [
            ("host:new-space", "workspace.create", None),
            ("host:new-tab", "tab.create", Some("workspace_id")),
            (
                "host:rename-space",
                "workspace.rename",
                Some("workspace_id"),
            ),
            ("host:rename-tab", "tab.rename", Some("tab_id")),
        ] {
            let mut core = Core::default();
            let context = Context {
                workspace_id: Some("chosen".into()),
                tab_id: Some("chosen".into()),
                ..Default::default()
            };
            assert!(matches!(
                core.prepare_palette(action, &context),
                Effect::Consumed
            ));
            assert!(core.editing());
            for condition in [
                Condition::NewTab,
                Condition::RenameSpace,
                Condition::RenameTab,
            ] {
                assert!(!core.visible(condition));
            }
            core.text("   ");
            assert!(matches!(
                core.invoke("host:create", &Context::default(), None),
                Effect::Consumed
            ));
            assert!(core.editing());
            core.input_text.clear();
            core.update_session(Some("changed".into()), Some("changed".into()), true);
            core.paste("Named 🦀");
            let Effect::Request {
                method: actual,
                params,
            } = core.invoke("host:create", &Context::default(), None)
            else {
                panic!("submit missing")
            };
            assert_eq!(actual, method);
            assert_eq!(params["label"], "Named 🦀");
            if let Some(key) = target {
                assert_eq!(params[key], "chosen");
            }
            assert!(!core.editing());
            assert!(core.text("shell").is_none());
        }
    }
    #[test]
    fn palette_cancellation_restores_pane_input_and_missing_ids_do_not_prepare_edits() {
        let mut core = Core {
            has_pane: true,
            ..Default::default()
        };
        for action in ["host:new-tab", "host:rename-space", "host:rename-tab"] {
            core.prepare_palette(action, &Context::default());
            assert!(!core.editing());
        }
        core.prepare_palette("host:new-space", &Context::default());
        core.text("draft");
        core.invoke("host:cancel", &Context::default(), None);
        assert!(!core.editing());
        assert!(core.input_text.is_empty());
        assert!(core.text("shell").is_none());
    }
    #[test]
    fn producer_close_and_collapsed_state_actions_remain_plugin_owned() {
        let mut core = Core::default();
        for action in ["close-space", "expand-workspace", "collapse-workspace"] {
            assert!(matches!(
                core.invoke(action, &Context::default(), None),
                Effect::Plugin
            ));
        }
    }
}
