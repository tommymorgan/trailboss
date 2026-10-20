//! Discovery and ordinary Use exports derived from the same current registry.
//! Execution stays in native Core dispatch and plugin.action.invoke.
use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::{
    api::schema::{ResponseResult, UiCommandInfo, UiCommandKey, UiCommandTarget},
    app::{App, AppState},
    core_actions::{
        CoreCommandInteraction, CoreCommandTarget, CoreFocusMode, COMMAND_FRAGMENT_PREFIX,
        CORE_COMMANDS,
    },
    protocol::{
        endpoint::EndpointPluginUiItem,
        widgets::{Context, Declaration, Fragment, Node},
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
        let (commands, _) = projection(&self.state, &ordinary);
        encode_success(
            id,
            ResponseResult::UiCommandList {
                boot_id: self.state.command_boot_id.clone(),
                commands,
            },
        )
    }
}

fn locator(boot_id: &str, key: &UiCommandKey) -> String {
    // JSON preserves tuple boundaries, unlike a dotted qualified action string.
    let identity = serde_json::to_vec(key).expect("command identity is serializable");
    format!(
        "{COMMAND_FRAGMENT_PREFIX}{boot_id}:{:x}",
        Sha256::digest(identity)
    )
}

fn row(title: &str, action: Option<&str>, menu: Option<Node>) -> Node {
    Node::Row {
        children: vec![Node::Label {
            text: title.to_owned(),
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

pub(crate) fn projection(
    state: &AppState,
    ordinary: &[EndpointPluginUiItem],
) -> (Vec<UiCommandInfo>, Vec<EndpointPluginUiItem>) {
    if state.command_boot_id.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut commands = Vec::new();
    let mut owners: BTreeMap<String, Vec<Fragment>> = BTreeMap::new();
    let mut publish = |owner: &str, mut command: UiCommandInfo, tree: Option<Node>| {
        command.fragment_name = locator(&state.command_boot_id, &command.key);
        if let Some(tree) = tree {
            owners.entry(owner.to_owned()).or_default().push(Fragment {
                name: command.fragment_name.clone(),
                priority: 0,
                tree,
            });
        }
        commands.push(command);
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
        );
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
            let (tree, diagnostic) = match action.confirmation_fragment.as_deref() {
                None => (Some(row(&action.title, Some(&action.id), None)), None),
                Some(name) => match confirmation(ordinary, &plugin.plugin_id, &action.id, name) {
                    Ok(menu) => (Some(row(&action.title, None, Some(menu.clone()))), None),
                    Err(reason) => (
                        None,
                        Some(format!("{}/{}: {reason}", plugin.plugin_id, action.id)),
                    ),
                },
            };
            publish(
                &plugin.plugin_id,
                UiCommandInfo {
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
            );
        }
    }
    let mut rows = Vec::new();
    for (owner, fragments) in owners {
        for (chunk, fragments) in fragments.chunks(128).enumerate() {
            rows.push(EndpointPluginUiItem {
                plugin_id: owner.clone(),
                ui_id: format!("trailboss.command.v1:{chunk}"),
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
                    fragments: fragments.to_vec(),
                }),
            });
        }
    }
    (commands, rows)
}
