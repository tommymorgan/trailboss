#[path = "support/console.rs"]
mod console;
mod support;
use console::Console;
use herdr_runtime::{
    api_client::{ApiClient, ConnectionTarget},
    api_schema::Request,
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use support::{Core, HOMES};

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn checked(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
struct Deployment {
    root: String,
}
impl Deployment {
    fn new() -> Self {
        let root = checked(Command::new("ssh").args([
            "-o",
            "BatchMode=yes",
            "achy",
            "mktemp -d /tmp/herdr-task51.XXXXXXXX",
        ]))
        .trim()
        .to_owned();
        let deployment = Self { root };
        checked(Command::new("scp").args([
            "-q",
            env!("CARGO_BIN_EXE_herdr-fork"),
            &format!("achy:{}/herdr-fork", deployment.root),
        ]));
        deployment
    }
}
impl Drop for Deployment {
    fn drop(&mut self) {
        let _ = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "achy",
                &format!("rm -rf -- {}", quote(&self.root)),
            ])
            .status();
    }
}
struct Decoy {
    child: Child,
    api: ApiClient,
}
impl Decoy {
    fn new(core: &Core) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-fork"));
        command.args(["server", "--session", "decoy"]);
        for key in HOMES {
            command.env(key, &core.home);
        }
        let child = command
            .env("SHELL", "/bin/sh")
            .env("HERDR_CONFIG_PATH", core.home.join("config.toml"))
            .env("HERDR_STARTUP_CWD", &core.home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(
            core.home.join("herdr-fork/sessions/decoy/herdr.sock"),
        ));
        let result = Self { child, api };
        let deadline = Instant::now() + Duration::from_secs(8);
        while result
            .api
            .request_value_with_timeout(
                &request("session.snapshot", json!({})),
                Duration::from_millis(100),
            )
            .is_err()
        {
            assert!(Instant::now() < deadline, "decoy startup");
            std::thread::sleep(Duration::from_millis(10));
        }
        let snapshot = result
            .api
            .request_value(&request("session.snapshot", json!({})))
            .unwrap();
        let pane = &snapshot["result"]["snapshot"]["panes"][0]["pane_id"];
        result
            .api
            .request_value(&request(
                "pane.send_text",
                json!({"pane_id":pane,"text":"printf 'decoy-%s\\n' forbidden\n"}),
            ))
            .unwrap();
        result
    }
}
impl Drop for Decoy {
    fn drop(&mut self) {
        let _ = self
            .api
            .request_value_with_timeout(&request("server.stop", json!({})), Duration::from_secs(2));
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().is_ok_and(|s| s.is_none()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn request(method: &str, params: serde_json::Value) -> Request {
    serde_json::from_value(json!({"id":"remote-proof", "method":method, "params":params})).unwrap()
}
fn wrapper(core: &Core) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = core.home.join("remote-core");
    let mut script = String::from("#!/bin/sh\n");
    for key in HOMES {
        script.push_str(&format!(
            "export {key}={}\n",
            quote(core.home.to_str().unwrap())
        ));
    }
    script.push_str(&format!(
        "export HERDR_CONFIG_PATH={}\nexec {} \"$@\"\n",
        quote(core.home.join("config.toml").to_str().unwrap()),
        quote(env!("CARGO_BIN_EXE_herdr-fork"))
    ));
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
fn seed(core: &Core) -> String {
    let pane = core.request("session.snapshot", json!({}))["snapshot"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    core.request("pane.send_text", json!({"pane_id":pane, "text":"printf '\\033[2J\\033[Hselected-%s\\n' live\n", "submit":false}));
    pane
}
#[test]
#[ignore = "requires real SSH to junkman; no remote installation"]
fn remote_transport_routes_live_surface_and_input_to_exact_session() {
    let core = Core::new();
    let decoy = Decoy::new(&core);
    let pane = seed(&core);
    let decoy_snapshot = decoy
        .api
        .request_value(&request("session.snapshot", json!({})))
        .unwrap();
    let decoy_pane = decoy_snapshot["result"]["snapshot"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap();
    let path = wrapper(&core);
    let mut local = core.attach();
    while local.boot_id.is_empty() {
        local.receive().unwrap();
    }
    for path in ["/definitely-missing-herdr-task51", "/bin/true"] {
        assert!(
            herdr_runtime::remote::RemoteConnection::connect("junkman", &core.name, Some(path))
                .is_err(),
            "missing/incompatible executable accepted: {path}"
        );
    }
    let connection = herdr_runtime::remote::RemoteConnection::connect(
        "junkman",
        &core.name,
        Some(path.to_str().unwrap()),
    )
    .unwrap();
    let mut remote = herdr_fork::Client::attach(connection.socket_path(), 80, 24).unwrap();
    wait_surface(&mut remote, "selected-live");
    assert_eq!(remote.boot_id, local.boot_id, "wrong Core boot identity");
    assert_eq!(remote.focused_pane_id.as_deref(), Some(pane.as_str()));
    let mut input = remote.input().unwrap();
    input.type_text("printf '\\033[2J\\033[Htransport-%s:%s:%s\\n' executed \"$(hostname)\" \"$HERDR_SESSION\"\r").unwrap();
    let expected = format!("transport-executed:junkman:{}", core.name);
    wait_surface(&mut remote, &expected);
    assert!(!remote
        .primary
        .as_ref()
        .unwrap()
        .text()
        .contains("decoy-forbidden"));
    assert_eq!(
        decoy
            .api
            .request_value(&request("session.snapshot", json!({})))
            .unwrap()["result"]["snapshot"]["focused_pane_id"],
        decoy_pane
    );
    input.detach().unwrap();
    println!(
        "real SSH transport: {expected}; exact pane {pane}; boot {}",
        remote.boot_id
    );
}
fn wait_surface(client: &mut herdr_fork::Client, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !client
        .primary
        .as_ref()
        .is_some_and(|surface| surface.text().contains(marker))
    {
        assert!(Instant::now() < deadline, "remote Surface missing {marker}");
        client.receive().unwrap_or_else(|error| {
            panic!(
                "remote Surface missing {marker}: {error}; last {:?}",
                client.primary.as_ref().map(|surface| surface.text())
            )
        });
    }
}
#[test]
#[ignore = "requires actual achy -> junkman SSH; deploys only a disposable binary"]
fn attach_from_achy_to_one_remote_session() {
    let mut session = None;
    let mut console = None;
    let mut steps = 0;
    for line in include_str!("../features/remote-attach.feature")
        .lines()
        .map(str::trim)
    {
        match line {
            "Given the remote core owns a named session and achy has SSH access to it" => {
                assert_eq!(steps, 0);
                let core = Core::new();
                let decoy = Decoy::new(&core);
                let pane = seed(&core);
                let path = wrapper(&core);
                session = Some(RemoteSession { decoy, pane, path, deployed: Deployment::new(), core });
                steps += 1;
            }
            "When I run the client with --remote on achy and it attaches over SSH to that session" => {
                assert_eq!(steps, 1);
                let session = session.as_ref().unwrap();
                let mut command = portable_pty::CommandBuilder::new("ssh");
                command.args(["-tt", "-o", "BatchMode=yes", "achy"]);
                command.arg(format!("env TERM=xterm-256color XDG_CONFIG_HOME={root} XDG_STATE_HOME={root} XDG_DATA_HOME={root} XDG_CACHE_HOME={root} XDG_RUNTIME_DIR={root} HERDR_CONFIG_PATH={root}/config.toml {binary} --remote junkman --remote-executable {path} --session {name}", root=quote(&session.deployed.root), binary=quote(&format!("{}/herdr-fork", session.deployed.root)), path=quote(session.path.to_str().unwrap()), name=quote(&session.core.name)));
                console = Some(Console::from_command(command));
                steps += 1;
            }
            "Then the client attaches to exactly that session and displays its live Surfaces while shell input executes on the remote core" => {
                assert_eq!(steps, 2);
                prove_remote_input(session.as_ref().unwrap(), console.as_mut().unwrap());
                steps += 1;
            }
            "" | "Feature: Remote session attach" | "Scenario: Attach from achy to one remote session" => {}
            _ => panic!("unbound remote living step: {line}"),
        }
    }
    assert_eq!(steps, 3);
}
struct RemoteSession {
    decoy: Decoy,
    pane: String,
    path: PathBuf,
    deployed: Deployment,
    core: Core,
}
fn prove_remote_input(session: &RemoteSession, console: &mut Console) {
    let core = &session.core;
    console.wait(|rows| rows.iter().any(|row| row.contains("selected-live")));
    console.text(
        "printf '\\033[2J\\033[Hexecuted-%s:%s:%s\\n' remote \"$(hostname)\" \"$HERDR_SESSION\"\r",
    );
    let expected = format!("executed-remote:junkman:{}", core.name);
    let rows = console.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    assert!(
        !rows.iter().any(|row| row.contains("decoy")),
        "mixed session Surface"
    );
    assert_eq!(
        core.request("session.snapshot", json!({}))["snapshot"]["focused_pane_id"],
        session.pane
    );
    println!(
        "actual achy client -> SSH junkman session {}; rendered {}",
        core.name, expected
    );
    let mut decoy = herdr_fork::Client::attach(
        &session
            .core
            .home
            .join("herdr-fork/sessions/decoy/herdr-client.sock"),
        80,
        24,
    )
    .unwrap();
    wait_surface(&mut decoy, "decoy-forbidden");
    assert!(
        !decoy.primary.as_ref().unwrap().text().contains(&expected),
        "input leaked into decoy session"
    );
    assert!(session
        .decoy
        .api
        .request_value(&request("session.snapshot", json!({})))
        .unwrap()
        .get("error")
        .is_none());
    console.text("\x11");
    let deadline = Instant::now() + Duration::from_secs(5);
    while console.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "normal detach");
        std::thread::sleep(Duration::from_millis(10));
    }
}
