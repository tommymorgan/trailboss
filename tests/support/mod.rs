use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
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
    owns_home: bool,
}
impl Drop for Core {
    fn drop(&mut self) {
        if self.home.as_os_str().is_empty() {
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
        // Handoff can replace the process while retaining this owned session socket.
        let socket = session_directory(&self.home, &self.name).join("trailboss.sock");
        while socket.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.owns_home {
            stop_transfer_helper(&self.home);
            let _ = std::fs::remove_dir_all(&self.home);
        }
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
fn session_directory(home: &std::path::Path, name: &str) -> PathBuf {
    let directory = home.join("trailboss");
    if name == trailboss_runtime::session::DEFAULT_SESSION_NAME {
        directory
    } else {
        directory.join("sessions").join(name)
    }
}

/// Linux start time of a live (non-zombie) process, the same identity token
/// the routing helper records for itself.
fn live_process_start(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    if fields.first() == Some(&"Z") {
        return None;
    }
    fields.get(19)?.parse().ok()
}

/// PID and start identity the routing helper recorded for this isolated home.
fn recorded_helper(home: &Path) -> Option<(u32, u64)> {
    let record = std::fs::read(home.join("trailboss/tab-transfer/helper.json")).ok()?;
    let identity: Value = serde_json::from_slice(&record).ok()?;
    let pid = u32::try_from(identity["pid"].as_u64()?).ok()?;
    Some((pid, identity["start"].as_u64()?))
}

/// PID of the live routing helper serving this isolated home, if any. The
/// helper records its own PID/start identity, so a reused PID never matches.
pub fn transfer_helper(home: &Path) -> Option<u32> {
    let (pid, start) = recorded_helper(home)?;
    (live_process_start(pid)? == start).then_some(pid)
}

// glibc's syscall(2); std links it, so no extra dependency is needed. These
// syscall numbers are shared by every Linux architecture.
unsafe extern "C" {
    fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
}
const SYS_PIDFD_SEND_SIGNAL: std::ffi::c_long = 424;
const SYS_PIDFD_OPEN: std::ffi::c_long = 434;
const SIGKILL: std::ffi::c_int = 9;

/// The routing helper is a per-user process that outlives sessions by design;
/// only the fixture knows its isolated home is ending, so it stops that home's
/// helper. Never matches by process name: other homes keep their helpers.
fn stop_transfer_helper(home: &Path) {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let Some((pid, start)) = recorded_helper(home) else {
        return;
    };
    // Pin the process before validating its identity: the pidfd keeps
    // referring to this exact process, so a PID reused after validation can
    // never receive the signal.
    let raw = unsafe {
        syscall(
            SYS_PIDFD_OPEN,
            pid as std::ffi::c_int,
            0 as std::ffi::c_uint,
        )
    };
    if raw < 0 {
        return;
    }
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw as std::ffi::c_int) };
    if live_process_start(pid) != Some(start) {
        return;
    }
    unsafe {
        syscall(
            SYS_PIDFD_SEND_SIGNAL,
            pidfd.as_raw_fd(),
            SIGKILL,
            std::ptr::null::<std::ffi::c_void>(),
            0 as std::ffi::c_uint,
        );
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while transfer_helper(home) == Some(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
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
        Self::launch_with_executable(home, name, executable(), true)
    }
    pub fn launch_with_executable(
        home: PathBuf,
        name: String,
        executable: PathBuf,
        owns_home: bool,
    ) -> Self {
        let mut command = Command::new(executable);
        command.args(["server", "--session", &name]);
        for key in HOMES {
            command.env(key, &home);
        }
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            let name = key.to_string_lossy();
            name.starts_with("TRAILBOSS_")
                || name.starts_with("HERDR_")
                || name.starts_with("PI_")
                || name.starts_with("OMP_")
                || name.starts_with("AWS_")
                || name.starts_with("AZURE_")
                || name.starts_with("GOOGLE_")
                || name.ends_with("_API_KEY")
                || name.ends_with("_AUTH_TOKEN")
                || name.ends_with("_ACCESS_TOKEN")
                || matches!(name.as_ref(), "GH_TOKEN" | "GITHUB_TOKEN")
        }) {
            command.env_remove(key);
        }
        if std::env::var_os("TRAILBOSS_LATENCY_CAPTURE").is_some() {
            command.env("TRAILBOSS_RENDER_PROF_TRACE", home.join("core-timing.log"));
        }
        command
            .env("SHELL", "/bin/sh")
            .env("PI_CODING_AGENT_DIR", home.join(".omp/agent"))
            .env("TRAILBOSS_CONFIG_PATH", home.join("config.toml"))
            .env("TRAILBOSS_STARTUP_CWD", &home)
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let child = command.spawn().unwrap();
        let socket = session_directory(&home, &name).join("trailboss.sock");
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(socket.clone()));
        let mut core = Self {
            child,
            home,
            name,
            api,
            owns_home,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let native_socket = session_directory(&core.home, &core.name).join("trailboss-client.sock");
        let ready: Request = serde_json::from_value(json!({
            "id":"owned-core-startup", "method":"session.snapshot", "params":{}
        }))
        .unwrap();
        loop {
            assert!(
                Instant::now() < deadline && core.child.try_wait().unwrap().is_none(),
                "isolated core startup failed"
            );
            if native_socket.exists()
                && core
                    .api
                    .request_value_with_timeout(&ready, Duration::from_millis(250))
                    .is_ok_and(|reply| {
                        reply.get("error").is_none() && reply.get("result").is_some()
                    })
            {
                break;
            }
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
            &session_directory(&self.home, &self.name).join("trailboss-client.sock"),
            80,
            24,
        )
        .unwrap()
    }
}
