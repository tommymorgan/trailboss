//! Public command palette producer. Core owns the catalog and every action.
use serde_json::{json, Value};
use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    sync::mpsc,
    thread,
};
use trailboss::{core::Core, sidebar::Declarations};
use trailboss_runtime::{
    api_client::{ApiClient, ConnectionTarget},
    api_schema::{Request, UiCommandInfo, UiCommandKey, UiCommandTarget},
    protocol::{
        self,
        endpoint::*,
        widgets::{Composition, Context, Declaration, Fragment, Node},
        ClientMessage, ClientShellSnapshot, ClientSurfaceSize, ServerMessage,
    },
};

const OWNER: &str = "tommymorgan.command-palette";

fn label(text: impl AsRef<str>) -> Node {
    // Match Core's public command-label policy, including whole control escapes.
    const LIMIT: usize = 200;
    let mut bytes = [0u8; LIMIT * 4];
    let mut used = 0;
    let mut scalars = 0;
    let mut chars = text.as_ref().chars().peekable();
    while let Some(ch) = chars.next() {
        let escaped = ch.is_control().then(|| ch.escape_debug());
        let token_len = escaped.as_ref().map_or(1, |token| token.clone().count());
        let budget = LIMIT - usize::from(chars.peek().is_some());
        if scalars + token_len > budget {
            used += '…'.encode_utf8(&mut bytes[used..]).len();
            break;
        }
        if let Some(token) = escaped {
            for part in token {
                used += part.encode_utf8(&mut bytes[used..]).len();
            }
        } else {
            used += ch.encode_utf8(&mut bytes[used..]).len();
        }
        scalars += token_len;
    }
    Node::Label {
        text: std::str::from_utf8(&bytes[..used])
            .expect("palette labels contain only encoded Unicode scalars")
            .to_owned(),
        role: "label".into(),
        tone: None,
    }
}
fn column(mut children: Vec<Node>) -> Node {
    while children.len() > 128 {
        let count = children.len().div_ceil(128);
        let mut entries = children.into_iter();
        children = (0..count)
            .map(|_| Node::Column {
                children: entries.by_ref().take(128).collect(),
            })
            .collect();
    }
    Node::Column { children }
}
fn branch(title: String, context: Context, children: Vec<Node>) -> Node {
    Node::Row {
        children: vec![label(title)],
        action: None,
        context,
        menu: Some(Box::new(column(children))),
    }
}
fn command_use(command: &UiCommandInfo) -> Node {
    Node::Use {
        name: command.fragment_name.clone(),
    }
}

fn owner_contexts(owner: &str, action: &str, declarations: &Declarations) -> Vec<Context> {
    fn visit(
        node: &Node,
        inherited: &Context,
        scope: (&str, &str, &Core, &Declarations),
        depth: usize,
        found: &mut Vec<Context>,
    ) {
        if depth > 32 {
            return;
        }
        let (owner, wanted, core, declarations) = scope;
        match node {
            Node::Use { name } => {
                if let Some((provider, tree)) = core.fragment(declarations, name) {
                    if provider == owner {
                        visit(tree, inherited, scope, depth + 1, found);
                    }
                }
            }
            Node::Row {
                children,
                action,
                context,
                menu,
            } => {
                let context = if context == &Context::default() {
                    inherited
                } else {
                    context
                };
                if action.as_deref() == Some(wanted)
                    && *context != Context::default()
                    && !found.contains(context)
                {
                    found.push(context.clone());
                }
                for child in children {
                    visit(child, context, scope, depth + 1, found);
                }
                if let Some(menu) = menu {
                    visit(menu, context, scope, depth + 1, found);
                }
            }
            Node::Column { children } | Node::List { entries: children } => {
                for child in children {
                    visit(child, inherited, scope, depth + 1, found);
                }
            }
            Node::Flex { child, .. } | Node::When { child, .. } | Node::Modal { child } => {
                visit(child, inherited, scope, depth + 1, found);
            }
            Node::Zone {
                fallback: Some(child),
                ..
            } => {
                visit(child, inherited, scope, depth + 1, found);
            }
            _ => {}
        }
    }
    let core = Core::default();
    let mut contexts = Vec::new();
    for (_, declaration) in declarations
        .iter()
        .filter(|(provider, _)| provider == owner)
    {
        for tree in declaration
            .zones
            .iter()
            .map(|zone| &zone.tree)
            .chain(declaration.fragments.iter().map(|fragment| &fragment.tree))
            .chain(
                declaration
                    .composition
                    .iter()
                    .map(|composition| &composition.tree),
            )
        {
            visit(
                tree,
                &Context::default(),
                (owner, action, &core, declarations),
                0,
                &mut contexts,
            );
        }
    }
    contexts
}

fn declaration(
    snapshot: &ClientShellSnapshot,
    commands: &[UiCommandInfo],
    declarations: &Declarations,
) -> Declaration {
    let core = Core::default();
    let mut children = Vec::new();
    for command in commands {
        if command.diagnostic.is_some()
            || core
                .fragment(declarations, &command.fragment_name)
                .is_none()
        {
            continue;
        }
        match command.target {
            UiCommandTarget::Global | UiCommandTarget::ClientLocal => {
                children.push(command_use(command))
            }
            UiCommandTarget::Owner => {
                let contexts = match &command.key {
                    UiCommandKey::Plugin {
                        plugin_id,
                        action_id,
                    } => owner_contexts(plugin_id, action_id, declarations),
                    UiCommandKey::Core { .. } => Vec::new(),
                };
                if contexts.is_empty() {
                    children.push(command_use(command));
                } else {
                    // Reuse Core's public display label, which already escapes
                    // controls and marks truncation, without changing catalog metadata.
                    let Some((
                        _,
                        Node::Row {
                            children: labels, ..
                        },
                    )) = core.fragment(declarations, &command.fragment_name)
                    else {
                        continue;
                    };
                    let Some(Node::Label { text: title, .. }) = labels.first() else {
                        continue;
                    };
                    // Keep the issued owner-bound Use. An existing owner row's
                    // nearer target wins over the interactive client's context;
                    // manifest context categories do not invent target bindings.
                    for context in contexts {
                        let workspace = context.workspace_id.as_ref().and_then(|id| {
                            snapshot
                                .workspaces
                                .iter()
                                .find(|workspace| workspace.workspace_id == *id)
                        });
                        if context.workspace_id.is_some() && workspace.is_none() {
                            continue;
                        }
                        let tab = context
                            .tab_id
                            .as_ref()
                            .and_then(|id| snapshot.tabs.iter().find(|tab| tab.tab_id == *id));
                        if context.tab_id.is_some() && tab.is_none() {
                            continue;
                        }
                        let pane = context.pane_id.as_ref().map(|id| {
                            snapshot
                                .panes
                                .iter()
                                .find(|pane| pane.pane_id == *id)
                                .and_then(|pane| pane.label.as_deref())
                                .unwrap_or(id.as_str())
                        });
                        let targets = [
                            workspace.map(|workspace| workspace.label.as_str()),
                            tab.map(|tab| tab.label.as_str()),
                            pane,
                        ];
                        // Keep each public Label independently bounded. Prefix
                        // targets so a long command cannot hide its destination.
                        let mut labels =
                            Vec::with_capacity(targets.iter().flatten().count() * 2 + 1);
                        for target in targets.into_iter().flatten() {
                            labels.push(label(target));
                            labels.push(label(" / "));
                        }
                        labels.push(label(title));
                        children.push(Node::Row {
                            children: labels,
                            action: None,
                            context,
                            menu: Some(Box::new(column(vec![command_use(command)]))),
                        });
                    }
                }
            }
            UiCommandTarget::Workspace | UiCommandTarget::Tab => {
                let mut workspaces = Vec::new();
                for workspace in &snapshot.workspaces {
                    let context = Context {
                        workspace_id: Some(workspace.workspace_id.clone()),
                        ..Context::default()
                    };
                    let choices = if command.target == UiCommandTarget::Workspace {
                        vec![command_use(command)]
                    } else {
                        snapshot
                            .tabs
                            .iter()
                            .filter(|tab| tab.workspace_id == workspace.workspace_id)
                            .map(|tab| {
                                branch(
                                    tab.label.clone(),
                                    Context {
                                        workspace_id: Some(workspace.workspace_id.clone()),
                                        tab_id: Some(tab.tab_id.clone()),
                                        ..Context::default()
                                    },
                                    vec![command_use(command)],
                                )
                            })
                            .collect()
                    };
                    if !choices.is_empty() {
                        workspaces.push(branch(workspace.label.clone(), context, choices));
                    }
                }
                if !workspaces.is_empty() {
                    children.push(branch(
                        command.title.clone(),
                        Context::default(),
                        workspaces,
                    ));
                }
            }
        }
    }
    Declaration {
        trailboss: "widget_tree".into(),
        version: 1,
        zones: Vec::new(),
        composition: None::<Composition>,
        fragments: vec![Fragment {
            name: "command-palette".into(),
            priority: 0,
            tree: column(children),
        }],
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    trailboss_runtime::session::configure_from_args(&std::env::args().collect::<Vec<_>>())?;
    let api_path = trailboss_runtime::session::active_api_socket_path();
    // The observer and catalog must address the same Core. Match the server's
    // API-socket stem mapping, including an inherited HERDR_SOCKET_PATH.
    let stem = api_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("trailboss");
    let path = api_path.with_file_name(format!("{stem}-client.sock"));
    let mut endpoint = UnixStream::connect(path)?;
    let hello = EndpointClientHello {
        generation: ENDPOINT_PROTOCOL_GENERATION,
        cell_width_px: 0,
        cell_height_px: 0,
        surface_size: ClientSurfaceSize { cols: 80, rows: 24 },
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: false,
        surface_reuse: false,
        surface_delta: false,
        surface_scroll: false,
        snapshot_codecs: vec![SNAPSHOT_CODEC_V1.into()],
        surface_codecs: vec![SURFACE_CODEC_V1.into()],
        input_codecs: vec![INPUT_CODEC_V1.into()],
        blob_codecs: vec![BLOB_CODEC_V1.into()],
    };
    protocol::write_message(
        &mut endpoint,
        &ClientMessage::EndpointControl {
            kind: ENDPOINT_HELLO_KIND.into(),
            data: serde_json::to_string(&hello)?,
        },
    )?;
    // The endpoint reader never sends input, activation, focus or resize. A bounded
    // channel lets updates coalesce while the single catalog request is in flight.
    let (send, receive) = mpsc::sync_channel(32);
    thread::spawn(move || loop {
        let message =
            protocol::read_message::<_, ServerMessage>(&mut endpoint, protocol::MAX_FRAME_SIZE);
        let failed = message.is_err();
        if send.send(message).is_err() || failed {
            break;
        }
    });
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(api_path));
    let request: Request = serde_json::from_value(
        json!({"id":"palette-catalog","method":"ui.command.list","params":{}}),
    )?;
    let mut snapshot: Option<ClientShellSnapshot> = None;
    let mut ui: Option<EndpointPluginUi> = None;
    let mut previous_inputs = Value::Null;
    let mut previous_output = String::new();
    loop {
        let first = receive.recv()?;
        for message in std::iter::once(first).chain(receive.try_iter()) {
            if let ServerMessage::EndpointControl { kind, data } = message? {
                match kind.as_str() {
                    ENDPOINT_SNAPSHOT_KIND => snapshot = Some(serde_json::from_str(&data)?),
                    PLUGIN_UI_KIND => {
                        let mut projection: EndpointPluginUi = serde_json::from_str(&data)?;
                        projection.ui.retain(|item| item.plugin_id != OWNER);
                        ui = Some(projection);
                    }
                    _ => {}
                }
            }
        }
        let (Some(snapshot), Some(ui)) = (&snapshot, &ui) else {
            continue;
        };
        if snapshot.boot_id != ui.boot_id || snapshot.revision != ui.revision {
            continue;
        }
        let declarations: Declarations = ui
            .ui
            .iter()
            .filter_map(|item| {
                item.declaration
                    .as_ref()
                    .map(|tree| (item.plugin_id.clone(), tree.clone()))
            })
            .collect();
        // Ignore frame/status/focus churn and our own declaration. Only semantic
        // owner declarations and resource choices require another catalog read.
        let inputs = json!({"boot":snapshot.boot_id,"declarations":declarations,
            "workspaces":snapshot.workspaces.iter().map(|w| (&w.workspace_id,&w.label)).collect::<Vec<_>>(),
            "tabs":snapshot.tabs.iter().map(|t| (&t.workspace_id,&t.tab_id,&t.label)).collect::<Vec<_>>(),
            "panes":snapshot.panes.iter().map(|p| (&p.pane_id,&p.label)).collect::<Vec<_>>()});
        if inputs == previous_inputs {
            continue;
        }
        let reply = api.request_value(&request)?;
        if let Some(error) = reply.get("error") {
            return Err(format!("command discovery failed: {error}").into());
        }
        let result = &reply["result"];
        if result["boot_id"] != snapshot.boot_id {
            continue;
        }
        let commands: Vec<UiCommandInfo> = serde_json::from_value(result["commands"].clone())?;
        let next = serde_json::to_string(&declaration(snapshot, &commands, &declarations))?;
        if next != previous_output {
            let mut output = io::stdout().lock();
            writeln!(output, "{next}")?;
            output.flush()?;
            previous_output = next;
        }
        previous_inputs = inputs;
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("command-palette: {error}");
        std::process::exit(1);
    }
}
