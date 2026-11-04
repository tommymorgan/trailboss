use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::persist::{ClosedTabSnapshot, WorkspaceSnapshot};

use super::{App, Mode};

mod droid_sessions;
mod grok_sessions;
mod letta_sessions;
mod local_sessions;
mod project_sessions;
static NEXT_CLOSED_TAB: AtomicU64 = AtomicU64::new(1);

#[derive(Hash, PartialEq, Eq)]
enum ConversationKey {
    File(PathBuf),
    Session(String, String),
}

fn native_session_id(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path)
        .map_err(|_| format!("Saved agent transcript is unavailable: {}", path.display()))?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(format!(
            "Saved agent transcript is unavailable: {}",
            path.display()
        ));
    }
    // OMP/pi JSONL begins with its header or a fixed-width title followed by the header.
    for line in BufReader::new(file.take(16 * 1024)).lines().take(2) {
        let entry: serde_json::Value = line
            .ok()
            .and_then(|line| serde_json::from_str(&line).ok())
            .ok_or_else(|| {
                format!(
                    "Saved agent transcript has an invalid header: {}",
                    path.display()
                )
            })?;
        if entry["type"] == "session" {
            return entry["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!(
                        "Saved agent transcript has no session ID: {}",
                        path.display()
                    )
                });
        }
        if entry["type"] != "title" {
            break;
        }
    }
    Err(format!(
        "Saved agent transcript has no session header: {}",
        path.display()
    ))
}

fn native_session_path(agent: &str, id: &str) -> Result<PathBuf, String> {
    let extensions = match agent {
        "omp" => crate::integration::omp_extension_dir(),
        _ => crate::integration::pi_extension_dir(),
    }
    .map_err(|error| format!("Saved {agent} conversation is unavailable: {error}"))?;
    let root = extensions
        .parent()
        .expect("extension directory has a parent")
        .join("sessions");
    let mut found = None;
    let buckets = std::fs::read_dir(&root)
        .map_err(|_| format!("Saved {agent} conversation is unavailable: {id}"))?;
    for bucket in buckets.flatten() {
        let Ok(files) = std::fs::read_dir(bucket.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            let candidate_id = name.split_once('_').map_or(name, |(_, suffix)| suffix);
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
                || !candidate_id.starts_with(id)
            {
                continue;
            }
            if native_session_id(&path).is_ok_and(|candidate| candidate.starts_with(id)) {
                if found.is_some() {
                    return Err(format!("Saved {agent} conversation ID is ambiguous: {id}"));
                }
                found = Some(path);
            }
        }
    }
    found.ok_or_else(|| format!("Saved {agent} conversation is unavailable: {id}"))
}

fn verify_native_id_conversation(agent: &str, id: &str, cwd: &Path) -> Result<(), String> {
    let unavailable = || format!("Saved {agent} conversation is unavailable: {id}");
    match agent {
        "droid" => return droid_sessions::verify_droid(id, cwd),
        "grok" => return grok_sessions::verify_grok(id, cwd),
        "copilot" => return local_sessions::verify_copilot(id, cwd),
        "kimi" => return local_sessions::verify_kimi(id, cwd),
        "qwen" => return project_sessions::verify_qwen(id, cwd),
        "letta" => return letta_sessions::verify_letta(id, cwd),
        _ => {}
    }
    let roots = match agent {
        "codex" => {
            let home = crate::integration::codex_dir().map_err(|_| unavailable())?;
            [
                Some(home.join("sessions")),
                Some(home.join("archived_sessions")),
            ]
        }
        "claude" => [
            Some(
                crate::integration::claude_dir()
                    .map_err(|_| unavailable())?
                    .join("projects"),
            ),
            None,
        ],
        _ => {
            return Err(format!(
                "Cannot verify saved {agent} conversation availability"
            ))
        }
    };
    let mut found = false;
    for root in roots.into_iter().flatten() {
        let mut directories = vec![(root, 0)];
        while let Some((directory, depth)) = directories.pop() {
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(unavailable()),
            };
            for entry in entries {
                let entry = entry.map_err(|_| unavailable())?;
                let kind = entry.file_type().map_err(|_| unavailable())?;
                if kind.is_dir() && depth < 3 {
                    directories.push((entry.path(), depth + 1));
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                let path = entry.path();
                if path
                    .extension()
                    .is_none_or(|extension| extension != "jsonl")
                    || !entry.file_name().to_string_lossy().contains(id)
                {
                    continue;
                }
                let file = std::fs::File::open(&path).map_err(|_| unavailable())?;
                let reader = BufReader::new(file);
                let matches = if agent == "codex" {
                    #[derive(serde::Deserialize)]
                    struct Payload {
                        id: String,
                    }
                    #[derive(serde::Deserialize)]
                    struct Metadata {
                        #[serde(rename = "type")]
                        kind: String,
                        payload: Payload,
                    }
                    serde_json::Deserializer::from_reader(reader)
                        .into_iter::<Metadata>()
                        .next()
                        .is_some_and(|record| {
                            record.is_ok_and(|record| {
                                record.kind == "session_meta" && record.payload.id == id
                            })
                        })
                } else {
                    #[derive(serde::Deserialize)]
                    struct Metadata {
                        #[serde(rename = "type")]
                        kind: Option<String>,
                        #[serde(rename = "sessionId")]
                        session_id: Option<String>,
                        message: Option<serde::de::IgnoredAny>,
                    }
                    // Stream and skip message bodies. Identity does not impose a
                    // prompt-size limit or allocate the saved prompt contents.
                    serde_json::Deserializer::from_reader(reader)
                        .into_iter::<Metadata>()
                        .any(|record| {
                            record.is_ok_and(|record| {
                                matches!(record.kind.as_deref(), Some("user" | "assistant"))
                                    && record.session_id.as_deref() == Some(id)
                                    && record.message.is_some()
                            })
                        })
                };
                if matches {
                    if found {
                        return Err(format!("Saved {agent} conversation ID is ambiguous: {id}"));
                    }
                    found = true;
                }
            }
        }
    }
    if found {
        Ok(())
    } else {
        Err(unavailable())
    }
}

fn conversation_keys(
    agent: &str,
    kind: crate::agent_resume::AgentSessionRefKind,
    value: &str,
) -> [Option<ConversationKey>; 2] {
    use crate::agent_resume::AgentSessionRefKind;
    let native = matches!(agent, "omp" | "pi");
    let owner = if native { "native-jsonl" } else { agent };
    if kind == AgentSessionRefKind::Id {
        return [
            Some(ConversationKey::Session(owner.into(), value.into())),
            None,
        ];
    }
    let path = Path::new(value);
    [
        Some(ConversationKey::File(
            path.canonicalize().unwrap_or_else(|_| path.into()),
        )),
        native
            .then(|| native_session_id(path).ok())
            .flatten()
            .map(|id| ConversationKey::Session(owner.into(), id)),
    ]
}

impl App {
    pub(super) fn capture_closed_tab(&self, ws_idx: usize, tab_idx: usize) -> ClosedTabSnapshot {
        let workspace = &self.state.workspaces[ws_idx];
        let tab = &workspace.tabs[tab_idx];
        let mut saved =
            crate::persist::capture_tab(tab, &self.state.terminals, &self.terminal_runtimes);
        // Only agent-specific session references can authorize a later resume.
        for pane in saved.panes.values_mut() {
            pane.launch_argv = None;
            pane.agent_resume = None;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        ClosedTabSnapshot {
            id: format!(
                "closed-{}-{}-{}",
                std::process::id(),
                now.as_nanos(),
                NEXT_CLOSED_TAB.fetch_add(1, Ordering::Relaxed)
            ),
            label: self
                .tab_info(ws_idx, tab_idx)
                .expect("captured tab exists")
                .label,
            workspace_label: workspace
                .display_name_from(&self.state.terminals, &self.terminal_runtimes),
            closed_at: now.as_millis().try_into().unwrap_or(u64::MAX),
            workspace: WorkspaceSnapshot {
                id: Some(workspace.id.clone()),
                custom_name: workspace.custom_name.clone(),
                identity_cwd: workspace.identity_cwd.clone(),
                worktree_space: None,
                public_pane_numbers: HashMap::new(),
                next_public_pane_number: 1,
                public_tab_numbers: vec![1],
                next_public_tab_number: 2,
                tabs: vec![saved],
                active_tab: 0,
            },
        }
    }

    pub(super) fn capture_closed_workspaces(&self, indices: &[usize]) -> Vec<ClosedTabSnapshot> {
        indices
            .iter()
            .flat_map(|&ws_idx| {
                (0..self.state.workspaces[ws_idx].tabs.len())
                    .map(move |tab_idx| self.capture_closed_tab(ws_idx, tab_idx))
            })
            .collect()
    }

    pub(super) fn close_workspaces_with_history(&mut self, indices: Vec<usize>) {
        let closed = self.capture_closed_workspaces(&indices);
        self.state.close_workspaces(indices);
        for entry in closed {
            self.remember_closed_tab(entry);
        }
    }

    pub(super) fn capture_final_pane_tab(
        &self,
        pane_id: crate::layout::PaneId,
    ) -> Option<ClosedTabSnapshot> {
        let (ws_idx, _) = self.find_pane(pane_id)?;
        let workspace = &self.state.workspaces[ws_idx];
        let tab_idx = workspace.find_tab_index_for_pane(pane_id)?;
        (workspace.tabs[tab_idx].panes.len() == 1).then(|| self.capture_closed_tab(ws_idx, tab_idx))
    }

    pub(super) fn remember_closed_tab(&mut self, entry: ClosedTabSnapshot) {
        self.state.closed_tabs.push_front(entry);
        self.state
            .closed_tabs
            .truncate(crate::persist::closed_tabs::CLOSED_TAB_LIMIT);
        self.state.mark_session_dirty();
    }

    fn preflight_closed_workspace(&self, snapshot: &mut WorkspaceSnapshot) -> Result<(), String> {
        let mut live = HashSet::new();
        if snapshot
            .tabs
            .iter()
            .any(|tab| tab.panes.values().any(|pane| pane.agent_session.is_some()))
        {
            for terminal in self.state.terminals.values().filter(|terminal| {
                terminal.pending_agent_resume_plan.is_some()
                    || terminal.effective_agent_label().is_some()
            }) {
                let session = terminal
                    .hook_authority
                    .as_ref()
                    .and_then(|authority| {
                        authority
                            .session_ref
                            .as_ref()
                            .map(|reference| (authority.agent_label.as_str(), reference))
                    })
                    .or_else(|| {
                        terminal
                            .persisted_agent_session
                            .as_ref()
                            .map(|session| (session.agent.as_str(), &session.session_ref))
                    });
                if let Some((agent, reference)) = session {
                    live.extend(
                        conversation_keys(agent, reference.kind, &reference.value)
                            .into_iter()
                            .flatten(),
                    );
                }
            }
        }
        let mut restoring = HashSet::new();
        for pane in snapshot
            .tabs
            .iter_mut()
            .flat_map(|tab| tab.panes.values_mut())
        {
            // History never grants permission to replay a saved shell command.
            pane.launch_argv = None;
            pane.agent_resume = None;
            if !pane.cwd.is_dir() {
                return Err(format!(
                    "Saved directory is unavailable: {}",
                    pane.cwd.display()
                ));
            }
            let Some(session) = pane.agent_session.as_mut() else {
                continue;
            };
            if !self.resume_agents_on_restore {
                return Err("Saved agent conversations cannot resume while session.resume_agents_on_restore is disabled".into());
            }
            let reference = crate::agent_resume::session_ref_from_snapshot(
                &session.source,
                &session.agent,
                session.kind,
                &session.value,
            )
            .ok_or_else(|| format!("Saved agent conversation is unsupported: {}", session.agent))?;
            if matches!(session.agent.as_str(), "omp" | "pi") {
                let path = if session.kind == crate::agent_resume::AgentSessionRefKind::Path {
                    PathBuf::from(&session.value)
                } else {
                    native_session_path(&session.agent, &session.value)?
                };
                native_session_id(&path)?;
                session.value = path
                    .canonicalize()
                    .map_err(|_| {
                        format!("Saved agent transcript is unavailable: {}", path.display())
                    })?
                    .to_str()
                    .ok_or("Saved agent transcript path is not valid UTF-8")?
                    .to_owned();
                session.kind = crate::agent_resume::AgentSessionRefKind::Path;
            }
            let plan = crate::agent_resume::plan(
                &reference.source,
                &reference.agent,
                &reference.session_ref,
            )
            .ok_or_else(|| format!("Saved agent conversation cannot resume: {}", session.agent))?;
            if !crate::integration::command_available(&plan.argv[0]) {
                return Err(format!(
                    "Saved agent executable is unavailable: {}",
                    plan.argv[0]
                ));
            }
            if !matches!(session.agent.as_str(), "omp" | "pi") {
                verify_native_id_conversation(&session.agent, &session.value, &pane.cwd)?;
            }
            for key in conversation_keys(&session.agent, session.kind, &session.value)
                .into_iter()
                .flatten()
            {
                if live.contains(&key) || !restoring.insert(key) {
                    return Err(
                        "Saved agent conversation is already running or pending in another pane"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn restore_closed_tab(
        &mut self,
        closed_tab_id: &str,
    ) -> Result<(String, String), String> {
        let entry_idx = self
            .state
            .closed_tabs
            .iter()
            .position(|entry| entry.id == closed_tab_id)
            .ok_or_else(|| "This closed tab is no longer in history".to_string())?;
        let entry = &self.state.closed_tabs[entry_idx];
        let original_workspace_id = entry.workspace.id.clone();
        let existing = self
            .state
            .workspaces
            .iter()
            .position(|workspace| Some(&workspace.id) == original_workspace_id.as_ref());
        let mut snapshot = entry.workspace.clone();
        if let Some(index) = existing {
            let workspace = &self.state.workspaces[index];
            snapshot.next_public_pane_number = workspace.next_public_pane_number;
            snapshot.public_tab_numbers = vec![workspace.next_public_tab_number];
            snapshot.next_public_tab_number = workspace.next_public_tab_number + 1;
        } else {
            snapshot.id = Some(crate::workspace::generate_workspace_id());
        }
        self.preflight_closed_workspace(&mut snapshot)?;
        if existing.is_none() && !snapshot.identity_cwd.is_dir() {
            return Err(format!(
                "Saved space directory is unavailable: {}",
                snapshot.identity_cwd.display()
            ));
        }
        let size = self.state.new_pane_size(crate::ui::NewPanePlacement::Alone);
        let context = crate::persist::RestoreRuntimeContext {
            scrollback_limit_bytes: self.state.pane_scrollback_limit_bytes,
            shell_config: crate::pane::PaneShellConfig::new(
                &self.state.default_shell,
                self.state.shell_mode,
            ),
            resume_agents_on_restore: self.resume_agents_on_restore,
            events: self.event_tx.clone(),
            render_notify: self.render_notify.clone(),
            render_dirty: self.render_dirty.clone(),
        };
        let (mut restored, terminals, runtimes) =
            crate::persist::restore_closed_workspace(&snapshot, size, &context)?;
        let workspace_id = restored.id.clone();
        let old_counters = existing.map(|index| {
            let workspace = &self.state.workspaces[index];
            (
                workspace.next_public_pane_number,
                workspace.next_public_tab_number,
            )
        });
        let terminal_ids: Vec<_> = terminals
            .iter()
            .map(|terminal| terminal.id.clone())
            .collect();
        let (ws_idx, tab_idx) = if let Some(index) = existing {
            let workspace = &mut self.state.workspaces[index];
            let tab_idx = workspace.tabs.len();
            workspace.tabs.append(&mut restored.tabs);
            workspace
                .public_pane_numbers
                .extend(restored.public_pane_numbers);
            workspace.next_public_pane_number = restored.next_public_pane_number;
            workspace.next_public_tab_number = restored.next_public_tab_number;
            (index, tab_idx)
        } else {
            self.state.workspaces.push(restored);
            (self.state.workspaces.len() - 1, 0)
        };
        for terminal in terminals {
            self.state.terminals.insert(terminal.id.clone(), terminal);
        }
        for (id, runtime) in runtimes {
            self.terminal_runtimes.insert(id, runtime);
        }
        let tab = &self.state.workspaces[ws_idx].tabs[tab_idx];
        let panes = crate::ui::apply_pane_chrome(
            tab.layout
                .panes(ratatui::layout::Rect::new(0, 0, size.1, size.0)),
            self.state.pane_borders,
            self.state.pane_gaps,
            self.state.pane_outer_borders,
        );
        let pending: Vec<_> = panes
            .into_iter()
            .filter_map(|pane| {
                let terminal_id = tab.terminal_id(pane.id)?;
                self.state
                    .terminals
                    .get(terminal_id)?
                    .pending_agent_resume_plan
                    .as_ref()?;
                let inner = crate::ui::pane_inner_rect(pane.rect, pane.borders);
                Some((terminal_id.clone(), inner.height, inner.width))
            })
            .collect();
        let mut startup_error = None;
        for (terminal_id, rows, cols) in pending {
            self.start_pending_agent_resume_for_terminal(&terminal_id, rows, cols, true);
            startup_error = self
                .state
                .terminals
                .get(&terminal_id)
                .and_then(|terminal| terminal.restore_error.clone());
            if startup_error.is_none() && self.terminal_runtimes.get(&terminal_id).is_none() {
                startup_error = Some("Could not start the saved agent conversation".into());
            }
            if startup_error.is_some() {
                break;
            }
        }
        if let Some(error) = startup_error {
            for terminal_id in terminal_ids {
                if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
                    runtime.shutdown();
                }
                self.state.terminals.remove(&terminal_id);
            }
            if let Some((pane_counter, tab_counter)) = old_counters {
                let workspace = &mut self.state.workspaces[ws_idx];
                workspace.close_tab(tab_idx);
                workspace.next_public_pane_number = pane_counter;
                workspace.next_public_tab_number = tab_counter;
            } else {
                self.state.workspaces.remove(ws_idx);
            }
            return Err(error);
        }
        for pane in self.state.workspaces[ws_idx].tabs[tab_idx]
            .layout
            .pane_ids()
        {
            self.state.remove_alias_shadowed_by_new_pane(pane);
        }
        self.state.closed_tabs.remove(entry_idx);
        if existing.is_none() {
            // Remaining tabs from a removed space restore into its one replacement.
            for entry in &mut self.state.closed_tabs {
                if entry.workspace.id == original_workspace_id {
                    entry.workspace.id = Some(workspace_id.clone());
                }
            }
        }
        self.state.switch_workspace_tab(ws_idx, tab_idx);
        self.state.mode = Mode::Terminal;
        self.schedule_session_save();
        self.emit_tab_created_events(ws_idx, tab_idx);
        let tab_id = self
            .public_tab_id(ws_idx, tab_idx)
            .expect("restored tab exists");
        Ok((workspace_id, tab_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_conversation_identity_survives_a_large_initial_message_and_rejects_other_ids() {
        let _guard = crate::integration::integration_env_lock();
        let root = std::env::temp_dir().join(format!(
            "closed-claude-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let project = root.join("projects/owned");
        std::fs::create_dir_all(&project).unwrap();
        let id = "019a4444-0000-7000-8000-000000000004";
        let transcript = project.join(format!("{id}.jsonl"));
        std::fs::write(
            &transcript,
            format!(
                "{}\n",
                serde_json::json!({
                    "type":"user", "sessionId":id,
                    "message":{"role":"user","content":"x".repeat(128 * 1024)},
                })
            ),
        )
        .unwrap();
        let previous = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", &root);
        let available = verify_native_id_conversation("claude", id, &root);
        std::fs::write(
            &transcript,
            format!(
                "{}\n",
                serde_json::json!({
                    "type":"user", "sessionId":"different-conversation",
                    "message":{"role":"user","content":"not the saved conversation"},
                })
            ),
        )
        .unwrap();
        let different = verify_native_id_conversation("claude", id, &root);
        match previous {
            Some(previous) => std::env::set_var("CLAUDE_CONFIG_DIR", previous),
            None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
        }
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(available, Ok(()), "large saved conversation was rejected");
        assert!(
            different.is_err(),
            "filename must not override native conversation identity"
        );
    }
}
