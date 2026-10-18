//! Cooperative OMP restart. Only the owning Core may write the recovered shell.
use std::time::Duration;

use crate::api::schema::{
    AgentInfo, AgentRestartIdleOutcome as Outcome, AgentRestartIdleResult, AgentStatus,
    ResponseResult,
};
use crate::app::App;
use crate::events::AppEvent;

use super::responses::{encode_error, encode_success};

const STEP_TIMEOUT: Duration = Duration::from_secs(3);
const EXIT_TIMEOUT: Duration = Duration::from_secs(15);
const START_TIMEOUT: Duration = Duration::from_secs(20);
const FRAME_LIMIT: usize = 8192;

#[derive(Debug, Clone)]
pub(crate) struct RestartIdleCheck {
    agent: AgentInfo,
    shell_pid: u32,
    input_epoch: u64,
    session_path: String,
    resume: Option<ResumeShell>,
    verify_since: Option<std::time::Instant>,
    expires_at: std::time::Instant,
}

#[derive(Debug, Clone)]
struct ResumeShell {
    shell: ProcessIdentity,
    old_agent: ProcessIdentity,
    cwd: std::path::PathBuf,
    session_file: (u64, u64),
    command: String,
}

#[derive(Debug)]
struct Candidate {
    check: RestartIdleCheck,
    input_barrier: std::sync::mpsc::Receiver<std::io::Result<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProcessIdentity {
    pub(super) pid: u32,
    pub(super) started: u64,
}

impl App {
    pub(super) fn start_restart_idle_agents(
        &mut self,
        id: String,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let mut results = Vec::new();
        let mut candidates = Vec::new();
        for agent in self.collect_agent_infos() {
            let candidate = (|| {
                initial_skip_reason(&agent)?;
                if !cfg!(target_os = "linux") {
                    return Err("unsupported_platform");
                }
                let session_path = exact_session_path(&agent).ok_or("unresolved_session_path")?;
                let resolved = self
                    .resolve_terminal_target(&agent.terminal_id)
                    .map_err(|_| "pane_unavailable")?;
                let runtime = self
                    .lookup_runtime_sender(resolved.ws_idx, resolved.pane_id)
                    .ok_or("pane_unavailable")?;
                let shell_pid = runtime.child_pid().ok_or("shell_identity_unavailable")?;
                // A zero-byte FIFO barrier waits out prior queued/delayed input
                // without typing keys or submitting a model message.
                let input_barrier = runtime
                    .queue_user_input_submission(
                        bytes::Bytes::new(),
                        bytes::Bytes::new(),
                        Duration::ZERO,
                        None,
                    )
                    .map_err(|_| "input_barrier_unavailable")?;
                Ok::<_, &str>(Candidate {
                    check: RestartIdleCheck {
                        agent: agent.clone(),
                        shell_pid,
                        input_epoch: runtime.input_epoch(),
                        session_path,
                        resume: None,
                        verify_since: None,
                        expires_at: std::time::Instant::now() + STEP_TIMEOUT,
                    },
                    input_barrier,
                })
            })();
            match candidate {
                Ok(candidate) => candidates.push(candidate),
                Err(reason) => results.push(row(&agent, Outcome::Skipped, reason)),
            }
        }
        let tx = self.event_tx.clone();
        let failure_tx = respond_to.clone();
        let failure_id = id.clone();
        let spawned = crate::thread_spawn::spawn_named("trailboss-restart-idle", move || {
            // Independent panes continue even if one endpoint or shutdown fails.
            for candidate in candidates {
                results.push(restart_one(candidate, &tx));
            }
            let restarted = results
                .iter()
                .filter(|r| r.outcome == Outcome::Restarted)
                .count();
            let skipped = results
                .iter()
                .filter(|r| r.outcome == Outcome::Skipped)
                .count();
            let failed = results.len() - restarted - skipped;
            let response = encode_success(id, ResponseResult::AgentRestartIdle { results });
            let _ = respond_to.send(response);
            let _ = send_event(
                &tx,
                AppEvent::AgentRestartIdleFinished {
                    restarted,
                    skipped,
                    failed,
                },
            );
        });
        if let Err(err) = spawned {
            let _ = failure_tx.send(encode_error(
                failure_id,
                "agent_restart_idle_failed",
                err.to_string(),
            ));
        }
    }

    pub(super) fn check_restart_idle_agent(
        &mut self,
        check: RestartIdleCheck,
    ) -> Result<(), String> {
        if std::time::Instant::now() >= check.expires_at {
            return Err("core_check_expired".into());
        }
        let resolved = self
            .resolve_terminal_target(&check.agent.terminal_id)
            .map_err(|_| "pane_identity_changed")?;
        let current = self
            .pane_metadata(resolved.ws_idx, resolved.pane_id)
            .ok_or("pane_unavailable")?;
        if current.terminal_id != check.agent.terminal_id
            || current.pane_id != check.agent.pane_id
            || current.workspace_id != check.agent.workspace_id
            || current.tab_id != check.agent.tab_id
        {
            return Err("pane_identity_changed".into());
        }
        if current.focused {
            return Err("focused".into());
        }
        // session_shutdown may release the reporter's identity. A different
        // identity is never allowed; absence is allowed only for shell recovery.
        if current.agent_session != check.agent.agent_session
            && !(check.resume.is_some() && current.agent_session.is_none())
        {
            return Err("session_identity_changed".into());
        }
        let runtime = self
            .lookup_runtime_sender(resolved.ws_idx, resolved.pane_id)
            .ok_or("pane_unavailable")?;
        if runtime.child_pid() != Some(check.shell_pid) {
            return Err("shell_identity_changed".into());
        }
        if runtime.input_epoch() != check.input_epoch {
            return Err("input_changed".into());
        }
        if let Some(since) = check.verify_since {
            let terminal = self
                .state
                .terminals
                .get(current.terminal_id.as_str())
                .ok_or("pane_unavailable")?;
            let hook = terminal
                .hook_authority
                .as_ref()
                .ok_or("new_reporter_not_ready")?;
            if hook.source != "herdr:omp"
                || hook.reported_at < since
                || hook
                    .session_ref
                    .as_ref()
                    .map(|session| session.value.as_str())
                    != Some(check.session_path.as_str())
            {
                return Err("new_reporter_session_unconfirmed".into());
            }
            return Ok(());
        }
        if let Some(resume) = check.resume {
            // Check again on the Core thread, immediately before enqueueing bytes.
            // No screen-text heuristic stands in for the real foreground shell.
            recovered_shell(&resume.shell, &resume.old_agent)?;
            if shell_cwd(resume.shell.pid).as_ref() != Some(&resume.cwd) {
                return Err("shell_cwd_changed".into());
            }
            if session_file_identity(&check.session_path) != Some(resume.session_file) {
                return Err("saved_session_file_changed".into());
            }
            runtime
                .try_send_bytes(bytes::Bytes::from(resume.command))
                .map_err(|_| "resume_input_failed")?;
        } else {
            let agent = self
                .agent_info(resolved.ws_idx, resolved.pane_id)
                .ok_or("agent_identity_changed")?;
            initial_skip_reason(&agent).map_err(str::to_owned)?;
        }
        Ok(())
    }

    pub(super) fn show_restart_idle_summary(
        &mut self,
        restarted: usize,
        skipped: usize,
        failed: usize,
    ) {
        let _ = self.handle_notification_show(
            "restart-idle-summary".into(),
            crate::api::schema::NotificationShowParams {
                title: "Restart idle agents".into(),
                body: Some(format!(
                    "{restarted} restarted, {skipped} skipped, {failed} failed"
                )),
                position: None,
                sound: crate::api::schema::NotificationShowSound::None,
            },
        );
    }
}

fn initial_skip_reason(agent: &AgentInfo) -> Result<(), &'static str> {
    if agent.agent.as_deref() != Some("omp") {
        return Err("not_omp");
    }
    if agent.focused {
        return Err("focused");
    }
    // Done is the public projection of Idle before a human has viewed it.
    if !matches!(agent.agent_status, AgentStatus::Idle | AgentStatus::Done) {
        return Err("not_idle");
    }
    if agent.launch_pending {
        return Err("launch_pending");
    }
    Ok(())
}

pub(super) fn exact_session_path(agent: &AgentInfo) -> Option<String> {
    let session = agent.agent_session.as_ref()?;
    (session.source == "herdr:omp"
        && session.agent == "omp"
        && session.kind == crate::agent_resume::AgentSessionRefKind::Path
        && std::path::Path::new(&session.value).is_absolute())
    .then(|| session.value.clone())
}

fn row(agent: &AgentInfo, outcome: Outcome, reason: impl Into<String>) -> AgentRestartIdleResult {
    AgentRestartIdleResult {
        pane_id: agent.pane_id.clone(),
        outcome,
        reason: reason.into(),
        session_path: exact_session_path(agent),
    }
}

fn send_event(tx: &tokio::sync::mpsc::Sender<AppEvent>, mut event: AppEvent) -> Result<(), String> {
    let deadline = std::time::Instant::now() + STEP_TIMEOUT;
    loop {
        match tx.try_send(event) {
            Ok(()) => return Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(returned)) => event = returned,
            Err(_) => return Err("core_closed".into()),
        }
        if std::time::Instant::now() >= deadline {
            return Err("core_busy".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn core_check(
    tx: &tokio::sync::mpsc::Sender<AppEvent>,
    mut check: RestartIdleCheck,
) -> Result<(), String> {
    check.expires_at = std::time::Instant::now() + STEP_TIMEOUT;
    let (respond_to, rx) = std::sync::mpsc::channel();
    send_event(tx, AppEvent::AgentRestartIdleCheck { check, respond_to })?;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "core_check_timeout".to_owned())?
}

#[cfg(not(target_os = "linux"))]
fn restart_one(
    candidate: Candidate,
    _: &tokio::sync::mpsc::Sender<AppEvent>,
) -> AgentRestartIdleResult {
    row(
        &candidate.check.agent,
        Outcome::Skipped,
        "unsupported_platform",
    )
}

#[cfg(target_os = "linux")]
fn restart_one(
    candidate: Candidate,
    tx: &tokio::sync::mpsc::Sender<AppEvent>,
) -> AgentRestartIdleResult {
    let check = candidate.check;
    if !matches!(
        candidate.input_barrier.recv_timeout(STEP_TIMEOUT),
        Ok(Ok(()))
    ) {
        return row(&check.agent, Outcome::Skipped, "pending_terminal_input");
    }
    let prepared = prepare(&check).and_then(|prepared| {
        core_check(tx, check.clone())?;
        Ok(prepared)
    });
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(reason) => return row(&check.agent, Outcome::Skipped, reason),
    };
    match request_shutdown(&prepared, &check.session_path) {
        Ok(ControlReply { status, reason, .. }) if status == "skipped" => {
            return row(&check.agent, Outcome::Skipped, reason)
        }
        Ok(ControlReply { status, .. }) if status == "accepted" => {}
        Ok(_) => return row(&check.agent, Outcome::Failed, "invalid_control_response"),
        Err(reason) if reason == "unsupported_control_endpoint" => {
            return row(&check.agent, Outcome::Skipped, reason)
        }
        Err(reason) => return row(&check.agent, Outcome::Failed, reason),
    }
    let deadline = std::time::Instant::now() + EXIT_TIMEOUT;
    loop {
        if recovered_shell(&prepared.shell, &prepared.omp).is_ok() {
            break;
        }
        if process_identity(prepared.shell.pid).as_ref() != Some(&prepared.shell) {
            return row(
                &check.agent,
                Outcome::Failed,
                "shell_identity_changed_after_shutdown",
            );
        }
        if std::time::Instant::now() >= deadline {
            return row(
                &check.agent,
                Outcome::Failed,
                "graceful_shutdown_or_shell_recovery_timeout",
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Revalidate Core ownership, focus, persisted session and editor/input activity.
    let mut resume_check = check.clone();
    resume_check.resume = Some(ResumeShell {
        shell: prepared.shell.clone(),
        old_agent: prepared.omp.clone(),
        cwd: prepared.cwd.clone(),
        session_file: prepared.session_file,
        command: prepared.command.clone(),
    });
    let resumed_at = std::time::Instant::now();
    if let Err(reason) = core_check(tx, resume_check) {
        return row(&check.agent, Outcome::Failed, reason);
    }
    let deadline = std::time::Instant::now() + START_TIMEOUT;
    loop {
        if let Ok(new) = prepare(&check) {
            if new.omp != prepared.omp && new.pane_identity == prepared.pane_identity {
                let mut verify = check.clone();
                verify.input_epoch = verify.input_epoch.wrapping_add(1);
                verify.verify_since = Some(resumed_at);
                if core_check(tx, verify).is_ok() {
                    return row(&check.agent, Outcome::Restarted, "exact_session_resumed");
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return row(&check.agent, Outcome::Failed, "resume_verification_timeout");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[derive(serde::Deserialize)]
pub(super) struct ControlReply {
    pub(super) status: String,
    pub(super) reason: String,
    #[serde(default)]
    pub(super) turn_id: Option<String>,
}

/// The identity facts a reporter control op needs; restart preparation adds
/// resume specifics on top of this shared target.
#[cfg(target_os = "linux")]
pub(super) struct ControlTarget {
    pub(super) shell: ProcessIdentity,
    pub(super) omp: ProcessIdentity,
    pub(super) pane_identity: String,
}

#[cfg(target_os = "linux")]
impl From<&Prepared> for ControlTarget {
    fn from(prepared: &Prepared) -> Self {
        ControlTarget {
            shell: prepared.shell.clone(),
            omp: prepared.omp.clone(),
            pane_identity: prepared.pane_identity.clone(),
        }
    }
}

/// Resolves the live OMP process behind a pane for non-restart control ops:
/// same process-tree admission as restart, without resume argv reconstruction.
#[cfg(target_os = "linux")]
pub(super) fn prepare_control_target(
    shell_pid: u32,
    // Kept for signature parity with restart preparation; admission of the
    // session path happens reporter-side, where the live session is known.
    _session_path: &str,
) -> Result<ControlTarget, String> {
    let shell = process_identity(shell_pid).ok_or("shell_identity_unavailable")?;
    let job = crate::detect::foreground_job(shell.pid).ok_or("foreground_unavailable")?;
    let (agent, _, pid) =
        crate::detect::identify_agent_process_in_job(&job).ok_or("foreground_not_omp")?;
    if agent != crate::detect::Agent::Omp || pid == shell.pid || pid != job.process_group_id {
        return Err("foreground_not_root_omp".into());
    }
    let omp = process_identity(pid).ok_or("agent_identity_unavailable")?;
    if process_stat(pid).and_then(|s| s.get(1)?.parse::<u32>().ok()) != Some(shell.pid) {
        return Err("omp_not_original_shell_child".into());
    }
    // Session-path admission is enforced by the reporter against its live
    // session; a local existence probe would misclassify socketless (v11)
    // reporters before the structured-unavailable mapping can apply.
    let pane_identity =
        process_environment_value(pid, "HERDR_PANE_ID").ok_or("pane_environment_unavailable")?;
    Ok(ControlTarget {
        shell,
        omp,
        pane_identity,
    })
}

#[cfg(target_os = "linux")]
struct Prepared {
    shell: ProcessIdentity,
    omp: ProcessIdentity,
    pane_identity: String,
    cwd: std::path::PathBuf,
    session_file: (u64, u64),
    command: String,
}

#[cfg(target_os = "linux")]
fn prepare(check: &RestartIdleCheck) -> Result<Prepared, String> {
    let shell = process_identity(check.shell_pid).ok_or("shell_identity_unavailable")?;
    let shell_executable = std::fs::read_link(format!("/proc/{}/exe", shell.pid))
        .map_err(|_| "shell_executable_unavailable")?;
    if !matches!(
        shell_executable.file_name().and_then(|name| name.to_str()),
        Some("bash" | "zsh" | "fish" | "sh" | "dash" | "ksh" | "mksh" | "ash")
    ) {
        return Err("unsupported_shell_syntax".into());
    }
    let job = crate::detect::foreground_job(shell.pid).ok_or("foreground_unavailable")?;
    let (agent, _, pid) =
        crate::detect::identify_agent_process_in_job(&job).ok_or("foreground_not_omp")?;
    if agent != crate::detect::Agent::Omp || pid == shell.pid || pid != job.process_group_id {
        return Err("foreground_not_root_omp".into());
    }
    let omp = process_identity(pid).ok_or("agent_identity_unavailable")?;
    for process in &job.processes {
        if process.pid != pid && !is_zombie(process.pid) && !is_resident_worker(process.pid, pid) {
            return Err("other_live_foreground_work".into());
        }
    }
    let mut argv = process_argv(pid).ok_or("original_argv_unavailable")?;
    if argv
        .first()
        .and_then(|arg| std::path::Path::new(arg).file_name())
        .and_then(|name| name.to_str())
        != Some("omp")
    {
        return Err("unsupported_original_launcher".into());
    }
    let executable = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(|_| "original_executable_unavailable")?;
    if !executable.is_file() {
        return Err("original_executable_unavailable".into());
    }
    argv[0] = executable
        .to_str()
        .ok_or("original_executable_not_utf8")?
        .to_owned();
    let pane_identity =
        process_environment_value(pid, "HERDR_PANE_ID").ok_or("pane_environment_unavailable")?;
    let cwd = shell_cwd(shell.pid).ok_or("shell_cwd_unavailable")?;
    let session_file =
        session_file_identity(&check.session_path).ok_or("saved_session_file_unavailable")?;
    if !compatible_launch_environment(pid, shell.pid)? {
        return Err("unsupported_environment_override".into());
    }
    let argv = exact_resume_argv(&argv, &check.session_path)?;
    let command = format!(
        "{}\r",
        argv.iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(Prepared {
        shell,
        omp,
        pane_identity,
        cwd,
        session_file,
        command,
    })
}

fn exact_resume_argv(argv: &[String], session_path: &str) -> Result<Vec<String>, String> {
    if argv.is_empty()
        || argv[0].is_empty()
        || !std::path::Path::new(session_path).is_absolute()
        || session_path.chars().any(char::is_control)
        || argv.iter().any(|arg| arg.chars().any(char::is_control))
    {
        return Err("unsafe_original_argv".into());
    }
    let mut output = Vec::with_capacity(argv.len() + 2);
    let mut args = argv.iter();
    output.push(args.next().unwrap().clone());
    while let Some(arg) = args.next() {
        let (name, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(name, value)| (name, Some(value)));
        if name == "--resume" || name == "-r" {
            let value = inline.or_else(|| args.next().map(String::as_str));
            if value.is_none_or(|value| value.is_empty() || value.starts_with('-')) {
                return Err("unsafe_original_resume_argument".into());
            }
        } else if matches!(
            name,
            "--model"
                | "--smol"
                | "--slow"
                | "--provider"
                | "--system-prompt"
                | "--system-prompt-template"
                | "--append-system-prompt"
                | "--profile"
                | "--alias"
                | "--cwd"
                | "--config"
                | "--add-dir"
                | "--session-dir"
                | "--models"
                | "--tools"
                | "--thinking"
                | "--service-tier"
                | "--hook"
                | "--extension"
                | "-e"
                | "--skills"
                | "--max-time"
                | "--approval-mode"
                | "--plugin-dir"
        ) {
            output.push(arg.clone());
            if inline.is_none() {
                let value = args
                    .next()
                    .filter(|value| !value.starts_with('-'))
                    .ok_or("unsafe_original_option_value")?;
                output.push(value.clone());
            }
        } else if inline.is_none()
            && matches!(
                name,
                "--allow-home"
                    | "--no-tools"
                    | "--no-lsp"
                    | "--no-pty"
                    | "--hide-thinking"
                    | "--advisor"
                    | "--external-thinking"
                    | "--no-extensions"
                    | "--no-skills"
                    | "--no-rules"
                    | "--no-title"
                    | "--print-thoughts"
                    | "--auto-approve"
            )
        {
            output.push(arg.clone());
        } else {
            // In particular, never replay positional initial messages, @files,
            // --restore, --goal, --continue, import/export or one-shot modes.
            return Err("unsupported_original_launch_argument".into());
        }
    }
    output.extend(["--resume".into(), session_path.into()]);
    Ok(output)
}

fn shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(target_os = "linux")]
fn shell_cwd(pid: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(target_os = "linux"))]
fn shell_cwd(_: u32) -> Option<std::path::PathBuf> {
    None
}

#[cfg(target_os = "linux")]
fn session_file_identity(path: &str) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    metadata
        .is_file()
        .then_some((metadata.dev(), metadata.ino()))
}

#[cfg(not(target_os = "linux"))]
fn session_file_identity(_: &str) -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
fn compatible_launch_environment(agent_pid: u32, shell_pid: u32) -> Result<bool, String> {
    let agent = process_environment(agent_pid).ok_or("agent_environment_unavailable")?;
    let shell = process_environment(shell_pid).ok_or("shell_environment_unavailable")?;
    Ok(agent
        .iter()
        .chain(shell.iter())
        .filter(|(key, _)| {
            key.starts_with("PI_")
                || key.starts_with("OMP_")
                || key.starts_with("HERDR_")
                || key.starts_with("XDG_")
        })
        .all(|(key, _)| agent.get(key) == shell.get(key)))
}

#[cfg(target_os = "linux")]
fn process_environment(pid: u32) -> Option<std::collections::HashMap<String, String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            let (key, value) = entry.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn process_stat(pid: u32) -> Option<Vec<String>> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    Some(
        stat.rsplit_once(") ")?
            .1
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(target_os = "linux")]
fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    let fields = process_stat(pid)?;
    if matches!(fields.first()?.as_str(), "Z" | "X") {
        return None;
    }
    Some(ProcessIdentity {
        pid,
        started: fields.get(19)?.parse().ok()?,
    })
}

#[cfg(target_os = "linux")]
fn is_zombie(pid: u32) -> bool {
    process_stat(pid).is_some_and(|s| s.first().is_some_and(|state| state == "Z" || state == "X"))
}

#[cfg(target_os = "linux")]
fn process_argv(pid: u32) -> Option<Vec<String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if bytes.last() != Some(&0) {
        return None;
    }
    bytes[..bytes.len() - 1]
        .split(|byte| *byte == 0)
        .map(|arg| String::from_utf8(arg.to_vec()).ok())
        .collect()
}

#[cfg(target_os = "linux")]
fn process_environment_value(pid: u32, key: &str) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let prefix = format!("{key}=");
    bytes.split(|byte| *byte == 0).find_map(|entry| {
        std::str::from_utf8(entry)
            .ok()?
            .strip_prefix(&prefix)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

#[cfg(target_os = "linux")]
fn recovered_shell(shell: &ProcessIdentity, old_agent: &ProcessIdentity) -> Result<(), String> {
    if process_identity(shell.pid).as_ref() != Some(shell) {
        return Err("shell_identity_changed".into());
    }
    if process_identity(old_agent.pid).as_ref() == Some(old_agent) {
        return Err("original_agent_still_running".into());
    }
    let fields = process_stat(shell.pid).ok_or("shell_identity_unavailable")?;
    let pgrp = fields.get(2).ok_or("shell_foreground_unavailable")?;
    let foreground = fields.get(5).ok_or("shell_foreground_unavailable")?;
    if pgrp != foreground || !crate::detect::pane_shell_is_idle(shell.pid) {
        return Err("same_shell_not_foreground".into());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn recovered_shell(_: &ProcessIdentity, _: &ProcessIdentity) -> Result<(), String> {
    Err("unsupported_platform".into())
}

#[cfg(target_os = "linux")]
fn request_shutdown(prepared: &Prepared, session_path: &str) -> Result<ControlReply, String> {
    control_request(
        prepared,
        serde_json::json!({ "operation": "restart-if-idle", "pid": prepared.omp.pid, "pane_id": prepared.pane_identity, "session_path": session_path }),
    )
}

#[cfg(target_os = "linux")]
fn control_request(
    prepared: &Prepared,
    request: serde_json::Value,
) -> Result<ControlReply, String> {
    control_roundtrip(&ControlTarget::from(prepared), request, FRAME_LIMIT, STEP_TIMEOUT)
}

/// One newline-framed request/response exchange with a reporter control
/// socket, revalidating both process identities before connecting.
#[cfg(target_os = "linux")]
pub(super) fn control_roundtrip(
    target: &ControlTarget,
    request: serde_json::Value,
    frame_limit: usize,
    timeout: Duration,
) -> Result<ControlReply, String> {
    use std::io::Write;
    let uid = unsafe { libc::geteuid() };
    if process_identity(target.omp.pid).as_ref() != Some(&target.omp)
        || process_identity(target.shell.pid).as_ref() != Some(&target.shell)
    {
        return Err("process_identity_changed".into());
    }
    let directory = std::path::PathBuf::from(format!("/tmp/trailboss-omp-control-{uid}"));
    let path = checked_control_socket(&directory, target.omp.pid, uid)?;
    // Unix local connect is bounded by nonblocking connect + poll below.
    let mut stream =
        connect_control(&path).map_err(|err| format!("control_connect_failed: {err}"))?;
    verify_peer(&stream, target.omp.pid)?;
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + timeout;
    let mut frame = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    frame.push(b'\n');
    if frame.len() > frame_limit {
        return Err("control_request_too_large".into());
    }
    let mut written = 0;
    while written < frame.len() {
        if std::time::Instant::now() >= deadline {
            return Err("control_timeout".into());
        }
        match stream.write(&frame[written..]) {
            Ok(0) => return Err("control_closed_before_request".into()),
            Ok(count) => written += count,
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(err) => return Err(format!("control_write_failed: {err}")),
        }
    }
    read_control_reply(&mut stream, deadline)
}

#[cfg(target_os = "linux")]
fn checked_control_socket(
    directory: &std::path::Path,
    pid: u32,
    uid: u32,
) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let metadata =
        std::fs::symlink_metadata(directory).map_err(|_| "unsupported_control_endpoint")?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
        return Err("unsafe_control_directory".into());
    }
    let path = directory.join(format!("{pid}.sock"));
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| "unsupported_control_endpoint")?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err("unsafe_control_socket".into());
    }
    Ok(path)
}

#[cfg(target_os = "linux")]
fn read_control_reply(
    stream: &mut std::os::unix::net::UnixStream,
    deadline: std::time::Instant,
) -> Result<ControlReply, String> {
    use std::io::Read;
    let mut reply = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        if std::time::Instant::now() >= deadline {
            return Err("control_timeout".into());
        }
        match stream.read(&mut buffer) {
            Ok(0) => return Err("control_closed_before_reply".into()),
            Ok(count) => {
                if reply.len() + count > FRAME_LIMIT {
                    return Err("control_reply_too_large".into());
                }
                reply.extend_from_slice(&buffer[..count]);
                if let Some(end) = reply.iter().position(|byte| *byte == b'\n') {
                    if end + 1 != reply.len() {
                        return Err("invalid_control_frame".into());
                    }
                    return serde_json::from_slice(&reply)
                        .map_err(|_| "invalid_control_response".into());
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(err) => return Err(format!("control_read_failed: {err}")),
        }
    }
}

#[cfg(target_os = "linux")]
fn verify_peer(stream: &std::os::unix::net::UnixStream, pid: u32) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&peer) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut peer as *mut _ as *mut libc::c_void,
            &mut length,
        )
    };
    if result != 0 || peer.uid != unsafe { libc::geteuid() } || peer.pid != pid as i32 {
        return Err("control_peer_identity_mismatch".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_resident_worker(pid: u32, root: u32) -> bool {
    let Some(argv) = process_argv(pid) else {
        return false;
    };
    if !argv.iter().any(|arg| arg == "__omp_worker_mnemopi_embed") {
        return false;
    }
    let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
        return false;
    };
    if std::fs::read_link(format!("/proc/{root}/exe"))
        .ok()
        .as_ref()
        != Some(&exe)
    {
        return false;
    }
    let mut parent = pid;
    for _ in 0..16 {
        let Some(next) = process_stat(parent).and_then(|s| s.get(1)?.parse::<u32>().ok()) else {
            return false;
        };
        if next == root {
            return true;
        }
        if next <= 1 || next == parent {
            return false;
        }
        parent = next;
    }
    false
}

#[cfg(target_os = "linux")]
fn connect_control(path: &std::path::Path) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        return Err(std::io::Error::other("control path too long"));
    }
    for (dest, source) in address.sun_path.iter_mut().zip(bytes) {
        *dest = *source as libc::c_char;
    }
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            &address as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    };
    if result < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(err);
        }
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, STEP_TIMEOUT.as_millis() as i32) } <= 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "control connect timed out",
            ));
        }
        let mut error: libc::c_int = 0;
        let mut len = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut error as *mut _ as *mut libc::c_void,
                &mut len,
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if error != 0 {
            return Err(std::io::Error::from_raw_os_error(error));
        }
    }
    let stream = std::os::unix::net::UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    fn agent() -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": "term-test", "agent": "omp", "agent_status": "idle",
            "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p1",
            "focused": false, "revision": 0,
            "agent_session": { "source": "herdr:omp", "agent": "omp", "kind": "path", "value": "/custom/session.jsonl" }
        })).unwrap()
    }

    #[test]
    fn resumes_the_exact_custom_path_while_preserving_original_launch_flags() {
        let path = "/custom sessions/owner's conversation.jsonl";
        let original = argv(&[
            "/usr/bin/omp",
            "--provider",
            "openai",
            "--model=gpt-test",
            "--profile",
            "work",
            "--session-dir",
            "/custom sessions",
            "--config",
            "one=two",
            "--extension",
            "/custom/plugin.ts",
            "--resume=old-uuid",
            "--no-lsp",
        ]);
        assert_eq!(
            exact_resume_argv(&original, path).unwrap(),
            argv(&[
                "/usr/bin/omp",
                "--provider",
                "openai",
                "--model=gpt-test",
                "--profile",
                "work",
                "--session-dir",
                "/custom sessions",
                "--config",
                "one=two",
                "--extension",
                "/custom/plugin.ts",
                "--no-lsp",
                "--resume",
                path
            ])
        );
        assert_eq!(
            shell_quote(path),
            "'/custom sessions/owner'\\''s conversation.jsonl'"
        );
        assert_eq!(
            exact_resume_argv(&argv(&["omp", "-r", "old"]), path).unwrap(),
            argv(&["omp", "--resume", path])
        );
    }

    #[test]
    fn refuses_launches_that_would_replay_a_prompt_or_resume_unfinished_work() {
        for args in [
            argv(&["omp", "do more work"]),
            argv(&["omp", "@instructions.md"]),
            argv(&["omp", "--continue"]),
            argv(&["omp", "--restore", "old"]),
            argv(&["omp", "--restore=old"]),
            argv(&["omp", "--goal", "work"]),
            argv(&["omp", "--print"]),
            argv(&["omp", "--mode=rpc"]),
            argv(&["omp", "--from-codex"]),
            argv(&["omp", "--no-session"]),
            argv(&["omp", "--api-key", "private"]),
            argv(&["omp", "--unknown"]),
            argv(&["omp", "--resume"]),
            argv(&["omp", "--resume", "--model", "x"]),
            argv(&["omp", "--model"]),
            argv(&["omp", "--model", "x\ninjected"]),
        ] {
            assert!(
                exact_resume_argv(&args, "/custom/saved.jsonl").is_err(),
                "{args:?}"
            );
        }
        assert!(exact_resume_argv(&argv(&["omp"]), "guessed-uuid").is_err());
    }

    #[test]
    fn unsafe_cached_states_and_focused_agents_are_explicit_skips() {
        let mut info = agent();
        assert_eq!(initial_skip_reason(&info), Ok(()));
        info.agent_status = AgentStatus::Done;
        assert_eq!(initial_skip_reason(&info), Ok(()));
        for status in [
            AgentStatus::Working,
            AgentStatus::Waiting,
            AgentStatus::Blocked,
            AgentStatus::Unknown,
        ] {
            info.agent_status = status;
            assert_eq!(initial_skip_reason(&info), Err("not_idle"));
        }
        info.agent_status = AgentStatus::Idle;
        info.focused = true;
        assert_eq!(initial_skip_reason(&info), Err("focused"));
        info.focused = false;
        info.agent = Some("codex".into());
        assert_eq!(initial_skip_reason(&info), Err("not_omp"));
        info.agent = Some("omp".into());
        info.launch_pending = true;
        assert_eq!(initial_skip_reason(&info), Err("launch_pending"));
    }

    #[test]
    fn session_identity_requires_an_actual_omp_reported_absolute_path() {
        let mut info = agent();
        assert_eq!(
            exact_session_path(&info).as_deref(),
            Some("/custom/session.jsonl")
        );
        info.agent_session.as_mut().unwrap().kind = crate::agent_resume::AgentSessionRefKind::Id;
        assert_eq!(exact_session_path(&info), None);
        info.agent_session.as_mut().unwrap().kind = crate::agent_resume::AgentSessionRefKind::Path;
        info.agent_session.as_mut().unwrap().value = "relative.jsonl".into();
        assert_eq!(exact_session_path(&info), None);
        info.agent_session.as_mut().unwrap().value = "/custom/session.jsonl".into();
        info.agent_session.as_mut().unwrap().source = "untrusted:omp".into();
        assert_eq!(exact_session_path(&info), None);
    }

    #[test]
    fn restart_api_uses_current_session_empty_params_and_per_pane_outcomes() {
        use crate::api::schema::{EmptyParams, Method, Request, SuccessResponse};
        let request = Request {
            id: "restart".into(),
            method: Method::AgentRestartIdle(EmptyParams::default()),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["method"], "agent.restart_idle");
        assert_eq!(value["params"], serde_json::json!({}));
        assert_eq!(serde_json::from_value::<Request>(value).unwrap(), request);
        let rows = vec![
            row(&agent(), Outcome::Skipped, "unsupported_control_endpoint"),
            row(&agent(), Outcome::Failed, "resume_verification_timeout"),
            row(&agent(), Outcome::Restarted, "exact_session_resumed"),
        ];
        let encoded = encode_success(
            "restart".into(),
            ResponseResult::AgentRestartIdle {
                results: rows.clone(),
            },
        );
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["result"]["type"], "agent_restart_idle");
        let response: SuccessResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            response.result,
            ResponseResult::AgentRestartIdle { results: rows }
        );
    }

    #[tokio::test]
    async fn attempted_input_invalidates_a_deferred_shell_operation() {
        let (runtime, _rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        let initial = runtime.input_epoch();
        runtime
            .try_send_bytes(bytes::Bytes::from_static(b"draft"))
            .unwrap();
        assert_ne!(runtime.input_epoch(), initial);
        let after_key = runtime.input_epoch();
        runtime.try_send_paste("more draft".into()).unwrap();
        assert_ne!(runtime.input_epoch(), after_key);
    }

    fn app() -> App {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            rx,
            crate::api::EventHub::default(),
        )
    }

    #[test]
    fn expired_core_checks_cannot_later_write_a_shell() {
        let mut app = app();
        let check = RestartIdleCheck {
            agent: agent(),
            shell_pid: 1,
            input_epoch: 0,
            session_path: "/custom/session.jsonl".into(),
            resume: None,
            verify_since: None,
            expires_at: std::time::Instant::now() - Duration::from_secs(1),
        };
        assert_eq!(
            app.check_restart_idle_agent(check),
            Err("core_check_expired".into())
        );
    }

    #[test]
    fn deferred_restart_returns_a_structured_empty_result_without_blocking_core() {
        let mut app = app();
        app.state.workspaces.clear();
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(app.handle_deferred_agent_api_request(
            crate::api::schema::Request {
                id: "empty".into(),
                method: crate::api::schema::Method::AgentRestartIdle(
                    crate::api::schema::EmptyParams::default()
                ),
            },
            tx
        ));
        let response = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let response: crate::api::schema::SuccessResponse =
            serde_json::from_str(&response).unwrap();
        assert_eq!(
            response.result,
            ResponseResult::AgentRestartIdle {
                results: Vec::new()
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cooperative_reply_has_a_total_deadline_and_rejects_unbounded_or_truncated_frames() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        for bytes in [
            b"{\"status\":\"accepted\",\"reason\":\"idle\"}".to_vec(),
            vec![b'x'; FRAME_LIMIT + 1],
        ] {
            let (mut client, mut server) = UnixStream::pair().unwrap();
            client.set_nonblocking(true).unwrap();
            server.write_all(&bytes).unwrap();
            drop(server);
            assert!(read_control_reply(
                &mut client,
                std::time::Instant::now() + Duration::from_secs(1)
            )
            .is_err());
        }
        let (mut client, _server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        assert_eq!(
            read_control_reply(
                &mut client,
                std::time::Instant::now() + Duration::from_millis(30)
            )
            .err()
            .as_deref(),
            Some("control_timeout")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn accepted_and_skipped_control_replies_preserve_the_authoritative_reason() {
        use std::io::Write;
        for status in ["accepted", "skipped"] {
            let (mut client, mut server) = std::os::unix::net::UnixStream::pair().unwrap();
            client.set_nonblocking(true).unwrap();
            writeln!(
                server,
                "{{\"status\":\"{status}\",\"reason\":\"pending_delivery\"}}"
            )
            .unwrap();
            let reply = read_control_reply(
                &mut client,
                std::time::Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
            assert_eq!(reply.status, status);
            assert_eq!(reply.reason, "pending_delivery");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_control_endpoint_is_explicitly_unsupported_without_sending_input() {
        let directory = std::env::temp_dir().join(format!(
            "trailboss-missing-control-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert_eq!(
            checked_control_socket(&directory, std::process::id(), unsafe { libc::geteuid() })
                .err()
                .as_deref(),
            Some("unsupported_control_endpoint")
        );
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn one_unsafe_agent_does_not_prevent_other_panes_from_receiving_outcomes() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = ["other", "working", "waiting"]
            .into_iter()
            .map(crate::workspace::Workspace::test_new)
            .collect();
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        for (index, (agent, state)) in [
            (crate::detect::Agent::Codex, crate::detect::AgentState::Idle),
            (
                crate::detect::Agent::Omp,
                crate::detect::AgentState::Working,
            ),
            (
                crate::detect::Agent::Omp,
                crate::detect::AgentState::Waiting,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let workspace = &app.state.workspaces[index];
            let pane = workspace.tabs[0].root_pane;
            let terminal_id = workspace.terminal_id(pane).unwrap().clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .set_detected_state(Some(agent), state);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        app.start_restart_idle_agents("unsafe-panes".into(), tx);
        let response: crate::api::schema::SuccessResponse =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(2)).unwrap()).unwrap();
        let ResponseResult::AgentRestartIdle { results } = response.result else {
            panic!("restart result expected");
        };
        assert_eq!(results.len(), 3);
        assert_eq!(results.iter().filter(|r| r.reason == "not_omp").count(), 1);
        assert_eq!(results.iter().filter(|r| r.reason == "not_idle").count(), 2);
        assert!(results.iter().all(|r| r.outcome == Outcome::Skipped));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn control_socket_requires_private_owned_real_directory_and_socket() {
        use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};
        let directory = std::env::temp_dir().join(format!(
            "trailboss-control-permissions-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let socket = directory.join("1.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let uid = unsafe { libc::geteuid() };
        assert_eq!(checked_control_socket(&directory, 1, uid).unwrap(), socket);
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            checked_control_socket(&directory, 1, uid).err().as_deref(),
            Some("unsafe_control_socket")
        );
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&socket, directory.join("2.sock")).unwrap();
        assert_eq!(
            checked_control_socket(&directory, 2, uid).err().as_deref(),
            Some("unsafe_control_socket")
        );
        let alias = directory.with_extension("alias");
        symlink(&directory, &alias).unwrap();
        assert_eq!(
            checked_control_socket(&alias, 1, uid).err().as_deref(),
            Some("unsafe_control_directory")
        );
        std::fs::remove_file(alias).unwrap();
        drop(listener);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
