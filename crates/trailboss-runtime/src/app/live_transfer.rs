use super::App;
use crate::layout::PaneId;
use crate::server::tab_transfer::protocol::*;
use crate::terminal::{TerminalId, TerminalRuntime, TerminalState};
use std::collections::{HashMap, HashSet};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};

#[derive(Default)]
pub(crate) struct LiveTransfers {
    sources: HashMap<String, TransferBundle>,
    destinations: HashMap<String, Destination>,
    committed: HashSet<String>,
    aborted: HashSet<String>,
    destination_commits: HashMap<String, u64>,
    pub(crate) moved_panes: HashSet<PaneId>,
    generations: HashMap<TerminalId, (u64, LiveProcess)>,
    pub(crate) parked_popups: HashMap<String, super::state::PopupPaneState>,
}
struct Destination {
    workspace: String,
    new_workspace: Option<crate::workspace::Workspace>,
    tab: crate::workspace::Tab,
    terminals: Vec<TerminalState>,
    runtimes: HashMap<TerminalId, TerminalRuntime>,
    index: usize,
    transfer: crate::api::schema::TabTransferResult,
    routes: Vec<RouteTarget>,
    pane_numbers: HashMap<PaneId, usize>,
    popup: Option<super::state::PopupPaneState>,
    overlays: HashMap<PaneId, super::OverlayPaneState>,
    plugin_panes: HashMap<PaneId, super::state::PluginPaneRecord>,
    fingerprint: Vec<u8>,
}
impl App {
    pub(crate) fn has_pending_live_transfer(&self) -> bool {
        !self.live_transfers.destinations.is_empty()
            || self
                .live_transfers
                .sources
                .keys()
                .any(|operation| !self.live_transfers.committed.contains(operation))
    }
    pub(crate) fn live_route_admission(&self, id: &TerminalId) -> Option<RouteAdmission> {
        self.live_transfers
            .generations
            .get(id)
            .map(|(generation, process)| RouteAdmission {
                terminal_id: id.clone(),
                generation: *generation,
                process: process.clone(),
            })
    }
    pub(crate) fn restore_live_route_admissions(&mut self, admissions: Vec<RouteAdmission>) {
        for admission in admissions {
            self.live_transfers.generations.insert(
                admission.terminal_id,
                (admission.generation, admission.process),
            );
        }
    }
    pub(crate) fn owned_live_routes(&self) -> Vec<(RouteTarget, u64)> {
        self.live_transfers
            .generations
            .iter()
            .filter_map(|(id, (generation, process))| {
                let pane_id = self
                    .state
                    .workspaces
                    .iter()
                    .enumerate()
                    .find_map(|(ws, space)| {
                        space.tabs.iter().find_map(|tab| {
                            tab.panes.iter().find_map(|(pane, state)| {
                                (&state.attached_terminal_id == id)
                                    .then(|| self.public_pane_id(ws, *pane))
                                    .flatten()
                            })
                        })
                    })
                    .or_else(|| {
                        self.live_transfers
                            .parked_popups
                            .values()
                            .find(|popup| &popup.terminal_id == id)
                            .map(|popup| popup.pane_id.raw().to_string())
                    })
                    .or_else(|| {
                        self.state
                            .popup_pane
                            .as_ref()
                            .filter(|popup| &popup.terminal_id == id)
                            .map(|popup| popup.pane_id.raw().to_string())
                    })?;
                Some((
                    RouteTarget {
                        previous_pane_id: pane_id.clone(),
                        pane_id,
                        terminal_id: id.clone(),
                        process: process.clone(),
                    },
                    *generation,
                ))
            })
            .collect()
    }
    pub(crate) fn restore_live_handoff_popups(
        &mut self,
        imports: Vec<crate::handoff_runtime::ImportedHandoffRuntime>,
    ) -> std::io::Result<()> {
        for import in imports {
            let popup = import
                .state
                .popup
                .clone()
                .ok_or_else(|| std::io::Error::other("popup handoff metadata missing"))?;
            if self.parse_tab_id(&popup.tab_id).is_none() {
                return Err(std::io::Error::other(
                    "popup owner tab missing from handoff",
                ));
            }
            let mut state = popup.state;
            state.pane_id = PaneId::alloc();
            let agent = import.state.agent_state.clone();
            let ordinary = import.state.ordinary_command;
            let runtime = TerminalRuntime::from_handoff_fd(
                crate::handoff_runtime::ImportedHandoffRuntime {
                    master_fd: import.master_fd,
                    state: import.state.with_pane_id(state.pane_id),
                },
                self.state.pane_scrollback_limit_bytes,
                crate::terminal_theme::TerminalTheme::default(),
                None,
                self.event_tx.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            )?;
            let mut terminal = TerminalState::new(state.terminal_id.clone(), popup.cwd);
            if let Some(label) = popup.label {
                terminal.set_manual_label(label);
            }
            if let Some(agent) = agent {
                terminal.restore_handoff_agent_state(agent);
            }
            if let Some(ordinary) = ordinary {
                terminal.restore_handoff_ordinary_command_state(ordinary);
            }
            self.state
                .terminals
                .insert(state.terminal_id.clone(), terminal);
            self.terminal_runtimes
                .insert(state.terminal_id.clone(), runtime);
            self.live_transfers
                .parked_popups
                .insert(popup.tab_id, state);
        }
        Ok(())
    }
    fn reconcile_live_transfer_overlays(&mut self) {
        for (pane, overlay) in &mut self.overlay_panes {
            if let Some((ws, tab)) = self
                .state
                .workspaces
                .iter()
                .enumerate()
                .find_map(|(ws, space)| space.find_tab_index_for_pane(*pane).map(|tab| (ws, tab)))
            {
                overlay.ws_idx = ws;
                overlay.tab_idx = tab;
            }
        }
    }
    pub(crate) fn preserve_uncertain_live_sources_on_shutdown(&mut self) {
        let operations = self
            .live_transfers
            .sources
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for operation in operations {
            if self.live_transfers.committed.contains(&operation) {
                continue;
            }
            let safe_to_close = crate::server::tab_transfer::stored_transfer_status(&operation)
                .is_ok_and(|status| {
                    matches!(
                        status.phase,
                        crate::api::schema::TabTransferPhase::Aborted
                            | crate::api::schema::TabTransferPhase::Preparing
                    )
                });
            if !safe_to_close {
                let _ = self.commit_live_source(&operation);
            }
        }
    }
    pub(crate) fn live_transfer_reserves_focused_tab(&self) -> bool {
        self.state
            .active
            .and_then(|ws| self.public_tab_id(ws, self.state.workspaces[ws].active_tab))
            .is_some_and(|tab| {
                self.live_transfers
                    .sources
                    .iter()
                    .any(|(operation, source)| {
                        source.source_tab_id == tab
                            && !self.live_transfers.committed.contains(operation)
                    })
            })
    }
    pub(crate) fn live_transfer_reserves_request(
        &self,
        request: &crate::api::schema::Request,
    ) -> bool {
        if !self.has_pending_live_transfer() {
            return false;
        }
        let Ok(value) = serde_json::to_value(request) else {
            return true;
        };
        let method = value["method"].as_str().unwrap_or("");
        if method.ends_with(".get")
            || method.ends_with(".list")
            || method.ends_with(".read")
            || method == "session.snapshot"
            || method == "tab.transfer.status"
            || method == "ping"
        {
            return false;
        }
        let params = &value["params"];
        for (operation, source) in &self.live_transfers.sources {
            if self.live_transfers.committed.contains(operation) {
                continue;
            }
            if params["tab_id"].as_str() == Some(&source.source_tab_id)
                || params["workspace_id"].as_str() == Some(&source.source_workspace_id)
            {
                return true;
            }
            for key in [
                "pane_id",
                "target_pane_id",
                "source_pane_id",
                "destination_pane_id",
                "target",
            ] {
                if let Some(target) = params[key].as_str() {
                    let pane_id = self
                        .parse_pane_id(target)
                        .map(|(_, pane)| pane)
                        .or_else(|| {
                            self.resolve_agent_target(target)
                                .ok()
                                .map(|target| target.pane_id)
                        });
                    if pane_id.is_some_and(|id| {
                        source
                            .panes
                            .iter()
                            .any(|pane| pane.runtime.pane_id == id.raw())
                    }) {
                        return true;
                    }
                }
            }
            let terminal_target = params.get("terminal_id").or_else(|| {
                params
                    .get("binding")
                    .and_then(|binding| binding.get("terminal_id"))
            });
            if terminal_target.is_some_and(|target| {
                source.panes.iter().any(|pane| {
                    serde_json::to_value(&pane.terminal_id).is_ok_and(|id| &id == target)
                })
            }) {
                return true;
            }
            if let Some(id) = params["pane_id"].as_str() {
                if source.panes.iter().any(|pane| pane.public_pane_id == id) {
                    return true;
                }
            } else if ![
                "tab_id",
                "workspace_id",
                "pane_id",
                "target_pane_id",
                "source_pane_id",
                "destination_pane_id",
                "target",
                "terminal_id",
                "binding",
            ]
            .iter()
            .any(|key| params.get(*key).is_some_and(|value| !value.is_null()))
                && self.live_transfer_reserves_focused_tab()
            {
                return true;
            }
        }
        if !matches!(
            method,
            "workspace.close"
                | "tab.create"
                | "tab.move"
                | "tab.transfer"
                | "tab.close"
                | "pane.split"
                | "pane.swap"
                | "pane.move"
                | "pane.close"
        ) {
            return false;
        }
        let reserved_workspace = |ws| {
            self.live_transfers
                .destinations
                .values()
                .any(|destination| self.parse_workspace_id(&destination.workspace) == Some(ws))
        };
        if params["workspace_id"]
            .as_str()
            .is_some_and(|id| self.parse_workspace_id(id).is_some_and(reserved_workspace))
        {
            return true;
        }
        if params["tab_id"].as_str().is_some_and(|id| {
            self.parse_tab_id(id)
                .is_some_and(|(ws, _)| reserved_workspace(ws))
        }) {
            return true;
        }
        for key in [
            "pane_id",
            "target_pane_id",
            "source_pane_id",
            "destination_pane_id",
            "target",
        ] {
            if let Some(target) = params[key].as_str() {
                let workspace = self.parse_pane_id(target).map(|(ws, _)| ws).or_else(|| {
                    self.resolve_agent_target(target)
                        .ok()
                        .and_then(|target| self.find_pane(target.pane_id).map(|(ws, _)| ws))
                });
                if workspace.is_some_and(reserved_workspace) {
                    return true;
                }
            }
        }
        ![
            "workspace_id",
            "tab_id",
            "pane_id",
            "target_pane_id",
            "source_pane_id",
            "destination_pane_id",
            "target",
        ]
        .iter()
        .any(|key| params.get(*key).is_some_and(|value| !value.is_null()))
            && self.state.active.is_some_and(reserved_workspace)
    }
    pub(crate) fn canonical_forward_request(
        &self,
        id: &TerminalId,
        request: crate::api::schema::Request,
    ) -> Result<crate::api::schema::Request, String> {
        let (ws, pane) = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .find_map(|(ws, space)| {
                space.tabs.iter().find_map(|tab| {
                    tab.panes.iter().find_map(|(pane, state)| {
                        (&state.attached_terminal_id == id).then_some((ws, *pane))
                    })
                })
            })
            .ok_or("terminal is detached")?;
        let canonical = self
            .public_pane_id(ws, pane)
            .ok_or("terminal locator missing")?;
        let mut value = serde_json::to_value(request).map_err(|e| e.to_string())?;
        let params = value["params"]
            .as_object_mut()
            .ok_or("forwarded request has no explicit pane target")?;
        if let Some(terminal) = params.get("terminal_id").or_else(|| {
            params
                .get("binding")
                .and_then(|binding| binding.get("terminal_id"))
        }) {
            let expected = serde_json::to_value(id).map_err(|e| e.to_string())?;
            if terminal != &expected {
                return Err("forwarded terminal binding mismatch".into());
            }
        } else if params.contains_key("pane_id") {
            params.insert("pane_id".into(), canonical.into());
        } else if params.contains_key("caller_pane_id") {
            params.insert("caller_pane_id".into(), canonical.into());
        } else if params.contains_key("target") {
            params.insert("target".into(), canonical.into());
        } else if params.contains_key("target_pane_id") {
            params.insert("target_pane_id".into(), canonical.into());
        } else {
            return Err("forwarding requires an explicit terminal target".into());
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
    pub(crate) fn live_transfer_operation_status(&self, operation: &str) -> CoreReply {
        CoreReply::OperationStatus {
            source_prepared: self.live_transfers.sources.contains_key(operation),
            destination_prepared: self.live_transfers.destinations.contains_key(operation),
            aborted: self.live_transfers.aborted.contains(operation),
            source_committed: self.live_transfers.committed.contains(operation)
                && self.live_transfers.sources.contains_key(operation),
            destination_committed: self
                .live_transfers
                .destination_commits
                .contains_key(operation),
            generation: self
                .live_transfers
                .destination_commits
                .get(operation)
                .copied(),
        }
    }
    pub(crate) fn prepare_live_source(
        &mut self,
        operation: &str,
        tab_id: &str,
        source: CoreRegistration,
        include_popup: bool,
    ) -> Result<(TransferBundle, Vec<OwnedFd>), String> {
        if self.live_transfers.aborted.contains(operation) {
            return Err("operation already aborted".into());
        }
        if self.live_transfers.committed.contains(operation) {
            return Err("operation already committed".into());
        }
        if let Some(bundle) = self.live_transfers.sources.get(operation) {
            if bundle.source_tab_id != tab_id {
                return Err("operation identity conflict".into());
            }
            let mut fds = Vec::new();
            for pane in &bundle.panes {
                let runtime = self
                    .terminal_runtimes
                    .get(&pane.terminal_id)
                    .ok_or("reserved runtime disappeared")?;
                fds.push(unsafe {
                    OwnedFd::from_raw_fd(runtime.duplicate_handoff_fd().map_err(|e| e.to_string())?)
                });
            }
            return Ok((bundle.clone(), fds));
        }
        if self
            .live_transfers
            .sources
            .values()
            .any(|bundle| bundle.source_tab_id == tab_id)
        {
            return Err("tab is reserved by another operation".into());
        }
        let (ws, tab) = self.parse_tab_id(tab_id).ok_or("tab not found")?;
        let snapshot = crate::persist::capture_tab(
            &self.state.workspaces[ws].tabs[tab],
            &self.state.terminals,
            &self.terminal_runtimes,
        );
        let mut selected: Vec<_> = self.state.workspaces[ws].tabs[tab]
            .panes
            .iter()
            .map(|(id, pane)| (*id, pane.attached_terminal_id.clone()))
            .collect();
        let selected_popup = if include_popup {
            self.state.popup_pane.clone()
        } else {
            self.live_transfers.parked_popups.get(tab_id).cloned()
        };
        let popup = selected_popup.map(|state| {
            let metadata = self.state.terminals.get(&state.terminal_id);
            Box::new(TransferPopup {
                cwd: metadata
                    .map(|terminal| terminal.cwd.clone())
                    .unwrap_or_default(),
                label: metadata.and_then(|terminal| terminal.manual_label.clone()),
                owner_pane_id: self.state.workspaces[ws].tabs[tab].root_pane.raw(),
                state,
            })
        });
        if let Some(popup) = &popup {
            selected.push((popup.state.pane_id, popup.state.terminal_id.clone()));
        }
        let mut paused = Vec::new();
        let result = (|| {
            let mut panes = Vec::new();
            let mut fds = Vec::new();
            for (pane_id, terminal_id) in selected {
                let runtime = self
                    .terminal_runtimes
                    .get(&terminal_id)
                    .ok_or("pane has no live runtime")?;
                runtime
                    .pause_handoff_reader(std::time::Duration::from_secs(3))
                    .map_err(|e| e.to_string())?;
                paused.push(terminal_id.clone());
                let fd = unsafe {
                    OwnedFd::from_raw_fd(runtime.duplicate_handoff_fd().map_err(|e| e.to_string())?)
                };
                let pid = runtime.child_pid().ok_or("shell has exited")?;
                let process = crate::server::tab_transfer::live_process(pid, fd.as_raw_fd())
                    .map_err(|e| e.to_string())?;
                let mut state = runtime.handoff_runtime_state(pane_id.raw());
                state.terminal_snapshot = Some(
                    runtime
                        .complete_terminal_snapshot()
                        .map_err(|e| e.to_string())?,
                );
                let terminal = self
                    .state
                    .terminals
                    .get(&terminal_id)
                    .ok_or("terminal metadata missing")?;
                state.agent_state = terminal.handoff_agent_state();
                state.ordinary_command = terminal.handoff_ordinary_command_state();
                state.public_pane_aliases = self
                    .state
                    .public_pane_id_aliases
                    .iter()
                    .filter(|(_, target)| **target == pane_id)
                    .map(|(alias, _)| alias.clone())
                    .collect();
                #[cfg(target_os = "linux")]
                if let Some(alias) = crate::platform::inherited_pane_id_for_pty(
                    pid,
                    fd.as_raw_fd(),
                    &source.public_endpoint,
                ) {
                    if !state.public_pane_aliases.contains(&alias) {
                        state.public_pane_aliases.push(alias);
                    }
                }
                let placement = self.find_pane(pane_id).map(|(_, pane)| pane);
                let overlay = self
                    .overlay_panes
                    .get(&pane_id)
                    .map(|overlay| TransferOverlay {
                        previous_focus: overlay.previous_focus.raw(),
                        previous_zoomed: overlay.previous_zoomed,
                        temp_files: overlay.temp_files.clone(),
                    });
                panes.push(TransferPane {
                    public_pane_id: self
                        .public_pane_id(ws, pane_id)
                        .unwrap_or_else(|| pane_id.raw().to_string()),
                    terminal_id,
                    process,
                    runtime: state,
                    seen: placement.is_none_or(|pane| pane.seen),
                    right_click_passthrough: placement
                        .is_some_and(|pane| pane.right_click_passthrough),
                    overlay,
                    plugin_pane: self.state.plugin_panes.get(&pane_id).cloned(),
                });
                fds.push(fd);
            }
            Ok((
                TransferBundle {
                    source,
                    source_workspace_id: self.public_workspace_id(ws),
                    source_tab_id: tab_id.into(),
                    tab: snapshot,
                    panes,
                    popup,
                },
                fds,
            ))
        })();
        match result {
            Ok((bundle, fds)) => {
                self.live_transfers
                    .sources
                    .insert(operation.into(), bundle.clone());
                Ok((bundle, fds))
            }
            Err(error) => {
                for id in paused {
                    if let Some(runtime) = self.terminal_runtimes.get(&id) {
                        runtime.set_handoff_reader_paused(false);
                    }
                }
                Err(error)
            }
        }
    }

    pub(crate) fn prepare_live_destination(
        &mut self,
        operation: &str,
        bundle: TransferBundle,
        descriptors: Vec<OwnedFd>,
        workspace_id: Option<String>,
        workspace_label: Option<String>,
        index: Option<usize>,
    ) -> Result<(crate::api::schema::TabTransferResult, Vec<RouteTarget>), String> {
        if self.live_transfers.aborted.contains(operation) {
            return Err("operation already aborted".into());
        }
        if self.live_transfers.committed.contains(operation) {
            return Err("operation already committed".into());
        }
        let fingerprint = serde_json::to_vec(&(
            &bundle.source.boot,
            &bundle.source_tab_id,
            &workspace_id,
            &workspace_label,
            index,
            bundle
                .panes
                .iter()
                .map(|pane| (&pane.terminal_id, &pane.process))
                .collect::<Vec<_>>(),
        ))
        .map_err(|error| error.to_string())?;
        if workspace_id.is_some() == workspace_label.is_some() {
            return Err("select exactly one destination workspace or label".into());
        }
        if let Some(destination) = self.live_transfers.destinations.get(operation) {
            return if destination.fingerprint == fingerprint {
                Ok((destination.transfer.clone(), destination.routes.clone()))
            } else {
                Err("operation identity conflict".into())
            };
        }
        if descriptors.len() != bundle.panes.len() {
            return Err("runtime descriptor count mismatch".into());
        }
        if bundle
            .panes
            .iter()
            .any(|pane| self.state.terminals.contains_key(&pane.terminal_id))
        {
            return Err("terminal already belongs to destination".into());
        }
        let workspace = match workspace_id {
            Some(id) => {
                self.parse_workspace_id(&id)
                    .ok_or("destination workspace not found")?;
                id
            }
            None if workspace_label.is_some() => crate::workspace::generate_workspace_id(),
            None => return Err("destination workspace or explicit label required".into()),
        };
        let count = self
            .parse_workspace_id(&workspace)
            .map(|ws| self.state.workspaces[ws].tabs.len())
            .unwrap_or(0);
        let index = index.unwrap_or(count);
        if index > count {
            return Err("destination index out of bounds".into());
        }
        if bundle
            .panes
            .iter()
            .map(|pane| pane.runtime.pane_id)
            .collect::<HashSet<_>>()
            .len()
            != bundle.panes.len()
            || bundle
                .panes
                .iter()
                .map(|pane| &pane.terminal_id)
                .collect::<HashSet<_>>()
                .len()
                != bundle.panes.len()
        {
            return Err("duplicate transfer runtime identity".into());
        }
        for (pane, fd) in bundle.panes.iter().zip(&descriptors) {
            if crate::server::tab_transfer::live_process(pane.process.pid, fd.as_raw_fd())
                .map_err(|e| e.to_string())?
                != pane.process
            {
                return Err("live process identity changed".into());
            }
        }
        let mut imports = HashMap::new();
        let mut identities = HashMap::new();
        for (pane, fd) in bundle.panes.iter().zip(descriptors) {
            identities.insert(pane.runtime.pane_id, pane.terminal_id.clone());
            imports.insert(
                pane.runtime.pane_id,
                crate::handoff_runtime::ImportedHandoffRuntime {
                    master_fd: fd.into_raw_fd(),
                    state: pane.runtime.clone(),
                },
            );
        }
        let context = crate::persist::RestoreRuntimeContext {
            scrollback_limit_bytes: self.state.pane_scrollback_limit_bytes,
            shell_config: crate::pane::PaneShellConfig::new(
                "",
                crate::config::ShellModeConfig::NonLogin,
            ),
            resume_agents_on_restore: false,
            events: self.event_tx.clone(),
            render_notify: self.render_notify.clone(),
            render_dirty: self.render_dirty.clone(),
        };
        let restored =
            crate::persist::restore_transfer_tab(&bundle.tab, &context, &mut imports, &identities);
        let (mut tab, mut terminals, mut runtimes, mapping) = match restored {
            Ok(restored) => restored,
            Err(error) => {
                for import in imports.into_values() {
                    drop(unsafe { OwnedFd::from_raw_fd(import.master_fd) });
                }
                return Err(error);
            }
        };
        for (new_id, old_id) in &mapping {
            if let Some(source) = bundle
                .panes
                .iter()
                .find(|pane| pane.runtime.pane_id == *old_id)
            {
                let pane = tab
                    .panes
                    .get_mut(new_id)
                    .ok_or("restored placement missing")?;
                pane.seen = source.seen;
                pane.right_click_passthrough = source.right_click_passthrough;
            }
        }
        let mut overlays = HashMap::new();
        let mut plugin_panes = HashMap::new();
        for (new_id, old_id) in &mapping {
            let source = bundle
                .panes
                .iter()
                .find(|pane| pane.runtime.pane_id == *old_id)
                .ok_or("source placement missing")?;
            if let Some(overlay) = &source.overlay {
                let previous_focus = mapping
                    .iter()
                    .find_map(|(new, old)| (*old == overlay.previous_focus).then_some(*new))
                    .ok_or("overlay owner missing from transfer")?;
                overlays.insert(
                    *new_id,
                    super::OverlayPaneState {
                        ws_idx: 0,
                        tab_idx: 0,
                        previous_focus,
                        previous_zoomed: overlay.previous_zoomed,
                        temp_files: overlay.temp_files.clone(),
                    },
                );
            }
            if let Some(plugin) = &source.plugin_pane {
                plugin_panes.insert(*new_id, plugin.clone());
            }
        }
        let popup = if let Some(popup) = &bundle.popup {
            let mut state = popup.state.clone();
            state.pane_id = PaneId::alloc();
            let import = imports
                .remove(&popup.state.pane_id.raw())
                .ok_or("missing popup runtime")?;
            let agent = import.state.agent_state.clone();
            let ordinary = import.state.ordinary_command;
            let runtime = TerminalRuntime::from_handoff_fd(
                crate::handoff_runtime::ImportedHandoffRuntime {
                    master_fd: import.master_fd,
                    state: import.state.with_pane_id(state.pane_id),
                },
                context.scrollback_limit_bytes,
                crate::terminal_theme::TerminalTheme::default(),
                None,
                self.event_tx.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            )
            .map_err(|e| e.to_string())?;
            let mut terminal = TerminalState::new(state.terminal_id.clone(), popup.cwd.clone());
            if let Some(label) = &popup.label {
                terminal.set_manual_label(label.clone());
            }
            if let Some(agent) = agent {
                terminal.restore_handoff_agent_state(agent);
            }
            if let Some(ordinary) = ordinary {
                terminal.restore_handoff_ordinary_command_state(ordinary);
            }
            runtimes.insert(state.terminal_id.clone(), runtime);
            terminals.push(terminal);
            Some(state)
        } else {
            None
        };
        let cwd = terminals.first().ok_or("empty transfer")?.cwd.clone();
        let new_workspace = if self.parse_workspace_id(&workspace).is_none() {
            Some(crate::workspace::Workspace {
                id: workspace.clone(),
                custom_name: workspace_label,
                identity_cwd: cwd.clone(),
                cached_identity_cwd: cwd.clone(),
                cached_auto_label: String::new(),
                cached_git_status_key: cwd,
                cached_git_branch: None,
                cached_git_ahead_behind: None,
                cached_git_space: None,
                worktree_space: None,
                metadata_tokens: Default::default(),
                metadata_token_sequences: HashMap::new(),
                public_pane_numbers: HashMap::new(),
                next_public_pane_number: 1,
                next_public_tab_number: 1,
                tabs: Vec::new(),
                active_tab: 0,
                #[cfg(test)]
                test_runtimes: HashMap::new(),
            })
        } else {
            None
        };
        let mut preview = new_workspace;
        let ws_index = self
            .parse_workspace_id(&workspace)
            .unwrap_or(self.state.workspaces.len());
        let is_new = self.parse_workspace_id(&workspace).is_none();
        if let Some(ws) = preview.take() {
            self.state.workspaces.push(ws);
        }
        self.state.workspaces[ws_index].insert_transferred_tab(tab, index);
        for terminal in terminals {
            self.state.terminals.insert(terminal.id.clone(), terminal);
        }
        let mut routes = mapping
            .iter()
            .map(|(new, old)| {
                let pane = bundle
                    .panes
                    .iter()
                    .find(|pane| pane.runtime.pane_id == *old)
                    .unwrap();
                RouteTarget {
                    previous_pane_id: pane.public_pane_id.clone(),
                    pane_id: self.public_pane_id(ws_index, *new).unwrap(),
                    terminal_id: pane.terminal_id.clone(),
                    process: pane.process.clone(),
                }
            })
            .collect::<Vec<_>>();
        if let Some(state) = &popup {
            let pane = bundle
                .panes
                .iter()
                .find(|pane| pane.terminal_id == state.terminal_id)
                .unwrap();
            routes.push(RouteTarget {
                previous_pane_id: pane.public_pane_id.clone(),
                pane_id: state.pane_id.raw().to_string(),
                terminal_id: state.terminal_id.clone(),
                process: pane.process.clone(),
            });
        }
        let transfer = crate::api::schema::TabTransferResult {
            changed: true,
            previous_tab_id: bundle.source_tab_id,
            previous_workspace_id: bundle.source_workspace_id,
            tab: self.tab_info(ws_index, index).unwrap(),
            index,
            panes: routes
                .iter()
                .map(|route| crate::api::schema::TabTransferPane {
                    previous_pane_id: route.previous_pane_id.clone(),
                    pane_id: route.pane_id.clone(),
                })
                .collect(),
            closed_workspace_id: None,
        };
        let mut inherited_routes = Vec::new();
        for pane in &bundle.panes {
            if let Some(current) = routes
                .iter()
                .find(|route| route.terminal_id == pane.terminal_id)
            {
                for alias in &pane.runtime.public_pane_aliases {
                    if alias != &current.previous_pane_id {
                        let mut route = current.clone();
                        route.previous_pane_id = alias.clone();
                        inherited_routes.push(route);
                    }
                }
            }
        }
        routes.extend(inherited_routes);
        let pane_numbers = self.state.workspaces[ws_index].tabs[index]
            .panes
            .keys()
            .map(|id| (*id, self.state.workspaces[ws_index].public_pane_numbers[id]))
            .collect();
        let tab = self.state.workspaces[ws_index].take_tab_for_transfer(index);
        let mut terminals: Vec<_> = tab
            .panes
            .values()
            .filter_map(|pane| self.state.terminals.remove(&pane.attached_terminal_id))
            .collect();
        if let Some(popup) = &popup {
            if let Some(terminal) = self.state.terminals.remove(&popup.terminal_id) {
                terminals.push(terminal);
            }
        }
        let new_workspace = if is_new {
            Some(self.state.workspaces.remove(ws_index))
        } else {
            None
        };
        self.live_transfers.destinations.insert(
            operation.into(),
            Destination {
                workspace,
                new_workspace,
                tab,
                terminals,
                runtimes,
                index,
                transfer: transfer.clone(),
                routes: routes.clone(),
                pane_numbers,
                popup,
                overlays,
                plugin_panes,
                fingerprint,
            },
        );
        Ok((transfer, routes))
    }
    pub(crate) fn commit_live_source(&mut self, operation: &str) -> Result<(), String> {
        if self.live_transfers.committed.contains(operation) {
            return Ok(());
        }
        let bundle = self
            .live_transfers
            .sources
            .get(operation)
            .ok_or("source operation not prepared")?
            .clone();
        let (ws, tab) = self
            .parse_tab_id(&bundle.source_tab_id)
            .ok_or("reserved source tab disappeared")?;
        let tab = self.state.workspaces[ws].take_tab_for_transfer(tab);
        if let Some(popup) = &bundle.popup {
            self.live_transfers.moved_panes.insert(popup.state.pane_id);
            if let Some(runtime) = self.terminal_runtimes.remove(&popup.state.terminal_id) {
                runtime.preserve_for_handoff();
            }
            self.state.terminals.remove(&popup.state.terminal_id);
            if self
                .state
                .popup_pane
                .as_ref()
                .is_some_and(|active| active.terminal_id == popup.state.terminal_id)
            {
                self.state.popup_pane = None;
            }
            self.live_transfers
                .parked_popups
                .remove(&bundle.source_tab_id);
        }
        for (pane_id, pane) in tab.panes {
            self.live_transfers.moved_panes.insert(pane_id);
            if let Some(runtime) = self.terminal_runtimes.remove(&pane.attached_terminal_id) {
                runtime.preserve_for_handoff();
            }
            self.state.terminals.remove(&pane.attached_terminal_id);
            self.overlay_panes.remove(&pane_id);
            self.pending_worktree_remove_runtime_exits.remove(&pane_id);
            self.pending_worktree_remove_runtime_restores
                .remove(&pane_id);
            self.state
                .public_pane_id_aliases
                .retain(|_, target| *target != pane_id);
            self.state.remove_pane_records([pane_id]);
            self.state.pending_agent_notifications.remove(&pane_id);
            self.state
                .direct_attach_resize_locks
                .remove(&pane.attached_terminal_id);
        }
        if self.state.workspaces[ws].tabs.is_empty() {
            self.emit_event(crate::api::schema::EventEnvelope {
                event: crate::api::schema::EventKind::WorkspaceClosed,
                data: crate::api::schema::EventData::WorkspaceClosed {
                    workspace_id: bundle.source_workspace_id.clone(),
                    workspace: None,
                },
            });
            self.state.workspaces.remove(ws);
            self.state.active = self.state.active.and_then(|active| {
                if self.state.workspaces.is_empty() {
                    None
                } else {
                    Some(if active > ws {
                        active - 1
                    } else {
                        active.min(self.state.workspaces.len() - 1)
                    })
                }
            });
            self.state.selected = if self.state.selected > ws {
                self.state.selected - 1
            } else {
                self.state
                    .selected
                    .min(self.state.workspaces.len().saturating_sub(1))
            };
        }
        if let Some(ws) = self.parse_workspace_id(&bundle.source_workspace_id) {
            self.emit_event(crate::api::schema::EventEnvelope {
                event: crate::api::schema::EventKind::WorkspaceUpdated,
                data: crate::api::schema::EventData::WorkspaceUpdated {
                    workspace: self.workspace_info(ws),
                },
            });
        }
        self.reconcile_live_transfer_overlays();
        self.live_transfers.committed.insert(operation.into());
        self.state.mark_session_dirty();
        self.schedule_session_save();
        Ok(())
    }
    pub(crate) fn commit_live_destination(
        &mut self,
        operation: &str,
        generation: u64,
    ) -> Result<(), String> {
        if let Some(committed_generation) = self.live_transfers.destination_commits.get(operation) {
            return if *committed_generation == generation {
                Ok(())
            } else {
                Err("conflicting commit generation".into())
            };
        }
        let prepared = self
            .live_transfers
            .destinations
            .get(operation)
            .ok_or("destination operation not prepared")?;
        let tabs = if let Some(workspace) = &prepared.new_workspace {
            if self.parse_workspace_id(&prepared.workspace).is_some() {
                return Err("prepared destination workspace identity already exists".into());
            }
            workspace.tabs.len()
        } else {
            let ws = self
                .parse_workspace_id(&prepared.workspace)
                .ok_or("reserved destination disappeared")?;
            self.state.workspaces[ws].tabs.len()
        };
        if prepared.index > tabs {
            return Err("destination topology changed before commit".into());
        }
        let destination = self
            .live_transfers
            .destinations
            .remove(operation)
            .ok_or("destination operation not prepared")?;
        let created_workspace = destination.new_workspace.is_some();
        if let Some(ws) = destination.new_workspace {
            self.state.workspaces.push(ws);
        }
        let ws = self
            .parse_workspace_id(&destination.workspace)
            .ok_or("reserved destination disappeared")?;
        let workspace = &mut self.state.workspaces[ws];
        workspace
            .public_pane_numbers
            .extend(destination.pane_numbers);
        if destination.index <= workspace.active_tab && !workspace.tabs.is_empty() {
            workspace.active_tab += 1;
        }
        workspace.tabs.insert(destination.index, destination.tab);
        for terminal in destination.terminals {
            self.state.terminals.insert(terminal.id.clone(), terminal);
        }
        self.state.plugin_panes.extend(destination.plugin_panes);
        for (pane, mut overlay) in destination.overlays {
            overlay.ws_idx = ws;
            overlay.tab_idx = destination.index;
            self.overlay_panes.insert(pane, overlay);
        }
        self.reconcile_live_transfer_overlays();
        if let Some(popup) = destination.popup {
            let tab_id = self
                .public_tab_id(ws, destination.index)
                .ok_or("committed tab locator missing")?;
            self.live_transfers.parked_popups.insert(tab_id, popup);
        }
        for (id, mut runtime) in destination.runtimes {
            runtime.assume_handoff_ownership();
            runtime.set_handoff_reader_paused(false);
            self.terminal_runtimes.insert(id, runtime);
        }
        for route in destination.routes {
            self.live_transfers
                .generations
                .insert(route.terminal_id, (generation, route.process));
        }
        self.live_transfers.committed.insert(operation.into());
        self.live_transfers
            .destination_commits
            .insert(operation.into(), generation);
        self.state.mark_session_dirty();
        self.schedule_session_save();
        self.emit_event(crate::api::schema::EventEnvelope {
            event: if created_workspace {
                crate::api::schema::EventKind::WorkspaceCreated
            } else {
                crate::api::schema::EventKind::WorkspaceUpdated
            },
            data: if created_workspace {
                crate::api::schema::EventData::WorkspaceCreated {
                    workspace: self.workspace_info(ws),
                }
            } else {
                crate::api::schema::EventData::WorkspaceUpdated {
                    workspace: self.workspace_info(ws),
                }
            },
        });
        self.emit_event(crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::TabTransferred,
            data: crate::api::schema::EventData::TabTransferred {
                transfer: destination.transfer,
            },
        });
        Ok(())
    }
    pub(crate) fn abort_live_transfer(&mut self, operation: &str) -> Result<(), String> {
        if self.live_transfers.committed.contains(operation)
            || self
                .live_transfers
                .destination_commits
                .contains_key(operation)
        {
            return Err("committed operation cannot abort".into());
        }
        if self.live_transfers.aborted.contains(operation) {
            return Ok(());
        }
        if let Some(bundle) = self.live_transfers.sources.remove(operation) {
            for pane in bundle.panes {
                if let Some(runtime) = self.terminal_runtimes.get(&pane.terminal_id) {
                    runtime.set_handoff_reader_paused(false);
                }
            }
        }
        if let Some(destination) = self.live_transfers.destinations.remove(operation) {
            for runtime in destination.runtimes.into_values() {
                runtime.preserve_for_handoff();
            }
        }
        self.live_transfers.aborted.insert(operation.into());
        Ok(())
    }
    pub(crate) fn validate_forward(
        &self,
        id: &TerminalId,
        generation: u64,
        process: &LiveProcess,
    ) -> Result<(), String> {
        if self.live_transfers.generations.get(id) != Some(&(generation, process.clone())) {
            return Err("stale terminal route generation".into());
        }
        let runtime = self
            .terminal_runtimes
            .get(id)
            .ok_or("terminal is no longer owned")?;
        if runtime.child_pid() != Some(process.pid) {
            return Err("terminal shell PID changed".into());
        }
        let pty = runtime.pty_identity().map_err(|e| e.to_string())?;
        if crate::server::tab_transfer::live_process_on_pty(process.pid, pty)
            .map_err(|e| e.to_string())?
            != *process
        {
            return Err("terminal process identity changed".into());
        }
        Ok(())
    }
}
