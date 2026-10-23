//! Shared Core semantics and declaration selection. No renderer types or geometry.
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use trailboss_runtime::core_actions::CoreAction;
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
    NewSpace,
}
#[derive(Debug)]
struct TabFocusRequest {
    id: u64,
    tab: String,
    cancelled: bool,
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
    tab_cycle_commit: Option<String>,
    tab_focus_request: Option<TabFocusRequest>,
    tab_focus_sequence: u64,
    tab_focus_response: Vec<u8>,
    completed_tab_focus: Option<String>,
    tab_cycle_deadline: Option<Instant>,
    copy_feedback_deadline: Option<Instant>,
    require_name: bool,
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
        let preview = self
            .tab_focus_request
            .as_ref()
            .is_some_and(|request| Some(&request.tab) == tab.as_ref())
            || self
                .completed_tab_focus
                .as_ref()
                .is_some_and(|id| Some(id) == tab.as_ref())
            || (self.tab_focus_request.is_none()
                && self.completed_tab_focus.is_none()
                && self
                    .tab_cycle_order
                    .iter()
                    .any(|id| Some(id) == tab.as_ref()));
        if self.focused_tab_id != tab && !preview {
            self.cancel_tab_cycle();
        }
        if self.tab_cycle_cursor.is_none() && self.tab_cycle_commit.is_none() {
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
        self.cancel_tab_cycle();
        self.tab_focus_request = None;
        self.tab_focus_response.clear();
    }
    fn cancel_tab_cycle(&mut self) {
        self.tab_cycle_order.clear();
        self.tab_cycle_cursor = None;
        self.tab_cycle_deadline = None;
        self.tab_cycle_commit = None;
        // Cancel intent, not the server's outstanding command lease.
        if let Some(request) = &mut self.tab_focus_request {
            request.cancelled = true;
        }
        self.completed_tab_focus = None;
    }
    /// Discard closed visits when this client's live resource snapshot changes.
    pub fn reconcile_tab_history(
        &mut self,
        snapshot: &trailboss_runtime::protocol::ClientShellSnapshot,
    ) {
        self.recent_tabs
            .retain(|id| snapshot.tabs.iter().any(|tab| &tab.tab_id == id));
        let selected = self
            .tab_cycle_cursor
            .map(|cursor| &self.tab_cycle_order[cursor])
            .or(self.tab_cycle_commit.as_ref());
        if selected.is_some_and(|id| !snapshot.tabs.iter().any(|tab| &tab.tab_id == id)) {
            self.cancel_tab_cycle();
        }
        if self.tab_focus_request.is_none()
            && self
                .completed_tab_focus
                .as_ref()
                .is_some_and(|tab| snapshot.focused_tab_id.as_ref() != Some(tab))
        {
            self.cancel_tab_cycle();
            self.commit_current_tab_selection();
        }
    }
    pub(crate) fn has_previous_tab(&self) -> bool {
        self.recent_tabs
            .iter()
            .any(|id| self.focused_tab_id.as_ref() != Some(id))
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
            if let Some(selected) = &self.tab_cycle_commit {
                if let Some(index) = self.tab_cycle_order.iter().position(|id| id == selected) {
                    self.tab_cycle_order[..=index].rotate_right(1);
                }
            }
            if self.tab_cycle_order.len() < 2 {
                self.tab_cycle_order.clear();
                return None;
            }
            let current = self
                .tab_cycle_commit
                .as_ref()
                .or(self.completed_tab_focus.as_ref())
                .or(self.focused_tab_id.as_ref());
            self.tab_cycle_cursor = Some(
                self.tab_cycle_order
                    .iter()
                    .position(|id| Some(id) == current)
                    .unwrap_or(0),
            );
            self.tab_cycle_commit = None;
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
        self.cancel_tab_cycle();
        None
    }
    /// End navigation now; an in-flight request must succeed before history settles.
    pub fn finish_tab_cycle(&mut self) -> bool {
        self.tab_cycle_deadline = None;
        let Some(cursor) = self.tab_cycle_cursor.take() else {
            return false;
        };
        let selected = self.tab_cycle_order.swap_remove(cursor);
        if self.tab_focus_request.is_some() {
            self.tab_cycle_commit = Some(selected);
        } else {
            self.commit_tab_selection(&selected);
        }
        self.tab_cycle_order.clear();
        true
    }
    fn commit_tab_selection(&mut self, selected: &str) {
        if let Some(index) = self.recent_tabs.iter().position(|id| id == selected) {
            self.recent_tabs[..=index].rotate_right(1);
        }
    }
    fn commit_current_tab_selection(&mut self) {
        if let Some(index) = self
            .recent_tabs
            .iter()
            .position(|id| Some(id) == self.focused_tab_id.as_ref())
        {
            self.recent_tabs[..=index].rotate_right(1);
        }
    }
    /// Explicit selection supersedes unsent navigation, but not the endpoint lease.
    pub fn interrupt_tab_cycle(&mut self) -> bool {
        let changed = self.finish_tab_cycle();
        self.cancel_tab_cycle();
        self.commit_current_tab_selection();
        changed
    }
    pub fn tab_cycle_request_id(&self) -> Option<u64> {
        self.tab_focus_request.as_ref().map(|request| request.id)
    }
    /// Pane input must wait for both command completion and its focus projection.
    pub fn tab_focus_settled(&self) -> bool {
        self.tab_focus_request.is_none()
            && self.tab_cycle_commit.is_none()
            && self
                .completed_tab_focus
                .as_ref()
                .is_none_or(|tab| self.focused_tab_id.as_ref() == Some(tab))
    }
    pub fn request_tab_focus(&mut self, tab: String) -> Effect {
        self.cancel_tab_cycle();
        self.tab_cycle_commit = Some(tab);
        self.tab_cycle_focus_effect()
    }
    fn tab_cycle_focus_effect(&mut self) -> Effect {
        if self.tab_focus_request.is_some() {
            return Effect::Consumed;
        }
        let target = self
            .tab_cycle_cursor
            .map(|cursor| &self.tab_cycle_order[cursor])
            .or(self.tab_cycle_commit.as_ref());
        let Some(target) = target else {
            return Effect::Consumed;
        };
        let params = json!({"tab_id": target});
        self.tab_focus_sequence += 1;
        self.tab_focus_request = Some(TabFocusRequest {
            id: self.tab_focus_sequence,
            tab: target.clone(),
            cancelled: false,
        });
        self.tab_focus_response.clear();
        Effect::Request {
            method: "tab.focus",
            params,
        }
    }
    /// The endpoint accepts one command at a time. Coalesce only unsent previews.
    pub fn request_tab_cycle(
        &mut self,
        snapshot: &trailboss_runtime::protocol::ClientShellSnapshot,
    ) -> Effect {
        if self.cycle_tab(snapshot).is_none() {
            return Effect::Consumed;
        }
        self.tab_cycle_focus_effect()
    }
    pub fn tab_cycle_response(
        &mut self,
        request_id: &str,
        final_chunk: bool,
        data: &[u8],
        snapshot: Option<&trailboss_runtime::protocol::ClientShellSnapshot>,
    ) -> Option<Effect> {
        let id = request_id.strip_prefix("tab-cycle-")?.parse::<u64>().ok()?;
        if self.tab_cycle_request_id() != Some(id) {
            return None;
        }
        self.tab_focus_response.extend_from_slice(data);
        if !final_chunk {
            return None;
        }
        let success =
            serde_json::from_slice::<Value>(&self.tab_focus_response).is_ok_and(|response| {
                response.get("result").is_some() && response.get("error").is_none()
            });
        self.tab_focus_response.clear();
        let completed = self.tab_focus_request.take().unwrap();
        if !completed.cancelled && !success {
            self.cancel_tab_cycle();
            self.commit_current_tab_selection();
            return None;
        }
        if !completed.cancelled {
            self.completed_tab_focus = Some(completed.tab);
        }
        let desired = self
            .tab_cycle_cursor
            .map(|cursor| &self.tab_cycle_order[cursor])
            .or(self.tab_cycle_commit.as_ref());
        if let Some(desired) = desired {
            if !snapshot.is_some_and(|s| s.tabs.iter().any(|tab| &tab.tab_id == desired)) {
                self.cancel_tab_cycle();
            } else if completed.cancelled || self.completed_tab_focus.as_ref() != Some(desired) {
                return Some(self.tab_cycle_focus_effect());
            } else if let Some(selected) = self.tab_cycle_commit.take() {
                self.commit_tab_selection(&selected);
            }
        }
        None
    }
    /// Finalization changes MRU history only; it never sends a focus request.
    pub fn tick_tab_cycle(&mut self, now: Instant) -> bool {
        self.tab_cycle_deadline
            .is_some_and(|deadline| now >= deadline)
            && self.finish_tab_cycle()
    }
    /// F6 is nonmodal; the legacy prefix cycle retains its explicit finish keys.
    pub fn tab_cycle_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        is_prefix: bool,
        snapshot: Option<&trailboss_runtime::protocol::ClientShellSnapshot>,
        now: Instant,
    ) -> Option<Effect> {
        use crossterm::event::KeyCode;
        if key.modifiers.is_empty() && key.code == KeyCode::F(6) {
            let effect = snapshot.map_or(Effect::Consumed, |snapshot| {
                self.request_tab_cycle(snapshot)
            });
            self.tab_cycle_deadline = self
                .tab_cycle_cursor
                .map(|_| now + Duration::from_millis(800));
            return Some(effect);
        }
        if self.tab_cycle_deadline.is_some() || self.tab_cycle_commit.is_some() {
            self.finish_tab_cycle();
            return None;
        }
        if self.tab_cycle_cursor.is_none() || is_prefix {
            return None;
        }
        if key.modifiers.is_empty() && key.code == KeyCode::Char('l') {
            return Some(snapshot.map_or(Effect::Consumed, |snapshot| {
                self.request_tab_cycle(snapshot)
            }));
        }
        self.finish_tab_cycle();
        (key.modifiers.is_empty() && matches!(key.code, KeyCode::Enter | KeyCode::Esc))
            .then_some(Effect::Consumed)
    }
    pub fn tab_cycle_hint(&self) -> Option<&'static str> {
        self.tab_cycle_cursor.map(|_| {
            if self.tab_cycle_deadline.is_some() {
                "MRU: F6 next"
            } else {
                "MRU: L next · Enter/Esc done"
            }
        })
    }
    pub fn editing(&self) -> bool {
        self.interaction != Interaction::None
    }
    /// Prepare a named command using the same interaction as native entrypoints.
    pub fn prepare_palette(&mut self, action: &str, context: &Context) -> Effect {
        let interaction = match CoreAction::parse(action) {
            Some(CoreAction::NewSpace) => Some(Interaction::NewSpace),
            Some(CoreAction::New) => Some(Interaction::NewTab {
                workspace_id: self.workspace_id.clone(),
            }),
            Some(CoreAction::NewTab) => {
                context.workspace_id.as_ref().map(|id| Interaction::NewTab {
                    workspace_id: Some(id.clone()),
                })
            }
            Some(CoreAction::RenameSpace) => {
                context
                    .workspace_id
                    .as_ref()
                    .map(|id| Interaction::RenameSpace {
                        workspace_id: id.clone(),
                    })
            }
            Some(CoreAction::RenameTab) => context
                .tab_id
                .as_ref()
                .map(|id| Interaction::RenameTab { tab_id: id.clone() }),
            _ => return self.invoke(action, context, None),
        };
        if let Some(interaction) = interaction {
            self.interaction = interaction;
            self.require_name = true;
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
        let Some(action) = CoreAction::parse(action) else {
            return Effect::Plugin;
        };
        if matches!(
            action,
            CoreAction::New | CoreAction::RenameSpace | CoreAction::RenameTab
        ) {
            self.require_name = false;
        }
        if action == CoreAction::Focus && context.tab_id.is_none() {
            return snapshot
                .and_then(|snapshot| self.previous_tab(snapshot))
                .map_or(Effect::Consumed, |tab| Effect::Request {
                    method: "tab.focus",
                    params: json!({"tab_id":tab}),
                });
        }
        match action {
            CoreAction::NewSpace => {
                self.interaction = Interaction::None;
                Effect::Request {
                    method: "workspace.create",
                    params: json!({"label":"New space","focus":true}),
                }
            }
            CoreAction::New => {
                self.interaction = Interaction::NewTab {
                    workspace_id: self.workspace_id.clone(),
                };
                self.input_text.clear();
                Effect::Consumed
            }
            CoreAction::NewTab => {
                context
                    .workspace_id
                    .as_ref()
                    .map_or(Effect::Consumed, |workspace| Effect::Request {
                        method: "tab.create",
                        params: json!({"workspace_id":workspace,"label":"New tab","focus":true}),
                    })
            }
            CoreAction::RenameSpace => {
                if let Some(workspace_id) = &context.workspace_id {
                    self.interaction = Interaction::RenameSpace {
                        workspace_id: workspace_id.clone(),
                    };
                    self.input_text.clear();
                }
                Effect::Consumed
            }
            CoreAction::RenameTab => {
                if let Some(tab_id) = &context.tab_id {
                    self.interaction = Interaction::RenameTab {
                        tab_id: tab_id.clone(),
                    };
                    self.input_text.clear();
                }
                Effect::Consumed
            }
            CoreAction::CloseTab => {
                context
                    .tab_id
                    .as_ref()
                    .map_or(Effect::Consumed, |tab| Effect::Request {
                        method: "tab.close",
                        params: json!({"tab_id":tab}),
                    })
            }
            CoreAction::RestartIdleAgents => {
                self.interaction = Interaction::None;
                Effect::Request {
                    method: "agent.restart_idle",
                    params: json!({}),
                }
            }
            CoreAction::Detach => {
                self.detach_requested = true;
                Effect::Consumed
            }
            CoreAction::Cancel => {
                self.interaction = Interaction::None;
                self.input_text.clear();
                Effect::Consumed
            }
            CoreAction::Create => {
                if (self.require_name
                    || matches!(
                        self.interaction,
                        Interaction::RenameSpace { .. } | Interaction::RenameTab { .. }
                    ))
                    && self.input_text.trim().is_empty()
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
                    Interaction::NewSpace => Effect::Request {
                        method: "workspace.create",
                        params: json!({"label":self.input_text,"focus":true}),
                    },
                    Interaction::None => Effect::Consumed,
                }
            }
            CoreAction::Focus => {
                context
                    .tab_id
                    .as_ref()
                    .map_or(Effect::Consumed, |tab| Effect::Request {
                        method: "tab.focus",
                        params: json!({"tab_id":tab}),
                    })
            }
        }
    }
    /// Returns None only when text belongs to the focused PTY.
    pub fn text(&mut self, text: &str) -> Option<Effect> {
        if self.interaction == Interaction::None {
            return (!self.has_pane).then_some(Effect::Consumed);
        }
        if text == "\x1b" {
            return Some(self.invoke(CoreAction::Cancel.id(), &Context::default(), None));
        }
        if text == "\r" {
            return Some(self.invoke(CoreAction::Create.id(), &Context::default(), None));
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
