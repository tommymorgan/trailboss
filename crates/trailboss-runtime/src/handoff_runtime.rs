#[cfg(unix)]
pub(crate) mod clock;
#[cfg(unix)]
use serde::{Deserialize, Serialize};

/// Long-lived pane runtime transferred during server replacement.
///
/// Handoff preserves server-owned session state such as PTYs, processes, agent
/// identity, and durable plugin/session metadata. It intentionally does not
/// preserve transient coordination such as in-flight requests, waits,
/// subscriptions, client sockets, or pane-to-pane messages; clients reconnect
/// and retry those operations after replacement.
#[cfg(unix)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HandoffRuntimeState {
    pub pane_id: u32,
    /// Old public locators owned by this live runtime, not cold persistence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_pane_aliases: Vec<String>,
    pub child_pid: u32,
    pub rows: u16,
    pub cols: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    #[serde(default)]
    pub keyboard_protocol_flags: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_protocol_ansi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_state: Option<crate::pane::InputState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_history_ansi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_snapshot: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup: Option<HandoffPopupState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<crate::terminal::TerminalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_admission: Option<crate::server::tab_transfer::RouteAdmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<crate::terminal::state::HandoffAgentState>,
    /// Ordinary OSC 133 command lifecycle at handoff time. Private internal
    /// snapshot; serde-optional so older snapshots import unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinary_command: Option<crate::terminal::state::OrdinaryCommandState>,
}

#[cfg(unix)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HandoffPopupState {
    pub tab_id: String,
    pub state: crate::app::state::PopupPaneState,
    pub cwd: std::path::PathBuf,
    pub label: Option<String>,
}

#[cfg(unix)]
impl HandoffRuntimeState {
    pub fn with_pane_id(mut self, pane_id: crate::layout::PaneId) -> Self {
        self.pane_id = pane_id.raw();
        self
    }
}

#[derive(Debug)]
pub(crate) struct ImportedHandoffRuntime {
    #[cfg(unix)]
    pub master_fd: std::os::fd::RawFd,
    #[cfg(unix)]
    pub state: HandoffRuntimeState,
}
