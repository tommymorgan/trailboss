#![cfg(target_os = "linux")]
//! Restricted app-profile agent controls after live cross-session transfer.
//!
//! Each witness runs the installed interactive OMP with the repository
//! reporter against owned isolated Cores. Model requests reach an owned
//! loopback stub provider, so no external model is contacted.

mod support;

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use support::Core;

const RESTRICTED: &[&str] = &[
    "restricted_app_bridge_v1",
    "single_outstanding_v1",
    "input_status_v1",
    "reply_style_v1",
];

fn wait_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if text.ends_with('\n') {
                return text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "owned shell did not finish {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

type Identity = (String, String, PathBuf, PathBuf);

fn identity(pid: &str) -> Identity {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[4], "0", "process lost its controlling PTY");
    (
        fields[19].into(),
        fields[4].into(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}

fn cli(core: &Core, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(support::executable());
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let key = key.to_string_lossy();
        key.starts_with("HERDR_") || key.starts_with("TRAILBOSS_")
    }) {
        command.env_remove(key);
    }
    for key in support::HOMES {
        command.env(key, &core.home);
    }
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
        "id":"restricted-transfer-proof", "method":method, "params":params
    }))
    .unwrap();
    api.request_value_with_timeout(&request, Duration::from_secs(15))
        .unwrap()
}

fn pane_for(core: &Core, terminal: &str) -> Value {
    core.request("session.snapshot", json!({}))["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["terminal_id"] == terminal)
        .unwrap_or_else(|| panic!("terminal {terminal} absent from {}", core.name))["pane_id"]
        .clone()
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

fn stop(core: &mut Core) {
    core.request("server.stop", json!({}));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = core.child.try_wait().unwrap() {
            assert!(status.success(), "owned source Core did not exit cleanly");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "owned source Core remained alive"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn running(observer: &Core, name: &str) -> bool {
    observer.request("session.list", json!({}))["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["name"] == name)
        .unwrap()["running"]
        .as_bool()
        .unwrap()
}

/// OpenAI-compatible loopback provider owned by the test. Each completion is
/// held until the test releases it, so work and single-outstanding admission
/// are observed deterministically without contacting a real model.
struct StubProvider {
    port: u16,
    requests: mpsc::Receiver<String>,
    release: mpsc::Sender<()>,
}

impl StubProvider {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_tx, requests) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let request_tx = request_tx.clone();
                let release_rx = Arc::clone(&release_rx);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                length = value.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let mut body = vec![0; length];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let _ = request_tx.send(String::from_utf8_lossy(&body).into_owned());
                    let released = release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(60));
                    if released.is_err() {
                        return;
                    }
                    let chunk = |delta: Value, finish: Value| {
                        format!(
                            "data: {}\n\n",
                            json!({"id":"owned-stub","object":"chat.completion.chunk","created":0,
                                "model":"stub","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
                        )
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{}{}data: [DONE]\n\n",
                        chunk(json!({"role":"assistant","content":"owned stub reply"}), Value::Null),
                        chunk(json!({}), json!("stop")),
                    );
                    let _ = stream.flush();
                });
            }
        });
        Self {
            port,
            requests,
            release,
        }
    }

    fn next_request(&self) -> String {
        self.requests
            .recv_timeout(Duration::from_secs(20))
            .expect("restricted submission never reached the owned model provider")
    }

    fn assert_no_request(&self, within: Duration, context: &str) {
        if let Ok(body) = self.requests.recv_timeout(within) {
            panic!("{context}: an unexpected model request was invoked: {body}");
        }
    }

    fn release(&self) {
        self.release.send(()).unwrap();
    }
}

/// Removes the owned opt-in marker; it never touches another process's grant.
struct Grant(PathBuf);
impl Drop for Grant {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct RestrictedOmp {
    terminal: String,
    pid: String,
    identity: Identity,
    session_path: String,
    grant: Option<Grant>,
}

impl RestrictedOmp {
    /// Starts managed OMP in `pane` and enrolls exactly that process,
    /// terminal and conversation in the restricted app profile.
    fn start(core: &Core, pane: &str, provider: &StubProvider, label: &str) -> Self {
        let terminal = core.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let agent_dir = core.home.join(".omp/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("config.yml"), "setupVersion: 2\n").unwrap();
        std::fs::write(
            agent_dir.join("models.yml"),
            format!(
                "providers:\n  owned-stub:\n    baseUrl: http://127.0.0.1:{}/v1\n    api: openai-completions\n    auth: none\n    models:\n      - id: stub\n        name: Owned stub\n        reasoning: false\n        input: [text]\n        contextWindow: 32768\n        maxTokens: 256\n        cost: {{input: 0, output: 0, cacheRead: 0, cacheWrite: 0}}\n",
                provider.port
            ),
        )
        .unwrap();
        let reporter = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("crates/trailboss-runtime/src/integration/assets/omp/trailboss-agent-state.ts");
        let sessions = core.home.join(format!("{label}-sessions"));
        std::fs::create_dir(&sessions).unwrap();
        let started = cli(
            core,
            &[
                "--session",
                &core.name,
                "agent",
                "start",
                label,
                "--kind",
                "omp",
                "--pane",
                pane,
                "--",
                "--no-tools",
                "--no-lsp",
                "--no-extensions",
                "--no-context-files",
                "--no-rules",
                "--no-skills",
                "--no-title",
                "--session-dir",
                sessions.to_str().unwrap(),
                "--model",
                "owned-stub/stub",
                "--max-time",
                "120",
                "-e",
                reporter.to_str().unwrap(),
            ],
        );
        assert!(
            started.status.success(),
            "owned managed OMP startup failed: {}",
            String::from_utf8_lossy(&started.stderr)
        );
        let reported = wait_session(core, &terminal, None);
        let pid = core.request("pane.process_info", json!({"pane_id":pane}))["process_info"]
            ["foreground_process_group_id"]
            .as_u64()
            .unwrap()
            .to_string();
        let mut omp = Self {
            terminal,
            identity: identity(&pid),
            pid,
            session_path: String::new(),
            grant: None,
        };
        omp.enroll(&reported);
        omp
    }

    /// Writes the documented opt-in marker for the current conversation.
    fn enroll(&mut self, reported: &Value) {
        assert_eq!(reported["agent_session"]["kind"], "path");
        let session_path = reported["agent_session"]["value"]
            .as_str()
            .unwrap()
            .to_owned();
        let conversation = Path::new(&session_path)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .rsplit_once('_')
            .unwrap()
            .1
            .to_owned();
        let uid = std::fs::metadata("/proc/self").unwrap().uid();
        let directory = PathBuf::from(format!("/tmp/trailboss-omp-control-{uid}"));
        let deadline = Instant::now() + Duration::from_secs(8);
        while !directory.join(format!("{}.sock", self.pid)).exists() {
            assert!(
                Instant::now() < deadline,
                "owned OMP reporter control socket did not appear"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let path = directory.join(format!("{}.restricted-app.json", self.pid));
        self.grant = None;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        self.grant = Some(Grant(path));
        file.write_all(
            json!({
                "version":1, "pid":self.pid.parse::<u64>().unwrap(),
                "proc_start_ticks":self.identity.0, "terminal_id":self.terminal,
                "conversation_id":conversation, "session_path":session_path
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        self.session_path = session_path;
    }

    /// Metadata-only restricted read through `core`, waiting for the
    /// reporter's independent refresh and an expected work state.
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
                // Idle reads must offer submission; whether a working read
                // withholds it is asserted only after the refusal under test.
                if restricted
                    && result["work"]["state"] == work
                    && (submittable || work != "not_working")
                {
                    assert_eq!(result["binding"]["terminal_id"], self.terminal);
                    assert!(
                        !capabilities.iter().any(|item| item == "guarded_abort_v1"
                            || item == "guarded_submit_v1"
                            || item == "restricted_abort_v1"),
                        "restricted profile advertised native guarded controls: {result}"
                    );
                    return result.clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "restricted {work} read never became available through {}: {reply}",
                core.name
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

    fn abort(&self, core: &Core, read: &Value) -> Value {
        raw(
            core,
            "agent.conversation.abort",
            json!({
                "binding":read["binding"],"reporter_epoch":read["reporter_epoch"],
                "context_epoch":read["context_epoch"],"work_fence":"restricted-profile-has-no-fence"
            }),
        )
    }

    fn input_status(&self, core: &Core, read: &Value, input: &str) -> Value {
        raw(
            core,
            "agent.conversation.input_status",
            json!({"binding":read["binding"],"reporter_epoch":read["reporter_epoch"],"input_id":input}),
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

fn wait_session(core: &Core, terminal: &str, previous: Option<&Value>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let current = agent(core, terminal);
        if current["agent"] == "omp"
            && current["agent_session"]["source"] == "herdr:omp"
            && current["agent_session"]["value"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && previous.is_none_or(|previous| current["agent_session"] != previous["agent_session"])
        {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "genuine OMP report did not arrive for terminal {terminal}: {}",
            json!({"agent":current["agent"],"session":current["agent_session"]})
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_status(core: &Core, terminal: &str, status: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let current = agent(core, terminal);
        if current["agent_status"] == status {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "moved OMP never reported {status} through {}: {}",
            core.name,
            current["agent_status"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_withholds_submission(read: &Value) {
    assert!(
        !read["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "restricted_submit_v1"),
        "an outstanding restricted instruction still offered submission: {read}"
    );
}

fn assert_refused(reply: &Value, reasons: &[&str], context: &str) {
    let error = &reply["error"];
    assert!(
        reasons.iter().any(|reason| error["reason"] == *reason)
            && error["invocation"] == "not_started"
            && reply.get("result").is_none(),
        "{context}: expected a not-started {reasons:?} refusal, got {reply}"
    );
}

/// Unrelated live shell whose observable agent metadata must not change.
struct Bystander {
    pane: Value,
    marker: String,
    pid: String,
    identity: Identity,
    before: Value,
}

impl Bystander {
    fn start(core: &Core, pane: &Value, marker: &str) -> Self {
        core.request(
            "pane.send_text",
            json!({"pane_id":pane,"text":format!(
            "bystander_local={marker}; printf '%s\\n' \"$$\" > {marker}.pid\n")}),
        );
        let pid = wait_file(&core.home.join(format!("{marker}.pid")))
            .trim()
            .to_owned();
        Self {
            pane: pane.clone(),
            marker: marker.to_owned(),
            identity: identity(&pid),
            pid,
            before: core.request("pane.get", json!({"pane_id":pane}))["pane"].clone(),
        }
    }

    fn assert_unchanged(&self, core: &Core, stage: &str) {
        let after = core.request("pane.get", json!({"pane_id":self.pane}))["pane"].clone();
        for field in [
            "terminal_id",
            "agent",
            "agent_status",
            "agent_session",
            "state_labels",
            "title",
        ] {
            assert_eq!(
                after[field], self.before[field],
                "moved agent activity changed the unrelated pane's {field}"
            );
        }
        assert_eq!(identity(&self.pid), self.identity);
        core.request(
            "pane.send_text",
            json!({"pane_id":self.pane,"text":format!(
            "printf '%s\\n' \"$bystander_local\" > {stage}-after\n")}),
        );
        assert_eq!(
            wait_file(&core.home.join(format!("{stage}-after"))),
            format!("{}\n", self.marker)
        );
    }
}

#[test]
#[ignore = "requires installed interactive OMP; owned loopback model provider"]
fn moved_tab_and_restricted_agent_controls_survive_source_core_exit() {
    let provider = StubProvider::start();
    let mut alpha = Core::new();
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let source = alpha.request(
        "workspace.create",
        json!({"label":"Owned source","cwd":alpha.home,"focus":true}),
    );
    let shell = source["root_pane"]["pane_id"].as_str().unwrap();
    let shell_terminal = alpha.request("pane.get", json!({"pane_id":shell}))["pane"]["terminal_id"]
        .as_str()
        .unwrap()
        .to_owned();
    alpha.request("pane.send_text", json!({"pane_id":shell,
        "text":"transfer_marker=owned-source; sleep 300 & transfer_job=$!; printf '%s %s\\n' \"$$\" \"$transfer_job\" > restricted-identities\n"}));
    let ids: Vec<String> = wait_file(&alpha.home.join("restricted-identities"))
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let shell_identity = identity(&ids[0]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while identity(&ids[1]).3 == shell_identity.3 {
        assert!(Instant::now() < deadline, "background job did not exec");
        std::thread::sleep(Duration::from_millis(10));
    }
    let job_identity = identity(&ids[1]);
    let split = alpha.request(
        "pane.split",
        json!({"target_pane_id":shell,"direction":"right","cwd":alpha.home,"focus":true}),
    );
    let omp = RestrictedOmp::start(
        &alpha,
        split["pane"]["pane_id"].as_str().unwrap(),
        &provider,
        "restricted-exit",
    );
    let destination = beta.request("workspace.create", json!({"label":"Work","cwd":beta.home}));
    let bystander = Bystander::start(
        &beta,
        &destination["root_pane"]["pane_id"],
        "restricted-exit-bystander",
    );

    // The unmoved restricted tab defines the refusal moved controls must keep.
    let unmoved = omp.read(&alpha, "not_working");
    let unmoved_abort = omp.abort(&alpha, &unmoved);
    assert_refused(&unmoved_abort, &["structured_unavailable"], "unmoved abort");

    alpha.request("tab.transfer", json!({
        "tab_id":source["tab"]["tab_id"],"destination_session":beta.name,
        "workspace_id":destination["workspace"]["workspace_id"],"operation_id":"restricted-source-exit"
    }));
    stop(&mut alpha);
    assert!(
        !running(&beta, &alpha.name),
        "alpha reported running after exit"
    );
    assert_eq!(identity(&ids[0]), shell_identity);
    assert_eq!(identity(&ids[1]), job_identity);
    assert_eq!(identity(&omp.pid), omp.identity);
    beta.request("pane.send_text", json!({"pane_id":pane_for(&beta, &shell_terminal),
        "text":"kill -0 \"$transfer_job\" && printf '%s\\n' \"$transfer_marker\" > restricted-shell-after\n"}));
    assert_eq!(
        wait_file(&beta.home.join("restricted-shell-after")),
        "owned-source\n"
    );

    let fresh = omp.read(&beta, "not_working");
    assert_eq!(fresh["binding"], unmoved["binding"]);
    assert_eq!(fresh["reporter_epoch"], unmoved["reporter_epoch"]);
    assert_eq!(fresh["context_epoch"], unmoved["context_epoch"]);
    let text = "owned restricted instruction after source exit";
    let submitted = omp.submit(&beta, &fresh, "restricted-after-exit", text);
    assert_eq!(
        submitted["result"]["result"]["status"], "accepted_by_reporter",
        "restricted submit through beta failed: {submitted}"
    );
    let receipt = submitted["result"]["result"]["receipt"].clone();
    assert!(receipt.as_str().is_some_and(|value| !value.is_empty()));
    assert!(
        provider.next_request().contains(text),
        "owned provider received a different instruction"
    );
    // The working transition is a genuine report through the inherited context.
    wait_status(&beta, &omp.terminal, "working");
    let working = omp.read(&beta, "working");
    assert_eq!(working["binding"], unmoved["binding"]);
    let status = omp.input_status(&beta, &working, "restricted-after-exit");
    assert_eq!(status["result"]["result"]["status"], "accepted_by_reporter");
    assert_eq!(status["result"]["result"]["receipt"], receipt);
    let second = omp.submit(&beta, &working, "restricted-second", "must not run");
    assert_refused(
        &second,
        &[
            "structured_unavailable",
            "instruction_outstanding",
            "agent_busy",
        ],
        "second outstanding submit",
    );
    assert_withholds_submission(&omp.read(&beta, "working"));
    let moved_abort = omp.abort(&beta, &working);
    assert_eq!(
        moved_abort["error"], unmoved_abort["error"],
        "moved abort refusal differs from the unmoved restricted tab"
    );
    provider.assert_no_request(Duration::from_millis(750), "refused controls");
    provider.release();
    let settled = omp.read(&beta, "not_working");
    assert_eq!(settled["binding"], unmoved["binding"]);
    assert_eq!(omp.journal_count(text), 1);
    assert_eq!(omp.journal_count("must not run"), 0);
    assert_eq!(identity(&omp.pid), omp.identity);
    bystander.assert_unchanged(&beta, "restricted-exit-bystander");
}

/// Owned fixture: alpha root-pane OMP enrolled in the restricted profile and a
/// beta bystander whose public pane locator collides with the moved one.
struct Moved {
    alpha: Core,
    beta: Core,
    omp: RestrictedOmp,
    bystander: Bystander,
    original_pane: String,
    original_socket: PathBuf,
}

fn move_restricted_omp(provider: &StubProvider, label: &str, stop_source: bool) -> Moved {
    let mut alpha = Core::new();
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let source = alpha.request(
        "workspace.create",
        json!({"label":"Same root","cwd":alpha.home,"focus":true}),
    );
    let destination = beta.request(
        "workspace.create",
        json!({"label":"Same root","cwd":beta.home,"focus":true}),
    );
    assert_eq!(
        source["root_pane"]["pane_id"], destination["root_pane"]["pane_id"],
        "fixture must collide public locators"
    );
    let bystander = Bystander::start(
        &beta,
        &destination["root_pane"]["pane_id"],
        &format!("{label}-bystander"),
    );
    let omp = RestrictedOmp::start(
        &alpha,
        source["root_pane"]["pane_id"].as_str().unwrap(),
        provider,
        label,
    );
    let environment = std::fs::read(format!("/proc/{}/environ", omp.pid)).unwrap();
    let inherited = |key: &str| {
        environment
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(format!("{key}=").as_bytes()))
            .map(|value| String::from_utf8(value.to_vec()).unwrap())
            .unwrap_or_else(|| panic!("moved OMP lacks inherited {key}"))
    };
    let original_pane = inherited("HERDR_PANE_ID");
    let original_socket = PathBuf::from(inherited("HERDR_SOCKET_PATH"));
    omp.read(&alpha, "not_working");
    alpha.request("tab.transfer", json!({
        "tab_id":source["tab"]["tab_id"],"destination_session":beta.name,
        "workspace_id":destination["workspace"]["workspace_id"],"operation_id":format!("{label}-move")
    }));
    if stop_source {
        stop(&mut alpha);
    }
    assert_eq!(identity(&omp.pid), omp.identity);
    Moved {
        alpha,
        beta,
        omp,
        bystander,
        original_pane,
        original_socket,
    }
}

/// A fresh admitted restricted submit reaches the same original OMP in beta,
/// and its genuine lifecycle report is still accepted for the moved terminal.
fn assert_fresh_control_reaches_original(moved: &Moved, provider: &StubProvider, input: &str) {
    let fresh = moved.omp.read(&moved.beta, "not_working");
    let text = format!("owned fresh restricted instruction {input}");
    let submitted = moved.omp.submit(&moved.beta, &fresh, input, &text);
    assert_eq!(
        submitted["result"]["result"]["status"], "accepted_by_reporter",
        "fresh restricted submit failed: {submitted}"
    );
    assert!(provider.next_request().contains(&text));
    wait_status(&moved.beta, &moved.omp.terminal, "working");
    provider.release();
    moved.omp.read(&moved.beta, "not_working");
    assert_eq!(moved.omp.journal_count(&text), 1);
    assert_eq!(identity(&moved.omp.pid), moved.omp.identity);
}

#[test]
#[ignore = "requires installed interactive OMP; owned loopback model provider"]
fn second_restricted_submit_while_one_is_outstanding_is_rejected_after_transfer() {
    let provider = StubProvider::start();
    let moved = move_restricted_omp(&provider, "restricted-outstanding", false);
    let retained = moved.omp.read(&moved.beta, "not_working");
    let first = "owned first outstanding instruction";
    let submitted = moved
        .omp
        .submit(&moved.beta, &retained, "restricted-first", first);
    assert_eq!(
        submitted["result"]["result"]["status"], "accepted_by_reporter",
        "first restricted submit failed: {submitted}"
    );
    assert!(provider.next_request().contains(first));
    wait_status(&moved.beta, &moved.omp.terminal, "working");
    // Let the reporter's metadata observe the admitted instruction first.
    moved.omp.read(&moved.beta, "working");
    // The retained pre-submit context still names the same binding and epochs.
    let stale = moved.omp.submit(
        &moved.beta,
        &retained,
        "restricted-outstanding-second",
        "outstanding retry must not run",
    );
    assert_refused(
        &stale,
        &[
            "structured_unavailable",
            "instruction_outstanding",
            "agent_busy",
        ],
        "second submit while outstanding",
    );
    assert_withholds_submission(&moved.omp.read(&moved.beta, "working"));
    provider.assert_no_request(Duration::from_millis(750), "outstanding refusal");
    assert_eq!(
        moved
            .omp
            .input_status(&moved.beta, &retained, "restricted-outstanding-second")["result"]
            ["result"]["status"],
        "unknown",
        "a refused outstanding submit acquired a reporter receipt"
    );
    provider.release();
    moved.omp.read(&moved.beta, "not_working");
    assert_eq!(moved.omp.journal_count("outstanding retry must not run"), 0);
    assert_eq!(moved.omp.journal_count(first), 1);
    moved
        .bystander
        .assert_unchanged(&moved.beta, "restricted-outstanding-bystander");
    assert_fresh_control_reaches_original(&moved, &provider, "restricted-outstanding-fresh");
}

#[test]
#[ignore = "requires installed interactive OMP; owned loopback model provider"]
fn restricted_submit_bound_to_a_replaced_conversation_is_rejected_after_transfer() {
    let provider = StubProvider::start();
    let mut moved = move_restricted_omp(&provider, "restricted-replaced", true);
    let retained = moved.omp.read(&moved.beta, "not_working");
    let before = agent(&moved.beta, &moved.omp.terminal);
    // A local /new replaces the conversation without a model request.
    let pane = pane_for(&moved.beta, &moved.omp.terminal);
    moved.beta.request("pane.focus", json!({"pane_id":pane}));
    let mut client = moved.beta.attach();
    let deadline = Instant::now() + Duration::from_secs(8);
    while client.focused_pane_id.as_deref() != pane.as_str() {
        assert!(Instant::now() < deadline, "client did not select moved OMP");
        client.receive().unwrap();
    }
    client.type_text("/new").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !moved.beta.request(
        "pane.read",
        json!({"pane_id":pane,"source":"visible","format":"text"}),
    )["read"]["text"]
        .as_str()
        .is_some_and(|text| text.contains("/new"))
    {
        assert!(Instant::now() < deadline, "OMP editor did not render /new");
        std::thread::sleep(Duration::from_millis(20));
    }
    moved
        .beta
        .request("pane.send_keys", json!({"pane_id":pane,"keys":["enter"]}));
    let replaced = wait_session(&moved.beta, &moved.omp.terminal, Some(&before));
    // Pairing is per conversation: enroll the replacement explicitly.
    moved.omp.enroll(&replaced);
    let stale = moved.omp.submit(
        &moved.beta,
        &retained,
        "restricted-replaced-stale",
        "replaced conversation must not run",
    );
    assert_refused(&stale, &["binding_mismatch"], "stale conversation submit");
    provider.assert_no_request(Duration::from_millis(750), "stale conversation refusal");
    let fresh = moved.omp.read(&moved.beta, "not_working");
    assert_ne!(
        fresh["binding"]["conversation_id"],
        retained["binding"]["conversation_id"]
    );
    assert_eq!(
        moved
            .omp
            .journal_count("replaced conversation must not run"),
        0
    );
    moved
        .bystander
        .assert_unchanged(&moved.beta, "restricted-replaced-bystander");
    assert_fresh_control_reaches_original(&moved, &provider, "restricted-replaced-fresh");
}

#[test]
#[ignore = "requires installed interactive OMP; owned loopback model provider"]
fn conflicting_nested_root_report_is_rejected_after_transfer() {
    let provider = StubProvider::start();
    let moved = move_restricted_omp(&provider, "restricted-root", true);
    let retained = moved.omp.read(&moved.beta, "not_working");
    let before = agent(&moved.beta, &moved.omp.terminal);
    let api = trailboss_runtime::api_client::ApiClient::for_target(
        trailboss_runtime::api_client::ConnectionTarget::SocketPath(moved.original_socket.clone()),
    );
    let conflicting = moved.alpha.home.join("conflicting-nested-root.jsonl");
    let request = serde_json::from_value(json!({
        "id":"conflicting-nested-root","method":"pane.report_agent","params":{
            "pane_id":moved.original_pane,"source":"herdr:omp","agent":"omp",
            // Fresher than any genuine reporter sequence (milliseconds * 1000).
            "state":"working","seq":9_007_199_254_740_991_u64,"agent_session_path":conflicting
        }
    }))
    .unwrap();
    let reply = api
        .request_value_with_timeout(&request, Duration::from_secs(5))
        .unwrap();
    assert!(
        reply.get("error").is_none(),
        "inherited report transport failed: {reply}"
    );
    std::thread::sleep(Duration::from_millis(250));
    let after = agent(&moved.beta, &moved.omp.terminal);
    assert_eq!(
        after["agent_session"], before["agent_session"],
        "conflicting root report rebound the moved OMP"
    );
    assert_eq!(
        after["agent_status"], before["agent_status"],
        "conflicting root report changed the moved lifecycle"
    );
    let unchanged = moved.omp.read(&moved.beta, "not_working");
    assert_eq!(unchanged["binding"], retained["binding"]);
    assert_eq!(unchanged["reporter_epoch"], retained["reporter_epoch"]);
    moved
        .bystander
        .assert_unchanged(&moved.beta, "restricted-root-bystander");
    // Genuine lower-sequence reports remain accepted for the moved terminal.
    assert_fresh_control_reaches_original(&moved, &provider, "restricted-root-fresh");
}

#[test]
#[ignore = "requires installed interactive OMP; owned loopback model provider"]
fn deleting_stopped_source_keeps_restricted_controls_until_moved_processes_end() {
    let provider = StubProvider::start();
    let moved = move_restricted_omp(&provider, "restricted-delete", true);
    let refused = cli(
        &moved.alpha,
        &["session", "delete", &moved.alpha.name, "--json"],
    );
    assert!(
        !refused.status.success(),
        "deletion must reject a live moved process route"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("live moved process"),
        "deletion must explain that live routing retains the source endpoint"
    );
    assert!(moved.original_socket.exists());
    assert_fresh_control_reaches_original(&moved, &provider, "restricted-delete-live");
    moved
        .bystander
        .assert_unchanged(&moved.beta, "restricted-delete-bystander");
    let pane = pane_for(&moved.beta, &moved.omp.terminal);
    let shell = moved
        .beta
        .request("pane.process_info", json!({"pane_id":pane}))["process_info"]["shell_pid"]
        .as_u64()
        .unwrap()
        .to_string();
    // End the owned OMP, then its original shell.
    assert!(Command::new("kill")
        .args(["-TERM", &moved.omp.pid])
        .status()
        .unwrap()
        .success());
    let deadline = Instant::now() + Duration::from_secs(8);
    while std::fs::read_link(format!("/proc/{}/exe", moved.omp.pid)).is_ok() {
        assert!(Instant::now() < deadline, "owned OMP did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    moved
        .beta
        .request("pane.send_text", json!({"pane_id":pane,"text":"exit\n"}));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let ended = std::fs::read_link(format!("/proc/{shell}/exe")).is_err();
        let removed = !moved.beta.request("session.snapshot", json!({}))["snapshot"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pane| pane["terminal_id"] == moved.omp.terminal);
        if ended && removed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "moved shell did not end and close its pane"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let deleted = cli(
        &moved.alpha,
        &["session", "delete", &moved.alpha.name, "--json"],
    );
    assert!(
        deleted.status.success(),
        "ordinary deletion failed after process exit: {}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    assert!(!moved.original_socket.exists());
    assert!(std::os::unix::net::UnixStream::connect(&moved.original_socket).is_err());
    moved
        .bystander
        .assert_unchanged(&moved.beta, "restricted-delete-bystander-after");
}
