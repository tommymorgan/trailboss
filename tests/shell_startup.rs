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
