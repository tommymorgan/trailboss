#![cfg(target_os = "linux")]

mod support;

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use support::Core;

fn wait_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(value) = std::fs::read_to_string(path) {
            if value.ends_with('\n') {
                return value;
            }
        }
        assert!(
            Instant::now() < deadline,
            "missing marker: {}",
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
    assert_ne!(fields[4], "0", "original shell lost controlling PTY");
    assert!(
        !matches!(fields[0], "T" | "t"),
        "transfer suspended original process {pid}"
    );
    (
        fields[19].into(),
        fields[4].into(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
    )
}

fn public_request(endpoint: &Path, method: &str, params: Value) -> Value {
    let mut wire = BufReader::new(UnixStream::connect(endpoint).unwrap());
    wire.get_mut()
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    writeln!(
        wire.get_mut(),
        "{}",
        json!({"id":"topology-witness","method":method,"params":params})
    )
    .unwrap();
    let mut line = String::new();
    wire.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn endpoint(core: &Core) -> PathBuf {
    core.home
        .join("trailboss/sessions")
        .join(&core.name)
        .join("trailboss.sock")
}

// Only the source coordinator's routes.lock acquisition is delayed. Receipt
// requires real PTY SCM_RIGHTS receipt followed by destination preparation;
// the App threads, unrelated terminals and destination Core remain runnable.
// No packets are inspected, replaced or fabricated, and no process is stopped.
const HOLD_SHIM: &str = r#"
#define _GNU_SOURCE
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/file.h>
#include <unistd.h>
#include <fcntl.h>
#include <dlfcn.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <stdatomic.h>
static atomic_int armed;
static atomic_int fired;
static int owned(void) {
    const char *pid = getenv("TOPOLOGY_CORE_PID");
    return pid && strtol(pid, 0, 10) == getpid();
}
ssize_t recvmsg(int fd, struct msghdr *message, int flags) {
    static ssize_t (*real_recvmsg)(int, struct msghdr *, int);
    if (!real_recvmsg) real_recvmsg = dlsym(RTLD_NEXT, "recvmsg");
    ssize_t result = real_recvmsg(fd, message, flags);
    if (result <= 0 || !owned()) return result;
    for (struct cmsghdr *c = CMSG_FIRSTHDR(message); c; c = CMSG_NXTHDR(message, c)) {
        if (c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_RIGHTS || c->cmsg_len < CMSG_LEN(0)) continue;
        size_t count = (c->cmsg_len - CMSG_LEN(0)) / sizeof(int);
        int *fds = (int *)CMSG_DATA(c);
        for (size_t i = 0; i < count; i++) {
            struct stat info;
            char name[64], target[256];
            snprintf(name, sizeof(name), "/proc/self/fd/%d", fds[i]);
            ssize_t length = readlink(name, target, sizeof(target) - 1);
            if (length < 0) continue;
            target[length] = 0;
            if (!fstat(fds[i], &info) && S_ISCHR(info.st_mode) && strstr(target, "/ptmx")) atomic_store(&armed, 1);
        }
    }
    return result;
}
int flock(int fd, int operation) {
    static int (*real_flock)(int, int);
    if (!real_flock) real_flock = dlsym(RTLD_NEXT, "flock");
    if (owned() && atomic_load(&armed) && (operation & LOCK_EX)) {
        char name[64], target[1024];
        snprintf(name, sizeof(name), "/proc/self/fd/%d", fd);
        ssize_t length = readlink(name, target, sizeof(target) - 1);
        if (length > 0) {
            target[length] = 0;
            const char *suffix = "/routes.lock";
            size_t n = strlen(suffix);
            if ((size_t)length >= n && !strcmp(target + length - n, suffix) && !atomic_exchange(&fired, 1)) {
                const char *receipt = getenv("TOPOLOGY_RECEIPT");
                const char *release = getenv("TOPOLOGY_RELEASE");
                if (!receipt || !release) _exit(90);
                int out = open(receipt, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
                char line[256];
                int size = snprintf(line, sizeof(line), "pid=%ld PTY received; post-preparation routes.lock held\n", (long)getpid());
                if (out < 0 || write(out, line, size) != size) _exit(91);
                close(out);
                int remaining = 3000;
                while (access(release, F_OK)) {
                    if (!--remaining) _exit(92);
                    usleep(10000);
                }
            }
        }
    }
    return real_flock(fd, operation);
}
"#;

fn wrapper(home: &Path) -> PathBuf {
    let source = home.join("topology-hold.c");
    let library = home.join("topology-hold.so");
    std::fs::write(&source, HOLD_SHIM).unwrap();
    let compiled = std::process::Command::new("/usr/bin/cc")
        .args(["-shared", "-fPIC", "-std=c11"])
        .arg(source)
        .arg("-o")
        .arg(&library)
        .arg("-ldl")
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "shim compile: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let wrapper = home.join("topology-core.sh");
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\nexport TOPOLOGY_CORE_PID=$$\nexport TOPOLOGY_RECEIPT='{}'\nexport TOPOLOGY_RELEASE='{}'\nexport LD_PRELOAD='{}'\nexec '{}' \"$@\"\n",
        home.join("topology-held").display(), home.join("topology-release").display(),
        library.display(), support::executable().display()
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    wrapper
}

struct Release(PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, b"release\n");
    }
}

#[test]
fn closing_destination_tab_during_preparation_preserves_live_transfer_and_other_input() {
    assert_destination_topology_change(false);
}

#[test]
fn last_destination_shell_exit_during_preparation_keeps_authoritative_owner_and_live_input() {
    assert_destination_topology_change(true);
}

fn assert_destination_topology_change(natural_exit: bool) {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("topology-{nonce:x}");
    let home = std::env::temp_dir().join(&name);
    std::fs::create_dir(&home).unwrap();
    let executable = wrapper(&home);
    let alpha = Core::launch_with_executable(home, name, executable, true);
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let from = alpha.request(
        "workspace.create",
        json!({"label":"Selected","cwd":alpha.home}),
    );
    let to = beta.request(
        "workspace.create",
        json!({"label":"Destination","cwd":alpha.home}),
    );
    let closing = if natural_exit {
        to.clone()
    } else {
        beta.request("tab.create", json!({"workspace_id":to["workspace"]["workspace_id"],"label":"Closing","cwd":alpha.home,"focus":false}))
    };
    let survivor_tab = if natural_exit {
        beta.request(
            "workspace.create",
            json!({"label":"Unaffected","cwd":alpha.home}),
        )
    } else {
        to.clone()
    };
    let source_pane = from["root_pane"]["pane_id"].as_str().unwrap();
    let survivor = survivor_tab["root_pane"]["pane_id"].as_str().unwrap();
    let terminal =
        alpha.request("pane.get", json!({"pane_id":source_pane}))["pane"]["terminal_id"].clone();
    let progress = alpha.home.join("selected-progress");
    let progress_script = alpha.home.join("selected-progress.py");
    std::fs::write(
        &progress_script,
        r#"from pathlib import Path
import sys, time
path = Path(sys.argv[1])
pending = path.with_suffix(".next")
count = 0
while True:
    count += 1
    pending.write_text(f"{count}\n")
    pending.replace(path)
    time.sleep(0.02)
"#,
    )
    .unwrap();
    alpha.request("pane.send_text", json!({"pane_id":source_pane,"text":format!(
        "topology_local=original; python3 '{}' '{}' & topology_job=$!; printf '%s %s\\n' \"$$\" \"$topology_job\" > '{}'\n", progress_script.display(), progress.display(), alpha.home.join("selected-before").display()
    )}));
    beta.request("pane.send_text", json!({"pane_id":survivor,"text":format!(
        "survivor_local=unchanged; printf '%s\\n' \"$$\" > '{}'\n", alpha.home.join("survivor-before").display()
    )}));
    let selected = wait_file(&alpha.home.join("selected-before"));
    let pids: Vec<_> = selected.split_whitespace().collect();
    let shell = identity(pids[0]);
    let job = identity(pids[1]);
    let survivor_pid = wait_file(&alpha.home.join("survivor-before"));
    let survivor_identity = identity(survivor_pid.trim());
    let alpha_pid = alpha.child.id();
    let beta_pid = beta.child.id();
    let operation = if natural_exit {
        "topology-natural-exit"
    } else {
        "topology-close-during-preparation"
    };
    let source_endpoint = endpoint(&alpha);
    let params = json!({"tab_id":from["tab"]["tab_id"],"destination_session":beta.name,"workspace_id":to["workspace"]["workspace_id"],"insert_index":if natural_exit { 1 } else { 2 },"operation_id":operation});
    let transfer_endpoint = source_endpoint.clone();
    let transfer =
        std::thread::spawn(move || public_request(&transfer_endpoint, "tab.transfer", params));
    let release = Release(alpha.home.join("topology-release"));
    assert_eq!(
        wait_file(&alpha.home.join("topology-held")),
        format!("pid={alpha_pid} PTY received; post-preparation routes.lock held\n")
    );
    let progress_before = wait_file(&progress).trim().parse::<u64>().unwrap();
    let progress_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert_eq!(identity(pids[0]), shell);
        assert_eq!(identity(pids[1]), job);
        if wait_file(&progress).trim().parse::<u64>().unwrap() >= progress_before + 2 {
            break;
        }
        assert!(
            Instant::now() < progress_deadline,
            "the original job stopped progressing during prepared transfer"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let close = if natural_exit {
        // The existing destination shell exits itself. This bypasses public
        // topology reservations and removes the last tab and workspace.
        let exiting = closing["root_pane"]["pane_id"].as_str().unwrap();
        let response = public_request(
            &endpoint(&beta),
            "pane.send_text",
            json!({
                "pane_id":exiting,"text":format!("printf '%s\\n' \"$$\" > '{}'; exit\n", alpha.home.join("naturally-exited").display())
            }),
        );
        assert!(response.get("error").is_none(), "{response}");
        let exited_pid = wait_file(&alpha.home.join("naturally-exited"));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let tabs = beta.request("tab.list", json!({}));
            if tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tab| tab["tab_id"] != closing["tab"]["tab_id"])
                && !Path::new(&format!("/proc/{}", exited_pid.trim())).exists()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "natural shell exit did not remove its original tab: {tabs}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let workspaces = beta.request("workspace.list", json!({}));
        assert!(
            workspaces["workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .all(|workspace| workspace["workspace_id"] != to["workspace"]["workspace_id"]),
            "{workspaces}"
        );
        response
    } else {
        let response = public_request(
            &endpoint(&beta),
            "tab.close",
            json!({"tab_id":closing["tab"]["tab_id"]}),
        );
        if response.get("error").is_some() {
            assert_eq!(
                response["error"]["code"], "transfer_in_progress",
                "{response}"
            );
        } else {
            assert!(response.get("result").is_some(), "{response}");
        }
        let tabs = beta.request("tab.list", json!({}));
        assert_eq!(
            tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tab| tab["tab_id"] == closing["tab"]["tab_id"]),
            response.get("error").is_some(),
            "public close result must describe actual topology: {response}; {tabs}"
        );
        response
    };
    // A workspace topology reservation must not freeze its unrelated live PTY.
    beta.request("pane.focus", json!({"pane_id":survivor}));
    let mut view = beta.attach();
    while view.focused_pane_id.as_deref() != Some(survivor) {
        view.receive().unwrap();
    }
    let mut input = view.input().unwrap();
    input.focus_pane_target(survivor);
    input
        .type_text(&format!(
            "printf '%s %s\\n' \"$$\" \"$survivor_local\" > '{}'\n",
            alpha.home.join("survivor-held").display()
        ))
        .unwrap();
    assert_eq!(
        wait_file(&alpha.home.join("survivor-held")),
        format!("{} unchanged\n", survivor_pid.trim())
    );
    drop(release);
    let moved = transfer.join().unwrap();
    let status = alpha.request("tab.transfer.status", json!({"operation_id":operation}));
    let committed = status["status"]["phase"] == "committed";
    if natural_exit {
        assert!(committed || status["status"]["phase"] == "aborted", "natural destination exit must resolve to an authoritative owner: move={moved}; status={status}");
    } else {
        assert!(committed, "normal destination close lost the original transfer: move={moved}; status={status}; close={close}");
    }
    if committed {
        assert!(moved.get("error").is_none(), "{moved}");
    } else {
        assert_eq!(moved["error"]["code"], "tab_transfer_failed", "{moved}");
    }
    let owner = if committed { &beta } else { &alpha };
    let nonowner = if committed { &alpha } else { &beta };
    let panes = owner.request("pane.list", json!({}));
    let retained: Vec<_> = panes["panes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["terminal_id"] == terminal)
        .collect();
    assert_eq!(
        retained.len(),
        1,
        "original terminal must have exactly one owner pane: {panes}"
    );
    let current = retained[0]["pane_id"].as_str().unwrap();
    owner.request("pane.send_text", json!({"pane_id":current,"text":format!(
        "kill -0 \"$topology_job\" && printf '%s %s %s\\n' \"$$\" \"$topology_job\" \"$topology_local\" > '{}'\n", alpha.home.join("selected-after").display()
    )}));
    assert_eq!(
        wait_file(&alpha.home.join("selected-after")),
        format!("{} {} original\n", pids[0], pids[1])
    );
    assert_eq!(identity(pids[0]), shell);
    assert_eq!(identity(pids[1]), job);
    assert_eq!(identity(survivor_pid.trim()), survivor_identity);
    assert_eq!(alpha.child.id(), alpha_pid);
    assert_eq!(beta.child.id(), beta_pid);
    assert!(nonowner.request("pane.list", json!({}))["panes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["terminal_id"] != terminal));
    beta.request("pane.send_text", json!({"pane_id":survivor,"text":format!("printf '%s\\n' \"$survivor_local\" > '{}'\n", alpha.home.join("survivor-after").display())}));
    assert_eq!(wait_file(&alpha.home.join("survivor-after")), "unchanged\n");
}
