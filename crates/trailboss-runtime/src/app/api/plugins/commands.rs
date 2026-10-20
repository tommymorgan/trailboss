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
        let (commands, _) = projection(&self.state);
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

pub(crate) fn projection(state: &AppState) -> (Vec<UiCommandInfo>, Vec<EndpointPluginUiItem>) {
    if state.command_boot_id.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut commands = Vec::new();
    let mut owners: BTreeMap<String, Vec<Fragment>> = BTreeMap::new();
    let mut publish = |owner: &str, mut command: UiCommandInfo, tree: Node| {
        command.fragment_name = locator(&state.command_boot_id, &command.key);
        owners.entry(owner.to_owned()).or_default().push(Fragment {
            name: command.fragment_name.clone(),
            priority: 0,
            tree,
        });
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
            },
            row(
                descriptor.title,
                (!matches!(descriptor.interaction, CoreCommandInteraction::Confirm))
                    .then_some(action),
                menu,
            ),
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
                },
                row(&action.title, Some(&action.id), None),
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
