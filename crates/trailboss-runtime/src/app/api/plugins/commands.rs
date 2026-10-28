//! Discovery and ordinary Use exports derived from the same current registry.
//! Execution stays in native Core dispatch and plugin.action.invoke.
use std::{
    collections::{BTreeMap, HashSet},
    io,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

use crate::{
    api::schema::{ResponseResult, UiCommandInfo, UiCommandKey, UiCommandTarget},
    app::{App, AppState},
    core_actions::{
        CoreAction, CoreCommandInteraction, CoreCommandTarget, CoreFocusMode,
        COMMAND_FRAGMENT_PREFIX, CORE_COMMANDS,
    },
    protocol::{
        endpoint::{EndpointPluginUi, EndpointPluginUiItem, PLUGIN_UI_KIND},
        widgets::{Context, Declaration, Fragment, Node},
        ServerMessage, MAX_FRAME_SIZE,
    },
};

use super::super::responses::{encode_error, encode_success};
use super::{manifest, plugin_manifest_available};

impl App {
    pub(in crate::app) fn handle_ui_command_list(&mut self, id: String) -> String {
        if let Err(error) = self.refresh_installed_plugins() {
            return encode_error(id, "plugin_registry_load_failed", error.to_string());
        }
        let ordinary =
            super::sidebar_ui_rows(&self.state.installed_plugins, &self.state.plugin_ui_dynamic);
        let (commands, _) = match projection(&self.state, ordinary) {
            Ok(publication) => publication,
            Err(error) => return encode_error(id, "ui_command_projection_failed", error),
        };
        encode_success(
            id,
            ResponseResult::UiCommandList {
                boot_id: self.state.command_boot_id.clone(),
                capabilities: vec![
                    crate::protocol::endpoint::PLUGIN_ACTION_ARGUMENTS_CAPABILITY.to_owned(),
                ],
                commands,
            },
        )
    }
}

fn locator(boot_id: &str, key: &UiCommandKey) -> (String, [u8; 32]) {
    // Hash the typed boot/key tuple without allocating a serialized identity.
    let mut identity = Sha256::new();
    serde_json::to_writer(&mut identity, &(boot_id, key))
        .expect("command identity is serializable");
    let digest = identity.finalize();
    let mut name = String::with_capacity(COMMAND_FRAGMENT_PREFIX.len() + 43);
    name.push_str(COMMAND_FRAGMENT_PREFIX);
    URL_SAFE_NO_PAD.encode_string(digest, &mut name);
    (name, digest.into())
}

fn command_label(title: &str) -> String {
    // Public Labels allow 200 Unicode scalars. Build only that bounded display,
    // retaining each printable control escape as an indivisible token.
    const LIMIT: usize = 200;
    let mut bytes = [0u8; LIMIT * 4];
    let mut used = 0;
    let mut scalars = 0;
    let mut chars = title.chars().peekable();
    while let Some(ch) = chars.next() {
        let escaped = ch.is_control().then(|| ch.escape_debug());
        let token_len = escaped.as_ref().map_or(1, |token| token.clone().count());
        // A final token may fill all 200 scalars; otherwise reserve the ellipsis.
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
    std::str::from_utf8(&bytes[..used])
        .expect("command labels contain only encoded Unicode scalars")
        .to_owned()
}

fn row(title: &str, action: Option<&str>, menu: Option<Node>) -> Node {
    Node::Row {
        children: vec![Node::Label {
            text: command_label(title),
            role: "label".to_owned(),
            tone: None,
        }],
        action: action.map(str::to_owned),
        context: Context::default(),
        menu: menu.map(Box::new),
    }
}

fn confirmation<'a>(
    ordinary: &'a [EndpointPluginUiItem],
    owner: &str,
    action: &str,
    name: &str,
) -> Result<&'a Node, &'static str> {
    let mut exports = ordinary
        .iter()
        .filter(|item| item.plugin_id == owner)
        .filter_map(|item| item.declaration.as_ref())
        .flat_map(|declaration| &declaration.fragments)
        .filter(|fragment| fragment.name == name);
    let export = exports
        .next()
        .ok_or("confirmation export is not published")?;
    if exports.next().is_some() {
        return Err("confirmation export is ambiguous");
    }
    let Node::Column { children } = &export.tree else {
        return Err("confirmation export must be a self-contained Column");
    };
    let mut cancel = 0;
    let mut affirmative = 0;
    for child in children {
        match child {
            Node::Label { .. } => {}
            Node::Row {
                children,
                action: row_action,
                context,
                menu,
            } if menu.is_none()
                && *context == Context::default()
                && children
                    .iter()
                    .all(|child| matches!(child, Node::Label { .. })) =>
            {
                match row_action.as_deref() {
                    Some("host:cancel") => cancel += 1,
                    Some(id) if id == action => affirmative += 1,
                    _ => return Err("confirmation export contains another executable action"),
                }
            }
            _ => return Err("confirmation export contains unsupported content or target context"),
        }
    }
    if cancel != 1 || affirmative != 1 {
        return Err("confirmation export requires exactly one Cancel and one affirmative row");
    }
    Ok(&export.tree)
}

#[derive(Default)]
struct FrameJson {
    bytes: Vec<u8>,
}

impl io::Write for FrameJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FRAME_SIZE.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("complete command publication exceeds maximum {MAX_FRAME_SIZE}"),
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_publication(
    boot_id: &str,
    rows: Vec<EndpointPluginUiItem>,
) -> Result<Vec<EndpointPluginUiItem>, String> {
    // Reserve the largest existing revision encoding, so the same complete
    // publication fits every client's current paired snapshot/UI revision.
    let publication = EndpointPluginUi {
        boot_id: boot_id.to_owned(),
        revision: u64::MAX,
        ui: rows,
    };
    let mut json = FrameJson::default();
    serde_json::to_writer(&mut json, &publication).map_err(|error| error.to_string())?;
    let message = ServerMessage::EndpointControl {
        kind: PLUGIN_UI_KIND.to_owned(),
        data: String::from_utf8(json.bytes).expect("serde_json emits UTF-8"),
    };
    let mut size = bincode::enc::write::SizeWriter::default();
    bincode::serde::encode_into_writer(&message, &mut size, bincode::config::standard())
        .map_err(|error| error.to_string())?;
    if size.bytes_written > MAX_FRAME_SIZE {
        return Err(crate::protocol::FramingError::Oversized {
            claimed: size.bytes_written,
            max: MAX_FRAME_SIZE,
        }
        .to_string());
    }
    Ok(publication.ui)
}

pub(crate) fn projection(
    state: &AppState,
    ordinary: Vec<EndpointPluginUiItem>,
) -> Result<(Vec<UiCommandInfo>, Vec<EndpointPluginUiItem>), String> {
    if state.command_boot_id.is_empty() {
        return Ok((Vec::new(), ordinary));
    }
    let mut commands = Vec::new();
    let mut owners: BTreeMap<
        String,
        Vec<(
            Fragment,
            Option<crate::protocol::endpoint::EndpointCommandInput>,
        )>,
    > = BTreeMap::new();
    let mut issued = HashSet::new();
    let mut publish =
        |owner: &str, mut command: UiCommandInfo, tree: Option<Node>| -> Result<(), String> {
            let (name, digest) = locator(&state.command_boot_id, &command.key);
            if !issued.insert(digest) {
                return Err("command locator collision prevents complete publication".to_owned());
            }
            command.fragment_name = name;
            if let Some(tree) = tree {
                let input = (command.argument_input.is_some() || command.argument_hint.is_some())
                    .then(|| crate::protocol::endpoint::EndpointCommandInput {
                        fragment_name: command.fragment_name.clone(),
                        title: command_label(&command.title),
                        argument_input: command.argument_input.clone(),
                        argument_hint: command.argument_hint.as_deref().map(command_label),
                    });
                owners.entry(owner.to_owned()).or_default().push((
                    Fragment {
                        name: command.fragment_name.clone(),
                        priority: 0,
                        tree,
                    },
                    input,
                ));
            }
            commands.push(command);
            Ok(())
        };
    for descriptor in CORE_COMMANDS {
        let action = descriptor.action.id();
        let menu = match descriptor.interaction {
            CoreCommandInteraction::Immediate => None,
            CoreCommandInteraction::Name { submit_label } => Some(Node::Column {
                children: vec![
                    Node::Input,
                    row(submit_label, Some("host:create"), None),
                    row("Cancel", Some("host:cancel"), None),
                ],
            }),
            CoreCommandInteraction::Confirm => Some(Node::Column {
                children: vec![
                    row("Cancel", Some("host:cancel"), None),
                    row(descriptor.title, Some(action), None),
                ],
            }),
        };
        let target = match descriptor.target {
            CoreCommandTarget::Global => UiCommandTarget::Global,
            CoreCommandTarget::Workspace => UiCommandTarget::Workspace,
            CoreCommandTarget::Tab => UiCommandTarget::Tab,
            CoreCommandTarget::NativeScope | CoreCommandTarget::ClientLocal => {
                UiCommandTarget::ClientLocal
            }
        };
        publish(
            "trailboss.core",
            UiCommandInfo {
                argument_input: None,
                argument_hint: None,
                key: UiCommandKey::Core {
                    action_id: action.to_owned(),
                    mode: descriptor.focus_mode.map(|mode| match mode {
                        CoreFocusMode::Targeted => "targeted".to_owned(),
                        CoreFocusMode::Previous => "previous".to_owned(),
                    }),
                },
                title: descriptor.title.to_owned(),
                description: None,
                contexts: Vec::new(),
                target,
                fragment_name: String::new(),
                diagnostic: None,
            },
            Some(row(
                descriptor.title,
                (!matches!(descriptor.interaction, CoreCommandInteraction::Confirm))
                    .then_some(action),
                menu,
            )),
        )?;
    }
    let host = manifest::current_platform();
    let mut plugins: Vec<_> = state.installed_plugins.values().collect();
    plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    for plugin in plugins {
        if !plugin.enabled || !plugin_manifest_available(plugin) {
            continue;
        }
        for action in &plugin.actions {
            if manifest::effective_platforms(&action.platforms, &plugin.platforms)
                .as_ref()
                .is_some_and(|platforms| !platforms.contains(&host))
            {
                continue;
            }
            // Native dispatch reserves these identities before plugin routing.
            // Keep legacy registration intact, but never issue a plugin Use
            // that would execute a different (Core-owned) operation.
            let (tree, diagnostic) = if CoreAction::parse(&action.id).is_some() {
                (
                    None,
                    Some(format!(
                        "{}/{}: action identity is reserved by Core",
                        plugin.plugin_id, action.id
                    )),
                )
            } else {
                match action.confirmation_fragment.as_deref() {
                    None => (Some(row(&action.title, Some(&action.id), None)), None),
                    Some(name) => {
                        match confirmation(&ordinary, &plugin.plugin_id, &action.id, name) {
                            Ok(menu) => (Some(row(&action.title, None, Some(menu.clone()))), None),
                            Err(reason) => (
                                None,
                                Some(format!("{}/{}: {reason}", plugin.plugin_id, action.id)),
                            ),
                        }
                    }
                }
            };
            publish(
                &plugin.plugin_id,
                UiCommandInfo {
                    argument_input: action.argument_input.clone(),
                    argument_hint: action.argument_hint.clone(),
                    key: UiCommandKey::Plugin {
                        plugin_id: plugin.plugin_id.clone(),
                        action_id: action.id.clone(),
                    },
                    title: action.title.clone(),
                    description: action.description.clone(),
                    contexts: action.contexts.clone(),
                    target: UiCommandTarget::Owner,
                    fragment_name: String::new(),
                    diagnostic,
                },
                tree,
            )?;
        }
    }
    let ordinary_ids: HashSet<_> = ordinary
        .iter()
        .map(|item| (item.plugin_id.as_str(), item.ui_id.as_str()))
        .collect();
    let mut generated = Vec::new();
    for (owner, fragments) in owners {
        let mut fragments = fragments.into_iter();
        for chunk in 0usize.. {
            let mut chunk_fragments = Vec::with_capacity(fragments.len().min(128));
            let mut command_inputs = Vec::new();
            for (fragment, input) in fragments.by_ref().take(128) {
                chunk_fragments.push(fragment);
                if let Some(input) = input {
                    command_inputs.push(input);
                }
            }
            if chunk_fragments.is_empty() {
                break;
            }
            let ui_id = format!("trailboss.command.v1:{chunk}");
            if ordinary_ids.contains(&(owner.as_str(), ui_id.as_str())) {
                return Err(
                    "command UI identity collision prevents complete publication".to_owned(),
                );
            }
            generated.push(EndpointPluginUiItem {
                command_inputs,
                plugin_id: owner.clone(),
                ui_id,
                title: String::new(),
                action: None,
                tab_id: None,
                workspace_id: None,
                pane_id: None,
                segments: Vec::new(),
                declaration: Some(Declaration {
                    trailboss: "widget_tree".to_owned(),
                    version: 1,
                    zones: Vec::new(),
                    composition: None,
                    fragments: chunk_fragments,
                }),
            });
        }
    }
    drop(ordinary_ids);
    let mut rows = ordinary;
    rows.extend(generated);
    Ok((commands, bounded_publication(&state.command_boot_id, rows)?))
}
