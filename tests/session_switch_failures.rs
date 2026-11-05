#[path = "support/console.rs"]
mod console;
pub mod support;

use console::Console;
use serde_json::json;
use std::{
    io,
    os::unix::{fs::MetadataExt, net::UnixListener},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use support::Core;
use trailboss_runtime::protocol::{
    self,
    endpoint::{
        EndpointClientHello, EndpointServerWelcome, ENDPOINT_HELLO_KIND, ENDPOINT_WELCOME_KIND,
    },
    ClientMessage, ServerMessage, MAX_FRAME_SIZE,
};

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
            "shell did not write {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn process_identity(pid: &str) -> (String, String, PathBuf, PathBuf) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[4], "0", "process lost its controlling PTY");
    (
        fields[19].to_owned(),
        fields[4].to_owned(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}

struct OriginalShell {
    pane: String,
    shell: String,
    job: String,
    shell_identity: (String, String, PathBuf, PathBuf),
    job_identity: (String, String, PathBuf, PathBuf),
    state: String,
    boot: serde_json::Value,
}
impl OriginalShell {
    fn seed(core: &Core, marker: &str) -> Self {
        let snapshot = core.request("session.snapshot", json!({}));
        let pane = snapshot["snapshot"]["panes"][0]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned();
        core.request("pane.send_text", json!({
            "pane_id": pane,
            "text": format!("marker={marker}; sleep 60 & switch_job=$!; printf '%s %s %s\\n' \"$$\" \"$switch_job\" \"$(tty)\" > {marker}.state; printf '%s-original-shell\\n' \"$marker\"\n")
        }));
        let state = wait_file(&core.home.join(format!("{marker}.state")));
        let fields: Vec<_> = state.split_whitespace().collect();
        let shell = fields[0].to_owned();
        let job = fields[1].to_owned();
        let shell_identity = process_identity(&shell);
        let job_identity = process_identity(&job);
        assert_eq!(shell_identity.2, PathBuf::from(fields[2]));
        Self {
            pane,
            shell,
            job,
            shell_identity,
            job_identity,
            state,
            boot: snapshot["snapshot"]["boot_id"].clone(),
        }
    }
    fn assert_unchanged(&self, core: &Core, marker: &str) {
        assert_eq!(process_identity(&self.shell), self.shell_identity);
        assert_eq!(process_identity(&self.job), self.job_identity);
        assert_eq!(
            std::fs::read_to_string(core.home.join(format!("{marker}.state"))).unwrap(),
            self.state
        );
        let snapshot = core.request("session.snapshot", json!({}));
        assert_eq!(snapshot["snapshot"]["boot_id"], self.boot);
        assert_eq!(snapshot["snapshot"]["focused_pane_id"], self.pane);
        assert_eq!(snapshot["snapshot"]["panes"].as_array().unwrap().len(), 1);
    }
}

fn open_session_menu_picker(console: &mut Console) {
    console.text("\x1b[<35;20;24M");
    let rows = console.wait(|rows| rows[23].contains('⋯'));
    console.click(
        rows[23]
            .chars()
            .position(|character| character == '⋯')
            .unwrap(),
        23,
    );
    let rows = console.wait(|rows| rows.iter().any(|row| row.contains("Switch session")));
    let y = rows
        .iter()
        .position(|row| row.contains("Switch session"))
        .unwrap();
    let x = rows[y][..rows[y].find("Switch session").unwrap()]
        .chars()
        .count();
    console.click(x, y);
}

fn socket_identity(path: &Path) -> (u64, u64) {
    let metadata = std::fs::metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

// Only the disposable Beta client socket is interposed. Its real Core, API,
// shell and job remain running; Alpha always uses its real transport.
struct InterposedEndpoint {
    path: PathBuf,
    saved: PathBuf,
    original: (u64, u64),
    replacement: Option<(u64, u64)>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<(), String>>>,
}
impl InterposedEndpoint {
    fn new(path: PathBuf, incompatible: bool) -> Self {
        let saved = path.with_extension("sock.original");
        assert!(!saved.exists());
        let original = socket_identity(&path);
        std::fs::rename(&path, &saved).unwrap();
        let mut endpoint = Self {
            path,
            saved,
            original,
            replacement: None,
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        let listener = UnixListener::bind(&endpoint.path).unwrap();
        endpoint.replacement = Some(socket_identity(&endpoint.path));
        if incompatible {
            listener.set_nonblocking(true).unwrap();
            let cancel = Arc::clone(&endpoint.cancel);
            endpoint.worker = Some(thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(8);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
                                return Err("native destination Hello was not received".into());
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|error| error.to_string())?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .map_err(|error| error.to_string())?;
                let hello: ClientMessage = protocol::read_message(&mut stream, MAX_FRAME_SIZE)
                    .map_err(|error| error.to_string())?;
                let ClientMessage::EndpointControl { kind, data } = hello else {
                    return Err("native client did not send an endpoint Hello".into());
                };
                if kind != ENDPOINT_HELLO_KIND {
                    return Err("native client did not send an endpoint Hello".into());
                }
                let _: EndpointClientHello =
                    serde_json::from_str(&data).map_err(|error| error.to_string())?;
                let mut welcome = EndpointServerWelcome::compatible(Vec::new());
                welcome.generation = welcome.generation.wrapping_add(1);
                protocol::write_message(
                    &mut stream,
                    &ServerMessage::EndpointControl {
                        kind: ENDPOINT_WELCOME_KIND.into(),
                        data: serde_json::to_string(&welcome).map_err(|error| error.to_string())?,
                    },
                )
                .map_err(|error| error.to_string())
            }));
        } else {
            // A bound-but-unlistened Unix socket deterministically refuses connect,
            // unlike removing the path (which would only exercise NotFound).
            drop(listener);
        }
        endpoint
    }
    fn assert_observed_and_restore(mut self) {
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("incompatible endpoint worker panicked")
                .expect("native incompatible negotiation was not exercised");
        }
        assert_eq!(
            socket_identity(&self.path),
            self.replacement.unwrap(),
            "switch replaced the owned endpoint"
        );
        assert_eq!(
            socket_identity(&self.saved),
            self.original,
            "original Beta endpoint changed"
        );
        self.restore();
        assert_eq!(socket_identity(&self.path), self.original);
    }
    fn restore(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if self.saved.exists() {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::rename(&self.saved, &self.path);
        }
    }
}
impl Drop for InterposedEndpoint {
    fn drop(&mut self) {
        self.restore();
    }
}

#[derive(Debug, Clone, Copy)]
enum Failure {
    StoppedAfterDiscovery,
    IncompatibleHandshake,
    ConnectionRefused,
}

fn failed_switch_retains_original_attachment(failure: Failure) {
    // Alpha owns the isolated HOME; borrowed Beta drops first, including unwind.
    let mut alpha = Core::new();
    let mut beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let alpha_pid = alpha.child.id();
    let beta_pid = beta.child.id();
    let alpha_shell = OriginalShell::seed(&alpha, "alpha");
    let beta_shell = OriginalShell::seed(&beta, "beta");
    let alpha_session = alpha
        .home
        .join(format!("trailboss/sessions/{}", alpha.name));
    let beta_session = beta.home.join(format!("trailboss/sessions/{}", beta.name));
    let alpha_api = socket_identity(&alpha_session.join("trailboss.sock"));
    let alpha_client = socket_identity(&alpha_session.join("trailboss-client.sock"));
    let beta_api = socket_identity(&beta_session.join("trailboss.sock"));
    let mut console = Console::new(&alpha);
    let original_view =
        console.wait(|rows| rows.iter().any(|row| row.contains("alpha-original-shell")));
    open_session_menu_picker(&mut console);
    console.wait(|rows| rows.iter().any(|row| row.contains(&beta.name)));

    // Damage the destination only after native discovery visibly includes Beta.
    let endpoint = match failure {
        Failure::StoppedAfterDiscovery => {
            beta.request("server.stop", json!({}));
            let deadline = Instant::now() + Duration::from_secs(5);
            while beta.child.try_wait().unwrap().is_none()
                || beta_session.join("trailboss.sock").exists()
                || beta_session.join("trailboss-client.sock").exists()
            {
                assert!(Instant::now() < deadline, "owned Beta did not stop");
                thread::sleep(Duration::from_millis(10));
            }
            None
        }
        Failure::IncompatibleHandshake | Failure::ConnectionRefused => {
            Some(InterposedEndpoint::new(
                beta_session.join("trailboss-client.sock"),
                matches!(failure, Failure::IncompatibleHandshake),
            ))
        }
    };
    console.text(&beta.name);
    console.text("\r");
    // Observe a new top-of-window notice after the picker closes, without
    // coupling the behavioral proof to OS error wording or incidental prose.
    let failure_view = console.wait(|rows| {
        !rows.iter().any(|row| row.contains("Switch session on")) && rows[..4] != original_view[..4]
    });
    eprintln!(
        "native {failure:?} failure notice screen:\n{}",
        failure_view.join("\n")
    );
    console.text("printf '%s-retained-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("alpha-retained-{}", alpha_shell.shell);
    console.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    assert_eq!(alpha.child.id(), alpha_pid);
    assert!(alpha.child.try_wait().unwrap().is_none());
    alpha_shell.assert_unchanged(&alpha, "alpha");
    assert_eq!(
        socket_identity(&alpha_session.join("trailboss.sock")),
        alpha_api
    );
    assert_eq!(
        socket_identity(&alpha_session.join("trailboss-client.sock")),
        alpha_client
    );

    if let Some(endpoint) = endpoint {
        endpoint.assert_observed_and_restore();
        assert_eq!(beta.child.id(), beta_pid);
        assert!(beta.child.try_wait().unwrap().is_none());
        assert_eq!(
            socket_identity(&beta_session.join("trailboss.sock")),
            beta_api
        );
        beta_shell.assert_unchanged(&beta, "beta");
        // Restoring the saved socket returns access to the original real Beta
        // transport, rather than any replacement Core created during failure.
        let mut beta_window = Console::new(&beta);
        beta_window.wait(|rows| rows.iter().any(|row| row.contains("beta-original-shell")));
        beta_window.text("printf '%s-still-original-%s\\n' \"$marker\" \"$$\"\r");
        let expected = format!("beta-still-original-{}", beta_shell.shell);
        beta_window.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
        beta.request(
            "pane.send_text",
            json!({"pane_id":beta_shell.pane,"text":"kill \"$switch_job\"\n"}),
        );
    } else {
        assert!(
            beta.child.try_wait().unwrap().is_some(),
            "switch restarted stopped Beta"
        );
        assert!(
            !beta_session.join("trailboss.sock").exists(),
            "switch created a replacement Beta API"
        );
        assert!(
            !beta_session.join("trailboss-client.sock").exists(),
            "switch created a replacement Beta endpoint"
        );
        assert_eq!(
            std::fs::read_to_string(beta.home.join("beta.state")).unwrap(),
            beta_shell.state
        );
    }
    alpha.request(
        "pane.send_text",
        json!({"pane_id":alpha_shell.pane,"text":"kill \"$switch_job\"\n"}),
    );
}

#[test]
fn destination_stopping_after_discovery_keeps_the_original_alpha_attachment() {
    failed_switch_retains_original_attachment(Failure::StoppedAfterDiscovery);
}

#[test]
fn incompatible_destination_keeps_both_original_cores_and_the_alpha_attachment() {
    failed_switch_retains_original_attachment(Failure::IncompatibleHandshake);
}

#[test]
fn refused_destination_connection_keeps_both_original_cores_and_the_alpha_attachment() {
    failed_switch_retains_original_attachment(Failure::ConnectionRefused);
}
