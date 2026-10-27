use serde_json::{json, Value};
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

pub struct Core {
    pub child: Child,
    pub home: PathBuf,
    pub name: String,
    api: ApiClient,
}
impl Drop for Core {
    fn drop(&mut self) {
        if self.child.try_wait().is_ok_and(|status| status.is_some()) {
            let _ = std::fs::remove_dir_all(&self.home);
            return;
        }
        // Let Core tear down its supervised producer before a forced fallback.
        let request: Request = serde_json::from_value(
            json!({"id":"fixture-stop", "method":"server.stop", "params":{}}),
        )
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
pub const HOMES: &[&str] = &[
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
];
pub fn executable() -> PathBuf {
    option_env!("CARGO_BIN_EXE_trailboss")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("trailboss")
        })
}
impl Core {
    pub fn new() -> Self {
        let name = format!(
            "at-{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let home = std::env::temp_dir().join(&name);
        std::fs::create_dir_all(&home).unwrap();
        Self::launch(home, name)
    }
    pub fn launch(home: PathBuf, name: String) -> Self {
        Self::launch_with_executable(home, name, executable())
    }
    pub fn launch_with_executable(home: PathBuf, name: String, executable: PathBuf) -> Self {
        let mut command = Command::new(executable);
        command.args(["server", "--session", &name]);
        for key in HOMES {
            command.env(key, &home);
        }
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            let name = key.to_string_lossy();
            name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
        }) {
            command.env_remove(key);
        }
        if std::env::var_os("TRAILBOSS_LATENCY_CAPTURE").is_some() {
            command.env("TRAILBOSS_RENDER_PROF_TRACE", home.join("core-timing.log"));
        }
        command
            .env("SHELL", "/bin/sh")
            .env("TRAILBOSS_CONFIG_PATH", home.join("config.toml"))
            .env("TRAILBOSS_STARTUP_CWD", &home)
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let child = command.spawn().unwrap();
        let socket = home.join(format!("trailboss/sessions/{name}/trailboss.sock"));
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(socket.clone()));
        let mut core = Self {
            child,
            home,
            name,
            api,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !core
            .home
            .join(format!(
                "trailboss/sessions/{}/trailboss-client.sock",
                core.name
            ))
            .exists()
        {
            assert!(
                Instant::now() < deadline && core.child.try_wait().unwrap().is_none(),
                "isolated core startup failed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        core.request("session.snapshot", json!({}));
        core
    }
    pub fn request(&self, method: &str, params: Value) -> Value {
        let request: Request = serde_json::from_value(
            json!({"id":"agent-tree-fixture", "method":method, "params":params}),
        )
        .unwrap();
        let reply = self
            .api
            .request_value_with_timeout(&request, Duration::from_secs(5))
            .unwrap();
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }
    pub fn attach(&self) -> Client {
        Client::attach(
            &self.home.join(format!(
                "trailboss/sessions/{}/trailboss-client.sock",
                self.name
            )),
            80,
            24,
        )
        .unwrap()
    }
}
