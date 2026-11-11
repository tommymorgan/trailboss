//! Cooperative OMP and Pi restart. Only the owning Core may write the recovered shell.
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

/// Agents restart-idle can resume: each reports its session as
/// `herdr:<name>` and runs a reporter control socket under its own directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RestartAgentKind {
    Omp,
    Pi,
}

impl RestartAgentKind {
    fn of(agent: &AgentInfo) -> Option<Self> {
        match agent.agent.as_deref() {
            Some("omp") => Some(Self::Omp),
            Some("pi") => Some(Self::Pi),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Omp => "omp",
            Self::Pi => "pi",
        }
    }

    fn report_source(self) -> &'static str {
        match self {
            Self::Omp => "herdr:omp",
            Self::Pi => "herdr:pi",
        }
    }

    #[cfg(target_os = "linux")]
    fn detected(self) -> crate::detect::Agent {
        match self {
            Self::Omp => crate::detect::Agent::Omp,
            Self::Pi => crate::detect::Agent::Pi,
        }
    }
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
            let kind = RestartAgentKind::of(&check.agent).ok_or("agent_identity_changed")?;
            if hook.source != kind.report_source()
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
    if RestartAgentKind::of(agent).is_none() {
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
    let kind = RestartAgentKind::of(agent)?;
    let session = agent.agent_session.as_ref()?;
    (session.source == kind.report_source()
        && session.agent == kind.name()
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
    send_event(
        tx,
        AppEvent::AgentRestartIdleCheck {
            check: Box::new(check),
            respond_to,
        },
    )?;
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
        if recovered_shell(&prepared.shell, &prepared.agent).is_ok() {
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
        old_agent: prepared.agent.clone(),
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
            if new.agent != prepared.agent && new.pane_identity == prepared.pane_identity {
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
    #[serde(default)]
    pub(super) result: Option<serde_json::Value>,
    #[serde(default)]
    pub(super) error: Option<serde_json::Value>,
}

/// The identity facts a reporter control op needs; restart preparation adds
/// resume specifics on top of this shared target.
#[cfg(target_os = "linux")]
pub(super) struct ControlTarget {
    pub(super) kind: RestartAgentKind,
    pub(super) shell: ProcessIdentity,
    pub(super) agent: ProcessIdentity,
    pub(super) pane_identity: String,
}

#[cfg(target_os = "linux")]
impl From<&Prepared> for ControlTarget {
    fn from(prepared: &Prepared) -> Self {
        ControlTarget {
            kind: prepared.kind,
            shell: prepared.shell.clone(),
            agent: prepared.agent.clone(),
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
        kind: RestartAgentKind::Omp,
        shell,
        agent: omp,
        pane_identity,
    })
}

#[cfg(target_os = "linux")]
struct Prepared {
    kind: RestartAgentKind,
    shell: ProcessIdentity,
    agent: ProcessIdentity,
    pane_identity: String,
    cwd: std::path::PathBuf,
    session_file: (u64, u64),
    command: String,
}

#[cfg(target_os = "linux")]
fn prepare(check: &RestartIdleCheck) -> Result<Prepared, String> {
    let kind = RestartAgentKind::of(&check.agent).ok_or("not_omp")?;
    let shell = process_identity(check.shell_pid).ok_or("shell_identity_unavailable")?;
    let shell_executable = std::fs::read_link(format!("/proc/{}/exe", shell.pid))
        .map_err(|_| "shell_executable_unavailable")?;
    let shell_name = shell_executable.file_name().and_then(|name| name.to_str());
    if !matches!(
        shell_name,
        Some("bash" | "zsh" | "fish" | "sh" | "dash" | "ksh" | "mksh" | "ash")
    ) {
        return Err("unsupported_shell_syntax".into());
    }
    let fish = shell_name == Some("fish");
    let job = crate::detect::foreground_job(shell.pid).ok_or("foreground_unavailable")?;
    let (agent, _, pid) = crate::detect::identify_agent_process_in_job(&job)
        .ok_or_else(|| format!("foreground_not_{}", kind.name()))?;
    if agent != kind.detected() || pid == shell.pid || pid != job.process_group_id {
        return Err(format!("foreground_not_root_{}", kind.name()));
    }
    let agent = process_identity(pid).ok_or("agent_identity_unavailable")?;
    for process in &job.processes {
        if process.pid != pid
            && !is_zombie(process.pid)
            && !(kind == RestartAgentKind::Omp && is_resident_worker(process.pid, pid))
        {
            return Err("other_live_foreground_work".into());
        }
    }
    // Pi overwrites /proc/<pid>/cmdline through process.title, so its launch
    // arguments come from its own reporter over the PID-verified socket.
    let omp_argv = match kind {
        RestartAgentKind::Omp => Some(omp_original_argv(pid)?),
        RestartAgentKind::Pi => None,
    };
    let pane_identity =
        process_environment_value(pid, "HERDR_PANE_ID").ok_or("pane_environment_unavailable")?;
    let cwd = shell_cwd(shell.pid).ok_or("shell_cwd_unavailable")?;
    let session_file =
        session_file_identity(&check.session_path).ok_or("saved_session_file_unavailable")?;
    let restorations = launch_environment(pid, shell.pid)?;
    let argv = match omp_argv {
        Some(argv) => exact_resume_argv(&argv, &check.session_path)?,
        None => {
            let target = ControlTarget {
                kind,
                shell: shell.clone(),
                agent: agent.clone(),
                pane_identity: pane_identity.clone(),
            };
            pi_resume_argv(
                &pi_original_argv(&target, &check.session_path)?,
                &check.session_path,
            )?
        }
    };
    verify_launcher_restorations(kind, &restorations, argv.get(1).map_or("", String::as_str))?;
    let command = resume_command(&restorations, &argv, fish);
    Ok(Prepared {
        kind,
        shell,
        agent,
        pane_identity,
        cwd,
        session_file,
        command,
    })
}

#[cfg(target_os = "linux")]
fn omp_original_argv(pid: u32) -> Result<Vec<String>, String> {
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
    Ok(argv)
}

/// Launch facts the Pi reporter returns from its own `process.argv` and
/// `process.execPath` (`launch-info` control operation).
#[cfg(target_os = "linux")]
#[derive(serde::Deserialize)]
struct PiLaunchInfo {
    argv: Vec<String>,
    exec_path: String,
}

/// Reads the live Pi's launch through its reporter and accepts it only when
/// it is `<node> <existing cli script> ...` run by the PID-verified process.
#[cfg(target_os = "linux")]
fn pi_original_argv(target: &ControlTarget, session_path: &str) -> Result<Vec<String>, String> {
    let reply = control_roundtrip(
        target,
        serde_json::json!({ "operation": "launch-info", "pid": target.agent.pid, "pane_id": target.pane_identity, "session_path": session_path }),
        FRAME_LIMIT,
        STEP_TIMEOUT,
    )?;
    match reply.status.as_str() {
        "skipped" => return Err(reply.reason),
        "accepted" => {}
        _ => return Err("invalid_control_response".into()),
    }
    let launch: PiLaunchInfo = reply
        .result
        .and_then(|result| serde_json::from_value(result).ok())
        .ok_or("invalid_launch_info")?;
    let executable = std::fs::read_link(format!("/proc/{}/exe", target.agent.pid))
        .map_err(|_| "original_executable_unavailable")?;
    if executable.as_os_str() != launch.exec_path.as_str()
        || launch.argv.first() != Some(&launch.exec_path)
        || !launch
            .argv
            .get(1)
            .is_some_and(|script| std::fs::metadata(script).is_ok_and(|meta| meta.is_file()))
    {
        return Err("unsupported_original_launcher".into());
    }
    Ok(launch.argv)
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

/// Rebuilds `<node> <pi cli> <allowed flags> --session <exact path>`. Pi has
/// no `--flag=value` form for its own options, so any `=` spelling is refused.
fn pi_resume_argv(argv: &[String], session_path: &str) -> Result<Vec<String>, String> {
    if argv.len() < 2
        || argv[0].is_empty()
        || argv[1].is_empty()
        || !std::path::Path::new(session_path).is_absolute()
        || session_path.chars().any(char::is_control)
        || argv.iter().any(|arg| arg.chars().any(char::is_control))
    {
        return Err("unsafe_original_argv".into());
    }
    let mut output = argv[..2].to_vec();
    let mut args = argv[2..].iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // Earlier selectors are replaced by the exact saved session path.
            "--session" | "--session-id" | "--fork" => {
                args.next()
                    .filter(|value| !value.is_empty() && !value.starts_with('-'))
                    .ok_or("unsafe_original_resume_argument")?;
            }
            "--continue" | "-c" | "--resume" | "-r" => {}
            "--provider"
            | "--model"
            | "--models"
            | "--thinking"
            | "--system-prompt"
            | "--append-system-prompt"
            | "--tools"
            | "-t"
            | "--exclude-tools"
            | "-xt"
            | "--extension"
            | "-e"
            | "--skill"
            | "--prompt-template"
            | "--theme"
            | "--use-theme"
            | "--session-dir"
            | "--tui-mode"
            | "--name"
            | "-n" => {
                let value = args
                    .next()
                    .filter(|value| !value.starts_with('-'))
                    .ok_or("unsafe_original_option_value")?;
                output.extend([arg.clone(), value.clone()]);
            }
            "--no-tools"
            | "-nt"
            | "--no-builtin-tools"
            | "-nbt"
            | "--no-extensions"
            | "-ne"
            | "--no-skills"
            | "-ns"
            | "--no-prompt-templates"
            | "-np"
            | "--no-themes"
            | "--no-mcp"
            | "--no-context-files"
            | "-nc"
            | "--approve"
            | "-a"
            | "--no-approve"
            | "-na"
            | "--offline"
            | "--verbose" => output.push(arg.clone()),
            // In particular, never replay initial messages, @files, -p, --mode,
            // --export, --no-session, --api-key or extension-registered flags.
            _ => return Err("unsupported_original_launch_argument".into()),
        }
    }
    output.extend(["--session".into(), session_path.into()]);
    Ok(output)
}

fn shell_quote(arg: &str, fish: bool) -> String {
    if fish {
        return format!("'{}'", arg.replace('\\', "\\\\").replace('\'', "\\'"));
    }
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

/// Pi's managed launcher (`~/.local/bin/pi`) exports this before it execs the
/// release, so a launcher-started Pi has it while its shell does not.
#[cfg(target_os = "linux")]
const PI_MANAGED_INSTALL_ROOT: &str = "PI_MANAGED_INSTALL_ROOT";

#[cfg(target_os = "linux")]
fn launch_environment(agent_pid: u32, shell_pid: u32) -> Result<Vec<(String, String)>, String> {
    let agent = process_environment(agent_pid).ok_or("agent_environment_unavailable")?;
    let shell = process_environment(shell_pid).ok_or("shell_environment_unavailable")?;
    launch_environment_restorations(&agent, &shell)
}

/// Returns the agent variables the relaunch must set again because the
/// original launcher set them. Any other agent/shell difference in the
/// variables that steer Pi, OMP, Herdr or XDG lookups is an override the
/// shell cannot reproduce.
#[cfg(target_os = "linux")]
fn launch_environment_restorations(
    agent: &std::collections::HashMap<String, String>,
    shell: &std::collections::HashMap<String, String>,
) -> Result<Vec<(String, String)>, String> {
    let mut restorations = Vec::new();
    let mut keys: Vec<&String> = agent
        .keys()
        .chain(shell.keys())
        .filter(|key| {
            key.starts_with("PI_")
                || key.starts_with("OMP_")
                || key.starts_with("HERDR_")
                || key.starts_with("XDG_")
        })
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        match (agent.get(key), shell.get(key)) {
            (agent_value, shell_value) if agent_value == shell_value => {}
            (Some(value), None) if key == PI_MANAGED_INSTALL_ROOT => {
                restorations.push((key.clone(), value.clone()));
            }
            _ => return Err("unsupported_environment_override".into()),
        }
    }
    if let Some(path) = launcher_prepended_path(agent).filter(|_| !restorations.is_empty()) {
        restorations.insert(0, ("PATH".to_owned(), path));
    }
    Ok(restorations)
}

/// The managed launcher puts `${XDG_DATA_HOME:-$HOME/.local/share}/pi-node/current/bin`
/// at the front of PATH when the Pi installer placed Node.js there. Returns
/// that directory when the agent's PATH starts with it. The shell's own PATH
/// is not compared: `/proc` shows only the PATH the shell started with, not
/// what its startup files exported, so the relaunch prepends to the shell's
/// current `$PATH` instead.
#[cfg(target_os = "linux")]
fn launcher_prepended_path(agent: &std::collections::HashMap<String, String>) -> Option<String> {
    let data_home = match agent.get("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(value) => value.clone(),
        None => format!("{}/.local/share", agent.get("HOME")?),
    };
    let pi_node = format!("{data_home}/pi-node/current/bin");
    let rest = agent.get("PATH")?.strip_prefix(&pi_node)?;
    (rest.is_empty() || rest.starts_with(':')).then_some(pi_node)
}

/// A restored install root must be the one Pi's launcher derived for the
/// running release: the release script lives under `<root>/releases`, the same
/// check Pi applies before it trusts the variable. The relaunch runs the
/// release script directly rather than through the launcher, so it sets the
/// launcher's variables itself; a restored PATH prefix is accepted only alongside
/// this verified root, because it is restored only for launcher-started Pi.
#[cfg(target_os = "linux")]
fn verify_launcher_restorations(
    kind: RestartAgentKind,
    restorations: &[(String, String)],
    script: &str,
) -> Result<(), String> {
    for (key, root) in restorations {
        if key == "PATH" {
            continue;
        }
        let owned = key == PI_MANAGED_INSTALL_ROOT
            && kind == RestartAgentKind::Pi
            && std::fs::canonicalize(std::path::Path::new(root).join("releases"))
                .ok()
                .zip(std::fs::canonicalize(script).ok())
                .is_some_and(|(releases, script)| script.starts_with(releases));
        if !owned {
            return Err("unsupported_environment_override".into());
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn resume_command(restorations: &[(String, String)], argv: &[String], fish: bool) -> String {
    let assignments = restorations.iter().map(|(key, value)| match key.as_str() {
        // Both POSIX shells and fish (a quoted path variable joins with
        // ':') expand this to the directory before the shell's current PATH.
        "PATH" => format!("PATH={}:\"$PATH\"", shell_quote(value, fish)),
        _ => format!("{key}={}", shell_quote(value, fish)),
    });
    let words = argv.iter().map(|arg| shell_quote(arg, fish));
    format!(
        "{}\r",
        assignments.chain(words).collect::<Vec<_>>().join(" ")
    )
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
        serde_json::json!({ "operation": "restart-if-idle", "pid": prepared.agent.pid, "pane_id": prepared.pane_identity, "session_path": session_path }),
    )
}

#[cfg(target_os = "linux")]
fn control_request(
    prepared: &Prepared,
    request: serde_json::Value,
) -> Result<ControlReply, String> {
    control_roundtrip(
        &ControlTarget::from(prepared),
        request,
        FRAME_LIMIT,
        STEP_TIMEOUT,
    )
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
    if process_identity(target.agent.pid).as_ref() != Some(&target.agent)
        || process_identity(target.shell.pid).as_ref() != Some(&target.shell)
    {
        return Err("process_identity_changed".into());
    }
    let directory = std::path::PathBuf::from(format!(
        "/tmp/trailboss-{}-control-{uid}",
        target.kind.name()
    ));
    let path = checked_control_socket(&directory, target.agent.pid, uid)?;
    // Unix local connect is bounded by nonblocking connect + poll below.
    let mut stream =
        connect_control(&path).map_err(|err| format!("control_connect_failed: {err}"))?;
    verify_peer(&stream, target.agent.pid)?;
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
                if reply.len() + count > 65_536 {
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

    fn pi_agent(session_path: &str) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": "term-pi", "agent": "pi", "agent_status": "idle",
            "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2",
            "focused": false, "revision": 0,
            "agent_session": { "source": "herdr:pi", "agent": "pi", "kind": "path", "value": session_path }
        })).unwrap()
    }

    const PI_NODE: &str = "/usr/bin/node";
    const PI_CLI: &str = "/usr/lib/pi/cli.js";

    /// Reporter `launch-info` argv for `pi <launch>`.
    fn pi_launch(launch: &[&str]) -> Vec<String> {
        let mut args = argv(&[PI_NODE, PI_CLI]);
        args.extend(argv(launch));
        args
    }

    // PRI-01 Idle Pi restarts into its exact session in the same shell.
    #[test]
    fn idle_pi_is_a_candidate_and_resumes_its_exact_session_with_launch_flags() {
        let path = "/tmp/pri/sessions/a.jsonl";
        let info = pi_agent(path);
        assert_eq!(initial_skip_reason(&info), Ok(()));
        assert_eq!(exact_session_path(&info).as_deref(), Some(path));
        assert_eq!(
            pi_resume_argv(
                &pi_launch(&[
                    "--model",
                    "anthropic/claude-sonnet-4-5",
                    "--thinking",
                    "high"
                ]),
                path
            )
            .unwrap(),
            pi_launch(&[
                "--model",
                "anthropic/claude-sonnet-4-5",
                "--thinking",
                "high",
                "--session",
                path
            ])
        );
    }

    // PRI-04 Pi with background work is skipped: herdr:busy publishes Waiting.
    #[test]
    fn pi_waiting_on_background_runs_is_skipped_as_not_idle() {
        let mut info = pi_agent("/tmp/pri/sessions/e.jsonl");
        info.agent_status = AgentStatus::Waiting;
        assert_eq!(initial_skip_reason(&info), Err("not_idle"));
    }

    // PRI-05 Pi without pi-subagents restarts without a background-work query.
    #[test]
    fn pi_with_only_explicit_extensions_restarts_with_them() {
        let reporter = "/home/user/.pi/agent/extensions/trailboss-agent-state.ts";
        let path = "/tmp/pri/sessions/f.jsonl";
        assert_eq!(
            pi_resume_argv(&pi_launch(&["--no-extensions", "-e", reporter]), path),
            Ok(pi_launch(&[
                "--no-extensions",
                "-e",
                reporter,
                "--session",
                path
            ]))
        );
    }

    // PRI-06 Busy, drafted, blocked or focused Pi agents are skipped (Core rows;
    // queued follow-ups and drafts are reporter rows in the asset tests).
    #[test]
    fn working_blocked_or_focused_pi_agents_are_skipped() {
        let idle = pi_agent("/tmp/pri/sessions/g.jsonl");
        let mut working = idle.clone();
        working.agent_status = AgentStatus::Working;
        let mut blocked = idle.clone();
        blocked.agent_status = AgentStatus::Blocked;
        let mut focused = idle.clone();
        focused.focused = true;
        for (state, info, reason) in [
            ("Working on a turn", working, "not_idle"),
            ("reports Blocked", blocked, "not_idle"),
            ("Idle in the focused pane", focused, "focused"),
        ] {
            assert_eq!(initial_skip_reason(&info), Err(reason), "{state}");
        }
    }

    // PRI-02 An earlier session selector is replaced by the exact session path.
    #[test]
    fn earlier_pi_session_selectors_are_replaced_by_the_exact_session_path() {
        let path = "/tmp/pri/sessions/b.jsonl";
        for selector in [
            &["--session", "/tmp/pri/sessions/b.jsonl"][..],
            &["--continue"],
            &["--resume"],
            &["--session-id", "b0b0"],
            &["--fork", "/tmp/pri/sessions/old.jsonl"],
        ] {
            assert_eq!(
                pi_resume_argv(&pi_launch(selector), path).as_deref(),
                Ok(&pi_launch(&["--session", path])[..]),
                "{selector:?}"
            );
        }
    }

    /// Splits a scenario launch on spaces, keeping double-quoted words whole.
    fn launch_words(launch: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quoted = false;
        for character in launch.chars() {
            match character {
                '"' => quoted = !quoted,
                ' ' if !quoted => {
                    words.extend((!word.is_empty()).then(|| std::mem::take(&mut word)))
                }
                _ => word.push(character),
            }
        }
        words.extend((!word.is_empty()).then_some(word));
        words
    }

    // PRI-12 Each allowed launch option is kept on restart.
    #[test]
    fn each_allowed_pi_launch_option_is_kept_on_restart() {
        let path = "/tmp/pri/sessions/c.jsonl";
        for option in [
            "--provider anthropic --model claude-sonnet-4-5",
            "--models sonnet,haiku",
            "--thinking high",
            "--system-prompt \"Be brief\"",
            "--append-system-prompt /tmp/pri/extra.md",
            "--tools read,bash",
            "-t read",
            "--exclude-tools write",
            "-xt write",
            "--no-tools",
            "-nt",
            "--no-builtin-tools",
            "-nbt",
            "--extension /tmp/pri/ext.ts",
            "-e /tmp/pri/ext.ts",
            "--no-extensions -e /tmp/pri/ext.ts",
            "-ne -e /tmp/pri/ext.ts",
            "--skill /tmp/pri/skill",
            "--no-skills",
            "-ns",
            "--prompt-template /tmp/pri/prompts",
            "--no-prompt-templates",
            "-np",
            "--theme /tmp/pri/theme.json",
            "--use-theme dark",
            "--no-themes",
            "--session-dir /tmp/pri/sessions",
            "--tui-mode regular",
            "--no-mcp",
            "--no-context-files",
            "-nc",
            "--approve",
            "-a",
            "--no-approve",
            "-na",
            "--offline",
            "--verbose",
            "--name review",
            "-n review",
        ] {
            let words = launch_words(option);
            let launch: Vec<&str> = words.iter().map(String::as_str).collect();
            let mut expected = launch.clone();
            expected.extend(["--session", path]);
            assert_eq!(
                pi_resume_argv(&pi_launch(&launch), path).as_deref(),
                Ok(&pi_launch(&expected)[..]),
                "{option}"
            );
        }
        assert_eq!(
            launch_words("--system-prompt \"Be brief\""),
            argv(&["--system-prompt", "Be brief"])
        );
    }

    // PRI-13 Core refuses to rebuild any unreplayable Pi launch.
    #[test]
    fn core_refuses_to_rebuild_any_unreplayable_pi_launch() {
        for launch in [
            "\"fix the failing test\"",
            "@/tmp/pri/notes.md",
            "-p",
            "--print",
            "--mode rpc",
            "--export /tmp/pri/out.html",
            "--no-session",
            "--api-key sk-test-0000",
            "--adhd",
            "--preset fast",
            "--frobnicate",
        ] {
            let words = launch_words(launch);
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            assert_eq!(
                pi_resume_argv(&pi_launch(&words), "/tmp/pri/sessions/k.jsonl"),
                Err("unsupported_original_launch_argument".into()),
                "{launch}"
            );
        }
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
            shell_quote(path, false),
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
    fn fish_quoting_keeps_backslashes_and_quotes_inside_one_argument() {
        // Fish treats \\ and \' as escapes inside single quotes; POSIX shells do not.
        let arg = "x\\'; touch /tmp/pwn #\\";
        assert_eq!(shell_quote(arg, true), "'x\\\\\\'; touch /tmp/pwn #\\\\'");
        assert_eq!(shell_quote(arg, false), "'x\\'\\''; touch /tmp/pwn #\\'");
    }

    #[test]
    fn a_mixed_omp_and_pi_run_shows_the_restart_idle_summary_notice() {
        let mut app = app();
        app.state.toast_config.delivery = crate::config::ToastDelivery::Trailboss;
        app.handle_internal_event(AppEvent::AgentRestartIdleFinished {
            restarted: 2,
            skipped: 0,
            failed: 0,
        });
        let toast = app.state.toast.as_ref().expect("summary notice shown");
        assert_eq!(toast.title, "Restart idle agents");
        assert_eq!(toast.context, "2 restarted, 0 skipped, 0 failed");
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

#[cfg(all(test, target_os = "linux"))]
mod pi_launch_tests {
    use super::*;

    /// Serves one reporter connection on this test process's Pi control
    /// endpoint, answering `launch-info` with `argv`, and returns every
    /// operation the reporter received.
    fn with_fake_pi_reporter(
        launch: &[&str],
        session_path: &str,
    ) -> (Result<Vec<String>, String>, Vec<String>) {
        let _endpoint = own_pi_endpoint();
        use std::io::{BufRead, Write};
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let uid = unsafe { libc::geteuid() };
        let directory = std::path::PathBuf::from(format!("/tmp/trailboss-pi-control-{uid}"));
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&directory);
        let endpoint = directory.join(format!("{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&endpoint);
        let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
        let exec_path = std::fs::read_link("/proc/self/exe")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let mut argv = vec![exec_path.clone(), exec_path.clone()];
        argv.extend(launch.iter().map(|arg| (*arg).to_owned()));
        let server = std::thread::spawn(move || {
            let mut operations = Vec::new();
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut line = String::new();
                std::io::BufReader::new(&stream)
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                operations.push(request["operation"].as_str().unwrap().to_owned());
                let reply = serde_json::json!({ "status": "accepted", "reason": "", "result": { "argv": argv, "exec_path": exec_path } });
                writeln!(&stream, "{reply}").unwrap();
                break;
            }
            operations
        });
        let me = process_identity(std::process::id()).unwrap();
        let target = ControlTarget {
            kind: RestartAgentKind::Pi,
            shell: me.clone(),
            agent: me,
            pane_identity: "w1:p2".into(),
        };
        let rebuilt = pi_original_argv(&target, session_path)
            .and_then(|argv| pi_resume_argv(&argv, session_path).map(|argv| argv[2..].to_vec()));
        let operations = server.join().unwrap();
        let _ = std::fs::remove_file(&endpoint);
        (rebuilt, operations)
    }

    // Both tests use this process's endpoint path; `cargo test` runs them in one process.
    fn own_pi_endpoint() -> std::sync::MutexGuard<'static, ()> {
        static ENDPOINT: std::sync::Mutex<()> = std::sync::Mutex::new(());
        ENDPOINT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // PRI-07 Pi on an older reporter is skipped as unsupported.
    #[test]
    fn pi_reporter_without_a_control_socket_is_an_unsupported_endpoint() {
        let _endpoint = own_pi_endpoint();
        let uid = unsafe { libc::geteuid() };
        let endpoint = format!(
            "/tmp/trailboss-pi-control-{uid}/{}.sock",
            std::process::id()
        );
        let _ = std::fs::remove_file(endpoint);
        let me = process_identity(std::process::id()).unwrap();
        let target = ControlTarget {
            kind: RestartAgentKind::Pi,
            shell: me.clone(),
            agent: me,
            pane_identity: "w1:p2".into(),
        };
        assert_eq!(
            pi_original_argv(&target, "/tmp/pri/sessions/h.jsonl"),
            Err("unsupported_control_endpoint".into())
        );
    }

    // PRI-03 Pi launches that cannot be replayed safely are skipped before anything stops.
    #[test]
    fn unreplayable_pi_launches_are_refused_after_only_the_read_only_launch_query() {
        let path = "/tmp/pri/sessions/d.jsonl";
        for launch in [
            &["fix the failing test"][..],
            &["@/tmp/pri/notes.md"],
            &["--api-key", "sk-test-0000"],
            &["--adhd"],
            &["--preset", "fast"],
            &["--frobnicate"],
        ] {
            let (rebuilt, operations) = with_fake_pi_reporter(launch, path);
            assert_eq!(
                rebuilt,
                Err("unsupported_original_launch_argument".into()),
                "{launch:?}"
            );
            assert_eq!(operations, ["launch-info"], "{launch:?}");
        }
        let (rebuilt, operations) = with_fake_pi_reporter(&["--offline"], path);
        assert_eq!(
            rebuilt,
            Ok(vec!["--offline".into(), "--session".into(), path.into()])
        );
        assert_eq!(operations, ["launch-info"]);
    }

    fn environment(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    const SHARED_ENVIRONMENT: [(&str, &str); 2] = [
        ("HERDR_SOCKET_PATH", "/run/herdr.sock"),
        ("XDG_CONFIG_HOME", "/home/user/.config"),
    ];

    fn with_shared(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut pairs = SHARED_ENVIRONMENT.to_vec();
        pairs.extend_from_slice(extra);
        pairs
    }

    // Pi started through its managed launcher carries PI_MANAGED_INSTALL_ROOT,
    // which the shell lacks; the relaunch sets it again.
    #[test]
    fn launcher_set_install_root_is_restored_on_relaunch() {
        let root = "/home/user/.local/bin/../../.pi/agent/install";
        let agent = environment(&with_shared(&[("PI_MANAGED_INSTALL_ROOT", root)]));
        let shell = environment(&SHARED_ENVIRONMENT);
        let restored = launch_environment_restorations(&agent, &shell);
        assert_eq!(
            restored,
            Ok(vec![(
                "PI_MANAGED_INSTALL_ROOT".to_owned(),
                root.to_owned()
            )])
        );
        let argv: Vec<String> = [
            "/usr/bin/node",
            "/usr/lib/pi/cli.js",
            "--session",
            "/tmp/pri/sessions/m.jsonl",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(
            resume_command(&restored.unwrap(), &argv, false),
            "PI_MANAGED_INSTALL_ROOT='/home/user/.local/bin/../../.pi/agent/install' \
             '/usr/bin/node' '/usr/lib/pi/cli.js' '--session' '/tmp/pri/sessions/m.jsonl'\r"
        );
        assert_eq!(
            resume_command(&[], &argv, false),
            "'/usr/bin/node' '/usr/lib/pi/cli.js' '--session' '/tmp/pri/sessions/m.jsonl'\r"
        );
    }

    #[test]
    fn environment_overrides_the_launcher_did_not_set_are_still_refused() {
        let shell = environment(&with_shared(&[]));
        for (case, agent, shell) in [
            (
                "other Pi variable only in the agent",
                environment(&with_shared(&[("PI_CODING_AGENT_DIR", "/elsewhere")])),
                shell.clone(),
            ),
            (
                "XDG variable changed for the agent",
                environment(&[
                    ("HERDR_SOCKET_PATH", "/run/herdr.sock"),
                    ("XDG_CONFIG_HOME", "/elsewhere"),
                ]),
                shell.clone(),
            ),
            (
                "install root differs from the shell's",
                environment(&with_shared(&[("PI_MANAGED_INSTALL_ROOT", "/a/install")])),
                environment(&with_shared(&[("PI_MANAGED_INSTALL_ROOT", "/b/install")])),
            ),
            (
                "variable only in the shell",
                environment(&with_shared(&[])),
                environment(&with_shared(&[("OMP_PROFILE", "work")])),
            ),
        ] {
            assert_eq!(
                launch_environment_restorations(&agent, &shell),
                Err("unsupported_environment_override".to_owned()),
                "{case}"
            );
        }
        assert_eq!(
            launch_environment_restorations(&shell, &shell),
            Ok(Vec::new())
        );
    }

    // The launcher also puts the Node.js the Pi installer placed under
    // `pi-node` at the front of PATH; the relaunch puts it back.
    #[test]
    fn launcher_prepended_pi_node_path_is_restored_on_relaunch() {
        let root = "/home/user/.pi/agent/install";
        let pi_node = "/home/user/.local/share/pi-node/current/bin";
        // The shell's startup files extended PATH after the shell started, so
        // the agent's PATH suffix differs from the shell's starting PATH.
        let shell = environment(&with_shared(&[
            ("HOME", "/home/user"),
            ("PATH", "/usr/bin"),
        ]));
        let launched = environment(&with_shared(&[
            ("HOME", "/home/user"),
            (
                "PATH",
                "/home/user/.local/share/pi-node/current/bin:/home/user/.local/bin:/usr/bin",
            ),
            ("PI_MANAGED_INSTALL_ROOT", root),
        ]));
        let restored = launch_environment_restorations(&launched, &shell).unwrap();
        assert_eq!(
            restored,
            vec![
                ("PATH".to_owned(), pi_node.to_owned()),
                ("PI_MANAGED_INSTALL_ROOT".to_owned(), root.to_owned()),
            ]
        );
        let argv: Vec<String> = ["/usr/bin/node", "/r/pi"].map(str::to_owned).to_vec();
        assert_eq!(
            resume_command(&restored, &argv, false),
            "PATH='/home/user/.local/share/pi-node/current/bin':\"$PATH\" \
             PI_MANAGED_INSTALL_ROOT='/home/user/.pi/agent/install' '/usr/bin/node' '/r/pi'\r"
        );
        assert_eq!(
            resume_command(&restored, &argv, true),
            "PATH='/home/user/.local/share/pi-node/current/bin':\"$PATH\" \
             PI_MANAGED_INSTALL_ROOT='/home/user/.pi/agent/install' '/usr/bin/node' '/r/pi'\r"
        );
        for (case, agent) in [
            (
                "launcher did not prepend pi-node",
                environment(&with_shared(&[
                    ("HOME", "/home/user"),
                    ("PATH", "/home/user/.local/bin:/usr/bin"),
                    ("PI_MANAGED_INSTALL_ROOT", root),
                ])),
            ),
            (
                "not started by the managed launcher",
                environment(&with_shared(&[
                    ("HOME", "/home/user"),
                    (
                        "PATH",
                        "/home/user/.local/share/pi-node/current/bin:/usr/bin",
                    ),
                ])),
            ),
        ] {
            assert!(
                launch_environment_restorations(&agent, &shell)
                    .unwrap()
                    .iter()
                    .all(|(key, _)| key != "PATH"),
                "{case}"
            );
        }
    }

    // The tolerated install root must be the one Pi's launcher derived for the
    // running release; any other value is an override.
    #[test]
    fn restored_install_root_must_own_the_running_pi_release() {
        let directory = std::env::temp_dir().join(format!(
            "trailboss-managed-root-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = directory.join("install");
        let release = root.join("releases/1.5.2/node_modules/.bin");
        std::fs::create_dir_all(&release).unwrap();
        let script = release.join("pi");
        std::fs::write(&script, "").unwrap();
        let foreign = directory.join("cli.js");
        std::fs::write(&foreign, "").unwrap();
        let lexical_root = format!("{}/install/releases/../", directory.display());
        let restored = vec![("PI_MANAGED_INSTALL_ROOT".to_owned(), lexical_root)];
        let script = script.to_str().unwrap();
        let foreign = foreign.to_str().unwrap();
        assert_eq!(
            verify_launcher_restorations(RestartAgentKind::Pi, &restored, script),
            Ok(())
        );
        let with_path = [
            (
                "PATH".to_owned(),
                "/home/user/.local/share/pi-node/current/bin".to_owned(),
            ),
            restored[0].clone(),
        ];
        assert_eq!(
            verify_launcher_restorations(RestartAgentKind::Pi, &with_path, script),
            Ok(())
        );
        for (case, kind, script) in [
            ("script outside the root", RestartAgentKind::Pi, foreign),
            ("OMP has no Pi launcher", RestartAgentKind::Omp, script),
        ] {
            assert_eq!(
                verify_launcher_restorations(kind, &restored, script),
                Err("unsupported_environment_override".to_owned()),
                "{case}"
            );
        }
        assert_eq!(
            verify_launcher_restorations(RestartAgentKind::Omp, &[], "omp"),
            Ok(())
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
