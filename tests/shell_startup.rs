#![cfg(target_os = "linux")]

use serde_json::json;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use trailboss::Client;
use trailboss_runtime::{
    api_client::{ApiClient, ConnectionTarget},
    api_schema::Request,
};

struct ShellCore {
    child: Child,
    home: PathBuf,
    api: ApiClient,
    client: Option<Client>,
}

impl Drop for ShellCore {
    fn drop(&mut self) {
        let request: Request =
            serde_json::from_value(json!({"id":"shell-stop", "method":"server.stop", "params":{}}))
                .unwrap();
        let _ = self
            .api
            .request_value_with_timeout(&request, Duration::from_secs(2));
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().is_ok_and(|status| status.is_none())
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl ShellCore {
    fn launch(shell: Option<&str>, config: &str) -> Self {
        let name = format!(
            "shell-{:x}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::process::id()
        );
        let home = std::env::temp_dir().join(&name);
        std::fs::create_dir(&home).unwrap();
        std::fs::write(
            home.join(".bashrc"),
            "printf 'bash-startup-sentinel\\n'\nPS1='bash-ready> '\n",
        )
        .unwrap();
        std::fs::write(home.join("config.toml"), config).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_trailboss"));
        command.args(["server", "--session", &name]);
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
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            let name = key.to_string_lossy();
            name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
        }) {
            command.env_remove(key);
        }
        command.env_remove("SHELL");
        if let Some(shell) = shell {
            command.env("SHELL", shell);
        }
        command
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .env("TRAILBOSS_CONFIG_PATH", home.join("config.toml"))
            .env("TRAILBOSS_STARTUP_CWD", &home)
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let child = command.spawn().unwrap();
        let session_dir = home.join(format!("trailboss/sessions/{name}"));
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(
            session_dir.join("trailboss.sock"),
        ));
        let mut core = Self {
            child,
            home,
            api,
            client: None,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(client) = Client::attach(&session_dir.join("trailboss-client.sock"), 80, 24) {
                core.client = Some(client);
                break;
            }
            assert!(
                Instant::now() < deadline && core.child.try_wait().unwrap().is_none(),
                "isolated Core shell startup failed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        core
    }

    fn wait_for(&mut self, expected: &str) {
        let client = self.client.as_mut().unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !client
            .primary
            .as_ref()
            .is_some_and(|pane| pane.text().contains(expected))
        {
            assert!(
                Instant::now() < deadline,
                "shell did not produce {expected}"
            );
            client.receive().unwrap_or_else(|err| {
                panic!(
                    "shell did not produce {expected}: {err}; surface: {:?}",
                    client.primary.as_ref().map(|pane| pane.text())
                )
            });
        }
    }

    fn request(&self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let request: Request = serde_json::from_value(json!({
            "id": "shell-observation",
            "method": method,
            "params": params
        }))
        .unwrap();
        let mut response = self
            .api
            .request_value_with_timeout(&request, Duration::from_secs(2))
            .unwrap();
        assert!(response.get("result").is_some(), "{response}");
        response["result"].take()
    }

    fn wait_for_pane_status(&self, pane_id: &str, status: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let mut response = self.request("pane.get", json!({"pane_id": pane_id}));
            if response["pane"]["agent_status"] == status {
                return response["pane"].take();
            }
            assert!(
                Instant::now() < deadline,
                "expected {status}, got {response}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn absent_shell_starts_the_users_bash_configuration_and_prompt() {
    // This Linux integration fixture requires the effective user's NSS login shell to be bash.
    let mut core = ShellCore::launch(None, "");
    core.wait_for("bash-startup-sentinel");
    core.wait_for("bash-ready> ");
    core.client
        .as_mut()
        .unwrap()
        .type_text("printf 'interactive-%s\\n' ready\r")
        .unwrap();
    core.wait_for("interactive-ready");
}

#[test]
fn blank_shell_starts_the_users_bash_configuration_and_prompt() {
    for shell in ["", " \t\n"] {
        let mut core = ShellCore::launch(Some(shell), "");
        core.wait_for("bash-startup-sentinel");
        core.wait_for("bash-ready> ");
    }
}

fn assert_sh_selected(core: &mut ShellCore) {
    let client = core.client.as_mut().unwrap();
    while client.primary.is_none() || client.focused_pane_id.is_none() {
        client.receive().unwrap();
    }
    core.client
        .as_mut()
        .unwrap()
        .type_text("printf 'selected-%s\\n' \"$0\"\r")
        .unwrap();
    core.wait_for("selected-/bin/sh");
    assert!(!core
        .client
        .as_ref()
        .unwrap()
        .primary
        .as_ref()
        .unwrap()
        .text()
        .contains("bash-startup-sentinel"));
}

#[test]
fn nonempty_shell_overrides_the_users_login_shell() {
    let mut core = ShellCore::launch(Some("/bin/sh"), "");
    assert_sh_selected(&mut core);
}

#[test]
fn configured_shell_overrides_nonempty_shell_environment() {
    let mut core = ShellCore::launch(
        Some("/bin/bash"),
        "[terminal]\ndefault_shell = \"/bin/sh\"\n",
    );
    assert_sh_selected(&mut core);
}

#[test]
fn configured_shell_overrides_restored_login_shell() {
    let mut core = ShellCore::launch(None, "[terminal]\ndefault_shell = \"/bin/sh\"\n");
    assert_sh_selected(&mut core);
}

#[test]
fn ordinary_command_markers_show_busy_without_inventing_an_agent() {
    let mut core = ShellCore::launch(Some("/bin/bash"), "");
    core.wait_for("bash-ready> ");
    core.client
        .as_mut()
        .unwrap()
        .type_text("PS0=$'\\e]133;C\\a'; PROMPT_COMMAND='s=$?; printf \"\\033]133;D;%s\\007\\033]133;A\\007\" \"$s\"'; printf 'hooks-%s\\n' ready\r")
        .unwrap();
    core.wait_for("hooks-ready");
    core.client
        .as_mut()
        .unwrap()
        .type_text("(printf 'command-%s\\n' entered; while [ ! -e release-command ]; do sleep 0.02; done)\r")
        .unwrap();
    core.wait_for("command-entered");
    let pane_id = core
        .client
        .as_ref()
        .unwrap()
        .focused_pane_id
        .as_ref()
        .unwrap();
    let request: Request = serde_json::from_value(json!({
        "id": "command-state",
        "method": "pane.get",
        "params": {"pane_id": pane_id}
    }))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let response = core
            .api
            .request_value_with_timeout(&request, Duration::from_secs(2))
            .unwrap();
        let pane = &response["result"]["pane"];
        if pane["agent_status"] == "working" {
            assert!(pane["agent"].is_null(), "ordinary commands are not agents");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "ordinary command never became busy: {response}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(core.home.join("release-command"), "").unwrap();
}

#[test]
fn failed_command_is_red_until_acknowledged_and_retains_its_exit_code() {
    let mut core = ShellCore::launch(Some("/bin/bash"), "");
    core.wait_for("bash-ready> ");
    let snapshot = core.request("session.snapshot", json!({}));
    let snapshot = &snapshot["snapshot"];
    let pane_id = snapshot["focused_pane_id"].as_str().unwrap();
    let tab_id = snapshot["focused_tab_id"].as_str().unwrap();
    let workspace_id = snapshot["focused_workspace_id"].as_str().unwrap();
    let observer = core.request(
        "tab.create",
        json!({"workspace_id":workspace_id, "label":"Observer", "focus":false}),
    );
    let observer_id = observer["tab"]["tab_id"].as_str().unwrap();
    core.client
        .as_mut()
        .unwrap()
        .type_text("(while [ ! -e release-failure ]; do sleep 0.02; done; exit 7)\r")
        .unwrap();
    core.wait_for_pane_status(pane_id, "working");
    core.request("tab.focus", json!({"tab_id":observer_id}));
    std::fs::write(core.home.join("release-failure"), "").unwrap();
    let completed = core.wait_for_pane_status(pane_id, "blocked");
    assert_eq!(completed["tokens"]["command.status"], "failure");
    assert_eq!(completed["tokens"]["command.exit_code"], "7");
    assert!(completed["agent"].is_null());
    let tabs = core.request("tab.list", json!({}));
    let tab = tabs["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tab| tab["tab_id"] == tab_id)
        .unwrap();
    assert_eq!(tab["agent_status"], "blocked");
    let spaces = core.request("workspace.list", json!({}));
    let space = spaces["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|space| space["workspace_id"] == workspace_id)
        .unwrap();
    assert_eq!(space["agent_status"], "blocked");
    core.request("tab.focus", json!({"tab_id":tab_id}));
    let acknowledged = core.wait_for_pane_status(pane_id, "idle");
    assert_eq!(acknowledged["tokens"]["command.status"], "failure");
    assert_eq!(acknowledged["tokens"]["command.exit_code"], "7");
}

#[test]
fn completion_without_an_exit_code_never_uses_a_success_dot() {
    let mut core = ShellCore::launch(Some("/bin/bash"), "");
    core.wait_for("bash-ready> ");
    let snapshot = core.request("session.snapshot", json!({}));
    let pane_id = snapshot["snapshot"]["focused_pane_id"].as_str().unwrap();
    core.client
        .as_mut()
        .unwrap()
        .type_text(
            "(while [ ! -e release-unknown ]; do sleep 0.02; done; printf '\\033]133;D\\007')\r",
        )
        .unwrap();
    core.wait_for_pane_status(pane_id, "working");
    std::fs::write(core.home.join("release-unknown"), "").unwrap();
    let completed = core.wait_for_pane_status(pane_id, "unknown");
    assert_eq!(completed["tokens"]["command.status"], "unknown");
    assert!(completed["tokens"].get("command.exit_code").is_none());
}
