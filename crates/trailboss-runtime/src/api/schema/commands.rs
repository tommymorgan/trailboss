use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CommandInvokeParams {
    /// Opaque endpoint-issued command identifier from the client-shell projection.
    pub command_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// Client-owned selection coordinates, validated against the pane's content revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<super::PaneSelectionReadParams>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiCommandRunParams {
    /// Exact visible title or stable Core / qualified plugin identity.
    /// Named moves use operation_id = "command-tab-move-" + the enclosing request ID;
    /// use a unique request ID per intended move and retain it for outcome recovery.
    pub command: String,
    /// Ordered shell-decoded answers. Pickers match exact current visible labels;
    /// Move to session consumes a second space-name answer only for Create space….
    /// Missing answers report the next question; surplus answers fail before dispatch.
    /// Raw-tail plugins consume at most one unchanged string; omission differs
    /// from an explicit empty answer.
    #[serde(default)]
    pub answers: Vec<String>,
    /// Caller pane locator resolved by Core; never replaced by global focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiCommandListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}
