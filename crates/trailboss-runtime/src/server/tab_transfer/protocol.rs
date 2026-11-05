use std::fmt;
use std::os::fd::OwnedFd;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::schema::{Request, TabTransferResult};
use crate::handoff_runtime::HandoffRuntimeState;
use crate::persist::TabSnapshot;
use crate::terminal::TerminalId;

pub(crate) const COORDINATION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LiveProcess {
    pub pid: u32,
    pub start: u64,
    pub pty: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RouteAdmission {
    pub terminal_id: TerminalId,
    pub process: LiveProcess,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CoreRegistration {
    pub version: u32,
    pub session: String,
    pub boot: String,
    pub pid: u32,
    pub start: u64,
    pub config_root: PathBuf,
    pub public_endpoint: PathBuf,
    pub private_endpoint: PathBuf,
    pub control_endpoint: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TransferPane {
    pub public_pane_id: String,
    pub terminal_id: TerminalId,
    pub process: LiveProcess,
    pub runtime: HandoffRuntimeState,
    #[serde(default)]
    pub seen: bool,
    #[serde(default)]
    pub right_click_passthrough: bool,
    #[serde(default)]
    pub overlay: Option<TransferOverlay>,
    #[serde(default)]
    pub plugin_pane: Option<crate::app::state::PluginPaneRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TransferOverlay {
    pub previous_focus: u32,
    pub previous_zoomed: bool,
    pub temp_files: Vec<PathBuf>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TransferPopup {
    pub state: crate::app::state::PopupPaneState,
    pub owner_pane_id: u32,
    pub cwd: PathBuf,
    pub label: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TransferBundle {
    pub source: CoreRegistration,
    pub source_workspace_id: String,
    pub source_tab_id: String,
    pub tab: TabSnapshot,
    /// Descriptor order is identical to this vector, including a popup runtime.
    pub panes: Vec<TransferPane>,
    pub popup: Option<Box<TransferPopup>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RouteTarget {
    pub previous_pane_id: String,
    pub pane_id: String,
    pub terminal_id: TerminalId,
    pub process: LiveProcess,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub(crate) enum CoreRequest {
    Inspect,
    PrepareFrontDoor,
    CommitFrontDoor,
    ResumeFrontDoor,
    PrepareSource {
        operation_id: String,
        tab_id: String,
    },
    PrepareDestination {
        operation_id: String,
        bundle: TransferBundle,
        workspace_id: Option<String>,
        workspace_label: Option<String>,
        insert_index: Option<usize>,
    },
    CommitSource {
        operation_id: String,
    },
    CommitDestination {
        operation_id: String,
        generation: u64,
    },
    Abort {
        operation_id: String,
    },
    OperationStatus {
        operation_id: String,
    },
    Forward {
        terminal_id: TerminalId,
        process: LiveProcess,
        generation: u64,
        request: Request,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub(crate) enum CoreReply {
    Inspected(CoreRegistration),
    FrontDoorReady(CoreRegistration),
    SourcePrepared(TransferBundle),
    DestinationPrepared {
        transfer: TabTransferResult,
        panes: Vec<RouteTarget>,
    },
    Acknowledged,
    OperationStatus {
        #[serde(default)]
        source_prepared: bool,
        #[serde(default)]
        destination_prepared: bool,
        #[serde(default)]
        aborted: bool,
        source_committed: bool,
        destination_committed: bool,
        generation: Option<u64>,
    },
    Response(String),
}

/// Local-only event. Descriptors never enter the public JSON request schema.
pub(crate) struct CoreEnvelope {
    pub request: CoreRequest,
    pub descriptors: Vec<OwnedFd>,
    pub respond_to: std::sync::mpsc::Sender<Result<CorePacket, String>>,
}

pub(crate) struct CorePacket {
    pub reply: CoreReply,
    pub descriptors: Vec<OwnedFd>,
}

impl fmt::Debug for CoreEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CoreEnvelope")
            .field("descriptors", &self.descriptors.len())
            .finish_non_exhaustive()
    }
}
