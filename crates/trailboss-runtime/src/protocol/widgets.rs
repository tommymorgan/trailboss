//! Renderer-neutral plugin declarations. Geometry belongs to the consuming client.
use crate::core_actions::{CoreAction, COMMAND_FRAGMENT_PREFIX};
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
    RenameSpace,
    RenameTab,
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
        /// Declarative popup content; clients own its placement and interaction.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        menu: Option<Box<Node>>,
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
    /// Out-of-flow modal content; clients own centered placement and input capture.
    Modal {
        child: Box<Node>,
    },
    /// Core-owned editable text; producers own its placement.
    Input,
    /// Resolve a public named fragment supplied by any producer.
    Use {
        name: String,
    },
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
pub struct Fragment {
    pub name: String,
    pub priority: i32,
    pub tree: Node,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    // Product branding does not change the established declaration wire key.
    #[serde(rename = "herdr")]
    pub trailboss: String,
    pub version: u32,
    pub zones: Vec<Placement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Composition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<Fragment>,
}
impl Declaration {
    pub fn validate(&self, actions: impl Fn(&str) -> bool) -> bool {
        self.trailboss == "widget_tree"
            && self.version == 1
            && self.zones.len() <= 3
            && self.fragments.len() <= 128
            && self.fragments.iter().enumerate().all(|(i, fragment)| {
                valid_name(&fragment.name)
                    && self.fragments[..i]
                        .iter()
                        .all(|other| other.name != fragment.name)
                    && valid_node(&fragment.tree, &actions, 0)
            })
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

    /// Producer ingress additionally excludes exports owned by Core itself.
    /// Structural validation remains available for Core-generated declarations.
    pub fn validate_producer(&self, actions: impl Fn(&str) -> bool) -> bool {
        self.fragments
            .iter()
            .all(|fragment| !fragment.name.starts_with(COMMAND_FRAGMENT_PREFIX))
            && self.validate(actions)
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
            menu,
        } => {
            children.len() <= 128
                && action.as_deref().is_none_or(|id| {
                    CoreAction::parse(id).is_some() || (!id.starts_with("host:") && actions(id))
                })
                && [&context.workspace_id, &context.tab_id, &context.pane_id]
                    .into_iter()
                    .all(|id| id.as_ref().is_none_or(|id| !id.is_empty()))
                && menu.as_ref().is_none_or(|menu| {
                    matches!(menu.as_ref(), Node::Column { .. } | Node::Use { .. })
                        && valid_node(menu, actions, depth + 1)
                })
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
        Node::When { child, .. } | Node::Modal { child } => valid_node(child, actions, depth + 1),
        Node::Zone { fallback, .. } => fallback
            .as_ref()
            .is_none_or(|child| valid_node(child, actions, depth + 1)),
        Node::Use { name } => valid_name(name),
        Node::Leader { .. } | Node::Spacer { .. } | Node::Surface | Node::Input => true,
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 200 && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn declaration(menu: serde_json::Value) -> Declaration {
        serde_json::from_value(json!({
            "herdr": "widget_tree", "version": 1,
            "zones": [{"zone": "footer", "tree": {"type": "row", "children": [], "menu": menu}}]
        }))
        .unwrap()
    }

    #[test]
    fn menu_actions_and_context_round_trip_without_a_privileged_node() {
        let menu = json!({"type": "column", "children": [
            {"type": "row", "children": [{"type":"input"}], "action":"host:rename-space", "context":{"workspace_id":"exact"}},
            {"type": "row", "children": [], "action":"host:new-tab"},
            {"type": "row", "children": [], "action":"host:close-tab", "context":{"tab_id":"tab"}},
            {"type": "row", "children": [], "action":"host:detach"}
        ]});
        let value = declaration(menu);
        assert!(value.validate(|_| false));
        assert_eq!(
            serde_json::from_str::<Declaration>(&serde_json::to_string(&value).unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn menus_reject_invalid_content_and_do_not_restore_retired_host_actions() {
        for menu in [
            json!({"type":"row", "children":[]}),
            json!({"type":"column", "children":[{"type":"row", "children":[], "action":"undeclared"}]}),
            json!({"type":"column", "children":[{"type":"row", "children":[], "context":{"workspace_id":""}}]}),
            json!({"type":"column", "children":[{"type":"label", "text":"bad\n", "role":"label"}]}),
        ] {
            assert!(!declaration(menu).validate(|_| false));
        }
        assert!(!declaration(json!({"type":"column", "children":[{"type":"row", "children":[], "action":"host:menu"}]})).validate(|_| true));
        assert!(!declaration(
            json!({"type":"column", "children":vec![json!({"type":"input"}); 129]})
        )
        .validate(|_| true));
        let mut nested = json!({"type":"input"});
        for _ in 0..17 {
            nested = json!({"type":"column", "children":[nested]});
        }
        assert!(!declaration(nested).validate(|_| true));
        assert!(serde_json::from_value::<Node>(json!({"type":"tab_name"})).is_err());
        assert!(serde_json::from_value::<Condition>(json!("menu")).is_err());
    }
    #[test]
    fn fragments_validate_without_zones_and_names_are_not_privileged() {
        let value = json!({"herdr":"widget_tree", "version":1, "zones":[], "fragments":[
            {"name":"any.plugin.fragment", "priority":5, "tree":{"type":"use", "name":"another.fragment"}}
        ]});
        let parsed: Declaration = serde_json::from_value(value.clone()).unwrap();
        assert!(parsed.validate(|_| false));
        for fragments in [
            json!([{ "name":"", "priority":0, "tree":{"type":"input"}}]),
            json!([{ "name":"bad\nname", "priority":0, "tree":{"type":"input"}}]),
            json!([value["fragments"][0].clone(), value["fragments"][0].clone()]),
            json!([{ "name":"valid", "priority":0, "tree":{"type":"row", "children":[], "action":"missing"}}]),
        ] {
            let mut invalid = value.clone();
            invalid["fragments"] = fragments;
            assert!(!serde_json::from_value::<Declaration>(invalid)
                .unwrap()
                .validate(|_| false));
        }
        assert!(
            declaration(json!({"type":"use", "name":"any.plugin.fragment"})).validate(|_| false)
        );
    }
}
