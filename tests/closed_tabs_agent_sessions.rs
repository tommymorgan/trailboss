#![cfg(target_os = "linux")]

use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// A separate launcher is intentional: inherited CODEX_HOME/credentials must not
// make this owned, empty session store resolve a real user's conversation.
struct OwnedCore {
    child: Child,
    home: PathBuf,
    name: String,
}

impl OwnedCore {
    fn launch(home: PathBuf, name: String, codex: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_trailboss"));
        command.env_clear();
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
        ] {
            command.env(key, &home);
        }
        let path = std::env::join_paths([
            codex.parent().unwrap(),
            Path::new("/usr/bin"),
            Path::new("/bin"),
        ])
        .unwrap();
        let child = command
            .args(["server", "--session", &name])
            .env("PATH", path)
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm-256color")
            .env("CODEX_HOME", home.join(".codex"))
            .env("TRAILBOSS_CONFIG_PATH", home.join("config.toml"))
            .env("TRAILBOSS_STARTUP_CWD", &home)
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut core = Self { child, home, name };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !core.socket("trailboss-client.sock").exists() {
            assert!(
                Instant::now() < deadline && core.child.try_wait().unwrap().is_none(),
                "isolated core did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        core.result("session.snapshot", json!({}));
        core
    }

    fn socket(&self, file: &str) -> PathBuf {
        self.home
            .join(format!("trailboss/sessions/{}/{file}", self.name))
    }

    fn request(&self, method: &str, params: Value) -> Value {
        let mut stream = UnixStream::connect(self.socket("trailboss.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(
            &mut stream,
            &json!({"id":"missing-agent-session", "method":method, "params":params}),
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    fn result(&self, method: &str, params: Value) -> Value {
        let response = self.request(method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }
}

impl Drop for OwnedCore {
    fn drop(&mut self) {
        if self.home.as_os_str().is_empty() {
            return;
        }
        // Stop only the Core and its supervised panes owned by this fixture.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.request("server.stop", json!({}));
            let deadline = Instant::now() + Duration::from_secs(2);
            while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn actual_codex() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH is required"))
        .map(|directory| directory.join("codex"))
        .find(|path| {
            fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
        .expect(
            "this regression requires an installed actual codex executable, not a fake agent CLI",
        )
}

fn descendants(pid: u32) -> BTreeSet<u32> {
    let mut found = BTreeSet::new();
    let mut pending = vec![pid];
    while let Some(parent) = pending.pop() {
        let children = fs::read_to_string(format!("/proc/{parent}/task/{parent}/children"))
            .unwrap_or_default();
        for child in children
            .split_whitespace()
            .map(|value| value.parse::<u32>().unwrap())
        {
            if found.insert(child) {
                pending.push(child);
            }
        }
    }
    found
}

#[test]
#[ignore = "requires installed Codex; run explicitly for isolated native-session verification"]
fn missing_saved_codex_conversation_preserves_closed_entry_live_tabs_and_processes() {
    let codex = actual_codex();
    let name = format!(
        "closed-codex-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let home = std::env::temp_dir().join(&name);
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(
        home.join("config.toml"),
        "[terminal]\ndefault_shell = '/bin/sh'\n[session]\nresume_agents_on_restore = true\n",
    )
    .unwrap();
    let mut core = OwnedCore::launch(home.clone(), name.clone(), &codex);
    let workspace = core.result(
        "workspace.create",
        json!({"cwd":home,"label":"Owned Codex recovery"}),
    );
    let tab = core.result("tab.create", json!({"workspace_id":workspace["workspace"]["workspace_id"],"cwd":home,"label":"Missing saved Codex"}));
    core.result("tab.close", json!({"tab_id":tab["tab"]["tab_id"]}));
    core.result("server.stop", json!({}));
    assert!(core.child.wait().unwrap().success());
    let snapshot_path = core.socket("session.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
    let entry = &mut saved["closed_tabs"][0];
    let pane = entry["workspace"]["tabs"][0]["panes"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    // Valid official snapshot metadata, but no native Codex session exists in
    // this fixture's entire CODEX_HOME. No external agent or login is needed.
    let missing_id = "8ed7c5ac-cd61-47f0-b579-d18ecb1f1234";
    pane["agent_session"] =
        json!({"source":"herdr:codex","agent":"codex","kind":"id","value":missing_id});
    let entry_id = entry["id"].clone();
    fs::write(snapshot_path, serde_json::to_vec(&saved).unwrap()).unwrap();
    // Transfer this owned directory to the replacement fixture without Drop
    // removing it. The previous Core is already stopped and reaped.
    core.home = PathBuf::new();
    drop(core);
    let core = OwnedCore::launch(home, name, &codex);
    let _client = trailboss::Client::attach(&core.socket("trailboss-client.sock"), 80, 24).unwrap();
    let history = core.result("tab.closed.list", json!({}));
    assert_eq!(history["closed_tabs"][0]["closed_tab_id"], entry_id);
    let tabs = core.result("tab.list", json!({}));
    let panes = core.result("pane.list", json!({}));
    let deadline = Instant::now() + Duration::from_secs(8);
    for pane in panes["panes"].as_array().unwrap() {
        loop {
            let response = core.request("pane.process_info", json!({"pane_id":pane["pane_id"]}));
            if response["result"]["process_info"]["shell_pid"]
                .as_u64()
                .is_some()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned live shell did not become ready: {response}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let processes = descendants(core.child.id());
    let rejected = core.request("tab.closed.restore", json!({"closed_tab_id":entry_id}));
    // Capture state even on RED so an erroneously accepted restore is not
    // mistaken for a successful native resume merely because a shell queued it.
    let history_after = core.result("tab.closed.list", json!({}));
    let tabs_after = core.result("tab.list", json!({}));
    let panes_after = core.result("pane.list", json!({}));
    let processes_after = descendants(core.child.id());
    assert_eq!(
        rejected["error"]["code"], "closed_tab_restore_failed",
        "{rejected}"
    );
    let message = rejected["error"]["message"].as_str().unwrap();
    assert!(
        message.to_lowercase().contains("codex")
            && (message.contains("unavailable")
                || message.contains("missing")
                || message.contains("cannot verify")),
        "unclear missing conversation error: {message}"
    );
    assert_eq!(
        history_after, history,
        "failed restore consumed or changed closed history"
    );
    assert_eq!(tabs_after, tabs, "failed restore changed live tabs");
    assert_eq!(panes_after, panes, "failed restore changed live panes");
    assert_eq!(
        processes_after, processes,
        "failed restore started or replaced processes"
    );
}
