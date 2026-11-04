//! Session persistence — save/restore workspaces, layouts, and working directories.
//!
//! Stored at `~/.config/trailboss/session.json`.
//! Optional pane screen history is stored separately at `session-history.json`.
//! Installed plugins are persisted separately at `plugins.json`.

pub(crate) mod closed_tabs;
mod io;
pub mod plugin_registry;
mod restore;
mod snapshot;
mod writer;
pub(crate) use self::closed_tabs::ClosedTabSnapshot;
pub(crate) use self::restore::{restore_closed_workspace, RestoreRuntimeContext};
pub(crate) use self::snapshot::capture_tab;

pub use self::io::{clear_history, load, load_history};
pub use self::restore::restore;
pub(crate) use self::restore::restored_worktree_space_membership;
#[cfg(unix)]
pub use self::restore::{handoff_pane_aliases, restore_handoff};
pub use self::snapshot::{
    capture, capture_history, DirectionSnapshot, LayoutSnapshot, SessionHistorySnapshot,
    SessionSnapshot, TabSnapshot, WorkspaceSnapshot,
};
pub(crate) use self::writer::SessionWriter;
