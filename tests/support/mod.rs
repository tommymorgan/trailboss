use herdr_fork::Client;
use herdr_runtime::{
    api_client::{ApiClient, ConnectionTarget},
    api_schema::Request,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct Core {
    pub child: Child,
    pub home: PathBuf,
    pub name: String,
    api: ApiClient,
}
impl Drop for Core {
    fn drop(&mut self) {
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-fork"));
        command.args(["server", "--session", &name]);
        for key in HOMES {
            command.env(key, &home);
        }
        for (key, _) in
            std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("HERDR_"))
        {
            command.env_remove(key);
        }
        command
            .env("SHELL", "/bin/sh")
            .env("HERDR_CONFIG_PATH", home.join("config.toml"))
            .env("HERDR_STARTUP_CWD", &home)
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let child = command.spawn().unwrap();
        let socket = home.join(format!("herdr-fork/sessions/{name}/herdr.sock"));
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(socket.clone()));
        let mut core = Self {
            child,
            home,
            name,
            api,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            assert!(
                Instant::now() < deadline && core.child.try_wait().unwrap().is_none(),
                "isolated core startup failed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
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
                "herdr-fork/sessions/{}/herdr-client.sock",
                self.name
            )),
            80,
            24,
        )
        .unwrap()
    }
}
