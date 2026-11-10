#![cfg(target_os = "linux")]

#[path = "support/console.rs"]
mod console;
mod support;

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::Path,
    time::{Duration, Instant},
};
use support::Core;

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
            "shell marker missing: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn process_identity(pid: &str) -> (String, String, std::path::PathBuf) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[4], "0", "shell lost controlling PTY");
    (
        fields[19].to_owned(),
        fields[4].to_owned(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
    )
}

fn wire_request(wire: &mut BufReader<UnixStream>, id: &str, method: &str, params: Value) -> Value {
    writeln!(
        wire.get_mut(),
        "{}",
        json!({"id":id,"method":method,"params":params})
    )
    .unwrap();
    let mut line = String::new();
    wire.read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], id);
    response
}

fn public_peer(path: &Path) -> (u32, u32) {
    use std::os::fd::AsRawFd;
    let stream = UnixStream::connect(path).unwrap();
    #[repr(C)]
    struct Credentials {
        pid: i32,
        uid: u32,
        gid: u32,
    }
    unsafe extern "C" {
        fn getsockopt(
            fd: i32,
            level: i32,
            option: i32,
            value: *mut std::ffi::c_void,
            length: *mut u32,
        ) -> i32;
    }
    let mut credentials = Credentials {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of_val(&credentials) as u32;
    // Linux SOL_SOCKET=1 and SO_PEERCRED=17; this suite also requires /proc.
    assert_eq!(
        unsafe {
            getsockopt(
                stream.as_raw_fd(),
                1,
                17,
                &mut credentials as *mut _ as *mut _,
                &mut length,
            )
        },
        0
    );
    (credentials.pid as u32, credentials.uid)
}

struct ReplacedSocket {
    endpoint: std::path::PathBuf,
    original: std::path::PathBuf,
    replacement: std::path::PathBuf,
}
impl Drop for ReplacedSocket {
    fn drop(&mut self) {
        // Preserve even the substituted path rather than unlinking a foreign
        // socket or following its symlink during cleanup.
        std::fs::rename(&self.endpoint, &self.replacement).unwrap();
        std::fs::rename(&self.original, &self.endpoint).unwrap();
    }
}

#[test]
fn committed_routing_adoption_preserves_original_endpoint_and_selected_live_tab() {
    assert_adoption_preserves_original_endpoint(false, None);
}

#[test]
fn socket_identity_mismatch_refuses_adoption_without_unlinking_foreign_socket() {
    assert_adoption_preserves_original_endpoint(true, None);
}

#[test]
fn acceptor_quiescence_failure_preserves_original_endpoint_and_usable_input() {
    assert_adoption_preserves_original_endpoint(false, Some("quiesce-listener"));
}

#[test]
fn helper_exit_before_adoption_commit_preserves_original_endpoint_and_usable_input() {
    assert_adoption_preserves_original_endpoint(false, Some("helper-exit"));
}

#[test]
fn unresolved_activation_cannot_resume_a_second_source_acceptor() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("ar-{nonce:x}");
    let home = std::env::temp_dir().join(&name);
    std::fs::create_dir(&home).unwrap();
    let executable = fault_wrapper(&home, "activation-race");
    let alpha = Core::launch_with_executable(home, name, executable, true);
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let source = alpha.request("workspace.create", json!({"cwd":alpha.home}));
    let destination = beta.request("workspace.create", json!({"cwd":alpha.home}));
    let before = alpha.request("session.snapshot", json!({}));
    let endpoint = alpha
        .home
        .join("trailboss/sessions")
        .join(&alpha.name)
        .join("trailboss.sock");
    let identity = std::fs::symlink_metadata(&endpoint).unwrap();
    let connect = || {
        let stream = UnixStream::connect(&endpoint).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        BufReader::new(stream)
    };
    // Both public handlers exist before quiescence. The second coordinator's
    // ensure must finish before the helper stops dispatching Activate.
    let accepted = || {
        std::fs::read_to_string(alpha.home.join("os-fault-receipt"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("source public client accepted"))
            .count()
    };
    let baseline = accepted();
    let mut first = connect();
    let mut second = connect();
    let deadline = Instant::now() + Duration::from_secs(5);
    while accepted() < baseline + 2 {
        assert!(
            Instant::now() < deadline,
            "public handlers did not accept both clients"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let params = json!({
        "tab_id":source["tab"]["tab_id"], "destination_session":beta.name,
        "workspace_id":destination["workspace"]["workspace_id"]
    });
    std::thread::scope(|scope| {
        let first_call = scope.spawn(|| {
            wire_request(&mut first, "first", "tab.transfer", {
                let mut params = params.clone();
                params["operation_id"] = json!("first-uncertain-activation");
                params
            })
        });
        wait_fault_event(&alpha.home, "first Activate ready");
        let second_call = scope.spawn(|| {
            wire_request(&mut second, "second", "tab.transfer", {
                let mut params = params.clone();
                params["operation_id"] = json!("second-adoption");
                params
            })
        });
        wait_fault_event(&alpha.home, "second adoption lock attempted");
        std::fs::write(alpha.home.join("os-fault-armed.activate-send"), b"go").unwrap();
        wait_fault_event(&alpha.home, "helper Activate dispatch held");
        let first_reply = first_call.join().unwrap();
        assert_eq!(
            first_reply["error"]["code"], "tab_transfer_failed",
            "{first_reply}"
        );
        assert!(!alpha
            .home
            .join("trailboss/tab-transfer/retained.json")
            .exists());
        wait_fault_event(&alpha.home, "second helper control queued");
        std::fs::write(alpha.home.join("os-fault-armed.activate-dispatch"), b"go").unwrap();
        let second_reply = second_call.join().unwrap();
        assert_eq!(
            second_reply["error"]["code"], "tab_transfer_failed",
            "{second_reply}"
        );
        assert!(support::transfer_helper(&alpha.home).is_some());
    });
    // The helper's real public accept syscall is gated. A resumed source
    // would answer this queued request, proving two acceptors. Do not replay
    // the request after releasing the helper.
    let mut pending = connect();
    pending
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    writeln!(
        pending.get_mut(),
        "{}",
        json!({
            "id":"sole-acceptor", "method":"session.snapshot", "params":{}
        })
    )
    .unwrap();
    let mut line = String::new();
    let blocked = pending.read_line(&mut line).unwrap_err();
    assert!(matches!(
        blocked.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    std::fs::write(alpha.home.join("os-fault-armed.accept"), b"go").unwrap();
    pending
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    pending.read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["id"], "sole-acceptor");
    assert_eq!(reply["result"]["snapshot"], before["snapshot"]);
    let after = std::fs::symlink_metadata(&endpoint).unwrap();
    assert_eq!(
        (identity.dev(), identity.ino(), identity.mode()),
        (after.dev(), after.ino(), after.mode())
    );
}

fn wait_fault_event(home: &Path, event: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let receipt = std::fs::read_to_string(home.join("os-fault-receipt")).unwrap_or_default();
        if receipt.contains(event) {
            return;
        }
        assert!(Instant::now() < deadline, "missing {event}: {receipt}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_adoption_preserves_original_endpoint(replace_socket: bool, os_fault: Option<&str>) {
    let mut alpha = if let Some(mode) = os_fault {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("af-{nonce:x}");
        let home = std::env::temp_dir().join(&name);
        std::fs::create_dir(&home).unwrap();
        let executable = fault_wrapper(&home, mode);
        Core::launch_with_executable(home, name, executable, true)
    } else {
        Core::new()
    };
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let source = alpha.request(
        "workspace.create",
        json!({"label":"AdoptionSource","cwd":alpha.home}),
    );
    let destination = beta.request(
        "workspace.create",
        json!({"label":"AdoptionDestination","cwd":alpha.home}),
    );
    let pane = source["root_pane"]["pane_id"].as_str().unwrap();
    let tab = source["tab"]["tab_id"].as_str().unwrap();
    let marker_file = alpha.home.join("adoption-before");
    alpha.request("pane.send_text", json!({"pane_id":pane,"text":format!(
        "adoption_value=original; sleep 300 & adoption_job=$!; printf '%s %s\\n' \"$$\" \"$adoption_job\" > '{}'; printf 'ADOPTION_RETAINED_CONTENT\\n'\n", marker_file.display()
    )}));
    let processes = wait_file(&marker_file);
    let pids: Vec<_> = processes.split_whitespace().collect();
    let shell = process_identity(pids[0]);
    let job = process_identity(pids[1]);
    let before = alpha.request("session.snapshot", json!({}));
    let beta_before = beta.request("session.snapshot", json!({}));
    let closed_before = alpha.request("tab.closed.list", json!({}));
    let beta_closed_before = beta.request("tab.closed.list", json!({}));
    let endpoint = alpha
        .home
        .join("trailboss/sessions")
        .join(&alpha.name)
        .join("trailboss.sock");
    let original = std::fs::symlink_metadata(&endpoint).unwrap();
    assert_eq!(original.mode() & 0o777, 0o600);
    let mut original_client = BufReader::new(UnixStream::connect(&endpoint).unwrap());
    original_client
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    // Public API sockets carry one request each. Keep one pre-adoption
    // connection idle for its later mutation; use another for the transfer.
    let mut transfer_client = BufReader::new(UnixStream::connect(&endpoint).unwrap());
    transfer_client
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut foreign_listener = None;
    let mut foreign_identity = None;
    let foreign = alpha.home.join("foreign-owned.sock");
    let replacement = if replace_socket {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let listener = std::os::unix::net::UnixListener::bind(&foreign).unwrap();
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        foreign_identity = Some(std::fs::symlink_metadata(&foreign).unwrap());
        foreign_listener = Some(listener);
        let saved = endpoint.with_extension("original");
        std::fs::rename(&endpoint, &saved).unwrap();
        symlink(&foreign, &endpoint).unwrap();
        Some(ReplacedSocket {
            endpoint: endpoint.clone(),
            original: saved,
            replacement: endpoint.with_extension("substituted"),
        })
    } else {
        None
    };
    if os_fault.is_some() {
        std::fs::write(alpha.home.join("os-fault-armed"), b"armed").unwrap();
    }

    // A nonexistent destination workspace leaves the selected tab in place
    // after successful adoption. OS faults instead use a valid destination,
    // and must fail before selected-tab transfer preparation can start.
    let refused = wire_request(
        &mut transfer_client,
        "adopt-without-moving",
        "tab.transfer",
        json!({
            "tab_id":tab,"destination_session":beta.name,
            "workspace_id":if os_fault.is_some() { destination["workspace"]["workspace_id"].clone() }
                else { json!("nonexistent-adoption-workspace") },"operation_id":"adoption-selected-tab-retained"
        }),
    );
    if os_fault == Some("helper-exit") {
        // Stage has reached EOF and Cancel has failed, but the exact helper
        // remains alive behind the fixture gate until this response arrives.
        assert!(support::transfer_helper(&alpha.home).is_some());
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("failed to fill whole buffer"),
            "{refused}"
        );
        std::fs::write(alpha.home.join("os-fault-armed.release"), b"release").unwrap();
    }
    assert_eq!(refused["error"]["code"], "tab_transfer_failed");
    if replace_socket {
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("identity"),
            "{refused}"
        );
        let foreign_after = std::fs::symlink_metadata(&foreign).unwrap();
        let foreign_before = foreign_identity.as_ref().unwrap();
        assert_eq!(
            (
                foreign_after.dev(),
                foreign_after.ino(),
                foreign_after.uid()
            ),
            (
                foreign_before.dev(),
                foreign_before.ino(),
                foreign_before.uid()
            )
        );
        assert_eq!(
            foreign_listener
                .as_ref()
                .unwrap()
                .accept()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock,
            "adoption connected to substituted foreign socket"
        );
    } else if let Some(mode) = os_fault {
        let receipt = wait_file(&alpha.home.join("os-fault-receipt"));
        if mode == "quiesce-listener" {
            assert_eq!(
                receipt,
                format!(
                    "pid={} public listener quiescence duplicate injected EMFILE\n",
                    alpha.child.id()
                )
            );
        } else {
            assert_eq!(mode, "helper-exit");
            assert_eq!(receipt.lines().count(), 1, "{receipt}");
            assert!(
                receipt.contains(
                    "routing helper exited after listener delivery before adoption commit"
                ),
                "{receipt}"
            );
            let helper_pid: u32 = receipt
                .split_whitespace()
                .next()
                .unwrap()
                .strip_prefix("pid=")
                .unwrap()
                .parse()
                .unwrap();
            assert_ne!(
                helper_pid,
                alpha.child.id(),
                "injection terminated source instead of helper"
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while std::path::Path::new(&format!("/proc/{helper_pid}")).exists() {
                assert!(
                    Instant::now() < deadline,
                    "owned helper did not exit and reap"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    } else {
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("workspace"),
            "{refused}"
        );
    }
    drop(replacement);
    // SO_PEERCRED identifies the process that called listen(), not the
    // process accepting a transferred listener. Later source-exit routing
    // below supplies the positive helper-acceptance witness instead.
    let peer = public_peer(&endpoint);
    assert_eq!(peer, (alpha.child.id(), original.uid()));
    let after = std::fs::symlink_metadata(&endpoint).unwrap();
    assert_eq!(
        (after.dev(), after.ino(), after.uid(), after.mode()),
        (
            original.dev(),
            original.ino(),
            original.uid(),
            original.mode()
        ),
        "adoption replaced or weakened original protected endpoint"
    );
    assert_eq!(
        alpha.request("session.snapshot", json!({}))["snapshot"],
        before["snapshot"]
    );
    assert_eq!(
        beta.request("session.snapshot", json!({}))["snapshot"],
        beta_before["snapshot"]
    );
    assert_eq!(alpha.request("tab.closed.list", json!({})), closed_before);
    assert_eq!(
        beta.request("tab.closed.list", json!({})),
        beta_closed_before
    );
    assert_eq!(process_identity(pids[0]), shell);
    assert_eq!(process_identity(pids[1]), job);
    // The pre-adoption client and a newly accepted client both submit actual
    // acknowledged mutations through the same endpoint, each exactly once.
    for (client_id, label) in [
        ("old-client-create", "OldClientMutation"),
        ("new-client-create", "NewClientMutation"),
    ] {
        let response = if client_id == "old-client-create" {
            wire_request(
                &mut original_client,
                client_id,
                "tab.create",
                json!({"workspace_id":source["workspace"]["workspace_id"],"label":label,"cwd":alpha.home}),
            )
        } else {
            let mut client = BufReader::new(UnixStream::connect(&endpoint).unwrap());
            client
                .get_mut()
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            wire_request(
                &mut client,
                client_id,
                "tab.create",
                json!({"workspace_id":source["workspace"]["workspace_id"],"label":label,"cwd":alpha.home}),
            )
        };
        assert!(response.get("error").is_none(), "{response}");
        let created = &response["result"]["tab"]["tab_id"];
        let tabs = alpha.request("tab.list", json!({}));
        assert_eq!(
            tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["label"] == label && entry["tab_id"] == *created)
                .count(),
            1
        );
        assert!(!beta.request("tab.list", json!({}))["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["label"] == label));
    }
    let mut attached = alpha.attach();
    alpha.request("pane.focus", json!({"pane_id":pane}));
    while attached.focused_pane_id.as_deref() != Some(pane) {
        attached.receive().unwrap();
    }
    let mut input = attached.input().unwrap();
    input.focus_pane_target(pane);
    input.type_text(&format!(
        "kill -0 \"$adoption_job\" && printf '%s %s %s\\n' \"$$\" \"$adoption_job\" \"$adoption_value\" > '{}'\n", alpha.home.join("adoption-after").display()
    )).unwrap();
    assert_eq!(
        wait_file(&alpha.home.join("adoption-after")),
        format!("{} {} original\n", pids[0], pids[1])
    );
    assert!(alpha.request(
        "pane.read",
        json!({"pane_id":pane,"source":"visible","format":"text"})
    )["read"]["text"]
        .as_str()
        .unwrap()
        .contains("ADOPTION_RETAINED_CONTENT"));
    assert_eq!(
        alpha.request("pane.get", json!({"pane_id":pane}))["pane"]["tab_id"],
        tab
    );
    assert_eq!(
        beta.request(
            "pane.get",
            json!({"pane_id":destination["root_pane"]["pane_id"]})
        )["pane"]["tab_id"],
        destination["tab"]["tab_id"]
    );
    if !replace_socket && os_fault.is_none() {
        let mut native = console::Console::new(&beta);
        eprintln!("adopted discovery: waiting for beta native initial composition");
        native.wait(|rows| rows.iter().any(|row| row.contains("AdoptionDes")));
        assert_native_destination(&mut native, &alpha.name, "AdoptionSource");
        let moved = alpha.request("tab.transfer", json!({
            "tab_id":tab,"destination_session":beta.name,
            "workspace_id":destination["workspace"]["workspace_id"],"operation_id":"adoption-proof-move"
        }));
        let returned = beta.request("tab.transfer", json!({
            "tab_id":moved["transfer"]["tab"]["tab_id"],"destination_session":alpha.name,
            "destination_workspace_label":"AdoptionSource","operation_id":"adoption-proof-return"
        }));
        assert_native_destination(&mut native, &alpha.name, "AdoptionSource");
        alpha.request("tab.transfer", json!({
            "tab_id":returned["transfer"]["tab"]["tab_id"],"destination_session":beta.name,
            "workspace_id":destination["workspace"]["workspace_id"],"operation_id":"adoption-proof-exit"
        }));
        let original_pid = alpha.child.id();
        alpha.request("server.stop", json!({}));
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = alpha.child.try_wait().unwrap() {
                assert!(status.success(), "original Core exit failed");
                break;
            }
            assert!(Instant::now() < deadline, "original Core did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!Path::new(&format!("/proc/{original_pid}")).exists());
        let mut inherited = BufReader::new(UnixStream::connect(&endpoint).unwrap());
        inherited
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let routed = wire_request(
            &mut inherited,
            "after-source-exit",
            "pane.send_text",
            json!({
                "pane_id":pane,"text":format!("printf '%s %s %s\\n' \"$$\" \"$adoption_job\" \"$adoption_value\" > '{}'\n", alpha.home.join("helper-accepted-after-exit").display())
            }),
        );
        assert!(routed.get("error").is_none(), "{routed}");
        assert_eq!(
            wait_file(&alpha.home.join("helper-accepted-after-exit")),
            format!("{} {} original\n", pids[0], pids[1])
        );
        assert_eq!(process_identity(pids[0]), shell);
        eprintln!("adoption proof: original Core exited and helper accepted inherited input");
        assert_eq!(process_identity(pids[1]), job);
        eprintln!("adoption proof: launching original-name replacement Core");
        let restarted = Core::launch_with_executable(
            alpha.home.clone(),
            alpha.name.clone(),
            support::executable(),
            false,
        );
        eprintln!(
            "adoption proof: replacement Core snapshot ready; creating destination workspace"
        );
        restarted.request(
            "workspace.create",
            json!({"label":"RestartedDestination","cwd":alpha.home}),
        );
        assert_native_destination(&mut native, &alpha.name, "RestartedDestination");
        let retained = std::fs::symlink_metadata(&endpoint).unwrap();
        assert_eq!(
            (
                retained.dev(),
                retained.ino(),
                retained.uid(),
                retained.mode()
            ),
            (
                original.dev(),
                original.ino(),
                original.uid(),
                original.mode()
            )
        );
    }
}

fn assert_native_destination(native: &mut console::Console, session: &str, workspace: &str) {
    eprintln!("native discovery of {session}/{workspace}: opening command palette");
    native.text("\x1bOP");
    native.wait(|rows| rows.iter().any(|row| row.contains("Commands")));
    eprintln!("native discovery of {session}/{workspace}: finding Move command");
    native.text("Move to session");
    native.wait(|rows| rows.iter().any(|row| row.contains("Move to session")));
    native.text("\r");
    eprintln!("native discovery of {session}/{workspace}: waiting for destination");
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains(session) && row.contains(workspace))
    });
    native.text("\x1b");
    eprintln!("native discovery of {session}/{workspace}: cancelling picker");
    native.wait(|rows| !rows.iter().any(|row| row.contains("Move to session")));
}

// The shim is compiled only inside an owned temporary fixture by the test.
// It injects a recorded OS-boundary error, never a fabricated Core response.
const OS_FAULT_SHIM: &str = r#"
#define _GNU_SOURCE
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/stat.h>
#include <poll.h>
#include <fcntl.h>
#include <unistd.h>
#include <dlfcn.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <stdatomic.h>
#include <stdarg.h>
#include <stdint.h>
#include <sys/syscall.h>
static atomic_int armed;
static atomic_int fired;
static int owned(void) {
    const char *pid = getenv("BOUNDARY_FAULT_CORE");
    return pid && strtol(pid, 0, 10) == getpid();
}
static int mode(const char *wanted) {
    const char *value = getenv("BOUNDARY_FAULT_MODE");
    return value && strcmp(value, wanted) == 0;
}
static int pty_rights(const struct msghdr *message) {
    for (struct cmsghdr *c = CMSG_FIRSTHDR(message); c; c = CMSG_NXTHDR((struct msghdr *)message, c)) {
        if (c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_RIGHTS || c->cmsg_len < CMSG_LEN(0)) continue;
        size_t count = (c->cmsg_len - CMSG_LEN(0)) / sizeof(int);
        int *fds = (int *)CMSG_DATA(c);
        for (size_t i = 0; i < count; i++) {
            struct stat info;
            char target[256], name[64];
            snprintf(name, sizeof(name), "/proc/self/fd/%d", fds[i]);
            ssize_t length = readlink(name, target, sizeof(target) - 1);
            if (length < 0) continue;
            target[length] = 0;
            if (!fstat(fds[i], &info) && S_ISCHR(info.st_mode) && strstr(target, "/ptmx")) return 1;
        }
    }
    return 0;
}
static int listener_rights(const struct msghdr *message) {
    for (struct cmsghdr *c = CMSG_FIRSTHDR(message); c; c = CMSG_NXTHDR((struct msghdr *)message, c)) {
        if (c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_RIGHTS || c->cmsg_len < CMSG_LEN(0)) continue;
        size_t count = (c->cmsg_len - CMSG_LEN(0)) / sizeof(int);
        int *fds = (int *)CMSG_DATA(c);
        for (size_t i = 0; i < count; i++) {
            int accepting = 0;
            socklen_t length = sizeof(accepting);
            if (!getsockopt(fds[i], SOL_SOCKET, SO_ACCEPTCONN, &accepting, &length) && accepting) return 1;
        }
    }
    return 0;
}
static void receipt(const char *event) {
    const char *path = getenv("BOUNDARY_FAULT_RECEIPT");
    if (!path) _exit(90);
    int fd = open(path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0600);
    if (fd < 0) _exit(91);
    char line[256];
    int size = snprintf(line, sizeof(line), "pid=%ld %s\n", (long)getpid(), event);
    if (write(fd, line, size) != size) _exit(92);
    close(fd);
}
static void gate(const char *suffix, const char *event) {
    char path[4096];
    snprintf(path, sizeof(path), "%s.%s", getenv("BOUNDARY_FAULT_ARM"), suffix);
    if (mkfifo(path, 0600)) _exit(97);
    receipt(event);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    char token;
    if (fd < 0 || read(fd, &token, 1) != 1) _exit(98);
    close(fd);
}
static atomic_int activation_sent;
static atomic_int stage_reply_fd = -1;
ssize_t send(int fd, const void *bytes, size_t length, int flags) {
    static ssize_t (*real_send)(int, const void *, size_t, int);
    if (!real_send) real_send = dlsym(RTLD_NEXT, "send");
    int activation = owned() && mode("activation-race") && memmem(bytes, length, "\"Activate\"", 10);
    if (owned() && mode("activation-race") && memmem(bytes, length, "\"Stage\"", 7)
            && !atomic_load(&activation_sent))
        atomic_store(&stage_reply_fd, fd);
    if (activation) atomic_store(&activation_sent, 1);
    ssize_t result = real_send(fd, bytes, length, flags);
    if (activation && result == (ssize_t)length) {
        // Lose only the real Activate acknowledgement, leaving the helper
        // dispatch alive and pending, just as a timed-out coordinator does.
        shutdown(fd, SHUT_RD);
    } else if (owned() && mode("activation-race") && atomic_load(&activation_sent)
            && (memmem(bytes, length, "\"Stage\"", 7) || memmem(bytes, length, "\"Cancel\"", 8))) {
        receipt("second helper control queued");
    }
    return result;
}
ssize_t recv(int fd, void *bytes, size_t length, int flags) {
    static ssize_t (*real_recv)(int, void *, size_t, int);
    if (!real_recv) real_recv = dlsym(RTLD_NEXT, "recv");
    ssize_t result = real_recv(fd, bytes, length, flags);
    const char *source = getenv("BOUNDARY_FAULT_CORE");
    if (result > 0 && owned() && mode("activation-race")
            && fd == atomic_load(&stage_reply_fd)
            && memmem(bytes, result, "\"helper-v1\"", 11)) {
        atomic_store(&stage_reply_fd, -1);
        // Stage has replied, before the next Activate connection exists.
        gate("activate-send", "first Activate ready");
    }
    if (result > 0 && mode("activation-race") && source && strtol(source, 0, 10) == getppid()
            && memmem(bytes, result, "\"Activate\"", 10)) {
        gate("activate-dispatch", "helper Activate dispatch held");
    }
    return result;
}
int accept4(int fd, struct sockaddr *address, socklen_t *length, int flags) {
    static int (*real_accept4)(int, struct sockaddr *, socklen_t *, int);
    if (!real_accept4) real_accept4 = dlsym(RTLD_NEXT, "accept4");
    const char *source = getenv("BOUNDARY_FAULT_CORE");
    if (mode("activation-race") && source && strtol(source, 0, 10) == getppid()) {
        struct sockaddr_un local;
        socklen_t size = sizeof(local);
        if (!getsockname(fd, (struct sockaddr *)&local, &size) && local.sun_family == AF_UNIX
                && strstr(local.sun_path, "/trailboss.sock")) {
            char path[4096];
            snprintf(path, sizeof(path), "%s.accept", getenv("BOUNDARY_FAULT_ARM"));
            if (access(path, F_OK)) {
                errno = EAGAIN;
                return -1;
            }
        }
    }
    int result = real_accept4(fd, address, length, flags);
    if (result >= 0 && owned() && mode("activation-race")) {
        struct sockaddr_un local;
        socklen_t size = sizeof(local);
        if (!getsockname(fd, (struct sockaddr *)&local, &size) && local.sun_family == AF_UNIX
                && strstr(local.sun_path, "/trailboss.sock"))
            receipt("source public client accepted");
    }
    return result;
}
ssize_t sendmsg(int fd, const struct msghdr *message, int flags) {
    static ssize_t (*real_sendmsg)(int, const struct msghdr *, int);
    if (!real_sendmsg) real_sendmsg = dlsym(RTLD_NEXT, "sendmsg");
    if (owned() && mode("send-pty") && pty_rights(message) && !atomic_exchange(&fired, 1)) {
        receipt("sendmsg PTY SCM_RIGHTS injected EIO");
        errno = EIO;
        return -1;
    }
    if (owned() && mode("destination-channel") && pty_rights(message)) {
        struct ucred peer;
        socklen_t length = sizeof(peer);
        if (!getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &peer, &length) && peer.pid != getpid()
                && !atomic_exchange(&fired, 1)) {
            ssize_t result = real_sendmsg(fd, message, flags);
            if (result <= 0) return result;
            struct pollfd ready = { .fd = fd, .events = POLLIN };
            if (poll(&ready, 1, 10000) <= 0 || !(ready.revents & POLLIN)) _exit(93);
            // Discard only the actual destination's preparation response.
            // The coordinator still has not received acknowledgement and
            // must abort provisional destination resources over a new call.
            char discarded[8192];
            ssize_t received;
            size_t total = 0;
            while ((received = recv(fd, discarded, sizeof(discarded), 0)) > 0) total += received;
            if (!total || received < 0 || shutdown(fd, SHUT_RDWR)) _exit(94);
            receipt("destination preparation response lost; coordination channel shut down");
            return result;
        }
    }
    return real_sendmsg(fd, message, flags);
}
ssize_t recvmsg(int fd, struct msghdr *message, int flags) {
    static ssize_t (*real_recvmsg)(int, struct msghdr *, int);
    if (!real_recvmsg) real_recvmsg = dlsym(RTLD_NEXT, "recvmsg");
    ssize_t result = real_recvmsg(fd, message, flags);
    const char *source_pid = getenv("BOUNDARY_FAULT_CORE");
    const char *activation = getenv("BOUNDARY_FAULT_ARM");
    if (result > 0 && mode("helper-exit") && source_pid && activation && !access(activation, F_OK)
            && strtol(source_pid, 0, 10) == getppid() && listener_rights(message)) {
        // Close Stage/control and every listener before becoming observably
        // dead. This forces the pre-activation rollback race without signals.
        char release[4096];
        snprintf(release, sizeof(release), "%s.release", activation);
        if (mkfifo(release, 0600)) _exit(95);
        receipt("routing helper exited after listener delivery before adoption commit");
        for (int probe = 3; probe < 1024; probe++) {
            int accepting = 0;
            socklen_t size = sizeof(accepting);
            if (!getsockopt(probe, SOL_SOCKET, SO_ACCEPTCONN, &accepting, &size) && accepting)
                close(probe);
        }
        close(fd);
        int gate = open(release, O_RDONLY | O_CLOEXEC);
        char token;
        if (gate < 0 || read(gate, &token, 1) != 1) _exit(96);
        close(gate);
        _exit(86);
    }
    if (result > 0 && owned() && (mode("import-pipe") || mode("routes-lock")) && pty_rights(message)) {
        receipt("recvmsg delivered PTY SCM_RIGHTS");
        atomic_store(&armed, 1);
    }
    return result;
}
int pipe(int fds[2]) {
    static int (*real_pipe)(int[2]);
    if (!real_pipe) real_pipe = dlsym(RTLD_NEXT, "pipe");
    if (owned() && mode("import-pipe") && atomic_exchange(&armed, 0) && !atomic_exchange(&fired, 1)) {
        receipt("import actor pipe injected EMFILE");
        errno = EMFILE;
        return -1;
    }
    return real_pipe(fds);
}
int flock(int fd, int operation) {
    static int (*real_flock)(int, int);
    if (!real_flock) real_flock = dlsym(RTLD_NEXT, "flock");
    if (owned() && mode("activation-race")) {
        static atomic_int adoption_locks;
        char target[512], name[64];
        snprintf(name, sizeof(name), "/proc/self/fd/%d", fd);
        ssize_t length = readlink(name, target, sizeof(target) - 1);
        if (length > 0) {
            target[length] = 0;
            if (strstr(target, "/front-") && atomic_fetch_add(&adoption_locks, 1) == 1)
                receipt("second adoption lock attempted");
        }
    }
    if (owned() && mode("routes-lock") && atomic_load(&armed)) {
        char target[512], name[64];
        snprintf(name, sizeof(name), "/proc/self/fd/%d", fd);
        ssize_t length = readlink(name, target, sizeof(target) - 1);
        if (length > 0) {
            target[length] = 0;
            const char *suffix = "/routes.lock";
            size_t suffix_length = strlen(suffix);
            if ((size_t)length >= suffix_length && !strcmp(target + length - suffix_length, suffix)
                    && !atomic_exchange(&fired, 1)) {
                receipt("routing admission flock injected EIO after PTY delivery");
                errno = EIO;
                return -1;
            }
        }
    }
    return real_flock(fd, operation);
}
static int fcntl_boundary(int fd, int command, va_list arguments) {
    unsigned long argument = 0;
    switch (command) {
        case F_GETFD: case F_GETFL: case F_GETOWN: case F_GETSIG:
        case F_GETLEASE: case F_GETPIPE_SZ: case F_GET_SEALS:
            break;
        case F_DUPFD: case F_DUPFD_CLOEXEC: case F_SETFD: case F_SETFL:
        case F_SETOWN: case F_SETSIG: case F_SETLEASE: case F_NOTIFY:
        case F_SETPIPE_SZ: case F_ADD_SEALS:
            argument = (unsigned long)va_arg(arguments, int);
            break;
        default:
            argument = (uintptr_t)va_arg(arguments, void *);
            break;
    }
    const char *activation = getenv("BOUNDARY_FAULT_ARM");
    if (owned() && mode("quiesce-listener") && activation && !access(activation, F_OK)
            && command == F_DUPFD_CLOEXEC) {
        int accepting = 0;
        socklen_t length = sizeof(accepting);
        if (!getsockopt(fd, SOL_SOCKET, SO_ACCEPTCONN, &accepting, &length) && accepting
                && !atomic_exchange(&fired, 1)) {
            receipt("public listener quiescence duplicate injected EMFILE");
            errno = EMFILE;
            return -1;
        }
    }
    return (int)syscall(SYS_fcntl, fd, command, argument);
}
int fcntl(int fd, int command, ...) {
    va_list arguments;
    va_start(arguments, command);
    int result = fcntl_boundary(fd, command, arguments);
    va_end(arguments);
    return result;
}
int fcntl64(int fd, int command, ...) {
    va_list arguments;
    va_start(arguments, command);
    int result = fcntl_boundary(fd, command, arguments);
    va_end(arguments);
    return result;
}
"#;

fn fault_wrapper(home: &Path, mode: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let source = home.join("os-fault.c");
    let library = home.join("os-fault.so");
    std::fs::write(&source, OS_FAULT_SHIM).unwrap();
    let compiled = std::process::Command::new("/usr/bin/cc")
        .args(["-shared", "-fPIC", "-std=c11"])
        .arg(&source)
        .args(["-o"])
        .arg(&library)
        .arg("-ldl")
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "fault shim compilation: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let wrapper = home.join(format!("core-{mode}.sh"));
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\nexport BOUNDARY_FAULT_CORE=$$\nexport BOUNDARY_FAULT_MODE='{mode}'\nexport BOUNDARY_FAULT_RECEIPT='{}'\nexport BOUNDARY_FAULT_ARM='{}'\nexport LD_PRELOAD='{}'\nexec '{}' \"$@\"\n",
        home.join("os-fault-receipt").display(), home.join("os-fault-armed").display(),
        library.display(), support::executable().display()
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    wrapper
}

#[test]
fn failed_pty_descriptor_delivery_rolls_back_only_provisional_resources() {
    assert_preparation_os_fault("send-pty");
}

#[test]
fn imported_runtime_construction_failure_rolls_back_only_provisional_resources() {
    assert_preparation_os_fault("import-pipe");
}

#[test]
fn destination_coordination_channel_loss_during_preparation_rolls_back_provisional_resources() {
    assert_preparation_os_fault("destination-channel");
}

#[test]
fn routing_admission_failure_rolls_back_only_provisional_resources() {
    assert_preparation_os_fault("routes-lock");
}

fn assert_preparation_os_fault(mode: &str) {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("pf-{nonce:x}");
    let home = std::env::temp_dir().join(&name);
    std::fs::create_dir(&home).unwrap();
    let wrapper = fault_wrapper(&home, mode);
    let alpha_executable = if mode != "import-pipe" {
        wrapper.clone()
    } else {
        support::executable()
    };
    let alpha = Core::launch_with_executable(home, name.clone(), alpha_executable, true);
    let beta_executable = if mode != "import-pipe" {
        support::executable()
    } else {
        wrapper
    };
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{name}-beta"),
        beta_executable,
        false,
    );
    let source = alpha.request(
        "workspace.create",
        json!({"label":"RollbackSource","cwd":alpha.home}),
    );
    let destination = beta.request(
        "workspace.create",
        json!({"label":"RollbackDestination","cwd":alpha.home}),
    );
    let source_pane = source["root_pane"]["pane_id"].as_str().unwrap();
    let destination_pane = destination["root_pane"]["pane_id"].as_str().unwrap();
    let mut processes = Vec::new();
    for (core, pane, label) in [
        (&alpha, source_pane, "source"),
        (&beta, destination_pane, "destination"),
    ] {
        core.request("pane.send_text", json!({"pane_id":pane,"text":format!(
            "rollback_value={label}; sleep 300 & rollback_job=$!; printf '%s %s\\n' \"$$\" \"$rollback_job\" > '{}'; printf 'ROLLBACK_CONTENT_{label}\\n'\n",
            alpha.home.join(format!("rollback-before-{label}")).display()
        )}));
        let text = wait_file(&alpha.home.join(format!("rollback-before-{label}")));
        let pids: Vec<_> = text.split_whitespace().collect();
        processes.push((
            pids[0].to_owned(),
            process_identity(pids[0]),
            pids[1].to_owned(),
            process_identity(pids[1]),
        ));
    }
    let topology = alpha.request("session.snapshot", json!({}));
    let beta_topology = beta.request("session.snapshot", json!({}));
    let closed = alpha.request("tab.closed.list", json!({}));
    let beta_closed = beta.request("tab.closed.list", json!({}));
    let endpoint = alpha
        .home
        .join("trailboss/sessions")
        .join(&alpha.name)
        .join("trailboss.sock");
    let original_socket = std::fs::symlink_metadata(&endpoint).unwrap();
    let api = trailboss_runtime::api_client::ApiClient::for_target(
        trailboss_runtime::api_client::ConnectionTarget::SocketPath(endpoint.clone()),
    );
    let operation = format!("rollback-{mode}-{name}");
    let request = serde_json::from_value(
        json!({"id":"os-fault-transfer","method":"tab.transfer","params":{
            "tab_id":source["tab"]["tab_id"],"destination_session":beta.name,
            "workspace_id":destination["workspace"]["workspace_id"],"operation_id":operation
        }}),
    )
    .unwrap();
    let rejected = api
        .request_value_with_timeout(&request, Duration::from_secs(20))
        .unwrap();
    assert_eq!(
        rejected["error"]["code"], "tab_transfer_failed",
        "{rejected}"
    );
    let receipt = wait_file(&alpha.home.join("os-fault-receipt"));
    let fault_pid = if mode != "import-pipe" {
        alpha.child.id()
    } else {
        beta.child.id()
    };
    assert!(
        receipt
            .lines()
            .all(|line| line.starts_with(&format!("pid={fault_pid} "))),
        "{receipt}"
    );
    if mode == "send-pty" {
        assert_eq!(receipt.lines().count(), 1, "{receipt}");
        assert!(
            receipt.contains("sendmsg PTY SCM_RIGHTS injected EIO"),
            "{receipt}"
        );
    } else if mode == "destination-channel" {
        assert_eq!(receipt.lines().count(), 1, "{receipt}");
        assert!(
            receipt
                .contains("destination preparation response lost; coordination channel shut down"),
            "{receipt}"
        );
    } else if mode == "routes-lock" {
        assert!(
            receipt.contains("recvmsg delivered PTY SCM_RIGHTS")
                && receipt.contains("routing admission flock injected EIO after PTY delivery"),
            "{receipt}"
        );
    } else {
        assert!(
            receipt.contains("recvmsg delivered PTY SCM_RIGHTS")
                && receipt.contains("import actor pipe injected EMFILE"),
            "{receipt}"
        );
    }
    let status = alpha.request("tab.transfer.status", json!({"operation_id":operation}));
    assert_eq!(status["status"]["phase"], "aborted", "{status}");
    assert_eq!(
        alpha.request("session.snapshot", json!({}))["snapshot"],
        topology["snapshot"]
    );
    assert_eq!(
        beta.request("session.snapshot", json!({}))["snapshot"],
        beta_topology["snapshot"]
    );
    assert_eq!(alpha.request("tab.closed.list", json!({})), closed);
    assert_eq!(beta.request("tab.closed.list", json!({})), beta_closed);
    let retained_socket = std::fs::symlink_metadata(&endpoint).unwrap();
    assert_eq!(
        (
            retained_socket.dev(),
            retained_socket.ino(),
            retained_socket.mode(),
            retained_socket.uid()
        ),
        (
            original_socket.dev(),
            original_socket.ino(),
            original_socket.mode(),
            original_socket.uid()
        )
    );
    for ((core, pane, label), (shell_pid, shell, job_pid, job)) in [
        (&alpha, source_pane, "source"),
        (&beta, destination_pane, "destination"),
    ]
    .into_iter()
    .zip(processes)
    {
        assert_eq!(process_identity(&shell_pid), shell);
        assert_eq!(process_identity(&job_pid), job);
        core.request("pane.send_text", json!({"pane_id":pane,"text":format!(
            "kill -0 \"$rollback_job\" && printf '%s %s %s\\n' \"$$\" \"$rollback_job\" \"$rollback_value\" > '{}'; printf 'ROLLBACK_LIVE_{label}\\n'\n",
            alpha.home.join(format!("rollback-after-{label}")).display()
        )}));
        assert_eq!(
            wait_file(&alpha.home.join(format!("rollback-after-{label}"))),
            format!("{shell_pid} {job_pid} {label}\n")
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let visible = core.request(
                "pane.read",
                json!({"pane_id":pane,"source":"visible","format":"text"}),
            );
            let text = visible["read"]["text"].as_str().unwrap();
            if text
                .lines()
                .any(|line| line.trim() == format!("ROLLBACK_LIVE_{label}"))
            {
                assert!(text.contains(&format!("ROLLBACK_CONTENT_{label}")));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "original {label} reader did not resume"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
