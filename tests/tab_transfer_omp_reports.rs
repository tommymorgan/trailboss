#![cfg(target_os = "linux")]

mod support;

use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use support::Core;

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

fn identity(pid: &str) -> (String, String, PathBuf, PathBuf) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[4], "0", "OMP lost controlling PTY");
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
    command
        .args(["--session", &core.name])
        .args(args)
        .current_dir(&core.home)
        .output()
        .unwrap()
}

fn agent(core: &Core, terminal: &Value, stage: &str) -> Value {
    core.request("agent.list", json!({}))["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| &agent["terminal_id"] == terminal)
        .unwrap_or_else(|| {
            panic!(
                "{stage}: managed agent terminal {terminal} absent in {}",
                core.name
            )
        })
        .clone()
}

fn wait_session(core: &Core, terminal: &Value, previous: Option<&Value>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let current = core.request("agent.list", json!({}))["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|agent| &agent["terminal_id"] == terminal)
            .cloned()
            .unwrap_or(Value::Null);
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
            "genuine OMP {} report did not arrive for terminal {terminal}; last metadata: {}",
            if previous.is_some() {
                "post-source-exit /new"
            } else {
                "initial startup"
            },
            json!({"agent":current["agent"],"session":current["agent_session"],"launch_pending":current["launch_pending"],"interactive_ready":current["interactive_ready"]})
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires installed interactive OMP; explicit no-model report-routing proof"]
fn genuine_omp_local_new_session_reports_after_transfer_and_source_exit() {
    assert_genuine_omp_reports_after_source_exit(false);
}

#[test]
#[ignore = "requires installed interactive OMP; explicit no-model report-routing proof"]
fn restarted_source_cannot_capture_genuine_moved_omp_reports() {
    assert_genuine_omp_reports_after_source_exit(true);
}

fn assert_genuine_omp_reports_after_source_exit(restart_source: bool) {
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
    let pane = source["root_pane"]["pane_id"].as_str().unwrap();
    let terminal =
        alpha.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone();
    let bystander_terminal = beta.request(
        "pane.get",
        json!({"pane_id":destination["root_pane"]["pane_id"]}),
    )["pane"]["terminal_id"]
        .clone();
    beta.request(
        "pane.send_text",
        json!({"pane_id":destination["root_pane"]["pane_id"],
        "text":"bystander_local=original-beta; printf '%s\\n' \"$$\" > report-bystander.pid\n"}),
    );
    let bystander_pid = wait_file(&alpha.home.join("report-bystander.pid"))
        .trim()
        .to_owned();
    let bystander_process = identity(&bystander_pid);
    let before = start_managed_omp(&alpha, pane, &terminal);
    let process = alpha.request("pane.process_info", json!({"pane_id":pane}));
    let pid = process["process_info"]["foreground_process_group_id"]
        .as_u64()
        .unwrap()
        .to_string();
    let original_process = identity(&pid);
    let original_environment = std::fs::read(format!("/proc/{pid}/environ")).unwrap();
    let original_endpoint = alpha
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", alpha.name));
    use std::os::unix::fs::MetadataExt;
    let endpoint_metadata = std::fs::metadata(&original_endpoint).unwrap();
    let endpoint_identity = (endpoint_metadata.dev(), endpoint_metadata.ino());
    alpha.request("tab.transfer", json!({"tab_id":source["tab"]["tab_id"],"destination_session":beta.name,
        "workspace_id":destination["workspace"]["workspace_id"],"operation_id":"genuine-omp-reports"}));
    let retained = agent(&beta, &terminal, "after transfer");
    assert_eq!(
        retained["agent_session"], before["agent_session"],
        "transfer changed retained OMP session"
    );
    stop(&mut alpha);
    let restarted = restart_source.then(|| {
        Core::launch_with_executable(
            alpha.home.clone(),
            alpha.name.clone(),
            support::executable(),
            false,
        )
    });
    let restarted_work = restarted.as_ref().map(|core| {
        let workspace = core.request("workspace.create", json!({
            "label":"Same root","cwd":core.home,"focus":true,
        }));
        let pane = core.request("pane.get", json!({"pane_id":workspace["root_pane"]["pane_id"]}))["pane"].clone();
        assert_ne!(pane["terminal_id"], terminal, "restarted source reused the moved terminal identity");
        core.request("pane.send_text", json!({"pane_id":pane["pane_id"],
            "text":"new_alpha_local=original-new-alpha; printf '%s\\n' \"$$\" > report-restarted-alpha.pid\n"}));
        let pid = wait_file(&core.home.join("report-restarted-alpha.pid")).trim().to_owned();
        let process = identity(&pid);
        (pane, pid, process)
    });
    let bystander_before = beta.request(
        "pane.get",
        json!({"pane_id":destination["root_pane"]["pane_id"]}),
    )["pane"]
        .clone();
    assert_eq!(bystander_before["terminal_id"], bystander_terminal);
    let fresh = new_local_session(&beta, &terminal, &before);
    assert_ne!(
        fresh["agent_session"], before["agent_session"],
        "retained metadata is not evidence of a fresh report"
    );
    assert_eq!(fresh["terminal_id"], terminal);
    assert_eq!(identity(&pid), original_process);
    assert!(
        std::fs::read(format!("/proc/{pid}/environ")).unwrap() == original_environment,
        "OMP inherited environment changed"
    );
    let endpoint_after = std::fs::metadata(&original_endpoint).unwrap();
    assert_eq!(
        (endpoint_after.dev(), endpoint_after.ino()),
        endpoint_identity,
        "original protected endpoint was replaced"
    );
    let bystander_after = beta.request(
        "pane.get",
        json!({"pane_id":destination["root_pane"]["pane_id"]}),
    )["pane"]
        .clone();
    for field in [
        "terminal_id",
        "agent",
        "agent_status",
        "agent_session",
        "state_labels",
        "title",
    ] {
        assert_eq!(
            bystander_after[field], bystander_before[field],
            "moved OMP report changed beta's unrelated {field}"
        );
    }
    assert_eq!(identity(&bystander_pid), bystander_process);
    beta.request(
        "pane.send_text",
        json!({"pane_id":bystander_after["pane_id"],
        "text":"printf '%s\\n' \"$bystander_local\" > report-bystander-after\n"}),
    );
    assert_eq!(
        wait_file(&beta.home.join("report-bystander-after")),
        "original-beta\n"
    );
    if let (Some(core), Some((pane, restarted_pid, process))) = (&restarted, &restarted_work) {
        let after = core.request("pane.get", json!({"pane_id":pane["pane_id"]}))["pane"].clone();
        for field in [
            "terminal_id",
            "agent",
            "agent_status",
            "agent_session",
            "state_labels",
            "title",
        ] {
            assert_eq!(
                after[field], pane[field],
                "moved OMP report changed restarted alpha's {field}"
            );
        }
        assert_eq!(identity(restarted_pid), *process);
        core.request(
            "pane.send_text",
            json!({"pane_id":pane["pane_id"],
            "text":"printf '%s\\n' \"$new_alpha_local\" > report-restarted-alpha-after\n"}),
        );
        assert_eq!(
            wait_file(&core.home.join("report-restarted-alpha-after")),
            "original-new-alpha\n"
        );
    }
    let inventory = beta.request("session.list", json!({}));
    assert_eq!(
        inventory["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["name"] == alpha.name)
            .unwrap()["running"],
        restart_source
    );
}

fn start_managed_omp(core: &Core, pane: &str, terminal: &Value) -> Value {
    let host_agent_dir = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".omp/agent"));
    core.request("pane.send_text", json!({"pane_id":pane,"text":format!(
        "export PI_CODING_AGENT_DIR='{}'; printf 'prepared\\n' > report-omp-prepared\n", host_agent_dir.display())}));
    assert_eq!(
        wait_file(&core.home.join("report-omp-prepared")),
        "prepared\n"
    );
    let reporter = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("crates/trailboss-runtime/src/integration/assets/omp/trailboss-agent-state.ts");
    let sessions = core.home.join("owned-report-sessions");
    std::fs::create_dir(&sessions).unwrap();
    let started = cli(
        core,
        &[
            "agent",
            "start",
            "owned-report-omp",
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
            "--max-time",
            "120",
            "--max-cost",
            "0.01",
            "-e",
            reporter.to_str().unwrap(),
        ],
    );
    assert!(
        started.status.success(),
        "managed OMP startup failed: {}",
        String::from_utf8_lossy(&started.stderr)
    );
    wait_session(core, terminal, None)
}

fn stop(core: &mut Core) {
    core.request("server.stop", json!({}));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = core.child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "owned source Core did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn new_local_session(core: &Core, terminal: &Value, before: &Value) -> Value {
    let moved = agent(core, terminal, "before local /new");
    core.request("pane.focus", json!({"pane_id":moved["pane_id"]}));
    let mut client = core.attach();
    let attachment_deadline = Instant::now() + Duration::from_secs(8);
    while client.focused_pane_id.as_deref() != moved["pane_id"].as_str() {
        assert!(
            Instant::now() < attachment_deadline,
            "destination client did not select the moved OMP"
        );
        client.receive().unwrap();
    }
    // /new is local. Encoded Enter submits it without a model request.
    client.type_text("/new").unwrap();
    let input_deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let screen = core.request(
            "pane.read",
            json!({"pane_id":moved["pane_id"],"source":"visible","format":"text"}),
        );
        if screen["read"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("/new"))
        {
            break;
        }
        assert!(
            Instant::now() < input_deadline,
            "owned OMP editor did not render local /new command before Enter"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    core.request(
        "pane.send_keys",
        json!({"pane_id":moved["pane_id"],"keys":["enter"]}),
    );
    wait_session(core, terminal, Some(before))
}

#[test]
#[ignore = "requires installed interactive OMP; explicit no-model report-routing proof"]
fn genuine_omp_reports_reach_third_owner_after_both_previous_cores_exit() {
    assert_genuine_omp_reports_after_second_move(false);
}

#[test]
#[ignore = "requires installed interactive OMP; explicit no-model report-routing proof"]
fn genuine_omp_reports_return_to_original_owner_after_intermediate_core_exits() {
    assert_genuine_omp_reports_after_second_move(true);
}

fn assert_genuine_omp_reports_after_second_move(return_home: bool) {
    let mut alpha = Core::new();
    let mut beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let gamma = (!return_home).then(|| {
        Core::launch_with_executable(
            alpha.home.clone(),
            format!("{}-gamma", alpha.name),
            support::executable(),
            false,
        )
    });
    let owner = gamma.as_ref().unwrap_or(&alpha);
    let source = alpha.request(
        "workspace.create",
        json!({"label":"Original OMP","cwd":alpha.home}),
    );
    let middle = beta.request(
        "workspace.create",
        json!({"label":"Transit","cwd":beta.home}),
    );
    let destination = owner.request(
        "workspace.create",
        json!({"label":"Final owner","cwd":owner.home}),
    );
    let pane = source["root_pane"]["pane_id"].as_str().unwrap();
    let terminal =
        alpha.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone();
    let before = start_managed_omp(&alpha, pane, &terminal);
    let pid = alpha.request("pane.process_info", json!({"pane_id":pane}))["process_info"]
        ["foreground_process_group_id"]
        .as_u64()
        .unwrap()
        .to_string();
    let process = identity(&pid);
    let environment = std::fs::read(format!("/proc/{pid}/environ")).unwrap();
    let endpoint = alpha
        .home
        .join(format!("trailboss/sessions/{}/trailboss.sock", alpha.name));
    use std::os::unix::fs::MetadataExt;
    let original_socket = std::fs::metadata(&endpoint).unwrap();
    let bystander = owner.request(
        "pane.get",
        json!({"pane_id":destination["root_pane"]["pane_id"]}),
    )["pane"]
        .clone();
    owner.request(
        "pane.send_text",
        json!({"pane_id":bystander["pane_id"],
        "text":"owner_local=original-owner; printf '%s\\n' \"$$\" > report-owner.pid\n"}),
    );
    let owner_pid = wait_file(&owner.home.join("report-owner.pid"))
        .trim()
        .to_owned();
    let owner_process = identity(&owner_pid);
    alpha.request(
        "tab.transfer",
        json!({"tab_id":source["tab"]["tab_id"],"destination_session":beta.name,
        "workspace_id":middle["workspace"]["workspace_id"],"operation_id":"omp-to-beta"}),
    );
    let in_beta = agent(&beta, &terminal, "in intermediate owner beta");
    let moving = beta.request("pane.get", json!({"pane_id":in_beta["pane_id"]}));
    beta.request("tab.transfer",json!({"tab_id":moving["pane"]["tab_id"],"destination_session":owner.name,
        "workspace_id":destination["workspace"]["workspace_id"],"operation_id":"omp-to-final-owner"}));
    let fresh_before_exit = new_local_session(owner, &terminal, &before);
    // An adversarial stale packet uses the former owner's real pane/session
    // context. Fresh reporting above and below still comes from unchanged OMP.
    let old_session = &before["agent_session"];
    let mut stale = json!({
        "pane_id":in_beta["pane_id"],"source":old_session["source"],
        "agent":"omp","state":"working","seq":0
    });
    match old_session["kind"].as_str().unwrap() {
        "id" => stale["agent_session_id"] = old_session["value"].clone(),
        "path" => stale["agent_session_path"] = old_session["value"].clone(),
        kind => panic!("unsupported genuine session reference {kind}"),
    }
    beta.request("pane.report_agent", stale);
    let after_stale = agent(owner, &terminal, "after former-owner stale report");
    assert_eq!(
        after_stale["agent_session"], fresh_before_exit["agent_session"],
        "a former owner's stale report replaced the current genuine session"
    );
    assert_eq!(
        after_stale["agent_status"], fresh_before_exit["agent_status"],
        "a former owner's stale report changed the current lifecycle"
    );
    if !return_home {
        stop(&mut alpha);
    }
    stop(&mut beta);
    let owner = gamma.as_ref().unwrap_or(&alpha);
    let fresh = new_local_session(owner, &terminal, &fresh_before_exit);
    assert_eq!(fresh["terminal_id"], terminal);
    assert_ne!(fresh["agent_session"], before["agent_session"]);
    assert_eq!(identity(&pid), process);
    assert!(
        std::fs::read(format!("/proc/{pid}/environ")).unwrap() == environment,
        "OMP inherited environment changed"
    );
    let socket = std::fs::metadata(&endpoint).unwrap();
    assert_eq!(
        (socket.dev(), socket.ino()),
        (original_socket.dev(), original_socket.ino())
    );
    let after = owner.request("pane.get", json!({"pane_id":bystander["pane_id"]}))["pane"].clone();
    for field in [
        "terminal_id",
        "agent",
        "agent_status",
        "agent_session",
        "state_labels",
        "title",
    ] {
        assert_eq!(
            after[field], bystander[field],
            "moved OMP report changed final owner's unrelated {field}"
        );
    }
    assert_eq!(identity(&owner_pid), owner_process);
    owner.request(
        "pane.send_text",
        json!({"pane_id":after["pane_id"],
        "text":"printf '%s\\n' \"$owner_local\" > report-owner-after\n"}),
    );
    assert_eq!(
        wait_file(&owner.home.join("report-owner-after")),
        "original-owner\n"
    );
    let inventory = owner.request("session.list", json!({}));
    for (name, running) in [(&alpha.name, return_home), (&beta.name, false)] {
        assert_eq!(
            inventory["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|session| session["name"].as_str() == Some(name.as_str()))
                .unwrap()["running"],
            running
        );
    }
}
