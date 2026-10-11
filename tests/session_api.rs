use herdr_runtime::{
    api_client::{ApiClient, ApiClientError, ConnectionTarget},
    api_schema::{
        EventEnvelope, Request, ResponseResult, SubscriptionEventEnvelope, SuccessResponse,
    },
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SCHEMA: &str = include_str!("../crates/herdr-runtime/docs/next/api/herdr-api.schema.json");
const LEGACY: &str =
    include_str!("../crates/herdr-runtime/tests/fixtures/session/legacy-pre-tabs-v2.json");

struct Core {
    child: Child,
    home: PathBuf,
    socket: PathBuf,
    client: ApiClient,
    schemas: Value,
}
impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
impl Core {
    fn command_at(home: &std::path::Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-fork"));
        command
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("XDG_STATE_HOME", home)
            .env("XDG_DATA_HOME", home)
            .env("XDG_CACHE_HOME", home)
            .env("XDG_RUNTIME_DIR", home)
            .env("HERDR_STARTUP_CWD", home)
            .env("SHELL", "/bin/sh")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_CLIENT_SOCKET_PATH")
            .env_remove("HERDR_CONFIG_PATH")
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_PANE_ID")
            .current_dir(home)
            .stdin(Stdio::null());
        command
    }
    fn cli(&self, args: &[&str]) -> Output {
        Self::command_at(&self.home)
            .args(args)
            .args(["--session", "contract"])
            .output()
            .unwrap()
    }
    fn validate(&self, name: &str, value: &Value) {
        let schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "$ref":format!("#/schemas/{name}"), "schemas":self.schemas["schemas"]});
        let validator = jsonschema::validator_for(&schema).unwrap();
        validator
            .validate(value)
            .unwrap_or_else(|error| panic!("{name}: {error}: {value}"));
    }
    fn request(&self, method: &str, params: Value) -> Value {
        let wire = json!({"id":format!("real:{method}"),"method":method,"params":params});
        self.validate("request", &wire);
        let request: Request = serde_json::from_value(wire).unwrap();
        let response = self
            .client
            .request_value_with_timeout(&request, Duration::from_secs(5))
            .unwrap();
        self.response(&request.id, &response);
        response
    }
    fn response(&self, id: &str, response: &Value) {
        assert_eq!(response["id"], id, "request correlation changed");
        if response.get("error").is_some() {
            self.validate("error_response", response);
            let error: herdr_runtime::api_schema::ErrorResponse =
                serde_json::from_value(response.clone()).unwrap();
            assert!(!error.error.code.is_empty());
            assert!(!error.error.message.is_empty());
        } else {
            self.validate("success_response", response);
            let _: SuccessResponse = serde_json::from_value(response.clone()).unwrap();
        }
    }
}

fn given_reference_client_and_core() -> Core {
    let home = std::env::temp_dir().join(format!(
        "hf-api-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let session = home.join("herdr-fork/sessions/contract");
    std::fs::create_dir_all(&session).unwrap();
    // Restore the original compatibility fixture byte-for-byte, not a rewritten snapshot.
    std::fs::write(session.join("session.json"), LEGACY).unwrap();
    let socket = session.join("herdr.sock");
    let client = ApiClient::for_target(ConnectionTarget::SocketPath(socket.clone()));
    let mut core = Core {
        child: Core::command_at(&home)
            .args(["server", "--session", "contract"])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
        home,
        socket,
        client,
        schemas: serde_json::from_str(SCHEMA).unwrap(),
    };

    let deadline = Instant::now() + Duration::from_secs(8);
    while !core.socket.exists() {
        assert!(Instant::now() < deadline, "isolated core never started");
        assert!(
            core.child.try_wait().unwrap().is_none(),
            "isolated core exited"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    core
}

struct Observations {
    workspace: String,
    pane: String,
    events: Vec<Value>,
    methods: BTreeSet<String>,
}

fn read_line(reader: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn when_reference_automation_runs(core: &Core) -> Observations {
    let output = core.cli(&[
        "workspace",
        "create",
        "--label",
        "automation α",
        "--focus",
        "--cwd",
        core.home.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "existing workspace automation rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    core.response("cli:workspace:create", &created);
    let workspace = created["result"]["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let pane = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let snapshot = core.request("session.snapshot", json!({}));
    assert!(
        snapshot["result"]["snapshot"]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["label"] == "legacy"),
        "original legacy session fixture was not restored"
    );
    assert_eq!(core.client.status().unwrap().protocol, Some(22));

    let stream = UnixStream::connect(&core.socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut subscriber = BufReader::new(stream);
    let subscribe = json!({"id":"reference-sub","method":"events.subscribe","params":{"subscriptions":[{"type":"workspace.renamed"},{"type":"pane.output_matched","pane_id":pane,"source":"recent","match":{"type":"substring","value":"contract-pty-ready"}}]}});
    core.validate("request", &subscribe);
    // Original Request/serde transport; no new wire parser or translation.
    let request: Request = serde_json::from_value(subscribe).unwrap();
    serde_json::to_writer(subscriber.get_mut(), &request).unwrap();
    subscriber.get_mut().write_all(b"\n").unwrap();
    let started = read_line(&mut subscriber);
    core.response("reference-sub", &started);
    assert_eq!(started["result"]["type"], "subscription_started");
    let renamed = core.request(
        "workspace.rename",
        json!({"workspace_id":workspace,"label":"renamed β"}),
    );
    assert_eq!(renamed["result"]["workspace"]["label"], "renamed β");
    let event = read_line(&mut subscriber);
    core.validate("event", &event);
    let _: EventEnvelope = serde_json::from_value(event.clone()).unwrap();
    assert_eq!(event["event"], "workspace_renamed");
    assert_eq!(event["data"]["workspace_id"], workspace);
    assert_eq!(event["data"]["label"], "renamed β");
    // Build the marker in the shell so terminal input echo cannot satisfy the assertion.
    core.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":"printf 'contract-%s-ready\\n' pty\n"}),
    );
    let output_event = read_line(&mut subscriber);
    core.validate("subscription_event", &output_event);
    let _: SubscriptionEventEnvelope = serde_json::from_value(output_event.clone()).unwrap();
    assert_eq!(output_event["event"], "pane.output_matched");
    assert_eq!(output_event["data"]["pane_id"], pane);
    assert!(output_event["data"]["matched_line"]
        .as_str()
        .unwrap()
        .contains("contract-pty-ready"));
    let missing: Request = serde_json::from_value(
        json!({"id":"typed-missing","method":"workspace.get","params":{"workspace_id":"w999999"}}),
    )
    .unwrap();
    match core.client.request(missing).unwrap_err() {
        ApiClientError::ErrorResponse(error) => {
            assert_eq!(error.id, "typed-missing");
            assert_eq!(error.error.code, "workspace_not_found");
        }
        error => panic!("typed reference error changed: {error}"),
    }
    let mut methods = BTreeSet::new();
    let requests: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/session-api-methods.json")).unwrap();
    for wire in requests.iter().filter(|r| r["method"] != "server.stop") {
        core.validate("request", wire);
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        let response = core
            .client
            .request_value_with_timeout(&request, Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("{}: {e}", wire["method"]));
        core.response(&request.id, &response);
        assert!(methods.insert(wire["method"].as_str().unwrap().to_owned()));
    }
    Observations {
        workspace,
        pane,
        events: vec![event, output_event],
        methods,
    }
}

fn then_original_contract_holds(core: &Core, observations: Observations) {
    let snapshot = core.request("session.snapshot", json!({}));
    assert!(snapshot["result"]["snapshot"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["workspace_id"] == observations.workspace && w["label"] == "renamed β"));
    let pane = core.request("pane.get", json!({"pane_id":observations.pane}));
    assert_eq!(
        pane["result"]["pane"]["workspace_id"],
        observations.workspace
    );
    assert_eq!(observations.events.len(), 2);
    let mut exercised = observations.methods;
    let stop = core.request("server.stop", json!({}));
    assert_eq!(stop["result"]["type"], "ok");
    exercised.insert("server.stop".into());
    let reference: BTreeSet<String> = core.schemas["schemas"]["request"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            v["properties"]["method"]["const"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(exercised, reference, "reference methods omitted");
    assert_eq!(exercised.len(), 102);
}

#[test]
fn existing_session_automation_preserves_typed_wire_contracts() {
    let core = given_reference_client_and_core();
    let observations = when_reference_automation_runs(&core);
    then_original_contract_holds(&core, observations);
}

#[test]
fn living_session_api_feature() {
    let mut core = None;
    let mut observations = None;
    let mut steps = 0;
    for line in include_str!("../features/session-api.feature")
        .lines()
        .map(str::trim)
    {
        match line {
            "Feature: Session API compatibility" | "Scenario: Existing session automation works without protocol changes" | "" => continue,
            "Given the new core is built with the fork’s server-side session API reused verbatim and the fork’s unmodified session API client and compatibility fixtures are available" => { assert_eq!(steps, 0); core = Some(given_reference_client_and_core()); }
            "When the client exercises the reference session methods, response schemas and event subscriptions against the new core" => { assert_eq!(steps, 1); observations = Some(when_reference_automation_runs(core.as_ref().unwrap())); }
            "Then each result and event satisfies the original session API contract without rewriting the client or translating the protocol" => { assert_eq!(steps, 2); then_original_contract_holds(core.as_ref().unwrap(), observations.take().unwrap()); }
            _ => panic!("unbound living step: {line}"),
        }
        steps += 1;
    }
    assert_eq!(steps, 3);
}

#[test]
fn executable_session_automation_smoke() {
    let core = given_reference_client_and_core();
    let created = core.cli(&["workspace", "create", "--label", "binary smoke", "--focus"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created: Value = serde_json::from_slice(&created.stdout).unwrap();
    core.validate("success_response", &created);
    let id = created["result"]["workspace"]["workspace_id"]
        .as_str()
        .unwrap();
    let renamed = core.cli(&["workspace", "rename", id, "binary updated"]);
    assert!(renamed.status.success());
    let snapshot = core.cli(&["api", "snapshot"]);
    assert!(snapshot.status.success());
    let snapshot: Value = serde_json::from_slice(&snapshot.stdout).unwrap();
    core.validate("success_response", &snapshot);
    assert!(snapshot["result"]["snapshot"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["workspace_id"] == id && w["label"] == "binary updated"));
    let error = core.cli(&["workspace", "get", "w999999"]);
    assert_eq!(error.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&error.stderr).unwrap();
    core.validate("error_response", &error);
    let status = core
        .client
        .request(
            serde_json::from_value(json!({"id":"typed-ping","method":"ping","params":{}})).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        status.result,
        ResponseResult::Pong { protocol: 22, .. }
    ));
}
