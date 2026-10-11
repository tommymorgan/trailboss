//! Renderer-neutral plugin declarations. Geometry belongs to the consuming client.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    List,
    Footer,
    FooterRight,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tone {
    BgLime,
    FgCyan,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    NoPane,
    HasPane,
    NewTab,
    Menu,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
    },
    Leader {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
    },
    /// Empty logical spacing along the containing flow; clients choose its scale.
    Spacer {
        size: u16,
    },
    /// Ordered entries flowing vertically, without implicit markers or gaps.
    List {
        entries: Vec<Node>,
    },
    /// Relative allocation, not terminal-cell geometry.
    Flex {
        weight: u16,
        child: Box<Node>,
    },
    /// Reference to the Core's focused runtime-owned pane, never a plugin cell grid.
    Surface,
    Zone {
        zone: Zone,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<Box<Node>>,
    },
    When {
        condition: Condition,
        child: Box<Node>,
    },
    /// Core-owned editable new-tab name; producers own its placement.
    TabName,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub zone: Zone,
    pub tree: Node,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    /// Public precedence: any plugin can replace an application composition.
    pub priority: i32,
    pub tree: Node,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub herdr: String,
    pub version: u32,
    pub zones: Vec<Placement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Composition>,
}
impl Declaration {
    pub fn validate(&self, actions: impl Fn(&str) -> bool) -> bool {
        self.herdr == "widget_tree"
            && self.version == 1
            && self.zones.len() <= 3
            && self
                .zones
                .iter()
                .enumerate()
                .all(|(i, p)| self.zones[..i].iter().all(|other| other.zone != p.zone))
            && self.zones.iter().all(|p| valid_node(&p.tree, &actions, 0))
            && self
                .composition
                .as_ref()
                .is_none_or(|c| valid_node(&c.tree, &actions, 0))
    }
}
fn valid_node(node: &Node, actions: &impl Fn(&str) -> bool, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    match node {
        Node::Row {
            children,
            action,
            context,
        } => {
            children.len() <= 128
                && action.as_deref().is_none_or(|id| {
                    matches!(
                        id,
                        "host:new" | "host:menu" | "host:cancel" | "host:create" | "host:focus"
                    ) || actions(id)
                })
                && [&context.workspace_id, &context.tab_id, &context.pane_id]
                    .into_iter()
                    .all(|id| id.as_ref().is_none_or(|id| !id.is_empty()))
                && children
                    .iter()
                    .all(|child| valid_node(child, actions, depth + 1))
        }
        Node::Column { children } | Node::List { entries: children } => {
            children.len() <= 128
                && children
                    .iter()
                    .all(|child| valid_node(child, actions, depth + 1))
        }
        Node::Label { text, role, .. } => {
            text.chars().count() <= 200
                && !role.is_empty()
                && !text.chars().chain(role.chars()).any(char::is_control)
        }
        Node::Flex { weight, child } => *weight > 0 && valid_node(child, actions, depth + 1),
        Node::When { child, .. } => valid_node(child, actions, depth + 1),
        Node::Zone { fallback, .. } => fallback
            .as_ref()
            .is_none_or(|child| valid_node(child, actions, depth + 1)),
        Node::Leader { .. } | Node::Spacer { .. } | Node::Surface | Node::TabName => true,
    }
}
