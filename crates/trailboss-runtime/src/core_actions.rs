//! Reserved Core identities shared by native dispatch and declaration validation.
//! Descriptors describe user commands, not a new execution or registration route.

/// Core-issued command fragments are locators, never producer override hooks.
pub const COMMAND_FRAGMENT_PREFIX: &str = "trailboss.command.v1:";
pub const RECENTLY_CLOSED_FRAGMENT: &str = "trailboss.command.v1:recently-closed-tabs";

/// Resource policy for a user command. Native-scope and client-local commands
/// must not inherit a resource menu's captured target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreCommandTarget {
    Global,
    NativeScope,
    Workspace,
    Tab,
    ClientLocal,
}

/// The two existing meanings of `host:focus`; previous visits are client-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreFocusMode {
    Targeted,
    Previous,
}

/// Existing ordinary menu interaction, distinct from immediate stock invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreCommandInteraction {
    Immediate,
    Name { submit_label: &'static str },
    Confirm,
    Picker,
}

/// User-facing metadata. Interaction controls intentionally have no descriptor.
/// These Rust values do not add fields to the generation-1 wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreCommandDescriptor {
    pub action: CoreAction,
    pub title: &'static str,
    pub target: CoreCommandTarget,
    pub focus_mode: Option<CoreFocusMode>,
    pub interaction: CoreCommandInteraction,
}

// Each reserved spelling and its user-command modes live in one definition.
// In particular, focus is one dispatch identity with two user-facing modes.
macro_rules! core_actions {
    ($($variant:ident => $id:literal [$($title:literal, $target:ident, $mode:expr, $interaction:expr);* $(;)?]),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum CoreAction {
            $($variant),*
        }

        impl CoreAction {
            /// Parse only accepted reserved IDs; unknown IDs retain plugin fallback.
            pub fn parse(id: &str) -> Option<Self> {
                match id {
                    $($id => Some(Self::$variant),)*
                    id if id.strip_prefix("tab.closed.restore:").is_some_and(|id| !id.is_empty()) => Some(Self::RestoreClosedTab),
                    _ => None,
                }
            }

            pub const fn id(self) -> &'static str {
                match self {
                    $(Self::$variant => $id,)*
                }
            }

            /// All reserved identities, including the submit/cancel controls.
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];
        }

        /// User commands derived from the same reserved-action definition.
        /// No endpoint or declaration publication is implied by this inventory.
        pub const CORE_COMMANDS: &[CoreCommandDescriptor] = &[
            $($(CoreCommandDescriptor {
                action: CoreAction::$variant,
                title: $title,
                target: CoreCommandTarget::$target,
                focus_mode: $mode,
                interaction: $interaction,
            },)*)*
        ];
    };
}

core_actions! {
    NewSpace => "host:new-space" [
        "New space", Global, None, CoreCommandInteraction::Name { submit_label: "Create" }
    ],
    New => "host:new" [
        "New tab or space", NativeScope, None, CoreCommandInteraction::Name { submit_label: "Create" }
    ],
    NewTab => "host:new-tab" [
        "New tab", Workspace, None, CoreCommandInteraction::Name { submit_label: "Create" }
    ],
    RenameSpace => "host:rename-space" [
        "Rename space", Workspace, None, CoreCommandInteraction::Name { submit_label: "Save" }
    ],
    RenameTab => "host:rename-tab" [
        "Rename tab", Tab, None, CoreCommandInteraction::Name { submit_label: "Save" }
    ],
    CloseTab => "host:close-tab" [
        "Close tab", Tab, None, CoreCommandInteraction::Confirm
    ],
    Focus => "host:focus" [
        "Focus tab", Tab, Some(CoreFocusMode::Targeted), CoreCommandInteraction::Immediate;
        "Previous tab", ClientLocal, Some(CoreFocusMode::Previous), CoreCommandInteraction::Immediate
    ],
    RestartIdleAgents => "host:restart-idle-agents" [
        "Restart idle agents", Global, None, CoreCommandInteraction::Immediate
    ],
    Detach => "host:detach" [
        "Detach from session", ClientLocal, None, CoreCommandInteraction::Immediate
    ],
    SwitchSession => "host:switch-session" [
        "Switch session", ClientLocal, None, CoreCommandInteraction::Immediate
    ],
    MoveToSession => "host:move-to-session" [
        "Move to session…", Tab, None, CoreCommandInteraction::Immediate
    ],
    RecentlyClosedTabs => "host:recently-closed-tabs" [
        "Recently closed tabs", Global, None, CoreCommandInteraction::Picker
    ],
    RestoreClosedTab => "tab.closed.restore" [],
    Create => "host:create" [],
    Cancel => "host:cancel" [],
}

/// Named palette preparation shared by native submission and the API adapter.
/// Labels are validated for blankness, never normalized.
pub fn prepare_named(
    action: CoreAction,
    workspace: Option<&str>,
    tab: Option<&str>,
    answer: Option<&str>,
) -> Result<(&'static str, serde_json::Value), (&'static str, &'static str)> {
    use serde_json::json;
    let prompt = action.name_prompt();
    if let Some(prompt) = prompt {
        let Some(name) = answer else {
            return Err(("missing_answer", prompt));
        };
        if name.trim().is_empty() {
            return Err(("invalid_name", "invalid name: enter a nonblank name"));
        }
    }
    Ok(match action {
        CoreAction::NewSpace => ("workspace.create", json!({"label":answer,"focus":true})),
        CoreAction::New | CoreAction::NewTab => match workspace {
            Some(workspace) => (
                "tab.create",
                json!({"workspace_id":workspace,"label":answer,"focus":true}),
            ),
            None => ("workspace.create", json!({"label":answer,"focus":true})),
        },
        CoreAction::RenameSpace => (
            "workspace.rename",
            json!({"workspace_id":workspace,"label":answer}),
        ),
        CoreAction::RenameTab => ("tab.rename", json!({"tab_id":tab,"label":answer})),
        CoreAction::CloseTab => ("tab.close", json!({"tab_id":tab})),
        CoreAction::Focus => ("tab.focus", json!({"tab_id":tab})),
        CoreAction::RestartIdleAgents => ("agent.restart_idle", json!({})),
        _ => return Err(("command_not_found", "command not found")),
    })
}

impl CoreAction {
    pub fn name_prompt(self) -> Option<&'static str> {
        match self {
            Self::NewSpace => Some("New space name"),
            Self::New | Self::NewTab => Some("New tab name"),
            Self::RenameSpace => Some("Rename space"),
            Self::RenameTab => Some("Rename tab"),
            _ => None,
        }
    }
}

impl CoreCommandDescriptor {
    pub fn runnable_by_name(&self) -> bool {
        self.target != CoreCommandTarget::ClientLocal
            && self.action != CoreAction::MoveToSession
            && self.interaction != CoreCommandInteraction::Picker
    }
}

pub fn command_descriptor(
    key: &crate::api::schema::UiCommandKey,
) -> Option<&'static CoreCommandDescriptor> {
    let crate::api::schema::UiCommandKey::Core { action_id, mode } = key else {
        return None;
    };
    CORE_COMMANDS.iter().find(|descriptor| {
        descriptor.action.id() == action_id
            && descriptor.focus_mode.map(|focus| match focus {
                CoreFocusMode::Targeted => "targeted",
                CoreFocusMode::Previous => "previous",
            }) == mode.as_deref()
    })
}
