#![cfg(target_os = "linux")]

mod support;

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use support::Core;

fn socket(core: &Core) -> PathBuf {
    core.home
        .join("trailboss/sessions")
        .join(&core.name)
        .join("trailboss.sock")
}

fn wait_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if text.ends_with('\n') {
                return text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "missing shell marker: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn identity(pid: &str) -> (String, String, PathBuf) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[4], "0", "live shell lost its controlling PTY");
    (
        fields[19].to_owned(),
        fields[4].to_owned(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
    )
}

// This is a transport interruption, not a simulated Core: every request and
// response comes from the actual owned source. Only the move's real response
// is withheld after the Core has produced it. All later requests are recorded
// and forwarded, so a caller replay cannot hide behind the interruption.
#[test]
fn lost_result_is_resolved_without_repeating_the_move() {
    let source = Core::new();
    let destination = Core::launch_with_executable(
        source.home.clone(),
        format!("{}-destination", source.name),
        support::executable(),
        false,
    );
    let from = source.request(
        "workspace.create",
        json!({"label":"LostResultSource","cwd":source.home}),
    );
    let to = destination.request(
        "workspace.create",
        json!({"label":"LostResultDestination","cwd":source.home}),
    );
    let tab = from["tab"]["tab_id"].as_str().unwrap();
    let pane = from["root_pane"]["pane_id"].as_str().unwrap();
    let terminal =
        source.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone();
    let workspace = to["workspace"]["workspace_id"].as_str().unwrap();
    source.request(
        "tab.rename",
        json!({"tab_id":tab,"label":"LostResultLiveTab"}),
    );
    source.request("pane.send_text", json!({"pane_id":pane,"text":format!(
        "retained_value=original; sleep 300 & retained_job=$!; printf '%s %s\\n' \"$$\" \"$retained_job\" > '{}'\n",
        source.home.join("outcome-before").display()
    )}));
    let before = wait_file(&source.home.join("outcome-before"));
    let pids: Vec<_> = before.split_whitespace().collect();
    let shell = identity(pids[0]);
    let job = identity(pids[1]);
    let source_count = source.request("tab.list", json!({}))["tabs"]
        .as_array()
        .unwrap()
        .len();
    let destination_count = destination.request("tab.list", json!({}))["tabs"]
        .as_array()
        .unwrap()
        .len();

    let proxy_name = format!("{}-lost-result", source.name);
    let proxy_dir = source.home.join("trailboss/sessions").join(&proxy_name);
    std::fs::create_dir_all(&proxy_dir).unwrap();
    let listener = UnixListener::bind(proxy_dir.join("trailboss.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let upstream_path = socket(&source);
    let proxy = std::thread::spawn(move || {
        let mut moves = Vec::new();
        while !stopping.load(Ordering::SeqCst) {
            let (mut caller, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("proxy accept failed: {error}"),
            };
            caller
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = String::new();
            BufReader::new(caller.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let value: Value = serde_json::from_str(&request).unwrap();
            let is_move = value["method"] == "tab.transfer";
            let mut upstream = UnixStream::connect(&upstream_path).unwrap();
            upstream
                .set_read_timeout(Some(Duration::from_secs(20)))
                .unwrap();
            upstream.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            BufReader::new(upstream).read_line(&mut response).unwrap();
            if is_move {
                moves.push((value, serde_json::from_str::<Value>(&response).unwrap()));
                // Closing this connection loses the result, not the operation.
            } else {
                caller.write_all(response.as_bytes()).unwrap();
            }
        }
        moves
    });
    let mut command = Command::new(support::executable());
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let key = key.to_string_lossy();
        key.starts_with("HERDR_") || key.starts_with("TRAILBOSS_")
    }) {
        command.env_remove(key);
    }
    for key in support::HOMES {
        command.env(key, &source.home);
    }
    let output = command
        .args([
            "--session",
            &proxy_name,
            "tab",
            "move",
            tab,
            "--workspace",
            workspace,
            "--to-session",
            &destination.name,
        ])
        .current_dir(&source.home)
        .output()
        .unwrap();
    stop.store(true, Ordering::SeqCst);
    let moves = proxy.join().unwrap();
    assert_eq!(
        moves.len(),
        1,
        "uncertain result must never submit another move"
    );
    let operation = moves[0].0["params"]["operation_id"].as_str().unwrap();
    assert!(
        !operation.is_empty(),
        "operation ID must exist before the result is lost"
    );
    assert!(
        moves[0].1.get("error").is_none(),
        "real transfer failed: {}",
        moves[0].1
    );

    // Reconcile that exact operation through the authoritative public query;
    // never issue another tab.transfer, even with the same ID.
    let outcome = source.request("tab.transfer.status", json!({"operation_id":operation}));
    let status = &outcome["status"];
    assert_eq!(status["operation_id"], operation);
    assert_eq!(status["phase"], "committed");
    assert_eq!(status["destination_session"], destination.name);
    let mapping = &status["transfer"];
    assert_eq!(mapping["previous_tab_id"], tab);
    let moved_tab = mapping["tab"]["tab_id"].as_str().unwrap();
    let moved_pane = mapping["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["previous_pane_id"] == pane)
        .unwrap()["pane_id"]
        .as_str()
        .unwrap();
    let source_tabs = source.request("tab.list", json!({}));
    let destination_tabs = destination.request("tab.list", json!({}));
    assert_eq!(
        source_tabs["tabs"].as_array().unwrap().len(),
        source_count - 1
    );
    assert_eq!(
        destination_tabs["tabs"].as_array().unwrap().len(),
        destination_count + 1
    );
    assert!(!source_tabs["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["label"] == "LostResultLiveTab"));
    assert_eq!(
        destination_tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["label"] == "LostResultLiveTab" && entry["tab_id"] == moved_tab)
            .count(),
        1
    );
    assert_eq!(identity(pids[0]), shell, "shell was recreated");
    assert_eq!(identity(pids[1]), job, "background job was recreated");
    destination.request("pane.focus", json!({"pane_id":moved_pane}));
    let mut attached = destination.attach();
    while attached.focused_pane_id.as_deref() != Some(moved_pane) {
        attached.receive().unwrap();
    }
    let mut input = attached.input().unwrap();
    input.focus_pane_target(moved_pane);
    input.type_text(&format!(
        "kill -0 \"$retained_job\" && printf '%s %s %s\\n' \"$$\" \"$retained_job\" \"$retained_value\" > '{}'; printf 'LOST_RESULT_SINGLE_READER\\n'\n",
        source.home.join("outcome-after").display()
    )).unwrap();
    assert_eq!(
        wait_file(&source.home.join("outcome-after")),
        format!("{} {} original\n", pids[0], pids[1])
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let visible = destination.request(
            "pane.read",
            json!({"pane_id":moved_pane,"source":"visible","format":"text"}),
        );
        if visible["read"]["text"]
            .as_str()
            .unwrap()
            .lines()
            .any(|line| line.trim() == "LOST_RESULT_SINGLE_READER")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "destination reader did not receive live marker output"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Public old pane addresses intentionally route to the new owner. Owner
    // inventory, not failure of that retained route, proves relinquishment.
    let source_snapshot = source.request("session.snapshot", json!({}));
    let destination_snapshot = destination.request("session.snapshot", json!({}));
    assert_eq!(
        source_snapshot["snapshot"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["terminal_id"] == terminal)
            .count(),
        0,
        "source retained the transferred terminal reader"
    );
    let owners: Vec<_> = destination_snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["terminal_id"] == terminal)
        .collect();
    assert_eq!(
        owners.len(),
        1,
        "destination must own exactly one original terminal"
    );
    assert_eq!(owners[0]["pane_id"], moved_pane);
    assert_eq!(owners[0]["tab_id"], moved_tab);

    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "lost result must not claim success: {diagnostic}"
    );
    assert!(diagnostic.to_ascii_lowercase().contains("unresolved") && diagnostic.contains(operation),
        "caller must report unresolved with its original operation ID for status reconciliation, not false failure: {diagnostic}");
    let query: Value = diagnostic
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["method"] == "tab.transfer.status")
        .expect("unresolved diagnostic must expose a usable same-operation status query");
    assert_eq!(query["params"]["operation_id"], operation);
    let api = trailboss_runtime::api_client::ApiClient::for_target(
        trailboss_runtime::api_client::ConnectionTarget::SocketPath(socket(&source)),
    );
    let request = serde_json::from_value(query).unwrap();
    let reconciled = api
        .request_value_with_timeout(&request, Duration::from_secs(5))
        .unwrap();
    assert_eq!(reconciled["result"]["status"], *status);
}

// Fail the actual source coordinator's second routes.lock acquisition: the
// first admits prepared routes, the second reconciles an already committed
// destination. Child/helper PIDs are excluded even though they inherit preload.
const RECONCILE_FAULT: &str = r#"
#define _GNU_SOURCE
#include <sys/file.h>
#include <unistd.h>
#include <dlfcn.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <stdatomic.h>
static atomic_int locks;
int flock(int fd, int operation) {
    int (*real_flock)(int,int) = dlsym(RTLD_NEXT, "flock");
    const char *pid = getenv("OUTCOME_CORE_PID");
    const char *arm = getenv("OUTCOME_ARM");
    char name[64], target[4096];
    snprintf(name, sizeof(name), "/proc/self/fd/%d", fd);
    ssize_t length = readlink(name, target, sizeof(target)-1);
    if (length >= 0) target[length] = 0;
    if (pid && strtol(pid, 0, 10) == getpid() && arm && !access(arm, F_OK)
        && operation == LOCK_EX && length >= 0 && strstr(target, "/routes.lock")) {
        int acquisition = atomic_fetch_add(&locks, 1) + 1;
        FILE *receipt = fopen(getenv("OUTCOME_RECEIPT"), "a");
        if (receipt) { fprintf(receipt, "pid=%d routes.lock acquisition=%d%s\n", getpid(), acquisition, acquisition >= 2 ? " injected EIO" : " admitted"); fclose(receipt); }
        if (acquisition >= 2) { errno = EIO; return -1; }
    }
    return real_flock(fd, operation);
}
"#;

#[test]
fn uncertain_json_result_preserves_the_original_operation_without_replay() {
    use std::os::unix::fs::PermissionsExt;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("outcome-{nonce:x}");
    let home = std::env::temp_dir().join(&name);
    std::fs::create_dir(&home).unwrap();
    let shim = home.join("reconcile.c");
    let library = home.join("reconcile.so");
    std::fs::write(&shim, RECONCILE_FAULT).unwrap();
    let compiled = Command::new("cc")
        .args(["-shared", "-fPIC", "-o"])
        .arg(&library)
        .arg(&shim)
        .arg("-ldl")
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let arm = home.join("armed");
    let receipt = home.join("receipt");
    let wrapper = home.join("core.sh");
    std::fs::write(&wrapper, format!("#!/bin/sh\nexport OUTCOME_CORE_PID=$$\nexport OUTCOME_ARM='{}'\nexport OUTCOME_RECEIPT='{}'\nexport LD_PRELOAD='{}'\nexec '{}' \"$@\"\n", arm.display(), receipt.display(), library.display(), support::executable().display())).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = Core::launch_with_executable(home, name, wrapper, true);
    let destination = Core::launch_with_executable(
        source.home.clone(),
        format!("{}-dest", source.name),
        support::executable(),
        false,
    );
    let from = source.request(
        "workspace.create",
        json!({"label":"UncertainSource","cwd":source.home}),
    );
    let to = destination.request(
        "workspace.create",
        json!({"label":"UncertainDestination","cwd":source.home}),
    );
    let tab = from["tab"]["tab_id"].as_str().unwrap();
    source.request(
        "tab.rename",
        json!({"tab_id":tab,"label":"UncertainOriginal"}),
    );
    let proxy_name = format!("{}-proxy", source.name);
    let directory = source.home.join("trailboss/sessions").join(&proxy_name);
    std::fs::create_dir_all(&directory).unwrap();
    let listener = UnixListener::bind(directory.join("trailboss.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let upstream = socket(&source);
    let proxy = std::thread::spawn(move || {
        let mut moves = Vec::new();
        while !stopping.load(Ordering::SeqCst) {
            let (mut caller, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            caller
                .set_read_timeout(Some(Duration::from_secs(20)))
                .unwrap();
            let mut request = String::new();
            BufReader::new(caller.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let value: Value = serde_json::from_str(&request).unwrap();
            let mut actual = UnixStream::connect(&upstream).unwrap();
            actual
                .set_read_timeout(Some(Duration::from_secs(20)))
                .unwrap();
            actual.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            BufReader::new(actual).read_line(&mut response).unwrap();
            if value["method"] == "tab.transfer" {
                moves.push((value, serde_json::from_str::<Value>(&response).unwrap()));
            }
            // Deliver the actual JSON error unchanged, not a lost transport.
            caller.write_all(response.as_bytes()).unwrap();
        }
        moves
    });
    std::fs::write(&arm, "armed").unwrap();
    let mut command = Command::new(support::executable());
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let key = key.to_string_lossy();
        key.starts_with("HERDR_") || key.starts_with("TRAILBOSS_")
    }) {
        command.env_remove(key);
    }
    for key in support::HOMES {
        command.env(key, &source.home);
    }
    let output = command
        .args([
            "--session",
            &proxy_name,
            "tab",
            "move",
            tab,
            "--workspace",
            to["workspace"]["workspace_id"].as_str().unwrap(),
            "--to-session",
            &destination.name,
        ])
        .current_dir(&source.home)
        .output()
        .unwrap();
    stop.store(true, Ordering::SeqCst);
    let moves = proxy.join().unwrap();
    assert_eq!(moves.len(), 1, "uncertain JSON response must not replay");
    let operation = moves[0].0["params"]["operation_id"].as_str().unwrap();
    let fault = wait_file(&receipt);
    assert!(
        fault.contains(&format!(
            "pid={} routes.lock acquisition=2 injected EIO",
            source.child.id()
        )),
        "{fault}"
    );
    assert!(
        moves[0].1.get("error").is_some(),
        "must deliver a real JSON error: {}",
        moves[0].1
    );
    // Do not query status yet: public status may reconcile in the helper,
    // outside the deliberately source-PID-restricted fault. The initial actual
    // JSON response is the outcome observed by this CLI invocation.
    assert_eq!(
        destination.request("tab.list", json!({}))["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|tab| tab["label"] == "UncertainOriginal")
            .count(),
        1,
        "destination did not really commit before reconciliation failed"
    );
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{diagnostic}");
    assert_eq!(
        moves[0].1["error"]["code"], "transfer_unresolved",
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains("Do not repeat the move") && diagnostic.contains(operation),
        "{diagnostic}"
    );
    let query: Value = diagnostic
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["method"] == "tab.transfer.status")
        .expect("uncertain JSON response must expose same-operation reconciliation");
    assert_eq!(query["params"]["operation_id"], operation);
    std::fs::remove_file(&arm).unwrap();
    let resolved = source.request("tab.transfer.status", json!({"operation_id":operation}));
    assert_eq!(resolved["status"]["phase"], "committed");
    assert_eq!(resolved["status"]["operation_id"], operation);
}
