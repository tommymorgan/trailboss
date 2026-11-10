mod support;

use serde_json::json;
use std::process::{Command, Output};

fn cli(core: &support::Core, args: &[&str], pane: Option<&str>) -> Output {
    cli_command(core, args, pane).output().unwrap()
}

fn cli_command(core: &support::Core, args: &[&str], pane: Option<&str>) -> Command {
    cli_command_target(core, args, pane, true)
}

fn cli_command_target(
    core: &support::Core,
    args: &[&str],
    pane: Option<&str>,
    explicit: bool,
) -> Command {
    let mut command = Command::new(support::executable());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("HERDR_") || key == "TRAILBOSS_SESSION" {
            command.env_remove(key);
        }
    }
    for key in support::HOMES {
        command.env(key, &core.home);
    }
    if let Some(pane) = pane {
        command.env("HERDR_PANE_ID", pane);
    }
    if explicit {
        command.args(["--session", &core.name]);
    }
    command.args(args);
    command
}

#[test]
fn retained_shell_can_run_second_named_move_after_original_core_exit() {
    let mut alpha = owned_alpha();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    alpha.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":"printf '%s\\n' \"$$\" > retained-shell.pid\n"}),
    );
    let pid = wait_file(&alpha.home.join("retained-shell.pid"))
        .trim()
        .to_owned();
    let identity = process_identity(&pid);
    let terminal =
        alpha.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone();
    beta.request("workspace.create", json!({"label":"Work","cwd":beta.home}));
    let first = cli(
        &alpha,
        &["command", "run", "Move to session…", "beta / Work"],
        Some(pane),
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    alpha.request("server.stop", json!({}));
    assert!(
        alpha.child.wait().unwrap().success(),
        "original Core must have exited before the second named move"
    );
    let gamma = support::Core::launch_with_executable(
        alpha.home.clone(),
        "gamma".into(),
        support::executable(),
        false,
    );
    gamma.request("workspace.create", json!({"label":"Work","cwd":gamma.home}));
    let snapshot = beta.request("session.snapshot", json!({}));
    let moved = snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["terminal_id"] == terminal)
        .unwrap()["pane_id"]
        .clone();
    beta.request("pane.send_text", json!({"pane_id":moved,"text":format!(
        "'{}' command run 'Move to session…' 'gamma / Work' > retained-move.json 2> retained-move.err; printf '%s\\n' \"$?\" > retained-move.done\n",
        support::executable().display(),
    )}));
    assert_eq!(
        wait_file(&alpha.home.join("retained-move.done")).trim(),
        "0",
        "{}",
        std::fs::read_to_string(alpha.home.join("retained-move.err")).unwrap()
    );
    assert!(
        gamma.request("session.snapshot", json!({}))["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["label"] == "Planning")
    );
    assert_eq!(process_identity(&pid), identity);
}

#[test]
fn lost_named_move_response_keeps_caller_recoverable_operation_identity() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    let alpha = owned_alpha();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    beta.request("workspace.create", json!({"label":"Work","cwd":beta.home}));
    let proxy_path = alpha.home.join("lost-response.sock");
    let listener = UnixListener::bind(&proxy_path).unwrap();
    let source = alpha.home.join("trailboss/sessions/alpha/trailboss.sock");
    let proxy = std::thread::spawn(move || loop {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut client = BufReader::new(stream);
        let mut request = String::new();
        client.read_line(&mut request).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&request).unwrap();
        let mut upstream = BufReader::new(UnixStream::connect(&source).unwrap());
        upstream
            .get_mut()
            .set_read_timeout(Some(std::time::Duration::from_secs(15)))
            .unwrap();
        upstream.get_mut().write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        upstream.read_line(&mut response).unwrap();
        if parsed["method"] == "ui.command.run" {
            return (
                parsed,
                serde_json::from_str::<serde_json::Value>(&response).unwrap(),
            );
        }
        client.get_mut().write_all(response.as_bytes()).unwrap();
    });
    let output = cli_command_target(
        &alpha,
        &["command", "run", "Move to session…", "beta / Work"],
        Some(pane),
        false,
    )
    .env("HERDR_SOCKET_PATH", &proxy_path)
    .output()
    .unwrap();
    let (request, committed) = proxy.join().unwrap();
    assert!(committed["result"]["transfer"].is_object(), "{committed}");
    assert!(!output.status.success());
    let operation = committed["result"]["operation_id"]
        .as_str()
        .expect("success exposes recovery identity");
    assert_eq!(
        operation,
        format!("command-tab-move-{}", request["id"].as_str().unwrap())
    );
    let recovered = alpha.request("tab.transfer.status", json!({"operation_id":operation}));
    assert_eq!(recovered["status"]["operation_id"], operation);
    assert_eq!(recovered["status"]["phase"], "committed");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(operation),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        operation_files(&alpha),
        2,
        "one operation JSON and one lock; no hidden retry"
    );
}

fn owned_alpha() -> support::Core {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!("command-run-boundary-{stamp}"));
    std::fs::create_dir(&home).unwrap();
    support::Core::launch(home, "alpha".into())
}

fn process_identity(pid: &str) -> String {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[0], "Z");
    format!(
        "{}:{}:{:?}",
        fields[19],
        fields[4],
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap()
    )
}

fn start_owned_reporter(core: &support::Core, pane: &str) -> (String, Vec<(String, String)>) {
    let terminal = core.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"]
        .as_str()
        .unwrap()
        .to_owned();
    core.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":format!(
            "HERDR_AGENT=omp python3 '{}/tests/support/stale_pane_reporter.py' '{}'\n",
            env!("CARGO_MANIFEST_DIR"), core.home.display(),
        )}),
    );
    let ready: serde_json::Value =
        serde_json::from_str(&wait_file(&core.home.join("reporter.ready"))).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while core.request("pane.get", json!({"pane_id":pane}))["pane"]["agent"] != "omp" {
        assert!(
            std::time::Instant::now() < deadline,
            "owned foreground reporter was not detected"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::write(core.home.join("working.go"), "").unwrap();
    let replies: serde_json::Value =
        serde_json::from_str(&wait_file(&core.home.join("working.reply"))).unwrap();
    assert!(
        replies
            .as_array()
            .unwrap()
            .iter()
            .all(|reply| reply["result"]["type"] == "ok"),
        "{replies}"
    );
    let identities = ["shell_pid", "pid"]
        .iter()
        .map(|field| {
            let pid = ready[field].as_u64().unwrap().to_string();
            let identity = process_identity(&pid);
            (pid, identity)
        })
        .collect();
    (terminal, identities)
}

fn assert_owned_reporter_continues(core: &support::Core, proof: (String, Vec<(String, String)>)) {
    let snapshot = core.request("session.snapshot", json!({}));
    let pane = snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["terminal_id"] == proof.0)
        .unwrap()["pane_id"]
        .clone();
    std::fs::write(core.home.join("idle.go"), "").unwrap();
    let replies: serde_json::Value =
        serde_json::from_str(&wait_file(&core.home.join("idle.reply"))).unwrap();
    assert!(
        replies
            .as_array()
            .unwrap()
            .iter()
            .all(|reply| reply["result"]["type"] == "ok"),
        "{replies}"
    );
    assert_eq!(
        core.request("pane.get", json!({"pane_id":pane}))["pane"]["agent_session"]["value"],
        "retained-session"
    );
    for (pid, identity) in proof.1 {
        assert_eq!(process_identity(&pid), identity, "process/PTY restarted");
    }
    std::fs::write(core.home.join("reporter.done"), "").unwrap();
}

#[test]
fn list_shows_only_first_picker_prompt_hint() {
    let alpha = support::Core::new();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    for workspace in beta.request("workspace.list", json!({}))["workspaces"]
        .as_array()
        .unwrap()
    {
        beta.request(
            "workspace.close",
            json!({"workspace_id":workspace["workspace_id"]}),
        );
    }
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    let list = alpha.request("ui.command.list", json!({"pane_id":pane}));
    let command = list["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["title"] == "Move to session…")
        .unwrap();
    assert_eq!(command["argument_hint"], "Move to session");
    assert!(!list.to_string().contains("New destination space name"));
    let output = cli(&alpha, &["command", "list"], Some(pane));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Move to session…") && text.contains("Move to session"));
    assert!(!text.contains("New destination space name"));
    let first = cli(&alpha, &["command", "run", "Move to session…"], Some(pane));
    let first = String::from_utf8_lossy(&first.stderr);
    assert!(
        first.contains("Move to session") && first.contains("beta / Create space…"),
        "{first}"
    );
    assert!(!first.contains("New destination space name"));
    let second = cli(
        &alpha,
        &["command", "run", "Move to session…", "beta / Create space…"],
        Some(pane),
    );
    let second = String::from_utf8_lossy(&second.stderr);
    assert!(
        second.contains("New destination space name") && !second.contains("valid choices"),
        "{second}"
    );
    assert_eq!(operation_files(&alpha), 0);
}

#[test]
fn reject_surplus_picker_answers_on_selected_branch() {
    let alpha = support::Core::new();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    beta.request("workspace.create", json!({"label":"Work","cwd":beta.home}));
    for create in [false, true] {
        if create {
            for workspace in beta.request("workspace.list", json!({}))["workspaces"]
                .as_array()
                .unwrap()
            {
                beta.request(
                    "workspace.close",
                    json!({"workspace_id":workspace["workspace_id"]}),
                );
            }
        }
        let before = alpha.request("session.snapshot", json!({}));
        let beta_before = beta.request("session.snapshot", json!({}));
        let args = if create {
            vec![
                "command",
                "run",
                "Move to session…",
                "beta / Create space…",
                "Build",
                "surplus",
            ]
        } else {
            vec!["command", "run", "Move to session…", "beta / Work", "Build"]
        };
        let output = cli(&alpha, &args, Some(pane));
        assert!(
            !output.status.success(),
            "surplus answer started a transfer"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("extra_answer"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(operation_files(&alpha), 0);
        assert_eq!(
            before["snapshot"]["tabs"],
            alpha.request("session.snapshot", json!({}))["snapshot"]["tabs"]
        );
        let beta_after = beta.request("session.snapshot", json!({}));
        assert_eq!(
            beta_before["snapshot"]["workspaces"],
            beta_after["snapshot"]["workspaces"]
        );
        assert_eq!(
            beta_before["snapshot"]["tabs"],
            beta_after["snapshot"]["tabs"]
        );
    }
    alpha.request("tab.close", json!({"tab_id":caller["tab"]["tab_id"]}));
    let history = alpha.request("tab.closed.list", json!({}));
    let label = history_label(&history["closed_tabs"][0]);
    let output = cli(
        &alpha,
        &["command", "run", "Recently closed tabs", &label, "surplus"],
        None,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("extra_answer"));
    assert_eq!(history, alpha.request("tab.closed.list", json!({})));
}

fn operation_files(core: &support::Core) -> usize {
    std::fs::read_dir(core.home.join("trailboss/tab-transfer"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("op-")
        })
        .count()
}

#[test]
fn missing_followup_shows_actual_next_question_without_dispatch() {
    let alpha = support::Core::new();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    for workspace in beta.request("workspace.list", json!({}))["workspaces"]
        .as_array()
        .unwrap()
    {
        beta.request(
            "workspace.close",
            json!({"workspace_id":workspace["workspace_id"]}),
        );
    }
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    let before = alpha.request("session.snapshot", json!({}));
    for answer in [None, Some(""), Some("   ")] {
        let mut args = vec!["command", "run", "Move to session…", "beta / Create space…"];
        args.extend(answer);
        let output = cli(&alpha, &args, Some(pane));
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(
            diagnostic.contains("New destination space name"),
            "{diagnostic}"
        );
        assert!(!diagnostic.contains("valid choices"), "{diagnostic}");
        if answer.is_none() {
            assert!(diagnostic.contains("text answer required"), "{diagnostic}");
        }
        assert_eq!(
            operation_files(&alpha),
            0,
            "validation must precede tab.transfer"
        );
        assert!(beta.request("workspace.list", json!({}))["workspaces"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            before["snapshot"]["tabs"],
            alpha.request("session.snapshot", json!({}))["snapshot"]["tabs"]
        );
    }
}

#[test]
fn create_destination_space_with_next_ordered_answer() {
    let alpha = owned_alpha();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    for workspace in beta.request("workspace.list", json!({}))["workspaces"]
        .as_array()
        .unwrap()
    {
        beta.request(
            "workspace.close",
            json!({"workspace_id":workspace["workspace_id"]}),
        );
    }
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    let continuity = start_owned_reporter(&alpha, pane);
    let output = cli(
        &alpha,
        &[
            "command",
            "run",
            "Move to session…",
            "beta / Create space…",
            "Build",
        ],
        Some(pane),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot = beta.request("session.snapshot", json!({}));
    assert_eq!(
        snapshot["snapshot"]["workspaces"].as_array().unwrap().len(),
        1
    );
    assert_eq!(snapshot["snapshot"]["workspaces"][0]["label"], "Build");
    assert_eq!(snapshot["snapshot"]["tabs"][0]["label"], "Planning");
    assert_eq!(snapshot["snapshot"]["panes"].as_array().unwrap().len(), 1);
    assert_owned_reporter_continues(&beta, continuity);
}

#[test]
fn picker_uses_current_validation_and_preserves_unresolved_error() {
    use std::os::unix::fs::PermissionsExt;
    let alpha = support::Core::new();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    let work = beta.request("workspace.create", json!({"label":"Work","cwd":beta.home}));
    beta.request(
        "workspace.close",
        json!({"workspace_id":work["workspace"]["workspace_id"]}),
    );
    let rejected = cli(
        &alpha,
        &["command", "run", "Move to session…", "beta / Work"],
        Some(pane),
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("invalid_choice"));
    beta.request("workspace.create", json!({"label":"Work","cwd":beta.home}));
    let root = alpha.home.join("trailboss/tab-transfer");
    let registration = root.join("core-62657461.json");
    let original = std::fs::read(&registration).unwrap();
    let mut incompatible: serde_json::Value = serde_json::from_slice(&original).unwrap();
    incompatible["version"] = json!(0);
    std::fs::write(&registration, incompatible.to_string()).unwrap();
    let rejected = cli(
        &alpha,
        &["command", "run", "Move to session…", "beta / Work"],
        Some(pane),
    );
    std::fs::write(&registration, original).unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("invalid_choice"));
    // Discovery remains readable; creating the operation lock fails after dispatch.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500)).unwrap();
    let unresolved = cli(
        &alpha,
        &["command", "run", "Move to session…", "beta / Work"],
        Some(pane),
    );
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let diagnostic = String::from_utf8_lossy(&unresolved.stderr);
    assert!(!unresolved.status.success());
    assert!(
        diagnostic.contains("transfer_unresolved")
            && diagnostic.contains("operation_id=command-tab-move-")
            && diagnostic.contains("do not repeat the move"),
        "{diagnostic}"
    );
    assert_eq!(
        diagnostic.matches("operation_id=").count(),
        1,
        "{diagnostic}"
    );
    drop(beta);
    let stopped = cli(
        &alpha,
        &["command", "run", "Move to session…", "beta / Work"],
        Some(pane),
    );
    assert!(!stopped.status.success());
    assert!(String::from_utf8_lossy(&stopped.stderr).contains("invalid_choice"));
    alpha.request("tab.close", json!({"tab_id":caller["tab"]["tab_id"]}));
    let history = alpha.request("tab.closed.list", json!({}));
    let entry = &history["closed_tabs"][0];
    let label = history_label(entry);
    alpha.request(
        "tab.closed.restore",
        json!({"closed_tab_id":entry["closed_tab_id"]}),
    );
    let stale = cli(
        &alpha,
        &["command", "run", "Recently closed tabs", &label],
        None,
    );
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("invalid_choice"));
}

#[test]
fn reject_invalid_picker_labels_without_mutation() {
    let alpha = support::Core::new();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let (caller, _) = fixture(&alpha);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    for label in ["Work", "Work", "Other"] {
        beta.request("workspace.create", json!({"label":label,"cwd":beta.home}));
    }
    let before = alpha.request("session.snapshot", json!({}));
    for answer in [None, Some("missing"), Some("beta / Work")] {
        let mut args = vec!["command", "run", "Move to session…"];
        args.extend(answer);
        let output = cli(&alpha, &args, Some(pane));
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{diagnostic}");
        assert!(
            diagnostic.contains("Move to session")
                && diagnostic.contains("beta / Work")
                && diagnostic.contains("beta / Other"),
            "{diagnostic}"
        );
    }
    assert_eq!(
        before["snapshot"]["tabs"],
        alpha.request("session.snapshot", json!({}))["snapshot"]["tabs"]
    );
    for answer in [None, Some("missing")] {
        let mut args = vec!["command", "run", "Recently closed tabs"];
        args.extend(answer);
        let output = cli(&alpha, &args, None);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(
            diagnostic.contains("Recently closed tabs") && diagnostic.contains("valid choices"),
            "{diagnostic}"
        );
    }
    // Keep the workspace alive while closing same-named rows within one second.
    for _ in 0..3 {
        let tab = alpha.request("tab.create", json!({"workspace_id":caller["workspace"]["workspace_id"],"label":"Duplicate","cwd":alpha.home}));
        alpha.request("tab.close", json!({"tab_id":tab["tab"]["tab_id"]}));
    }
    let history = alpha.request("tab.closed.list", json!({}));
    let labels: Vec<_> = history["closed_tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(history_label)
        .collect();
    let duplicate = labels
        .iter()
        .find(|label| labels.iter().filter(|other| *other == *label).count() > 1)
        .expect("three fast closures share a visible second");
    for answer in [None, Some("missing"), Some(duplicate.as_str())] {
        let mut args = vec!["command", "run", "Recently closed tabs"];
        args.extend(answer);
        let output = cli(&alpha, &args, None);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{diagnostic}");
        assert!(
            diagnostic.contains("Recently closed tabs")
                && labels.iter().all(|label| diagnostic.contains(label)),
            "{diagnostic}"
        );
    }
    assert_eq!(history, alpha.request("tab.closed.list", json!({})));
}

fn history_label(entry: &serde_json::Value) -> String {
    let seconds = entry["closed_at"].as_u64().unwrap() / 1000;
    let date = Command::new("date")
        .env_clear()
        .args(["-u", "-d", &format!("@{seconds}"), "+%Y-%m-%d %H:%M:%S UTC"])
        .output()
        .unwrap();
    assert!(date.status.success());
    format!(
        "{} · {} · {}",
        entry["label"].as_str().unwrap(),
        entry["workspace_label"].as_str().unwrap(),
        String::from_utf8(date.stdout).unwrap().trim()
    )
}

#[test]
fn restore_closed_tab_by_visible_label_outside_pane() {
    let core = support::Core::new();
    let (caller, decoy) = fixture(&core);
    core.request("tab.close", json!({"tab_id":caller["tab"]["tab_id"]}));
    core.request("tab.close", json!({"tab_id":decoy["tab"]["tab_id"]}));
    let history = core.request("tab.closed.list", json!({}));
    let entry = history["closed_tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["label"] == "Planning")
        .unwrap();
    let label = history_label(entry);
    let output = cli(
        &core,
        &["command", "run", "Recently closed tabs", &label],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let snapshot = core.request("session.snapshot", json!({}));
    assert!(snapshot["snapshot"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |workspace| workspace["workspace_id"] == result["result"]["workspace_id"]
                && workspace["label"] == "Work"
        ));
    let remaining = core.request("tab.closed.list", json!({}));
    assert_eq!(remaining["closed_tabs"].as_array().unwrap().len(), 1);
    assert_eq!(remaining["closed_tabs"][0]["label"], "Other");
}

#[test]
fn move_tab_to_visible_destination() {
    let alpha = owned_alpha();
    let beta = support::Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let (caller, _) = fixture(&alpha);
    beta.request("workspace.create", json!({"label":"Work","cwd":beta.home}));
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    let continuity = start_owned_reporter(&alpha, pane);
    let output = cli(
        &alpha,
        &["command", "run", "Move to session…", "beta / Work"],
        Some(pane),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        result["result"]["operation_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("command-tab-move-"))
            && result["result"]["transfer"].is_object(),
        "{result}"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let snapshot = beta.request("session.snapshot", json!({}));
        if snapshot["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["label"] == "Planning")
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{snapshot}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let snapshot = alpha.request("session.snapshot", json!({}));
    assert!(!snapshot["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tab| tab["label"] == "Planning"));
    assert_owned_reporter_continues(&beta, continuity);
}

#[test]
fn create_space_by_title() {
    let core = support::Core::new();
    for workspace in core.request("workspace.list", json!({}))["workspaces"]
        .as_array()
        .unwrap()
    {
        core.request(
            "workspace.close",
            json!({"workspace_id":workspace["workspace_id"]}),
        );
    }
    assert!(
        core.request("session.snapshot", json!({}))["snapshot"]["workspaces"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let output = cli(&core, &["command", "run", "New space", "Research"], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["result"]["workspace"]["label"], "Research");
    let spaces = core.request("workspace.list", json!({}));
    assert!(spaces.to_string().contains("Research"));
}

fn fixture(core: &support::Core) -> (serde_json::Value, serde_json::Value) {
    let caller = core.request(
        "workspace.create",
        json!({"label":"Work","cwd":core.home,"focus":true}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":caller["tab"]["tab_id"],"label":"Planning"}),
    );
    let decoy = core.request(
        "workspace.create",
        json!({"label":"Personal","cwd":core.home,"focus":true}),
    );
    core.request(
        "tab.rename",
        json!({"tab_id":decoy["tab"]["tab_id"],"label":"Other"}),
    );
    (caller, decoy)
}

#[test]
fn run_core_commands_in_caller_context() {
    for title in [
        "New tab or space",
        "New tab",
        "Rename space",
        "Rename tab",
        "Focus tab",
        "Close tab",
    ] {
        let core = support::Core::new();
        let (caller, decoy) = fixture(&core);
        let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
        let mut args = vec!["command", "run", title];
        if !matches!(title, "Focus tab" | "Close tab") {
            args.push("Build");
        }
        let result = cli(&core, &args, Some(pane));
        assert!(
            result.status.success(),
            "{title}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
        let tabs = snapshot["tabs"].as_array().unwrap();
        let spaces = snapshot["workspaces"].as_array().unwrap();
        assert!(tabs
            .iter()
            .any(|t| t["tab_id"] == decoy["tab"]["tab_id"] && t["label"] == "Other"));
        assert!(spaces
            .iter()
            .any(|s| s["workspace_id"] == decoy["workspace"]["workspace_id"]
                && s["label"] == "Personal"));
        match title {
            "Rename space" => assert!(spaces
                .iter()
                .any(|s| s["workspace_id"] == caller["workspace"]["workspace_id"]
                    && s["label"] == "Build")),
            "Close tab" => assert!(!tabs.iter().any(|t| t["tab_id"] == caller["tab"]["tab_id"])),
            "Focus tab" => assert_eq!(snapshot["focused_tab_id"], caller["tab"]["tab_id"]),
            _ => assert!(tabs
                .iter()
                .any(|t| t["workspace_id"] == caller["workspace"]["workspace_id"]
                    && t["label"] == "Build")),
        }
    }
}

#[test]
fn preserve_core_input_meaning() {
    let core = support::Core::new();
    let (caller, _) = fixture(&core);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    for (title, answer, diagnostic) in [
        ("New space", None, "New space name"),
        ("New tab", None, "New tab name"),
        ("Rename tab", None, "Rename tab"),
        ("Rename space", Some("   "), "invalid name"),
    ] {
        let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
        let mut args = vec!["command", "run", title];
        if let Some(answer) = answer {
            args.push(answer);
        }
        let output = cli(&core, &args, Some(pane));
        assert!(
            !output.status.success(),
            "{title} accepted missing/blank input"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{title}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
        assert_eq!(before["workspaces"], after["workspaces"]);
        assert_eq!(before["tabs"], after["tabs"]);
    }
    let output = cli(
        &core,
        &["command", "run", "Rename tab", "  Planning v2  "],
        Some(pane),
    );
    assert!(output.status.success());
    let snapshot = core.request("session.snapshot", json!({}))["snapshot"].clone();
    assert!(snapshot["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["tab_id"] == caller["tab"]["tab_id"] && t["label"] == "  Planning v2  "));
}

#[test]
fn reject_surplus_core_answers_before_dispatch() {
    let core = support::Core::new();
    let (caller, _) = fixture(&core);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    for (title, answers) in [
        ("New space", vec!["Research", "surplus"]),
        ("Rename tab", vec!["Build", "surplus"]),
        ("Close tab", vec!["surplus"]),
        ("Restart idle agents", vec!["surplus"]),
    ] {
        let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
        let mut args = vec!["command", "run", title];
        args.extend(answers);
        let result = cli(&core, &args, Some(pane));
        assert!(!result.status.success(), "{title} accepted surplus");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("extra answer"),
            "{title}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
        assert_eq!(before["workspaces"], after["workspaces"]);
        assert_eq!(before["tabs"], after["tabs"]);
    }
}

#[test]
fn restart_idle_agents_by_name() {
    let core = support::Core::new();
    let (idle, busy) = fixture(&core);
    core.request(
        "workspace.create",
        json!({"label":"Unrelated focus","cwd":core.home,"focus":true}),
    );
    for (resource, state) in [(&idle, "idle"), (&busy, "working")] {
        core.request("pane.report_agent",json!({"pane_id":resource["root_pane"]["pane_id"],"source":"command-run-fixture","agent":"omp","state":state}));
    }
    let expected = core.request("agent.restart_idle", json!({}));
    let output = cli(&core, &["command", "run", "Restart idle agents"], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual["result"], expected);
    let rows = actual["result"]["results"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|r| r["pane_id"] == busy["root_pane"]["pane_id"]
                && r["outcome"] == "skipped"
                && r["reason"] == "not_idle"),
        "{actual}"
    );
    assert!(
        rows.iter()
            .any(|r| r["pane_id"] == idle["root_pane"]["pane_id"]
                && r["reason"] != "not_idle"
                && r["reason"] != "focused"),
        "{actual}"
    );
    let agents = core.request("agent.list", json!({}));
    assert!(agents.to_string().contains("working"));
}

#[test]
fn resolve_exact_titles_and_stable_ids() {
    let core = support::Core::new();
    let list = core.request("ui.command.list", json!({}));
    assert!(list["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["title"] == "New space" && c["key"]["action_id"] == "host:new-space"));
    for selector in ["New space", "host:new-space"] {
        let output = cli(&core, &["command", "run", selector, "Research"], None);
        assert!(
            output.status.success(),
            "{selector}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["result"]["workspace"]["label"], "Research");
    }
    let spaces = core.request("workspace.list", json!({}));
    assert_eq!(
        spaces["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["label"] == "Research")
            .count(),
        2
    );
}

#[test]
fn reject_missing_or_ambiguous_commands() {
    let core = support::Core::new();
    let plugin = core.home.join("collision");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("trailboss-plugin.toml"), "id = \"fixture.collision\"\nname = \"Collision\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"first\"\ntitle = \"Shared\"\ncommand = [\"true\"]\n[[actions]]\nid = \"second\"\ntitle = \"Shared\"\ncommand = [\"true\"]\n").unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let before = core.request("workspace.list", json!({}));
    for (selector, diagnostic) in [
        ("Shared", "ambiguous"),
        ("does-not-exist", "command not found"),
    ] {
        let output = cli(&core, &["command", "run", selector], None);
        assert!(!output.status.success());
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(message.contains(diagnostic), "{message}");
        if selector == "Shared" {
            for identity in [
                "Shared",
                "fixture.collision/first",
                "fixture.collision/second",
            ] {
                assert!(message.contains(identity), "{message}");
            }
        }
    }
    assert_eq!(before, core.request("workspace.list", json!({})));
    assert_eq!(
        core.request("plugin.log.list", json!({"limit":20}))["logs"],
        json!([])
    );
}

#[test]
fn require_real_caller_pane() {
    let core = support::Core::new();
    fixture(&core);
    let before = core.request("session.snapshot", json!({}))["snapshot"].clone();
    for (pane, diagnostic) in [
        (None, "no Trailboss pane context; run from a Trailboss pane"),
        (Some("w999:p999"), "w999:p999"),
    ] {
        let output = cli(&core, &["command", "run", "Rename tab", "Build"], pane);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let after = core.request("session.snapshot", json!({}))["snapshot"].clone();
        assert_eq!(before["tabs"], after["tabs"]);
        assert_eq!(before["workspaces"], after["workspaces"]);
    }
}

#[test]
fn resolve_moved_pane_rather_than_cached_location() {
    let mut source = support::Core::new();
    let destination = support::Core::launch_with_executable(
        source.home.clone(),
        format!("{}-moved", source.name),
        support::executable(),
        false,
    );
    let (caller, _) = fixture(&source);
    let (build, decoy) = fixture(&destination);
    destination.request(
        "workspace.rename",
        json!({"workspace_id":build["workspace"]["workspace_id"],"label":"Build"}),
    );
    let old_pane = caller["root_pane"]["pane_id"].clone();
    let terminal =
        source.request("pane.get", json!({"pane_id":old_pane}))["pane"]["terminal_id"].clone();
    let state = source.home.join("original-context");
    source.request("pane.send_text",json!({"pane_id":old_pane,"text":format!("printf '%s\\n' \"$HERDR_PANE_ID\" \"$HERDR_WORKSPACE_ID\" \"$HERDR_TAB_ID\" > '{}'\n",state.display())}));
    wait_file(&state);
    source.request("tab.transfer",json!({"tab_id":caller["tab"]["tab_id"],"destination_session":destination.name,"workspace_id":build["workspace"]["workspace_id"],"operation_id":"named-moved-context"}));
    let snapshot = destination.request("session.snapshot", json!({}))["snapshot"].clone();
    let moved = snapshot["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["terminal_id"] == terminal)
        .unwrap();
    destination.request("tab.focus", json!({"tab_id":decoy["tab"]["tab_id"]}));
    let done = source.home.join("moved-done");
    destination.request("pane.send_text",json!({"pane_id":moved["pane_id"],"text":format!("'{}' command run 'Rename tab' 'Current' > '{}' 2> '{}'; printf '%s\\n' \"$?\" > '{}'\n",support::executable().display(),source.home.join("moved-result").display(),source.home.join("moved-error").display(),done.display())}));
    assert_eq!(
        wait_file(&done),
        "0\n",
        "{}",
        std::fs::read_to_string(source.home.join("moved-error")).unwrap()
    );
    let after = destination.request("session.snapshot", json!({}))["snapshot"].clone();
    assert!(after["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["tab_id"] == moved["tab_id"] && t["label"] == "Current"));
    assert!(after["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["tab_id"] == decoy["tab"]["tab_id"] && t["label"] == "Other"));
    assert!(
        !source.request("session.snapshot", json!({}))["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["label"] == "Current")
    );
    // The retained caller must not depend on its previous Core staying alive.
    source.request("server.stop", json!({}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while source.child.try_wait().unwrap().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "owned source did not stop"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let done = source.home.join("moved-without-source");
    destination.request("pane.send_text",json!({"pane_id":moved["pane_id"],"text":format!("'{}' command run 'Rename tab' 'Still current' > '{}' 2> '{}'; printf '%s\\n' \"$?\" > '{}'\n",support::executable().display(),source.home.join("second-result").display(),source.home.join("second-error").display(),done.display())}));
    assert_eq!(
        wait_file(&done),
        "0\n",
        "{}",
        std::fs::read_to_string(source.home.join("second-error")).unwrap()
    );
    assert!(
        destination.request("session.snapshot", json!({}))["snapshot"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["tab_id"] == moved["tab_id"] && t["label"] == "Still current")
    );
}

fn wait_file(path: &std::path::Path) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        if let Ok(value) = std::fs::read_to_string(path) {
            if value.ends_with('\n') {
                return value;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fixture shell did not finish {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn exclude_window_only_commands_from_api() {
    let core = support::Core::new();
    let (caller, _) = fixture(&core);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    let list = core.request("ui.command.list", json!({"pane_id":pane}));
    for (title, id) in [
        ("Detach from session", "host:detach"),
        ("Switch session", "host:switch-session"),
        ("Previous tab", "host:focus:previous"),
    ] {
        assert!(
            !list["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["title"] == title),
            "{title} leaked to API"
        );
        for selector in [title, id] {
            let output = cli(&core, &["command", "run", selector], Some(pane));
            assert!(!output.status.success(), "{selector} ran through API");
        }
    }
    // Native metadata and owner-bound fragments arrive on the native endpoint,
    // not through the runnable API catalog.
    let mut native = core.attach();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while native.command_ui.is_none() {
        assert!(std::time::Instant::now() < deadline);
        native.receive().unwrap();
    }
    let metadata = serde_json::to_value(native.command_ui.as_ref().unwrap()).unwrap();
    for title in ["Detach from session", "Previous tab"] {
        assert!(
            metadata["commands"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|c| c["title"] == title)),
            "native lost {title}"
        );
    }
}

#[test]
fn list_runnable_core_commands_and_hints() {
    let core = support::Core::new();
    let (caller, _) = fixture(&core);
    let pane = caller["root_pane"]["pane_id"].as_str().unwrap();
    let output = cli(&core, &["command", "list"], Some(pane));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for (title, id) in [
        ("New space", "host:new-space"),
        ("New tab or space", "host:new"),
        ("New tab", "host:new-tab"),
        ("Rename space", "host:rename-space"),
        ("Rename tab", "host:rename-tab"),
        ("Close tab", "host:close-tab"),
        ("Focus tab", "host:focus"),
        ("Restart idle agents", "host:restart-idle-agents"),
        ("Move to session…", "host:move-to-session"),
        ("Recently closed tabs", "host:recently-closed-tabs"),
    ] {
        assert!(text.contains(title) && text.contains(id), "{text}");
    }
    for hint in [
        "New space name",
        "New tab name",
        "Rename space",
        "Rename tab",
    ] {
        assert!(text.contains(hint), "{text}");
    }
    let list = core.request("ui.command.list", json!({"pane_id":pane}));
    assert_eq!(list["commands"].as_array().unwrap().len(), 10);
    for title in ["Previous tab", "Detach from session", "Switch session"] {
        assert!(!text.contains(title), "{text}");
    }
    let outside = core.request("ui.command.list", json!({}));
    let titles: Vec<_> = outside["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        ["New space", "Restart idle agents", "Recently closed tabs"]
    );
    let outside_cli = cli(&core, &["command", "list"], None);
    assert!(outside_cli.status.success());
    assert!(!String::from_utf8_lossy(&outside_cli.stdout).contains("Rename tab"));
    assert!(!cli(
        &core,
        &[
            "command",
            "run",
            "Rename tab",
            "Build",
            "--workspace",
            "w999"
        ],
        Some(pane)
    )
    .status
    .success());
    let help = cli(&core, &["command", "run", "--help"], None);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("ANSWER"));
}

#[test]
fn stable_id_wins_over_a_colliding_visible_title() {
    let core = support::Core::new();
    let plugin = core.home.join("stable-id-collision");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("trailboss-plugin.toml"), "id = \"fixture.identity-collision\"\nname = \"Identity collision\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"decoy\"\ntitle = \"host:new-space\"\ncommand = [\"true\"]\n").unwrap();
    core.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let output = cli(
        &core,
        &["command", "run", "host:new-space", "Research"],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["result"]["workspace"]["label"], "Research");
    let workspaces = core.request("workspace.list", json!({}));
    assert_eq!(
        workspaces["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|workspace| workspace["label"] == "Research")
            .count(),
        1
    );
    assert_eq!(
        core.request("plugin.log.list", json!({"limit":20}))["logs"],
        json!([])
    );
}

#[test]
fn option_looking_answers_and_separators_preserve_text_and_selected_session() {
    let source = support::Core::new();
    let decoy = support::Core::launch_with_executable(
        source.home.clone(),
        "work".into(),
        support::executable(),
        false,
    );
    let decoy_before = decoy.request("workspace.list", json!({}));
    for (payload, label) in [
        (vec!["New space", "--session=work"], "--session=work"),
        (vec!["New space", "--session"], "--session"),
        (vec!["New space", "--", "--session=work"], "--session=work"),
        (vec!["--", "New space", "--session"], "--session"),
        (vec!["New space", "--", "--"], "--"),
        (vec!["New space", ""], ""),
        (vec!["New space", "--remote"], "--remote"),
        (
            vec!["New space", "--remote-executable=/not/a/program"],
            "--remote-executable=/not/a/program",
        ),
        (
            vec!["New space", "--remote-executable"],
            "--remote-executable",
        ),
        (vec!["New space", "--help"], "--help"),
        (vec!["New space", "--machine"], "--machine"),
        (
            vec!["New space", "--remote-keybindings=server"],
            "--remote-keybindings=server",
        ),
        (vec!["New space", "--handoff"], "--handoff"),
    ] {
        let mut args = vec!["command", "run"];
        args.extend(payload);
        let before = source.request("workspace.list", json!({}));
        let output = cli(&source, &args, None);
        if label.is_empty() {
            assert!(!output.status.success(), "explicit empty name was accepted");
            assert!(String::from_utf8_lossy(&output.stderr).contains("invalid_name"));
            assert_eq!(source.request("workspace.list", json!({})), before);
        } else {
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["result"]["workspace"]["label"], label, "{args:?}");
            let after = source.request("workspace.list", json!({}));
            assert_eq!(
                after["workspaces"].as_array().unwrap().len(),
                before["workspaces"].as_array().unwrap().len() + 1,
                "{args:?} ran on another Core"
            );
        }
        assert_eq!(
            decoy.request("workspace.list", json!({})),
            decoy_before,
            "{args:?} changed decoy session"
        );
    }
    let before = source.request("workspace.list", json!({}));
    let surplus = cli(
        &source,
        &["command", "run", "New space", "--session", "work"],
        None,
    );
    assert!(!surplus.status.success());
    assert!(String::from_utf8_lossy(&surplus.stderr).contains("extra_answer"));
    let selector = cli(&source, &["command", "run", "--session=work"], None);
    assert!(!selector.status.success());
    assert!(String::from_utf8_lossy(&selector.stderr).contains("command_not_found"));
    assert_eq!(source.request("workspace.list", json!({})), before);
    assert_eq!(decoy.request("workspace.list", json!({})), decoy_before);
    let global = format!("--session={}", source.name);
    let output = cli(
        &source,
        &[&global, "command", "run", "New space", "Prefix still works"],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["result"]["workspace"]["label"], "Prefix still works");
    assert_eq!(decoy.request("workspace.list", json!({})), decoy_before);
}

fn plugin_fixture(core: &support::Core, actions: &str) -> std::path::PathBuf {
    let root = core.home.join("omp");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("trailboss-plugin.toml"),
        format!("id = \"omp\"\nname = \"OMP fixture\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n{actions}"),
    ).unwrap();
    std::fs::write(root.join("record.py"),
        "import json,os,pathlib,time\nwith open('executed.jsonl','a') as f:\n f.write(json.dumps({'context':json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON']),'arguments':json.loads(os.environ['HERDR_PLUGIN_ARGUMENTS_JSON']) if 'HERDR_PLUGIN_ARGUMENTS_JSON' in os.environ else None})+'\\n')\nwhile not pathlib.Path('release').exists():\n time.sleep(0.01)\n").unwrap();
    core.request("plugin.link", json!({"path":root,"enabled":true}));
    root
}

#[test]
fn run_extension_by_visible_title() {
    let core = support::Core::new();
    let (caller, decoy) = fixture(&core);
    let root = plugin_fixture(&core, "[[actions]]\nid = \"clear\"\ntitle = \"omp: clear\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n");
    let output = cli(
        &core,
        &["command", "run", "omp: clear"],
        caller["root_pane"]["pane_id"].as_str(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let result = &response["result"];
    assert!(result["log"]["log_id"].is_string(), "{response}");
    assert_eq!(result["log"]["status"], "running");
    let trace = wait_file(&root.join("executed.jsonl"));
    let rows = trace
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    let context = &rows[0]["context"];
    assert_eq!(context["workspace_id"], caller["workspace"]["workspace_id"]);
    assert_eq!(context["tab_id"], caller["tab"]["tab_id"]);
    assert_eq!(context["pane_id"], caller["root_pane"]["pane_id"]);
    assert!(context.get("selected_text").is_none());
    let snapshot = core.request("session.snapshot", json!({}));
    assert!(snapshot
        .to_string()
        .contains(decoy["root_pane"]["pane_id"].as_str().unwrap()));
    assert!(snapshot.to_string().contains("Other"));
    std::fs::write(root.join("release"), "").unwrap();
}

#[test]
fn preserve_raw_plugin_argument_text() {
    for (selector, answer) in [
        ("omp: add-dir", None),
        ("omp/add-dir", Some("")),
        ("omp: add-dir", Some("  /tmp/a b  ")),
        ("omp/add-dir", Some("$HOME 'quoted' \"double\"\nnext")),
    ] {
        let core = support::Core::new();
        let (caller, _) = fixture(&core);
        let root = plugin_fixture(&core, "[[actions]]\nid = \"add-dir\"\ntitle = \"omp: add-dir\"\ncontexts = [\"pane\"]\nargument_input = { kind = \"raw_tail\" }\nargument_hint = \"<directory>\"\ncommand = [\"python3\", \"record.py\"]\n");
        let mut args = vec!["command", "run", selector];
        args.extend(answer);
        let output = cli(&core, &args, caller["root_pane"]["pane_id"].as_str());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(response["result"]["log"]["log_id"].is_string());
        let trace = wait_file(&root.join("executed.jsonl"));
        let row: serde_json::Value = serde_json::from_str(trace.trim_end()).unwrap();
        assert_eq!(row["arguments"], json!(answer), "{selector}");
        std::fs::write(root.join("release"), "").unwrap();
    }
}

#[test]
fn reject_unsupported_plugin_execution() {
    for (declaration, disabled, answers, code) in [
        (
            "platforms = [\"macos\"]\n",
            false,
            vec![],
            "platform_unsupported",
        ),
        ("", true, vec![], "plugin_disabled"),
        (
            "confirmation_fragment = \"missing.confirm\"\n",
            false,
            vec![],
            "command_unavailable",
        ),
        ("", false, vec!["text"], "unsupported_plugin_arguments"),
        (
            "argument_input = { kind = \"raw_tail\" }\n",
            false,
            vec!["one", "two"],
            "extra_answer",
        ),
    ] {
        let core = support::Core::new();
        let (caller, _) = fixture(&core);
        let root = plugin_fixture(&core, &format!("[[actions]]\nid = \"candidate\"\ntitle = \"omp: candidate\"\n{declaration}command = [\"python3\", \"record.py\"]\n"));
        if disabled {
            core.request("plugin.disable", json!({"plugin_id":"omp"}));
        }
        for selector in ["omp: candidate", "omp/candidate"] {
            let mut args = vec!["command", "run", selector];
            args.extend(answers.iter().copied());
            let output = cli(&core, &args, caller["root_pane"]["pane_id"].as_str());
            assert!(!output.status.success(), "{selector}: {declaration}");
            let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(response["error"]["code"], code, "{response}");
        }
        assert_eq!(
            core.request("plugin.log.list", json!({"plugin_id":"omp"}))["logs"],
            json!([])
        );
        assert!(!root.join("executed.jsonl").exists());
    }
}

#[test]
fn plugin_context_never_falls_back_to_global_focus() {
    let core = support::Core::new();
    let (_, decoy) = fixture(&core);
    let root = plugin_fixture(&core, "[[actions]]\nid = \"required\"\ntitle = \"omp: required\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n[[actions]]\nid = \"free\"\ntitle = \"omp: free\"\ncontexts = [\"global\"]\ncommand = [\"python3\", \"record.py\"]\n");
    let refused = cli(&core, &["command", "run", "omp/required"], None);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr)
        .contains("no Trailboss pane context; run from a Trailboss pane"));
    assert_eq!(
        core.request("plugin.log.list", json!({"plugin_id":"omp"}))["logs"],
        json!([])
    );
    let output = cli(&core, &["command", "run", "omp/free"], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result["result"]["log"]["log_id"].is_string());
    assert_eq!(result["result"]["log"]["status"], "running");
    let trace = wait_file(&root.join("executed.jsonl"));
    let row: serde_json::Value = serde_json::from_str(trace.trim_end()).unwrap();
    for field in [
        "workspace_id",
        "tab_id",
        "pane_id",
        "focused_pane_id",
        "selected_text",
    ] {
        assert!(row["context"].get(field).is_none(), "{row}");
    }
    // The legacy route intentionally still merges the current global focus.
    let legacy = core.request(
        "plugin.action.invoke",
        json!({"plugin_id":"omp","action_id":"free"}),
    );
    assert_eq!(
        legacy["context"]["workspace_id"],
        decoy["workspace"]["workspace_id"]
    );
    assert_eq!(legacy["context"]["tab_id"], decoy["tab"]["tab_id"]);
    std::fs::write(root.join("release"), "").unwrap();
}

#[test]
fn list_plugins_with_arguments_and_bypass_confirmation() {
    let core = support::Core::new();
    let (caller, _) = fixture(&core);
    let pane = caller["root_pane"]["pane_id"].as_str();
    let root = plugin_fixture(&core, "[[actions]]\nid = \"clear\"\ntitle = \"omp: clear\"\ncontexts = [\"pane\"]\ncommand = [\"python3\", \"record.py\"]\n[[actions]]\nid = \"add-dir\"\ntitle = \"omp: add-dir\"\nargument_input = { kind = \"raw_tail\" }\nargument_hint = \"<directory>\"\ncommand = [\"python3\", \"record.py\"]\n[[actions]]\nid = \"confirm\"\ntitle = \"omp: confirm\"\nconfirmation_fragment = \"confirm.action\"\ncommand = [\"python3\", \"record.py\"]\n[[actions]]\nid = \"broken\"\ntitle = \"Broken confirmation\"\nconfirmation_fragment = \"missing\"\ncommand = [\"true\"]\n");
    let pi = core.home.join("pi");
    std::fs::create_dir(&pi).unwrap();
    std::fs::write(pi.join("trailboss-plugin.toml"), "id = \"pi\"\nname = \"Pi fixture\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"help\"\ntitle = \"pi: help\"\nargument_input = { kind = \"raw_tail\" }\nargument_hint = \"<topic>\"\ncommand = [\"true\"]\n").unwrap();
    core.request("plugin.link", json!({"path":pi,"enabled":true}));
    let confirmation = json!({"herdr":"widget_tree","version":1,"zones":[],
    "fragments":[{"name":"confirm.action","priority":0,"tree":{"type":"column","children":[
        {"type":"row","action":"host:cancel","children":[{"type":"label","text":"Cancel","role":"label"}]},
        {"type":"row","action":"confirm","children":[{"type":"label","text":"Confirm","role":"label"}]}
    ]}}]});
    std::fs::write(
        root.join("confirmation.py"),
        format!("print({:?}, flush=True)\n", confirmation.to_string()),
    )
    .unwrap();
    let manifest = std::fs::read_to_string(root.join("trailboss-plugin.toml")).unwrap();
    std::fs::write(root.join("trailboss-plugin.toml"), format!("{manifest}[[ui]]\nid = \"confirmation\"\nsurface = \"sidebar\"\ncommand = [\"python3\", \"confirmation.py\"]\n")).unwrap();
    core.request("plugin.link", json!({"path":root,"enabled":true}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let logs = core.request("plugin.log.list", json!({"plugin_id":"omp"}));
        if logs["logs"].as_array().unwrap().iter().any(|log| {
            log["status"] == "succeeded" && log["command"] == json!(["python3", "confirmation.py"])
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{logs}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = cli(&core, &["command", "list"], pane);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for value in [
        "omp: clear",
        "omp/clear",
        "omp: add-dir",
        "omp/add-dir",
        "<directory>",
        "pi: help",
        "pi/help",
        "<topic>",
        "omp: confirm",
        "omp/confirm",
    ] {
        assert!(text.contains(value), "{text}");
    }
    assert!(!text.contains("Broken confirmation"), "{text}");
    let outside = cli(&core, &["command", "list"], None);
    let outside = String::from_utf8(outside.stdout).unwrap();
    assert!(!outside.contains("omp: clear"), "{outside}");
    assert!(outside.contains("omp: add-dir"), "{outside}");
    let output = cli(&core, &["command", "run", "omp: confirm"], pane);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(response["result"]["log"]["log_id"].is_string());
    assert_eq!(response["result"]["log"]["status"], "running");
    let trace = wait_file(&root.join("executed.jsonl"));
    assert_eq!(trace.lines().count(), 1);
    std::fs::write(root.join("release"), "").unwrap();
}

#[test]
fn unavailable_title_collisions_do_not_hide_runnable_commands() {
    for (declaration, disabled, code) in [
        ("", true, "plugin_disabled"),
        ("platforms = [\"macos\"]\n", false, "platform_unsupported"),
        (
            "confirmation_fragment = \"missing.confirm\"\n",
            false,
            "command_unavailable",
        ),
        ("contexts = [\"pane\"]\n", false, "missing_context"),
    ] {
        for title in ["New space", "Runnable plugin"] {
            let core = support::Core::new();
            let root = plugin_fixture(&core, &format!("[[actions]]\nid = \"candidate\"\ntitle = {title:?}\n{declaration}command = [\"python3\", \"record.py\"]\n"));
            if disabled {
                core.request("plugin.disable", json!({"plugin_id":"omp"}));
            }
            let pi = core.home.join("pi");
            std::fs::create_dir(&pi).unwrap();
            std::fs::write(pi.join("trailboss-plugin.toml"), "id = \"pi\"\nname = \"Runnable owner\"\nversion = \"0.1.0\"\nmin_herdr_version = \"0.9.0\"\n[[actions]]\nid = \"live\"\ntitle = \"Runnable plugin\"\ncommand = [\"true\"]\n").unwrap();
            core.request("plugin.link", json!({"path":pi,"enabled":true}));
            let mut args = vec!["command", "run", title];
            if title == "New space" {
                args.push("Research");
            }
            let output = cli(&core, &args, None);
            assert!(
                output.status.success(),
                "{declaration} disabled={disabled}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            if title == "New space" {
                assert_eq!(response["result"]["workspace"]["label"], "Research");
            } else {
                assert_eq!(response["result"]["action"]["plugin_id"], "pi");
            }
            let unavailable = cli(&core, &["command", "run", "omp/candidate"], None);
            assert!(!unavailable.status.success());
            let response: serde_json::Value = serde_json::from_slice(&unavailable.stderr).unwrap();
            assert_eq!(response["error"]["code"], code, "{response}");
            assert_eq!(
                core.request("plugin.log.list", json!({"plugin_id":"omp"}))["logs"],
                json!([])
            );
            assert!(!root.join("executed.jsonl").exists());
        }
    }
}
