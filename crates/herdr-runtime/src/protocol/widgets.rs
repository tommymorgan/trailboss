//! Renderer-neutral plugin declarations. Geometry belongs to the consuming client.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    List,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Column {
        children: Vec<Node>,
    },
    Row {
        children: Vec<Node>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(default)]
        context: Context,
    },
    Label {
        text: String,
        role: String,
    },
    Leader,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub zone: Zone,
    pub tree: Node,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub herdr: String,
    pub version: u32,
    pub zones: Vec<Placement>,
}
impl Declaration {
    /// Core validates structure and declared actions before forwarding data.
    pub fn validate(&self, actions: impl Fn(&str) -> bool) -> bool {
        self.herdr == "widget_tree"
            && self.version == 1
            && self.zones.len() == 1
            && self.zones.iter().all(|p| match &p.tree {
                Node::Column { children } => {
                    children.len() <= 128
                        && children.iter().all(|row| match row {
                            Node::Row {
                                children,
                                action,
                                context,
                            } => {
                                // A header can carry one glyph per tab admitted by the column.
                                children.len() <= 128
                                    && action.as_deref().is_none_or(&actions)
                                    && [&context.workspace_id, &context.tab_id, &context.pane_id]
                                        .into_iter()
                                        .all(|id| id.as_ref().is_none_or(|id| !id.is_empty()))
                                    && children.iter().all(|child| match child {
                                        Node::Label { text, role } => {
                                            text.chars().count() <= 200
                                                && !role.is_empty()
                                                && !text.chars().any(char::is_control)
                                        }
                                        Node::Leader => true,
                                        _ => false,
                                    })
                            }
                            _ => false,
                        })
                }
                _ => false,
            })
    }
}
