#![cfg(target_os = "linux")]
//! Restricted app conversation bridge for Pi.
//!
//! Runs the installed interactive Pi with the repository reporter in an
//! owned isolated Core. Model requests reach an owned loopback stub provider,
//! so no external model is contacted.

#[path = "support/stub_provider.rs"]
mod stub_provider;
pub mod support;

use serde_json::{json, Value};
use std::{
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use stub_provider::StubProvider;
use support::Core;

const RESTRICTED: &[&str] = &[
    "restricted_app_bridge_v1",
    "single_outstanding_v1",
    "input_status_v1",
    "reply_style_v1",
];

fn cli(core: &Core, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(support::executable());
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let key = key.to_string_lossy();
        key.starts_with("HERDR_") || key.starts_with("TRAILBOSS_") || key.starts_with("PI_")
    }) {
        command.env_remove(key);
    }
    for key in support::HOMES {
        command.env(key, &core.home);
    }
    command.env("PI_CODING_AGENT_DIR", core.home.join(".omp/agent"));
    command.args(args).current_dir(&core.home).output().unwrap()
}

/// Raw public API exchange; conversation refusals are error envelopes.
fn raw(core: &Core, method: &str, params: Value) -> Value {
    let api = trailboss_runtime::api_client::ApiClient::for_target(
        trailboss_runtime::api_client::ConnectionTarget::SocketPath(
            core.home
                .join("trailboss/sessions")
                .join(&core.name)
                .join("trailboss.sock"),
        ),
    );
    let request = serde_json::from_value(json!({
        "id":"pi-restricted-proof", "method":method, "params":params
    }))
    .unwrap();
    api.request_value_with_timeout(&request, Duration::from_secs(15))
        .unwrap()
}

fn agent(core: &Core, terminal: &str) -> Value {
    core.request("agent.list", json!({}))["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["terminal_id"] == terminal)
        .cloned()
        .unwrap_or(Value::Null)
}

fn wait_session(core: &Core, terminal: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let current = agent(core, terminal);
        if current["agent"] == "pi"
            && current["agent_session"]["source"] == "herdr:pi"
            && current["agent_session"]["value"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "genuine Pi report did not arrive for terminal {terminal}: {}",
            json!({"agent":current["agent"],"session":current["agent_session"]})
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Removes the owned opt-in marker; it never touches another process's grant.
struct Grant(PathBuf);
impl Drop for Grant {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct RestrictedPi {
    terminal: String,
    session_path: String,
    _grant: Grant,
}

impl RestrictedPi {
    /// Starts managed Pi in `pane` and enrolls exactly that process,
    /// terminal and conversation in the restricted app profile.
    fn start(core: &Core, pane: &str, provider: &StubProvider) -> Self {
        let terminal = core.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"]
            .as_str()
            .unwrap()
            .to_owned();
        // The owned Core sets PI_CODING_AGENT_DIR, which Pi also reads.
        let agent_dir = core.home.join(".omp/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("models.json"),
            json!({"providers":{"owned-stub":{
                "baseUrl":format!("http://127.0.0.1:{}/v1", provider.port),
                "api":"openai-completions","apiKey":"none",
                "models":[{"id":"stub"}]}}})
            .to_string(),
        )
        .unwrap();
        // Exercise the reporter exactly as the integration installer writes it.
        let installed = cli(core, &["integration", "install", "pi"]);
        assert!(
            installed.status.success(),
            "Pi integration install failed: {}",
            String::from_utf8_lossy(&installed.stderr)
        );
        let reporter = agent_dir.join("extensions/trailboss-agent-state.ts");
        let sessions = core.home.join("pi-sessions");
        std::fs::create_dir(&sessions).unwrap();
        let started = cli(
            core,
            &[
                "--session",
                &core.name,
                "agent",
                "start",
                "restricted-pi",
                "--kind",
                "pi",
                "--pane",
                pane,
                "--",
                "--offline",
                "--no-tools",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--no-themes",
                "--no-context-files",
                "--session-dir",
                sessions.to_str().unwrap(),
                "--model",
                "owned-stub/stub",
                "-e",
                reporter.to_str().unwrap(),
            ],
        );
        assert!(
            started.status.success(),
            "owned managed Pi startup failed: {}\npane: {}",
            String::from_utf8_lossy(&started.stderr),
            core.request(
                "pane.read",
                json!({"pane_id":pane,"source":"recent","lines":40})
            )
        );
        let reported = wait_session(core, &terminal);
        let pid = core.request("pane.process_info", json!({"pane_id":pane}))["process_info"]
            ["foreground_process_group_id"]
            .as_u64()
            .unwrap();
        let session_path = reported["agent_session"]["value"]
            .as_str()
            .unwrap()
            .to_owned();
        let grant = enroll(pid, &terminal, &session_path);
        Self {
            terminal,
            session_path,
            _grant: grant,
        }
    }

    /// Metadata-only restricted read, waiting for an expected work state.
    fn read(&self, core: &Core, work: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let reply = raw(
                core,
                "agent.conversation.read",
                json!({"terminal_id":self.terminal,"expected_context":null,"cursor":null,"max_bytes":0}),
            );
            if let Some(result) = reply.get("result").and_then(|result| result.get("result")) {
                let capabilities = result["capabilities"].as_array().unwrap();
                let restricted = RESTRICTED
                    .iter()
                    .all(|needed| capabilities.iter().any(|item| item == needed));
                let submittable = capabilities
                    .iter()
                    .any(|item| item == "restricted_submit_v1");
                if restricted
                    && result["work"]["state"] == work
                    && (submittable || work != "not_working")
                {
                    assert_eq!(result["binding"]["terminal_id"], self.terminal);
                    return result.clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "restricted {work} read never became available: {reply}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn submit(&self, core: &Core, read: &Value, input: &str, text: &str) -> Value {
        raw(
            core,
            "agent.conversation.submit",
            json!({
                "binding":read["binding"],"reporter_epoch":read["reporter_epoch"],
                "context_epoch":read["context_epoch"],"input_id":input,"text":text,"reply_style":"brief"
            }),
        )
    }

    /// Counts user entries carrying `text` in the native conversation journal.
    fn journal_count(&self, text: &str) -> usize {
        std::fs::read_to_string(&self.session_path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|entry| {
                entry["type"] == "message"
                    && entry["message"]["role"] == "user"
                    && entry["message"].to_string().contains(text)
            })
            .count()
    }
}

/// Writes the documented opt-in marker for the current Pi conversation.
fn enroll(pid: u64, terminal: &str, session_path: &str) -> Grant {
    let conversation = Path::new(session_path)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .rsplit_once('_')
        .unwrap()
        .1
        .to_owned();
    let uid = std::fs::metadata("/proc/self").unwrap().uid();
    let directory = PathBuf::from(format!("/tmp/trailboss-pi-control-{uid}"));
    let deadline = Instant::now() + Duration::from_secs(8);
    while !directory.join(format!("{pid}.sock")).exists() {
        assert!(
            Instant::now() < deadline,
            "owned Pi reporter control socket did not appear"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let start = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned();
    let path = directory.join(format!("{pid}.restricted-app.json"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    let grant = Grant(path);
    file.write_all(
        json!({
            "version":1, "pid":pid, "proc_start_ticks":start, "terminal_id":terminal,
            "conversation_id":conversation, "session_path":session_path
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    grant
}

#[test]
#[ignore = "requires installed interactive Pi; owned loopback model provider"]
fn restricted_app_submits_one_instruction_to_pi_and_reads_its_reply() {
    let provider = StubProvider::start();
    let core = Core::new();
    let workspace = core.request(
        "workspace.create",
        json!({"label":"Review","cwd":core.home,"focus":true}),
    );
    let pi = RestrictedPi::start(
        &core,
        workspace["root_pane"]["pane_id"].as_str().unwrap(),
        &provider,
    );

    let idle = pi.read(&core, "not_working");
    let text = "review this transcript once";
    let submitted = pi.submit(&core, &idle, "pi-review-1", text);
    assert_eq!(
        submitted["result"]["result"]["status"], "accepted_by_reporter",
        "restricted Pi submission was not accepted: {submitted}"
    );
    assert!(provider.next_request().contains(text));

    let working = pi.read(&core, "working");
    let second = pi.submit(&core, &working, "pi-review-2", "must not run");
    assert!(
        // Core refuses before the reporter when submission is no longer advertised.
        [
            "structured_unavailable",
            "instruction_outstanding",
            "agent_busy"
        ]
        .iter()
        .any(|reason| second["error"]["reason"] == *reason)
            && second["error"]["invocation"] == "not_started",
        "a second instruction was admitted while the first was outstanding: {second}"
    );

    assert!(
        !working["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "restricted_submit_v1"),
        "an outstanding instruction still offered submission: {working}"
    );

    provider.release();
    let settled = pi.read(&core, "not_working");
    let history = raw(
        &core,
        "agent.conversation.read",
        json!({"terminal_id":pi.terminal,
            "expected_context":{"conversation_id":settled["binding"]["conversation_id"],
                "reset_entry_id":settled["binding"]["reset_entry_id"]},
            "cursor":null,"max_bytes":65536}),
    );
    let records = history["result"]["result"]["records"].as_array().unwrap();
    assert!(
        records
            .iter()
            .any(|record| record["role"] == "assistant" && record["text"] == "owned stub reply"),
        "the Pi reply was not readable through the bridge: {history}"
    );
    assert_eq!(pi.journal_count(text), 1);
    assert_eq!(pi.journal_count("must not run"), 0);
    provider.assert_no_request(Duration::from_millis(500), "after the single instruction");
}
