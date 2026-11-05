use serde::Deserialize;
use std::{
    io,
    time::{Duration, Instant},
};
use trailboss::{sidebar::Declarations, Input};
use trailboss_runtime::{
    api_schema::SessionListEntry,
    core_actions::CoreAction,
    protocol::{
        widgets::{Context, Declaration, Fragment, Node},
        ClientMessage, ServerMessage,
    },
};

pub const OWNER: &str = "trailboss.native.sessions";
pub const FRAGMENT: &str = "native.session-picker";
pub const SELECT: &str = "native:select-session:";

struct Pending {
    id: String,
    boot: String,
    response: Vec<u8>,
    deadline: Instant,
}
#[derive(Default)]
pub struct Discovery {
    sequence: u64,
    pending: Option<Pending>,
    names: Vec<String>,
    open: bool,
    host: String,
}
#[derive(Deserialize)]
struct Reply {
    result: Option<List>,
    error: Option<Failure>,
}
#[derive(Deserialize)]
struct List {
    sessions: Vec<SessionListEntry>,
}
#[derive(Deserialize)]
struct Failure {
    message: String,
}
fn label(text: String) -> Node {
    Node::Label {
        text,
        role: "muted".into(),
        tone: None,
    }
}
fn declare(declarations: &mut Declarations, children: Vec<Node>) {
    let declaration = Declaration {
        trailboss: "widget_tree".into(),
        version: 1,
        zones: Vec::new(),
        composition: None,
        fragments: vec![Fragment {
            name: FRAGMENT.into(),
            priority: 0,
            tree: Node::Column { children },
        }],
    };
    if let Some((_, previous)) = declarations.iter_mut().find(|(owner, _)| owner == OWNER) {
        *previous = declaration;
    } else {
        declarations.push((OWNER.into(), declaration));
    }
}

fn contains_switch(node: &Node, fragment: &str) -> bool {
    match node {
        Node::Use { name } => name == fragment,
        Node::Row {
            children,
            action,
            menu,
            ..
        } => {
            action.as_deref() == Some(CoreAction::SwitchSession.id())
                || children.iter().any(|node| contains_switch(node, fragment))
                || menu
                    .as_ref()
                    .is_some_and(|node| contains_switch(node, fragment))
        }
        Node::Column { children } | Node::List { entries: children } => {
            children.iter().any(|node| contains_switch(node, fragment))
        }
        Node::Flex { child, .. } | Node::When { child, .. } | Node::Modal { child } => {
            contains_switch(child, fragment)
        }
        Node::Zone { fallback, .. } => fallback
            .as_ref()
            .is_some_and(|node| contains_switch(node, fragment)),
        _ => false,
    }
}

/// Place Core's capability-scoped command without changing producer declarations on Core.
pub fn mount_command(declarations: &mut Declarations) {
    let Some(name) = declarations
        .iter()
        .filter(|(owner, _)| owner == "trailboss.core")
        .flat_map(|(_, declaration)| &declaration.fragments)
        .find(|fragment| {
            matches!(&fragment.tree,
            Node::Row { action: Some(action), .. } if action == CoreAction::SwitchSession.id())
        })
        .map(|fragment| fragment.name.clone())
    else {
        return;
    };
    for (_, declaration) in declarations {
        for fragment in &mut declaration.fragments {
            if !matches!(
                fragment.name.as_str(),
                "sidebar.session-menu" | "command-palette"
            ) || contains_switch(&fragment.tree, &name)
            {
                continue;
            }
            if let Node::Column { children } = &mut fragment.tree {
                children.push(Node::Use { name: name.clone() });
            }
        }
    }
}
impl Discovery {
    pub fn begin(
        &mut self,
        input: &mut Input,
        boot: &str,
        host: &str,
        declarations: &mut Declarations,
    ) -> io::Result<()> {
        self.sequence += 1;
        let id = format!("session-list-{}", self.sequence);
        input.send(&ClientMessage::ClientShellEndpointRequest {
            boot_id: boot.to_owned(),
            request: serde_json::json!({"id":id,"method":"session.list","params":{}}).to_string(),
        })?;
        self.names.clear();
        self.pending = Some(Pending {
            id,
            boot: boot.to_owned(),
            response: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(5),
        });
        self.open = true;
        self.host.clear();
        self.host.push_str(host);
        declare(
            declarations,
            vec![
                label(format!("Switch session on {host}")),
                label("Loading sessions".into()),
            ],
        );
        Ok(())
    }
    pub fn response(
        &mut self,
        message: &ServerMessage,
        declarations: &mut Declarations,
    ) -> io::Result<()> {
        let ServerMessage::ClientShellEndpointResponseChunk {
            boot_id,
            request_id,
            final_chunk,
            data,
        } = message
        else {
            return Ok(());
        };
        let Some(pending) = self
            .pending
            .as_mut()
            .filter(|pending| pending.boot == *boot_id && pending.id == *request_id)
        else {
            return Ok(());
        };
        pending.response.extend_from_slice(data);
        if !final_chunk {
            return Ok(());
        }
        let pending = self.pending.take().expect("matched session discovery");
        let reply: Reply = serde_json::from_slice(&pending.response).map_err(io::Error::other)?;
        if let Some(error) = reply.error {
            return Err(io::Error::other(error.message));
        }
        let list = reply
            .result
            .ok_or_else(|| io::Error::other("missing session discovery result"))?;
        let mut children = vec![label(format!("Switch session on {}", self.host))];
        for session in list.sessions.into_iter().filter(|session| session.running) {
            let title = if session.default {
                format!("{} (default)", session.name)
            } else {
                session.name.clone()
            };
            children.push(Node::Row {
                children: vec![Node::Label {
                    text: title,
                    role: "text".into(),
                    tone: None,
                }],
                action: Some(format!("{SELECT}{}", session.name)),
                context: Context::default(),
                menu: None,
            });
            self.names.push(session.name);
        }
        if self.names.is_empty() {
            children.push(label("No running sessions".into()));
        }
        declare(declarations, children);
        Ok(())
    }
    pub fn contains(&self, name: &str) -> bool {
        self.open && self.names.iter().any(|candidate| candidate == name)
    }
    pub fn open(&self) -> bool {
        self.open
    }
    pub fn expired(&self, now: Instant) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| now >= pending.deadline)
    }
    pub fn cancel(&mut self, declarations: &mut Declarations) {
        self.pending = None;
        self.names.clear();
        self.open = false;
        declarations.retain(|(owner, _)| owner != OWNER);
    }
    pub fn preserve(declarations: &mut Declarations, next: &mut Declarations) {
        if let Some(index) = declarations.iter().position(|(owner, _)| owner == OWNER) {
            next.push(declarations.swap_remove(index));
        }
    }
}
