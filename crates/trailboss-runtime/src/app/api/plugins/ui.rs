//! Cached plugin UI projections from one-shot refreshes and owned streams.

use crate::api::schema::{
    InstalledPluginInfo, PluginCommandLogInfo, PluginInvocationContext, PluginManifestAction,
    PluginUiSurface,
};
use crate::app::state::InstalledPluginRegistry;
use crate::protocol::endpoint::{EndpointPluginUiItem, EndpointPluginUiSegment};

use super::manifest::{current_platform, effective_platforms};
use super::plugin_manifest_available;
use super::runtime::current_unix_ms;
use crate::app::App;

/// Cap on rows accepted from one refresh command.
const PLUGIN_UI_MAX_ROWS: usize = 128;
/// Cap on a single row title; the client clips to the sidebar width anyway.
const PLUGIN_UI_TITLE_MAX_CHARS: usize = 200;
/// Cap on semantic segments accepted in one row.
const PLUGIN_UI_MAX_SEGMENTS: usize = 16;
/// Ceiling for a forwarded producer action before its log entry is failed.
const PLUGIN_UI_ACTION_FORWARD_TIMEOUT_SECS: u64 = 10;

#[derive(serde::Deserialize)]
struct PluginUiSegmentJson {
    text: String,
    role: String,
}

#[derive(serde::Deserialize)]
struct PluginUiRowJson {
    title: String,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    tab_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    segments: Vec<PluginUiSegmentJson>,
}

/// Runtime ownership stays outside AppState. Dropping the sender cancels the
/// supervisor even while stdout delivery is backpressured; it kills and reaps.
pub(crate) struct PluginUiStream {
    generation: u64,
    plugin: crate::api::schema::InstalledPluginInfo,
    running: bool,
    forward: Option<std::sync::Arc<PluginUiForward>>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

/// Shared plumbing for forwarding plugin actions to a running stream producer
/// over its stdin. `pending` guards the action-result events the reader emits
/// so duplicate or forged envelope ids cannot resolve uninvoked logs.
pub(crate) struct PluginUiForward {
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pending: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl PluginUiForward {
    fn take_pending(&self, id: &str) -> bool {
        self.pending
            .lock()
            .map(|mut set| set.remove(id))
            .unwrap_or(false)
    }

    fn insert_pending(&self, id: &str) {
        let _ = self
            .pending
            .lock()
            .map(|mut set| set.insert(id.to_string()));
    }

    async fn write_request(&self, line: &str) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await
    }
}

const PLUGIN_UI_STREAM_MAX_BYTES: usize = 64 * 1024;
const PLUGIN_UI_MAX_STREAMS: usize = 32;

impl App {
    /// A public application composition can declare the empty-session interaction.
    /// Legacy shell clients without one retain automatic initial workspace creation.
    pub(crate) fn has_widget_composition(&self) -> bool {
        self.state.plugin_ui_dynamic.values().flatten().any(|item| {
            item.declaration
                .as_ref()
                .is_some_and(|declaration| declaration.composition.is_some())
        })
    }
    pub(super) fn invalidate_plugin_ui(&mut self, plugin_id: &str) {
        self.plugin_ui_streams.retain(|key, _| key.0 != plugin_id);
        self.state
            .plugin_ui_dynamic
            .retain(|key, _| key.0 != plugin_id);
        self.state
            .plugin_ui_refresh_in_flight
            .retain(|_, key| key.0 != plugin_id);
        self.state
            .plugin_ui_refresh_dirty
            .retain(|key| key.0 != plugin_id);
    }

    pub(crate) fn reconcile_plugin_ui(&mut self) {
        let live = ui_entries(&self.state.installed_plugins, None);
        self.state
            .plugin_ui_dynamic
            .retain(|key, _| live.contains(key));
        self.state
            .plugin_ui_refresh_dirty
            .retain(|key| live.contains(key));
        self.state
            .plugin_ui_refresh_in_flight
            .retain(|_, key| live.contains(key));
        let streams = ui_entries(&self.state.installed_plugins, Some(true));
        self.plugin_ui_streams.retain(|key, runtime| {
            streams.contains(key)
                && self.state.installed_plugins.get(&key.0) == Some(&runtime.plugin)
        });
        for key in streams {
            if self.plugin_ui_streams.contains_key(&key) {
                continue;
            }
            self.state.plugin_ui_dynamic.remove(&key);
            if self.plugin_ui_streams.len() >= PLUGIN_UI_MAX_STREAMS {
                tracing::warn!(plugin_id = %key.0, ui_id = %key.1, "plugin UI stream limit reached");
                continue;
            }
            let plugin = self.state.installed_plugins[&key.0].clone();
            let Some(entry) = plugin.ui.iter().find(|entry| entry.id == key.1) else {
                continue;
            };
            let Some(command) = entry.command.as_ref() else {
                continue;
            };
            let mut context = self.current_plugin_context("plugin-ui-stream");
            context.invocation_source = Some("plugin_ui_stream".to_string());
            let prepared = super::runtime::prepare_plugin_command(
                &plugin, command, &context, None, None, None,
            );
            let prepared = match prepared {
                Ok(command) => command,
                Err((code, message)) => {
                    tracing::warn!(plugin_id = %key.0, ui_id = %key.1, code, "plugin UI stream failed to start: {message}");
                    continue;
                }
            };
            let generation = self.next_plugin_ui_stream_generation;
            self.next_plugin_ui_stream_generation += 1;
            let lease = crate::plugin_installations::command_lease(
                &self.plugin_installation_leases,
                &plugin,
            );
            match start_plugin_ui_stream(
                prepared,
                key.clone(),
                generation,
                plugin.actions.clone(),
                self.event_tx.clone(),
                lease,
            ) {
                Ok((stop, forward)) => {
                    self.plugin_ui_streams.insert(
                        key,
                        PluginUiStream {
                            generation,
                            plugin,
                            running: true,
                            forward,
                            _stop: stop,
                        },
                    );
                }
                Err(err) => {
                    tracing::warn!(plugin_id = %key.0, ui_id = %key.1, "plugin UI stream failed to start: {err}")
                }
            }
        }
        self.refresh_dynamic_plugin_ui();
    }

    pub(crate) fn handle_plugin_ui_stream_rows(
        &mut self,
        key: (String, String),
        generation: u64,
        rows: Vec<EndpointPluginUiItem>,
    ) -> bool {
        if self
            .plugin_ui_streams
            .get(&key)
            .is_none_or(|stream| stream.generation != generation || !stream.running)
        {
            return false;
        }
        if self.state.plugin_ui_dynamic.get(&key) == Some(&rows) {
            return false;
        }
        let count = rows.len();
        let (plugin_id, ui_id) = (key.0.clone(), key.1.clone());
        self.state.plugin_ui_dynamic.insert(key, rows);
        tracing::info!(event = "plugin.ui.perf", stage = "rows_applied", plugin_id = %plugin_id, ui_id = %ui_id, rows = count);
        true
    }

    pub(crate) fn handle_plugin_ui_stream_finished(
        &mut self,
        key: (String, String),
        generation: u64,
        error: Option<String>,
    ) -> bool {
        if self
            .plugin_ui_streams
            .get(&key)
            .is_none_or(|stream| stream.generation != generation)
        {
            return false;
        }
        // Keep the generation registered until a registry change: an exited
        // producer must not be restarted by metadata, renders or API reads.
        if let Some(stream) = self.plugin_ui_streams.get_mut(&key) {
            stream.running = false;
        }
        let changed = self.state.plugin_ui_dynamic.remove(&key).is_some();
        if let Some(error) = error {
            tracing::warn!(plugin_id = %key.0, ui_id = %key.1, "plugin UI stream stopped: {error}");
        }
        changed
    }
}

fn start_plugin_ui_stream(
    mut prepared: std::process::Command,
    key: (String, String),
    generation: u64,
    actions: Vec<PluginManifestAction>,
    event_tx: tokio::sync::mpsc::Sender<crate::events::AppEvent>,
    lease: Option<std::sync::Arc<std::fs::File>>,
) -> std::io::Result<(
    tokio::sync::oneshot::Sender<()>,
    Option<std::sync::Arc<PluginUiForward>>,
)> {
    use tokio::io::AsyncReadExt;
    tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;
    crate::platform::configure_status_command(&mut prepared);
    let mut command = tokio::process::Command::from(prepared);
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut process_tree = crate::platform::StatusCommandGuard::new(&child)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("missing plugin stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("missing plugin stderr"))?;
    let forward = child.stdin.take().map(|stdin| {
        std::sync::Arc::new(PluginUiForward {
            stdin: tokio::sync::Mutex::new(stdin),
            pending: std::sync::Mutex::new(std::collections::HashSet::new()),
        })
    });
    let reader_forward = forward.clone();
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _lease = lease;
        // Keep a bounded stderr tail so an instantly-exiting producer reports
        // why instead of just "exit status: 1".
        let stderr_tail = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
        let tail_for_drainer = stderr_tail.clone();
        let drainer = tokio::spawn(async move {
            let mut stderr = stderr;
            let mut chunk = [0; 4096];
            while let Ok(n) = stderr.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                let mut tail = tail_for_drainer.lock().await;
                tail.push_str(&String::from_utf8_lossy(&chunk[..n]));
                let len = tail.len();
                if len > 512 {
                    tail.drain(..len - 512);
                }
            }
        });
        let (result, cancelled): (Result<(), String>, bool) = tokio::select! {
            biased;
            _ = &mut stopped => (Ok(()), true),
            result = async {
                let mut stdout = tokio::io::BufReader::new(stdout);
                let mut line = Vec::with_capacity(4096);
                let mut previous = None;
                while read_plugin_ui_line(&mut stdout, &mut line).await.map_err(|err| err.to_string())? {
                    let text = std::str::from_utf8(&line).map_err(|err| err.to_string())?;
                    if text.starts_with('{')
                        && handle_plugin_ui_control(
                            text,
                            reader_forward.as_deref(),
                            &event_tx,
                        )
                        .await
                    {
                        continue;
                    }
                    let rows = try_parse_plugin_ui_rows(&key.0, &key.1, text, &actions).map_err(|err| err.to_string())?;
                    if previous.as_ref() == Some(&rows) { continue; }
                    tracing::info!(event = "plugin.ui.perf", stage = "rows_arrived", plugin_id = %key.0, ui_id = %key.1, rows = rows.len());
                    event_tx.send(crate::events::AppEvent::PluginUiStreamRows { key: key.clone(), generation, rows: rows.clone() }).await.map_err(|err| err.to_string())?;
                    previous = Some(rows);
                }
                let status = child.wait().await.map_err(|err| err.to_string())?;
                if !status.success() { return Err(format!("command exited with {status}")); }
                Ok(())
            } => (result, false),
        };
        // Cancellation and malformed output both terminate the actual process,
        // not merely the reader task. kill() waits for exit and reaps the child.
        process_tree.terminate();
        let _ = child.kill().await;
        let _ = child.wait().await;
        drainer.abort();
        let tail = stderr_tail.lock().await.trim().to_string();
        if cancelled {
            return;
        }
        let finished = crate::events::AppEvent::PluginUiStreamFinished {
            key,
            generation,
            error: result.err().map(|error| {
                if tail.is_empty() {
                    error
                } else {
                    format!("{error}; stderr: {tail}")
                }
            }),
        };
        tokio::select! {
            biased;
            _ = &mut stopped => {},
            _ = event_tx.send(finished) => {},
        }
    });
    Ok((stop, forward))
}

async fn read_plugin_ui_line(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
    line: &mut Vec<u8>,
) -> std::io::Result<bool> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    line.clear();
    let n = reader
        .take((PLUGIN_UI_STREAM_MAX_BYTES + 1) as u64)
        .read_until(b'\n', line)
        .await?;
    if n == 0 {
        return Ok(false);
    }
    if n > PLUGIN_UI_STREAM_MAX_BYTES {
        return Err(std::io::Error::other("plugin UI line exceeds 64 KiB"));
    }
    if line.last() != Some(&b'\n') {
        return Err(std::io::Error::other(
            "plugin UI line is not newline terminated",
        ));
    }
    Ok(true)
}

/// Handle a `{`-prefixed stdout line as a producer control envelope. Returns
/// true when the line was consumed; false lets the rows parser reject it.
async fn handle_plugin_ui_control(
    text: &str,
    forward: Option<&PluginUiForward>,
    event_tx: &tokio::sync::mpsc::Sender<crate::events::AppEvent>,
) -> bool {
    #[derive(serde::Deserialize)]
    struct PluginUiControl {
        #[serde(rename = "herdr")]
        trailboss: String,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        ok: Option<bool>,
        #[serde(default)]
        error: Option<String>,
    }
    let Ok(control) = serde_json::from_str::<PluginUiControl>(text) else {
        return false;
    };
    match control.trailboss.as_str() {
        "widget_tree" => false,
        "action_result" => {
            let Some(id) = control.id.filter(|id| !id.is_empty()) else {
                tracing::warn!("plugin UI action_result envelope missing id");
                return true;
            };
            let Some(forward) = forward else {
                return true;
            };
            if !forward.take_pending(&id) {
                tracing::warn!(id = %id, "plugin UI action_result for unknown request");
                return true;
            }
            let error = control.error.filter(|error| !error.is_empty());
            let succeeded = control.ok == Some(true) && error.is_none();
            tracing::info!(event = "plugin.ui.perf", stage = "action_result", request = %id, ok = succeeded);
            let finished = crate::events::AppEvent::PluginCommandFinished {
                log_id: id,
                finished_unix_ms: current_unix_ms(),
                exit_code: if succeeded { Some(0) } else { None },
                stdout: String::new(),
                stderr: String::new(),
                error,
            };
            let _ = event_tx.send(finished).await;
            true
        }
        other => {
            tracing::warn!(kind = %other, "plugin UI control envelope kind ignored");
            true
        }
    }
}

impl App {
    /// Forward a plugin action to its running stream producer when the plugin
    /// opted in with `stream_actions`. Returns `None` when the classic spawn
    /// path should run instead.
    pub(super) fn try_forward_plugin_action(
        &mut self,
        plugin: &InstalledPluginInfo,
        action_id: &str,
        action_command: &[String],
        context: &PluginInvocationContext,
    ) -> Option<Result<PluginCommandLogInfo, (&'static str, String)>> {
        if !plugin
            .ui
            .iter()
            .any(|entry| entry.stream && entry.stream_actions)
        {
            return None;
        }
        let forward = self.plugin_ui_streams.values().find_map(|stream| {
            if stream.running && stream.plugin.plugin_id == plugin.plugin_id {
                stream.forward.clone()
            } else {
                None
            }
        })?;
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return None;
        };
        let log = match self.register_forwarded_plugin_command(
            plugin,
            action_id.to_string(),
            action_command.to_vec(),
        ) {
            Ok(log) => log,
            Err(err) => return Some(Err(err)),
        };
        let mut line = serde_json::json!({
            "herdr": "action",
            "id": log.log_id,
            "action_id": action_id,
            "context": context,
        })
        .to_string();
        line.push('\n');
        forward.insert_pending(&log.log_id);
        tracing::info!(
            event = "plugin.ui.perf",
            stage = "action_forwarded",
            plugin_id = %plugin.plugin_id,
            request = %log.log_id,
            action = %action_id
        );
        let event_tx = self.event_tx.clone();
        let write_forward = forward.clone();
        let write_id = log.log_id.clone();
        let write_line = line;
        handle.spawn(async move {
            if let Err(err) = write_forward.write_request(&write_line).await {
                write_forward.take_pending(&write_id);
                let _ = event_tx
                    .send(crate::events::AppEvent::PluginCommandFinished {
                        log_id: write_id,
                        finished_unix_ms: super::runtime::current_unix_ms(),
                        exit_code: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        error: Some(format!("forward write failed: {err}")),
                    })
                    .await;
            }
        });
        // Watchdog: a hung producer must not leave the log in flight forever.
        let timeout_tx = self.event_tx.clone();
        let timeout_id = log.log_id.clone();
        handle.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(
                PLUGIN_UI_ACTION_FORWARD_TIMEOUT_SECS,
            ))
            .await;
            if forward.take_pending(&timeout_id) {
                let _ = timeout_tx
                    .send(crate::events::AppEvent::PluginCommandFinished {
                        log_id: timeout_id,
                        finished_unix_ms: super::runtime::current_unix_ms(),
                        exit_code: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        error: Some("producer action timed out".to_string()),
                    })
                    .await;
            }
        });
        Some(Ok(log))
    }

    /// Mark every enabled dynamic sidebar entry dirty and start refreshes for
    /// the ones without a command in flight. Called when session state that a
    /// tree-style panel reflects changes, and after the registry reloads.
    pub(crate) fn refresh_dynamic_plugin_ui(&mut self) {
        let entries = dynamic_ui_entries(&self.state.installed_plugins);
        for key in entries {
            self.state.plugin_ui_refresh_dirty.insert(key);
        }
        self.pump_plugin_ui_refreshes();
    }

    /// Refresh dynamic rows after a plugin action finished, so actions that
    /// only mutate plugin-local state (for example a collapse toggle) still
    /// produce updated rows. No-op when the plugin has no dynamic entries.
    pub(crate) fn refresh_dynamic_plugin_ui_after_action(&mut self, log_id: &str) {
        let Some(plugin_id) = self
            .state
            .plugin_command_logs
            .iter()
            .find(|log| log.log_id == log_id && log.action_id.is_some())
            .map(|log| log.plugin_id.clone())
        else {
            return;
        };
        let has_dynamic = self
            .state
            .installed_plugins
            .get(&plugin_id)
            .is_some_and(|plugin| {
                plugin.enabled
                    && plugin
                        .ui
                        .iter()
                        .any(|entry| entry.command.is_some() && !entry.stream)
            });
        if has_dynamic {
            self.refresh_dynamic_plugin_ui();
        }
    }

    /// Start refresh commands for dirty entries that have none in flight.
    pub(crate) fn pump_plugin_ui_refreshes(&mut self) {
        let dirty: Vec<(String, String)> =
            self.state.plugin_ui_refresh_dirty.iter().cloned().collect();
        for key in dirty {
            let (plugin_id, ui_id) = (&key.0, &key.1);
            if self
                .state
                .plugin_ui_refresh_in_flight
                .values()
                .any(|in_flight| *in_flight == key)
            {
                continue;
            }
            let Some(plugin) = self
                .state
                .installed_plugins
                .get(plugin_id.as_str())
                .cloned()
            else {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                self.state.plugin_ui_dynamic.remove(&key);
                continue;
            };
            if !plugin.enabled || !plugin_manifest_available(&plugin) {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                self.state.plugin_ui_dynamic.remove(&key);
                continue;
            }
            let Some(entry) = plugin.ui.iter().find(|entry| entry.id == *ui_id) else {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                self.state.plugin_ui_dynamic.remove(&key);
                continue;
            };
            if entry.stream {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                continue;
            }
            let Some(command) = entry.command.clone() else {
                self.state.plugin_ui_refresh_dirty.remove(&key);
                continue;
            };
            let mut context = self.current_plugin_context("plugin-ui-refresh");
            context.invocation_source = Some("plugin_ui_refresh".to_string());
            let started = self.start_plugin_command(&plugin, None, None, command, &context, None);
            self.state.plugin_ui_refresh_dirty.remove(&key);
            match started {
                Ok(log) => {
                    self.state
                        .plugin_ui_refresh_in_flight
                        .insert(log.log_id, key);
                }
                Err((code, message)) => {
                    tracing::warn!(
                        plugin_id = %plugin_id,
                        ui_id = %ui_id,
                        code,
                        "dynamic plugin ui refresh failed to start: {message}"
                    );
                }
            }
        }
    }

    /// Route a finished plugin command to the dynamic UI cache when it was a
    /// refresh command. Returns true when the rows projection changed.
    pub(crate) fn handle_plugin_ui_refresh_finished(
        &mut self,
        log_id: &str,
        exit_code: Option<i32>,
        stdout: &str,
    ) -> bool {
        let Some(key) = self.state.plugin_ui_refresh_in_flight.remove(log_id) else {
            return false;
        };
        let rows = if exit_code == Some(0) {
            let actions = self
                .state
                .installed_plugins
                .get(&key.0)
                .map(|plugin| plugin.actions.as_slice())
                .unwrap_or(&[]);
            parse_plugin_ui_rows(&key.0, &key.1, stdout, actions)
        } else {
            Vec::new()
        };
        let changed = self.state.plugin_ui_dynamic.get(&key) != Some(&rows);
        if changed {
            self.state.plugin_ui_dynamic.insert(key, rows);
        }
        changed
    }
}

/// Enabled dynamic sidebar entries (plugin_id, ui_id) for the current platform.
pub(super) fn dynamic_ui_entries(plugins: &InstalledPluginRegistry) -> Vec<(String, String)> {
    ui_entries(plugins, Some(false))
}

fn ui_entries(plugins: &InstalledPluginRegistry, stream: Option<bool>) -> Vec<(String, String)> {
    let host = current_platform();
    let mut entries = Vec::new();
    for plugin in plugins.values() {
        if !plugin.enabled || !plugin_manifest_available(plugin) {
            continue;
        }
        for entry in &plugin.ui {
            if entry.surface != PluginUiSurface::Sidebar
                || entry.command.is_none()
                || stream.is_some_and(|stream| entry.stream != stream)
            {
                continue;
            }
            if let Some(platforms) = effective_platforms(&entry.platforms, &plugin.platforms) {
                if !platforms.contains(&host) {
                    continue;
                }
            }
            entries.push((plugin.plugin_id.clone(), entry.id.clone()));
        }
    }
    entries.sort();
    entries
}

/// Parse the JSON rows printed by a dynamic UI refresh command. Invalid output
/// yields no rows; row action ids that the plugin never declared are stripped
/// so the row renders but stays non-clickable.
pub(super) fn parse_plugin_ui_rows(
    plugin_id: &str,
    ui_id: &str,
    stdout: &str,
    actions: &[PluginManifestAction],
) -> Vec<EndpointPluginUiItem> {
    match try_parse_plugin_ui_rows(plugin_id, ui_id, stdout, actions) {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                plugin_id,
                ui_id,
                "dynamic plugin ui rows failed to parse: {error}"
            );
            Vec::new()
        }
    }
}

fn try_parse_plugin_ui_rows(
    plugin_id: &str,
    ui_id: &str,
    stdout: &str,
    actions: &[PluginManifestAction],
) -> Result<Vec<EndpointPluginUiItem>, serde_json::Error> {
    if stdout.trim_start().starts_with('{') {
        let declaration: crate::protocol::widgets::Declaration = serde_json::from_str(stdout)?;
        if !declaration.validate(|id| actions.iter().any(|action| action.id == id)) {
            return Ok(Vec::new());
        }
        return Ok(vec![EndpointPluginUiItem {
            plugin_id: plugin_id.to_owned(),
            ui_id: ui_id.to_owned(),
            declaration: Some(declaration),
            title: String::new(),
            action: None,
            tab_id: None,
            workspace_id: None,
            pane_id: None,
            segments: Vec::new(),
        }]);
    }
    let parsed: Vec<PluginUiRowJson> = serde_json::from_str(stdout)?;
    Ok(parsed
        .into_iter()
        .take(PLUGIN_UI_MAX_ROWS)
        .filter_map(|row| {
            let title = row.title.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let title = title.chars().take(PLUGIN_UI_TITLE_MAX_CHARS).collect();
            // Reserved affordances retain Core-defined semantics; plugins own
            // their visible placement, not the action implementation.
            let action = row.action.filter(|action| {
                action.starts_with("host:") || actions.iter().any(|declared| &declared.id == action)
            });
            Some(EndpointPluginUiItem {
                declaration: None,
                plugin_id: plugin_id.to_string(),
                ui_id: ui_id.to_string(),
                title,
                action,
                tab_id: row.tab_id.filter(|id| !id.is_empty()),
                workspace_id: row.workspace_id.filter(|id| !id.is_empty()),
                pane_id: row.pane_id.filter(|id| !id.is_empty()),
                segments: row
                    .segments
                    .into_iter()
                    .take(PLUGIN_UI_MAX_SEGMENTS)
                    .filter(|segment| {
                        // Whitespace-only text is legitimate for elastic
                        // segments such as `leader`, whose width the client
                        // computes; role is what makes a segment significant.
                        !segment.role.trim().is_empty()
                    })
                    .map(|segment| EndpointPluginUiSegment {
                        text: segment
                            .text
                            .chars()
                            .take(PLUGIN_UI_TITLE_MAX_CHARS)
                            .collect(),
                        role: segment.role.chars().take(64).collect(),
                    })
                    .collect(),
            })
        })
        .collect())
}

/// Whether a dynamic plugin ui entry benefits from a refresh after this event.
pub(crate) fn event_touches_plugin_ui(event: &crate::api::schema::EventKind) -> bool {
    use crate::api::schema::EventKind as E;
    matches!(
        event,
        E::WorkspaceCreated
            | E::WorkspaceUpdated
            | E::WorkspaceMetadataUpdated
            | E::WorkspaceClosed
            | E::WorkspaceReordered
            | E::WorkspaceRenamed
            | E::WorkspaceMoved
            | E::WorkspaceFocused
            | E::WorktreeCreated
            | E::WorktreeOpened
            | E::WorktreeRemoved
            | E::TabCreated
            | E::TabClosed
            | E::TabRenamed
            | E::TabMoved
            | E::TabFocused
            | E::PaneAgentDetected
            | E::PaneAgentStatusChanged
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ui_json_defaults_to_one_shot_and_requires_stream_command() {
        let entry: crate::api::schema::PluginManifestUi =
            serde_json::from_str(r#"{"id":"tree","surface":"sidebar","title":"Static"}"#).unwrap();
        assert!(!entry.stream);
        for command in ["", r#", "command": []"#] {
            let json = format!(r#"{{"id":"tree","surface":"sidebar","stream":true{command}}}"#);
            assert!(serde_json::from_str::<crate::api::schema::PluginManifestUi>(&json).is_err());
        }
        let entry: crate::api::schema::PluginManifestUi = serde_json::from_str(
            r#"{"id":"tree","surface":"sidebar","stream":true,"command":["watch"]}"#,
        )
        .unwrap();
        assert!(entry.stream);
    }

    #[tokio::test]
    async fn plugin_ui_stream_line_size_and_newline_boundary_are_enforced() {
        let mut line = Vec::new();
        let mut exact = vec![b' '; PLUGIN_UI_STREAM_MAX_BYTES - 3];
        exact.extend_from_slice(b"[]\n");
        let mut reader = exact.as_slice();
        assert!(read_plugin_ui_line(&mut reader, &mut line).await.unwrap());
        assert_eq!(line.len(), PLUGIN_UI_STREAM_MAX_BYTES);
        assert_eq!(
            try_parse_plugin_ui_rows("demo", "tree", std::str::from_utf8(&line).unwrap(), &[])
                .unwrap(),
            Vec::new()
        );
        assert!(!read_plugin_ui_line(&mut reader, &mut line).await.unwrap());
        let oversized = vec![b'x'; PLUGIN_UI_STREAM_MAX_BYTES + 100];
        let mut reader = oversized.as_slice();
        assert!(read_plugin_ui_line(&mut reader, &mut line).await.is_err());
        assert_eq!(line.len(), PLUGIN_UI_STREAM_MAX_BYTES + 1);
        let mut incomplete = b"[]".as_slice();
        assert!(read_plugin_ui_line(&mut incomplete, &mut line)
            .await
            .is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn plugin_ui_stream_resolves_registered_action_envelopes() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let (stop, forward) = start_plugin_ui_stream(
            stream_command(
                r#"sleep 0.2; printf '{"herdr":"action_result","id":"plugin-log-99","ok":true}\n[{"title":"a"}]\n'; sleep 30"#,
            ),
            ("demo".into(), "tree".into()),
            1,
            vec![],
            tx,
            None,
        )
        .unwrap();
        let forward = forward.expect("stdin forward handle");
        forward.insert_pending("plugin-log-99");
        let finished = next_stream_event(&mut rx).await;
        let crate::events::AppEvent::PluginCommandFinished {
            log_id,
            exit_code,
            error,
            ..
        } = finished
        else {
            panic!("expected action result event, got {finished:?}");
        };
        assert_eq!(log_id, "plugin-log-99");
        assert_eq!(exit_code, Some(0));
        assert!(error.is_none());
        let crate::events::AppEvent::PluginUiStreamRows { rows, .. } =
            next_stream_event(&mut rx).await
        else {
            panic!("expected rows after envelope");
        };
        assert_eq!(rows[0].title, "a");
        drop(stop);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn plugin_ui_stream_skips_unknown_and_unrelated_envelopes() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let (stop, _forward) = start_plugin_ui_stream(
            stream_command(
                r#"printf '{"herdr":"action_result","id":"plugin-log-1","ok":true}\n{"herdr":"future_kind","x":1}\n[{"title":"kept"}]\n'; sleep 30"#,
            ),
            ("demo".into(), "tree".into()),
            1,
            vec![],
            tx,
            None,
        )
        .unwrap();
        let crate::events::AppEvent::PluginUiStreamRows { rows, .. } =
            next_stream_event(&mut rx).await
        else {
            panic!("expected rows despite envelopes, got stream finish");
        };
        assert_eq!(rows[0].title, "kept");
        drop(stop);
    }

    #[cfg(unix)]
    fn stream_command(script: &str) -> std::process::Command {
        let mut command = std::process::Command::new("sh");
        command
            .args(["-c", script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command
    }

    #[cfg(unix)]
    async fn next_stream_event(
        rx: &mut tokio::sync::mpsc::Receiver<crate::events::AppEvent>,
    ) -> crate::events::AppEvent {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap()
    }

    #[cfg(unix)]
    async fn wait_reaped(pid: i32) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if unsafe { libc::kill(pid, 0) } == -1 {
                    assert_eq!(
                        std::io::Error::last_os_error().raw_os_error(),
                        Some(libc::ESRCH)
                    );
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("stream child must be killed and reaped");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn plugin_ui_stream_cancellation_reaps_child_even_when_delivery_is_backpressured() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let lease = std::sync::Arc::new(std::fs::File::open("/dev/null").unwrap());
        let weak = std::sync::Arc::downgrade(&lease);
        let (stop, _forward) = start_plugin_ui_stream(stream_command(r#"printf '[{"title":"%s"}]\n' "$$"; i=0; while :; do printf '[{"title":"row%s"}]\n' "$i"; i=$((i+1)); done"#), ("demo".into(), "tree".into()), 1, vec![], tx, Some(lease)).unwrap();
        let crate::events::AppEvent::PluginUiStreamRows { rows, .. } =
            next_stream_event(&mut rx).await
        else {
            panic!("expected first projection");
        };
        let pid = rows[0].title.parse().unwrap();
        drop(stop);
        wait_reaped(pid).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while weak.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("installation lease released after reaping");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn plugin_ui_stream_drains_stderr_and_emits_only_changed_projections() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let script = r#"dd if=/dev/zero bs=65536 count=4 >&2 2>/dev/null; printf '[{"title":"one","action":"focus","tab_id":"t1","workspace_id":"w1"}]\n'; printf '[{"title":"one","action":"focus","tab_id":"t1","workspace_id":"w1"}]\n'; printf '[{"title":"two"}]\n'"#;
        let (_stop, _forward) = start_plugin_ui_stream(
            stream_command(script),
            ("demo".into(), "tree".into()),
            9,
            vec![action("focus")],
            tx,
            None,
        )
        .unwrap();
        let crate::events::AppEvent::PluginUiStreamRows {
            generation, rows, ..
        } = next_stream_event(&mut rx).await
        else {
            panic!("expected first projection");
        };
        assert_eq!(generation, 9);
        assert_eq!(rows[0].title, "one");
        assert_eq!(rows[0].action.as_deref(), Some("focus"));
        assert_eq!(rows[0].tab_id.as_deref(), Some("t1"));
        assert_eq!(rows[0].workspace_id.as_deref(), Some("w1"));
        let crate::events::AppEvent::PluginUiStreamRows { rows, .. } =
            next_stream_event(&mut rx).await
        else {
            panic!("expected changed projection, not duplicate");
        };
        assert_eq!(rows[0].title, "two");
        assert!(matches!(
            next_stream_event(&mut rx).await,
            crate::events::AppEvent::PluginUiStreamFinished { error: None, .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn plugin_ui_stream_invalid_and_oversized_lines_stop_and_reap_producer() {
        for invalid in ["printf 'not-json\\n'", "head -c 65537 /dev/zero"] {
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let script = format!(r#"printf '[{{"title":"%s"}}]\n' "$$"; {invalid}; exec sleep 30"#);
            let (_stop, _forward) = start_plugin_ui_stream(
                stream_command(&script),
                ("demo".into(), "tree".into()),
                1,
                vec![],
                tx,
                None,
            )
            .unwrap();
            let crate::events::AppEvent::PluginUiStreamRows { rows, .. } =
                next_stream_event(&mut rx).await
            else {
                panic!("expected pid projection");
            };
            let pid = rows[0].title.parse().unwrap();
            assert!(matches!(
                next_stream_event(&mut rx).await,
                crate::events::AppEvent::PluginUiStreamFinished { error: Some(_), .. }
            ));
            wait_reaped(pid).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn plugin_ui_stream_registry_lifecycle_rejects_old_producers_and_never_refreshes_one_shot(
    ) {
        use crate::api::schema::{PluginLinkParams, PluginSetEnabledParams, PluginUnlinkParams};
        let root = std::env::temp_dir().join(format!(
            "trailboss-stream-lifecycle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = r#"id = "example.stream-lifecycle"
name = "Stream lifecycle"
version = "0.1.0"
min_herdr_version = "0.7.0"
[[ui]]
id = "tree"
surface = "sidebar"
stream = true
command = ["sh", "-c", "printf '[{\"title\":\"%s\"}]\\n' \"$$\"; exec sleep 30"]
"#;
        std::fs::write(root.join("trailboss-plugin.toml"), manifest).unwrap();
        let (_, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let link = PluginLinkParams {
            path: root.display().to_string(),
            enabled: true,
            source: None,
        };
        let response = app.handle_plugin_link("link".into(), link.clone());
        assert!(
            serde_json::from_str::<crate::api::schema::SuccessResponse>(&response).is_ok(),
            "{response}"
        );
        let key = ("example.stream-lifecycle".to_string(), "tree".to_string());
        let first = app.plugin_ui_streams[&key].generation;
        let ev = next_stream_event(&mut app.event_rx).await;
        let crate::events::AppEvent::PluginUiStreamRows { ref rows, .. } = ev else {
            panic!("expected projection");
        };
        let first_pid = rows[0].title.parse().unwrap();
        assert!(app.handle_internal_event_with_render_impact(ev));
        assert!(!app.handle_plugin_ui_stream_rows(
            key.clone(),
            first,
            app.state.plugin_ui_dynamic[&key].clone()
        ));
        app.refresh_dynamic_plugin_ui();
        let plugin = app.state.installed_plugins[&key.0].clone();
        let context = app.current_plugin_context("action-test");
        app.start_plugin_command(
            &plugin,
            Some("focus".into()),
            None,
            vec!["true".into()],
            &context,
            None,
        )
        .unwrap();
        let completed = next_stream_event(&mut app.event_rx).await;
        assert!(matches!(
            completed,
            crate::events::AppEvent::PluginCommandFinished { .. }
        ));
        app.handle_internal_event_with_render_impact(completed);
        app.reconcile_plugin_ui();
        assert_eq!(app.plugin_ui_streams[&key].generation, first);
        assert!(app.state.plugin_ui_refresh_dirty.is_empty());
        assert!(app.state.plugin_ui_refresh_in_flight.is_empty());
        assert_eq!(app.state.plugin_commands_in_flight, 0);
        app.handle_plugin_link("reload".into(), link);
        let second = app.plugin_ui_streams[&key].generation;
        assert_ne!(first, second);
        let old_rows = parse(r#"[{"title":"obsolete"}]"#, &[]);
        assert!(!app.handle_plugin_ui_stream_rows(key.clone(), first, old_rows));
        assert!(!app.handle_plugin_ui_stream_finished(key.clone(), first, None));
        let ev = next_stream_event(&mut app.event_rx).await;
        let crate::events::AppEvent::PluginUiStreamRows { ref rows, .. } = ev else {
            panic!("expected replacement projection");
        };
        let second_pid = rows[0].title.parse().unwrap();
        app.handle_internal_event_with_render_impact(ev);
        wait_reaped(first_pid).await;
        app.handle_plugin_disable(
            "disable".into(),
            PluginSetEnabledParams {
                plugin_id: key.0.clone(),
            },
        );
        assert!(app.plugin_ui_streams.is_empty());
        assert!(app.state.plugin_ui_dynamic.is_empty());
        wait_reaped(second_pid).await;
        app.handle_plugin_enable(
            "enable".into(),
            PluginSetEnabledParams {
                plugin_id: key.0.clone(),
            },
        );
        let ev = next_stream_event(&mut app.event_rx).await;
        let crate::events::AppEvent::PluginUiStreamRows { rows, .. } = ev else {
            panic!("expected enabled projection");
        };
        let third_pid = rows[0].title.parse().unwrap();
        app.handle_plugin_unlink(
            "unlink".into(),
            PluginUnlinkParams {
                plugin_id: key.0.clone(),
            },
        );
        assert!(app.plugin_ui_streams.is_empty());
        wait_reaped(third_pid).await;
        app.handle_plugin_link(
            "drop-link".into(),
            PluginLinkParams {
                path: root.display().to_string(),
                enabled: true,
                source: None,
            },
        );
        let crate::events::AppEvent::PluginUiStreamRows { rows, .. } =
            next_stream_event(&mut app.event_rx).await
        else {
            panic!("expected projection before App drop");
        };
        let fourth_pid = rows[0].title.parse().unwrap();
        drop(app);
        wait_reaped(fourth_pid).await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn plugin_ui_stream_cancellation_terminates_background_descendants() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let (stop, _forward) = start_plugin_ui_stream(stream_command(r#"sleep 30 & descendant=$!; printf '[{"title":"%s","workspace_id":"%s"}]\n' "$$" "$descendant"; wait"#), ("demo".into(), "tree".into()), 1, vec![], tx, None).unwrap();
        let crate::events::AppEvent::PluginUiStreamRows { rows, .. } =
            next_stream_event(&mut rx).await
        else {
            panic!("expected process identity");
        };
        let parent = rows[0].title.parse().unwrap();
        let descendant = rows[0].workspace_id.as_deref().unwrap();
        drop(stop);
        wait_reaped(parent).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match std::fs::read_to_string(format!("/proc/{descendant}/stat")) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Ok(stat)
                        if stat
                            .rsplit_once(") ")
                            .is_some_and(|(_, rest)| rest.starts_with('Z')) =>
                    {
                        break
                    }
                    _ => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
                }
            }
        })
        .await
        .expect("background descendant must no longer run");
    }

    fn action(id: &str) -> PluginManifestAction {
        PluginManifestAction {
            id: id.to_string(),
            title: id.to_string(),
            description: None,
            contexts: Vec::new(),
            platforms: None,
            command: vec!["true".to_string()],
        }
    }

    fn parse(stdout: &str, actions: &[PluginManifestAction]) -> Vec<EndpointPluginUiItem> {
        parse_plugin_ui_rows("demo", "tree", stdout, actions)
    }

    #[test]
    fn parses_rows_with_context_and_declared_actions() {
        let rows = parse(
            r#"[{"title":"▾ main","action":"open","workspace_id":"ws1"},
                {"title":"  ● build","action":"focus","tab_id":"t2","workspace_id":"ws1"}]"#,
            &[action("open"), action("focus")],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "▾ main");
        assert_eq!(rows[0].action.as_deref(), Some("open"));
        assert_eq!(rows[0].workspace_id.as_deref(), Some("ws1"));
        assert_eq!(rows[1].tab_id.as_deref(), Some("t2"));
        assert_eq!(rows[1].ui_id, "tree");
    }

    #[test]
    fn undeclared_row_action_is_stripped() {
        let rows = parse(r#"[{"title":"row","action":"missing"}]"#, &[action("open")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, None);
    }

    #[test]
    fn reserved_host_actions_pass_without_manifest_declaration() {
        let rows = parse(r#"[{"title":" new","action":"host:new"}]"#, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action.as_deref(), Some("host:new"));
    }

    #[test]
    fn invalid_json_or_blank_titles_yield_no_rows() {
        assert!(parse("not json", &[action("open")]).is_empty());
        assert!(parse(r#"[{"title":"  "}]"#, &[action("open")]).is_empty());
    }

    #[test]
    fn caps_row_count_and_title_length() {
        let many: String = (0..500)
            .map(|i| format!(r#"{{"title":"row{i}"}},"#))
            .collect();
        let stdout = format!("[{}{}]", many, r#"{"title":"last"}"#);
        assert_eq!(parse(&stdout, &[]).len(), PLUGIN_UI_MAX_ROWS);
        let long = "x".repeat(500);
        let rows = parse(&format!(r#"[{{"title":"{long}"}}]"#), &[]);
        assert_eq!(rows[0].title.chars().count(), PLUGIN_UI_TITLE_MAX_CHARS);
    }

    #[test]
    fn parses_semantic_segments_with_caps_and_filters() {
        let rows = parse(
            r#"[{"title":"infilla ···· ● ◉",
                "segments":[
                    {"text":"infilla","role":"label"},
                    {"text":"●","role":"status-working"},
                    {"text":"◉","role":"status-blocked"},
                    {"text":" ","role":"leader"},
                    {"text":"hidden","role":" "}
                ]}]"#,
            &[],
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].segments,
            vec![
                EndpointPluginUiSegment {
                    text: "infilla".into(),
                    role: "label".into(),
                },
                EndpointPluginUiSegment {
                    text: "●".into(),
                    role: "status-working".into(),
                },
                EndpointPluginUiSegment {
                    text: "◉".into(),
                    role: "status-blocked".into(),
                },
                EndpointPluginUiSegment {
                    text: " ".into(),
                    role: "leader".into(),
                },
            ],
            "blank roles are dropped; whitespace-only elastic leader text survives"
        );

        let long: String = "x".repeat(500);
        let segments: String = (0..24)
            .map(|i| format!(r#"{{"text":"s{i}","role":"label"}},"#))
            .collect();
        let rows = parse(
            &format!(
                r#"[{{"title":"t","segments":[{segments}{{"text":"{long}","role":"muted"}}]}}]"#
            ),
            &[],
        );
        assert_eq!(
            rows[0].segments.len(),
            PLUGIN_UI_MAX_SEGMENTS,
            "segment count capped"
        );
        assert_eq!(
            rows[0].segments.last().expect("capped").text,
            "s15".to_string(),
            "cap keeps the leading segments"
        );

        let rows = parse(
            &format!(r#"[{{"title":"t","segments":[{{"text":"{long}","role":"muted"}}]}}]"#),
            &[],
        );
        assert_eq!(
            rows[0].segments[0].text.chars().count(),
            PLUGIN_UI_TITLE_MAX_CHARS,
            "segment text clamped to the title cap"
        );
    }

    #[test]
    fn workspace_tab_and_agent_events_touch_plugin_ui() {
        use crate::api::schema::EventKind as E;
        assert!(event_touches_plugin_ui(&E::TabFocused));
        assert!(event_touches_plugin_ui(&E::PaneAgentStatusChanged));
        assert!(event_touches_plugin_ui(&E::WorkspaceFocused));
        assert!(!event_touches_plugin_ui(&E::PaneUpdated));
    }
}
