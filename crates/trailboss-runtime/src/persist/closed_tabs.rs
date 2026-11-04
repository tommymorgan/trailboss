use serde::{Deserialize, Serialize};

use super::WorkspaceSnapshot;

pub(crate) const CLOSED_TAB_LIMIT: usize = 50;

/// Structural recovery data only. Terminal output and arbitrary commands are excluded.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ClosedTabSnapshot {
    pub id: String,
    pub label: String,
    pub workspace_label: String,
    pub closed_at: u64,
    pub workspace: WorkspaceSnapshot,
}

impl ClosedTabSnapshot {
    pub(crate) fn info(&self) -> crate::api::schema::ClosedTabInfo {
        crate::api::schema::ClosedTabInfo {
            closed_tab_id: self.id.clone(),
            label: self.label.clone(),
            workspace_id: self.workspace.id.clone().unwrap_or_default(),
            workspace_label: self.workspace_label.clone(),
            closed_at: self.closed_at,
            pane_count: self.workspace.tabs.iter().map(|tab| tab.panes.len()).sum(),
        }
    }
}
