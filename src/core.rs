//! Shared Core semantics and declaration selection. No renderer types or geometry.
use herdr_runtime::protocol::widgets::{Condition, Context, Declaration, Node};
use serde_json::{json, Value};

#[derive(Default, Debug, PartialEq, Eq)]
enum Interaction {
    #[default]
    None,
    NewTab,
    Menu,
}
#[derive(Default, Debug)]
pub struct Core {
    interaction: Interaction,
    pub tab_name: String,
    pub workspace_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub has_pane: bool,
}
pub enum Effect {
    Consumed,
    Request { method: &'static str, params: Value },
    Plugin,
}
impl Core {
    pub fn update_session(
        &mut self,
        workspace: Option<String>,
        tab: Option<String>,
        has_pane: bool,
    ) {
        self.workspace_id = workspace;
        self.focused_tab_id = tab;
        self.has_pane = has_pane;
    }
    pub fn invoke(&mut self, action: &str, context: &Context) -> Effect {
        match action {
            "host:new" => {
                self.interaction = Interaction::NewTab;
                self.tab_name.clear();
                Effect::Consumed
            }
            "host:menu" => {
                self.interaction = if self.interaction == Interaction::Menu {
                    Interaction::None
                } else {
                    Interaction::Menu
                };
                Effect::Consumed
            }
            "host:cancel" => {
                self.interaction = Interaction::None;
                Effect::Consumed
            }
            "host:create" if self.interaction == Interaction::NewTab => {
                self.interaction = Interaction::None;
                let label = if self.tab_name.is_empty() {
                    "New tab"
                } else {
                    &self.tab_name
                };
                if let Some(workspace) = &self.workspace_id {
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
            return Some(self.invoke("host:cancel", &Context::default()));
        }
        if self.interaction == Interaction::NewTab {
            if text == "\r" {
                return Some(self.invoke("host:create", &Context::default()));
            }
            if text == "\x7f" {
                self.tab_name.pop();
            } else if self.tab_name.chars().count() < 80 && !text.chars().any(char::is_control) {
                self.tab_name.push_str(text);
            }
        }
        Some(Effect::Consumed)
    }
    pub fn visible(&self, condition: Condition) -> bool {
        match condition {
            Condition::NoPane => !self.has_pane,
            Condition::HasPane => self.has_pane,
            Condition::NewTab => self.interaction == Interaction::NewTab,
            Condition::Menu => self.interaction == Interaction::Menu,
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
