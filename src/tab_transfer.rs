//! Client-local destination picker. The attached Core performs host discovery and transfer.
use serde::Deserialize;
use std::{
    io,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use trailboss::{core::Core, sidebar::Declarations, Input};
use trailboss_runtime::{
    core_actions::CoreAction,
    protocol::{
        endpoint::{
            TAB_TRANSFER_DESTINATIONS_REQUEST_KIND, TAB_TRANSFER_DESTINATIONS_RESPONSE_KIND,
        },
        widgets::{Context, Declaration, Fragment, Node},
        ClientMessage, ServerMessage,
    },
};

pub const OWNER: &str = "trailboss.native.tab-transfer";
pub const FRAGMENT: &str = "native.tab-transfer-picker";
pub const SELECT: &str = "native:tab-transfer-destination:";

#[derive(Deserialize)]
struct Destination {
    session: String,
    workspace_id: Option<String>,
    workspace_label: Option<String>,
}
#[derive(Deserialize)]
struct Destinations {
    request_id: String,
    boot_id: String,
    destinations: Vec<Destination>,
    error: Option<String>,
}
struct Request {
    id: String,
    response: Vec<u8>,
    deadline: Instant,
    status: bool,
}
struct Operation {
    id: String,
    request: Request,
}
#[derive(Default)]
pub struct Picker {
    sequence: u64,
    boot: String,
    tab: String,
    discovery: Option<Request>,
    destinations: Vec<Destination>,
    open: bool,
    create: Option<usize>,
    operation: Option<Operation>,
}
fn label(text: impl Into<String>) -> Node {
    Node::Label {
        text: text.into(),
        role: "text".into(),
        tone: None,
    }
}
fn row(text: impl Into<String>, action: &str) -> Node {
    Node::Row {
        children: vec![label(text)],
        action: Some(action.into()),
        context: Context::default(),
        menu: None,
    }
}
fn declare(declarations: &mut Declarations, children: Vec<Node>) {
    declarations.retain(|(owner, _)| owner != OWNER);
    declarations.push((
        OWNER.into(),
        Declaration {
            trailboss: "widget_tree".into(),
            version: 1,
            zones: Vec::new(),
            composition: None,
            fragments: vec![Fragment {
                name: FRAGMENT.into(),
                priority: 0,
                tree: Node::Column { children },
            }],
        },
    ));
}

/// Only mount a command actually issued to this capable native client by Core.
pub fn mount_command(declarations: &mut Declarations, core: &Core) {
    let Some((name, mut command)) = declarations.iter().filter(|(owner, _)| owner == "trailboss.core")
        .flat_map(|(_, declaration)| &declaration.fragments)
        .find(|fragment| matches!(&fragment.tree, Node::Row { action: Some(action), .. } if action == CoreAction::MoveToSession.id()))
        .map(|fragment| (fragment.name.clone(), fragment.tree.clone())) else { return; };
    if let Node::Row { context, .. } = &mut command {
        *context = Context {
            workspace_id: core.workspace_id.clone(),
            tab_id: core.focused_tab_id.clone(),
            ..Context::default()
        };
    }
    for (owner, declaration) in declarations {
        for fragment in &mut declaration.fragments {
            if owner.as_str() == "trailboss.core" && fragment.name == name {
                fragment.tree = command.clone();
            } else if fragment.name == "command-palette" {
                if let Node::Column { children } = &mut fragment.tree {
                    if !children.iter().any(
                        |node| matches!(node, Node::Use { name: candidate } if candidate == &name),
                    ) {
                        children.push(Node::Use { name: name.clone() });
                    }
                }
            } else if let Some(tab) = fragment.name.strip_prefix("sidebar.tab-menu:") {
                if let Node::Column { children } = &mut fragment.tree {
                    if !children.iter().any(|node| matches!(node, Node::Row { action: Some(action), .. } if action == CoreAction::MoveToSession.id())) {
                        let mut targeted = command.clone();
                        if let Node::Row { context, .. } = &mut targeted {
                            context.tab_id = Some(tab.into());
                            context.workspace_id = None;
                        }
                        children.push(targeted);
                    }
                }
            }
        }
    }
}
impl Picker {
    pub fn begin(
        &mut self,
        input: &mut Input,
        boot: &str,
        tab: String,
        declarations: &mut Declarations,
    ) -> io::Result<()> {
        if self.operation.is_some() {
            return Err(io::Error::other("A tab move is still awaiting its outcome"));
        }
        self.cancel(declarations);
        self.sequence += 1;
        let id = format!("tab-move-destinations-{}", self.sequence);
        input.send(&ClientMessage::EndpointControl {
            kind: TAB_TRANSFER_DESTINATIONS_REQUEST_KIND.into(),
            data: serde_json::json!({"request_id":id,"boot_id":boot}).to_string(),
        })?;
        self.boot = boot.into();
        self.tab = tab;
        self.open = true;
        self.discovery = Some(Request {
            id,
            response: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(5),
            status: false,
        });
        declare(
            declarations,
            vec![label("Move to session"), label("Loading destinations")],
        );
        Ok(())
    }
    pub fn response(
        &mut self,
        message: &ServerMessage,
        core: &mut Core,
        declarations: &mut Declarations,
    ) -> io::Result<()> {
        if let ServerMessage::EndpointControl { kind, data } = message {
            if kind != TAB_TRANSFER_DESTINATIONS_RESPONSE_KIND || self.discovery.is_none() {
                return Ok(());
            }
            let reply: Destinations = serde_json::from_str(data).map_err(io::Error::other)?;
            if reply.boot_id != self.boot
                || self
                    .discovery
                    .as_ref()
                    .is_none_or(|pending| pending.id != reply.request_id)
            {
                return Ok(());
            }
            self.discovery = None;
            if let Some(error) = reply.error {
                return Err(io::Error::other(error));
            }
            self.destinations = reply.destinations;
            let mut children = vec![label("Move to session")];
            for (index, destination) in self.destinations.iter().enumerate() {
                let title = trailboss_runtime::core_actions::transfer_destination_label(
                    &destination.session,
                    destination.workspace_label.as_deref(),
                );
                let mut choice = row(title, &format!("{SELECT}{index}"));
                if destination.workspace_id.is_none() {
                    if let Node::Row { menu, .. } = &mut choice {
                        *menu = Some(Box::new(Node::Column {
                            children: vec![
                                label("New destination space name"),
                                Node::Input,
                                row("Move tab", "host:create"),
                                row("Cancel", "host:cancel"),
                            ],
                        }));
                    }
                }
                children.push(choice);
            }
            if self.destinations.is_empty() {
                children.push(label("No compatible running destinations"));
            }
            declare(declarations, children);
        }
        let ServerMessage::ClientShellEndpointResponseChunk {
            boot_id,
            request_id,
            final_chunk,
            data,
        } = message
        else {
            return Ok(());
        };
        if boot_id != &self.boot {
            return Ok(());
        }
        let Some(operation) = self.operation.as_mut() else {
            return Ok(());
        };
        if request_id != &operation.request.id {
            return Ok(());
        }
        operation.request.response.extend_from_slice(data);
        if !final_chunk {
            return Ok(());
        }
        let reply: serde_json::Value = match serde_json::from_slice(&operation.request.response) {
            Ok(reply) => reply,
            Err(_) => {
                operation.request.deadline = Instant::now();
                return Ok(());
            }
        };
        if let Some(error) = reply.get("error") {
            let text = error
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("Cannot resolve move outcome");
            if operation.request.status {
                core.show_endpoint_notice(format!(
                    "Move outcome unknown: {text}. Operation {}",
                    operation.id
                ));
                operation.request.deadline = Instant::now() + Duration::from_secs(5);
            } else {
                let code = error
                    .get("code")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if matches!(code, "transfer_unresolved" | "transfer_unavailable") {
                    core.show_endpoint_notice(format!(
                        "Checking move outcome: {text}. Operation {}. Do not repeat the move",
                        operation.id
                    ));
                    operation.request.deadline = Instant::now();
                } else {
                    core.show_endpoint_notice(format!("Cannot move tab: {text}"));
                    self.operation = None;
                }
            }
            return Ok(());
        }
        if let Some(status) = reply.pointer("/result/status") {
            match status.get("phase").and_then(|phase| phase.as_str()) {
                Some("committed") => {
                    self.operation = None;
                    core.show_endpoint_notice("Tab moved to destination session".into());
                }
                Some("aborted") => {
                    core.show_endpoint_notice(format!(
                        "Tab move aborted: {}",
                        status
                            .get("error")
                            .and_then(|error| error.as_str())
                            .unwrap_or("source retained")
                    ));
                    self.operation = None;
                }
                _ => {
                    operation.request.deadline = Instant::now() + Duration::from_secs(2);
                }
            }
        } else if reply.pointer("/result/transfer").is_some() {
            self.operation = None;
            core.show_endpoint_notice("Tab moved to destination session".into());
        } else {
            operation.request.deadline = Instant::now();
        }
        Ok(())
    }
    pub fn select(
        &mut self,
        index: usize,
        core: &mut Core,
        input: &mut Input,
        declarations: &mut Declarations,
    ) -> io::Result<()> {
        let destination = self
            .destinations
            .get(index)
            .ok_or_else(|| io::Error::other("Destination selection is no longer available"))?;
        self.open = false;
        if destination.workspace_id.is_none() {
            self.create = Some(index);
            core.begin_transfer_space_input();
            return Ok(());
        }
        self.submit(index, None, input, declarations)
    }
    pub fn label(
        &mut self,
        label: String,
        input: &mut Input,
        declarations: &mut Declarations,
    ) -> io::Result<()> {
        let index = self
            .create
            .take()
            .ok_or_else(|| io::Error::other("Destination selection was cancelled"))?;
        if label.trim().is_empty() {
            return Err(io::Error::other("A destination space name is required"));
        }
        self.submit(index, Some(label), input, declarations)
    }
    fn submit(
        &mut self,
        index: usize,
        label: Option<String>,
        input: &mut Input,
        declarations: &mut Declarations,
    ) -> io::Result<()> {
        let destination = self
            .destinations
            .get(index)
            .ok_or_else(|| io::Error::other("Destination selection is no longer available"))?;
        self.sequence += 1;
        let operation_id = format!(
            "native-tab-move-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos(),
            self.sequence
        );
        let mut params = serde_json::json!({"tab_id":self.tab,"destination_session":destination.session,"operation_id":operation_id});
        if let Some(workspace) = &destination.workspace_id {
            params["workspace_id"] = serde_json::json!(workspace);
        } else {
            params["destination_workspace_label"] =
                serde_json::json!(label
                    .ok_or_else(|| io::Error::other("A destination space name is required"))?);
        }
        let id = format!("tab-move-submit-{}", self.sequence);
        // Record the operation before writing: even an uncertain write must never replay it.
        self.operation = Some(Operation {
            id: operation_id,
            request: Request {
                id: id.clone(),
                response: Vec::new(),
                deadline: Instant::now() + Duration::from_secs(5),
                status: false,
            },
        });
        self.cancel(declarations);
        input.send(&ClientMessage::ClientShellEndpointRequest {
            boot_id: self.boot.clone(),
            request: serde_json::json!({"id":id,"method":"tab.transfer","params":params})
                .to_string(),
        })
    }
    pub fn tick(&mut self, input: &mut Input, now: Instant) -> io::Result<()> {
        if self
            .discovery
            .as_ref()
            .is_some_and(|request| now >= request.deadline)
        {
            return Err(io::Error::other("Destination discovery timed out"));
        }
        if let Some(operation) = self
            .operation
            .as_mut()
            .filter(|operation| now >= operation.request.deadline)
        {
            self.sequence += 1;
            let id = format!("tab-move-status-{}", self.sequence);
            operation.request = Request {
                id: id.clone(),
                response: Vec::new(),
                deadline: now + Duration::from_secs(5),
                status: true,
            };
            input.send(&ClientMessage::ClientShellEndpointRequest { boot_id: self.boot.clone(), request: serde_json::json!({"id":id,"method":"tab.transfer.status","params":{"operation_id":operation.id}}).to_string() })?;
        }
        Ok(())
    }
    pub fn open(&self) -> bool {
        self.open
    }
    pub fn creating(&self) -> bool {
        self.create.is_some()
    }
    pub fn cancel(&mut self, declarations: &mut Declarations) {
        self.discovery = None;
        self.open = false;
        self.create = None;
        self.destinations.clear();
        declarations.retain(|(owner, _)| owner != OWNER);
    }
    pub fn reset_boot(&mut self, boot: &str, declarations: &mut Declarations, core: &mut Core) {
        self.boot = boot.into();
        if let Some(operation) = self.operation.as_mut() {
            operation.request.id.clear();
            operation.request.response.clear();
            operation.request.deadline = Instant::now();
            core.show_endpoint_notice(format!(
                "Checking move outcome after Core restart: {}",
                operation.id
            ));
        }
        self.cancel(declarations);
    }
    pub fn preserve(declarations: &mut Declarations, next: &mut Declarations) {
        if let Some(index) = declarations.iter().position(|(owner, _)| owner == OWNER) {
            next.push(declarations.swap_remove(index));
        }
    }
}
