//! Reserved Core identities shared by native dispatch and declaration validation.
//! Descriptors describe user commands, not a new execution or registration route.

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
    Create => "host:create" [],
    Cancel => "host:cancel" [],
}
