#![cfg(target_os = "linux")]

#[path = "support/console.rs"]
mod console;
pub mod support;

use console::Console;
use serde_json::json;
use std::{
    io::{self, Read},
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use support::Core;
use trailboss_runtime::protocol::{self, widgets::Node, ClientMessage, ServerMessage};

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
            "missing owned witness {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn identity(pid: &str) -> (String, String, PathBuf, PathBuf) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    assert_ne!(fields[4], "0", "original process lost its controlling PTY");
    (
        fields[19].into(),
        fields[4].into(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}
fn marker_tabs(core: &Core, prefix: &str) -> Vec<(String, String)> {
    let snapshot = core.request("session.snapshot", json!({}));
    let workspace = snapshot["snapshot"]["workspaces"][0]["workspace_id"]
        .as_str()
        .unwrap();
    let mut tabs = vec![(
        snapshot["snapshot"]["tabs"][0]["tab_id"]
            .as_str()
            .unwrap()
            .to_owned(),
        snapshot["snapshot"]["panes"][0]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned(),
    )];
    for number in 2..=3 {
        let created = core.request("tab.create", json!({"workspace_id":workspace,"label":format!("{prefix}-{number}"),"cwd":core.home,"focus":false}));
        tabs.push((
            created["tab"]["tab_id"].as_str().unwrap().into(),
            created["root_pane"]["pane_id"].as_str().unwrap().into(),
        ));
    }
    for (index, (_, pane)) in tabs.iter().enumerate() {
        core.request("pane.send_text", json!({"pane_id":pane,"text":format!("marker={prefix}-{}; printf '\\033[2J\\033[H'; printf '%s-live\\n' \"$marker\"\n",index+1)}));
    }
    core.request("tab.focus", json!({"tab_id":tabs[1].0}));
    tabs
}
fn original_job(core: &Core, pane: &str, name: &str) -> (String, String) {
    core.request("pane.send_text", json!({"pane_id":pane,"text":format!("sleep 120 & fence_job=$!; printf '%s %s\\n' \"$$\" \"$fence_job\" > {name}.state\n")}));
    let state = wait_file(&core.home.join(format!("{name}.state")));
    let mut fields = state.split_whitespace();
    (fields.next().unwrap().into(), fields.next().unwrap().into())
}

// Only an owned destination socket is interposed. Every byte still comes from
// the real native client or the real destination Core; the gate delays readiness.
struct DestinationGate {
    path: PathBuf,
    original: PathBuf,
    release: Option<mpsc::SyncSender<bool>>,
    ready: mpsc::Receiver<io::Result<()>>,
    stop: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<UnixStream>>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl DestinationGate {
    fn new(core: &Core) -> Self {
        let path = core.home.join(format!(
            "trailboss/sessions/{}/trailboss-client.sock",
            core.name
        ));
        let original = path.with_file_name("fence-original-client.sock");
        std::fs::rename(&path, &original).unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (release, gate) = mpsc::sync_channel(1);
        let (ready_tx, ready) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let streams = Arc::new(Mutex::new(Vec::new()));
        let worker_stop = stop.clone();
        let worker_streams = streams.clone();
        let original_path = original.clone();
        let worker = thread::spawn(move || {
            let run = || -> io::Result<()> {
                let deadline = Instant::now() + Duration::from_secs(8);
                let mut native = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == io::ErrorKind::WouldBlock
                                && !worker_stop.load(Ordering::Acquire)
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => return Err(error),
                    }
                };
                native.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut destination = UnixStream::connect(&original_path)?;
                destination.set_read_timeout(Some(Duration::from_secs(5)))?;
                worker_streams
                    .lock()
                    .unwrap()
                    .extend([native.try_clone()?, destination.try_clone()?]);
                let hello: ClientMessage =
                    protocol::read_message(&mut native, protocol::MAX_FRAME_SIZE)
                        .map_err(io::Error::other)?;
                protocol::write_message(&mut destination, &hello).map_err(io::Error::other)?;
                let first: ServerMessage =
                    protocol::read_message(&mut destination, protocol::MAX_GRAPHICS_FRAME_SIZE)
                        .map_err(io::Error::other)?;
                ready_tx.send(Ok(())).map_err(io::Error::other)?;
                if !gate
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(io::Error::other)?
                {
                    return Ok(());
                }
                protocol::write_message(&mut native, &first).map_err(io::Error::other)?;
                native.set_read_timeout(None)?;
                destination.set_read_timeout(None)?;
                let mut downstream_read = destination.try_clone()?;
                let mut downstream_write = native.try_clone()?;
                let downstream =
                    thread::spawn(move || io::copy(&mut downstream_read, &mut downstream_write));
                let result = io::copy(&mut native, &mut destination);
                let _ = native.shutdown(Shutdown::Both);
                let _ = destination.shutdown(Shutdown::Both);
                let _ = downstream.join();
                result.map(|_| ())
            };
            if let Err(error) = run() {
                let _ = ready_tx.send(Err(error));
            }
        });
        Self {
            path,
            original,
            release: Some(release),
            ready,
            stop,
            streams,
            worker: Some(worker),
        }
    }
    fn wait_preparing(&self) {
        self.ready
            .recv_timeout(Duration::from_secs(5))
            .expect("native did not prepare destination")
            .expect("real destination handshake failed");
    }
    fn wait_cancelled(&self) {
        let mut native = self.streams.lock().unwrap()[0].try_clone().unwrap();
        native
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(
            native
                .read(&mut byte)
                .expect("cancelled destination did not close"),
            0
        );
    }
    fn release(&mut self) {
        self.release.take().unwrap().send(true).unwrap();
    }
}
impl Drop for DestinationGate {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(release) = self.release.take() {
            let _ = release.send(false);
        }
        for stream in self.streams.lock().unwrap().iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::rename(&self.original, &self.path);
    }
}

fn source_owner(core: &Core) -> PathBuf {
    let root = core.home.join("source-owner");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("source-session"), &core.name).unwrap();
    let snapshot = core.request("session.snapshot", json!({}));
    std::fs::write(
        root.join("context.json"),
        json!({
            "pane_id":snapshot["snapshot"]["focused_pane_id"],
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        root.join("trailboss-plugin.toml"),
        r#"id = "fixture.source-fence"
name = "Source fence owner"
version = "1.0.0"
min_herdr_version = "0.9.0"
[[actions]]
id = "late"
title = "Late source action"
contexts = ["pane"]
argument_input = { kind = "raw_tail" }
command = ["python3", "owner.py", "late"]
[[ui]]
id = "source-menu"
surface = "sidebar"
command = ["python3", "owner.py", "stream"]
stream = true
"#,
    )
    .unwrap();
    std::fs::write(root.join("owner.py"), r#"import json,os,pathlib,socket,sys,time
root=pathlib.Path(__file__).parent
if sys.argv[1]=='late':
 (root/'started').write_text('started\n')
 deadline=time.monotonic()+15
 while not (root/'release').exists():
  if time.monotonic()>deadline: raise RuntimeError('owned action barrier timed out')
  time.sleep(.005)
 endpoint=os.environ['HERDR_SOCKET_PATH']; pane=json.loads(os.environ['HERDR_PLUGIN_CONTEXT_JSON'])['pane_id']
 with socket.socket(socket.AF_UNIX) as sock:
  sock.settimeout(5)
  sock.connect(endpoint)
  request={'id':'source-fence','method':'pane.send_text','params':{'pane_id':pane,'text':'printf "SOURCE_ACTION_%s\\n" "$marker"\n'}}
  sock.sendall((json.dumps(request)+'\n').encode())
  reply=b''
  while not reply.endswith(b'\n'):
   part=sock.recv(65536)
   if not part: raise RuntimeError('original source API closed before its reply')
   reply+=part
 (root/'completed').write_text(json.dumps({'endpoint':endpoint,'pane':pane,'reply':json.loads(reply)})+'\n')
else:
 if os.environ['HERDR_SESSION'] != (root/'source-session').read_text():
  print(json.dumps({'herdr':'widget_tree','version':1,'zones':[],'fragments':[]}),flush=True)
  while True: time.sleep(1)
 previous=None
 while True:
  phase=(root/'phase').read_text().strip() if (root/'phase').exists() else 'SOURCE_INITIAL_MENU'
  if phase!=previous:
   tree={'type':'column','children':[{'type':'label','text':phase,'role':'muted'},{'type':'row','action':'host:switch-session','children':[{'type':'label','text':'Switch session','role':'label'}]},{'type':'row','action':'host:detach','children':[{'type':'label','text':'Detach from session','role':'label'}]}]}
   binding={'type':'row','action':'late','context':json.loads((root/'context.json').read_text()),'children':[{'type':'label','text':'Late source action','role':'label'}]}
   print(json.dumps({'herdr':'widget_tree','version':1,'zones':[],'fragments':[{'name':'sidebar.session-menu','priority':100,'tree':tree},{'name':'source.bound-action','priority':0,'tree':binding}]}),flush=True)
   previous=phase
  time.sleep(.005)
"#).unwrap();
    core.request("plugin.link", json!({"path":root,"enabled":true}));
    root
}
fn has_bound_source_action(client: &trailboss::Client, pane: &str) -> bool {
    client.declarations.as_ref().is_some_and(|items| items.iter().any(|(_, declaration)| {
        declaration.fragments.iter().any(|fragment| {
            fragment.name == "command-palette"
                && matches!(&fragment.tree, Node::Column { children } if children.iter().any(|node| {
                    matches!(node, Node::Row { context, children, .. }
                        if context.pane_id.as_deref() == Some(pane)
                            && children.iter().any(|node| matches!(node, Node::Label { text, .. } if text == "Late source action")))
                }))
        })
    }))
}
fn has_source_phase(client: &trailboss::Client, phase: &str) -> bool {
    client.declarations.as_ref().is_some_and(|items| items.iter().any(|(_, declaration)| declaration.fragments.iter().any(|fragment| {
        fragment.name == "sidebar.session-menu" && matches!(&fragment.tree, Node::Column { children } if children.iter().any(|node| matches!(node, Node::Label { text, .. } if text == phase)))
    })))
}
fn select_command(console: &mut Console, title: &str) {
    console.text("\x1bOP");
    console.wait(|rows| rows.iter().any(|row| row.contains("Commands")));
    console.text(title);
    console.wait(|rows| rows.iter().filter(|row| row.contains(title)).count() >= 2);
    console.text("\r");
}
fn selected_clipboard_payload(console: &mut Console) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut captured = Vec::new();
    loop {
        if let Some(start) = captured.windows(5).position(|bytes| bytes == b"]52;c") {
            let payload = &captured[start + 6..];
            if let Some(end) = payload.iter().position(|byte| matches!(byte, 7 | 27)) {
                return payload[..end].to_vec();
            }
        }
        let bytes = console
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| {
                panic!(
                    "native selection emitted no OSC52: {error}; screen={:?}; output={:?}",
                    console.rows(),
                    String::from_utf8_lossy(&captured[captured.len().saturating_sub(2500)..]),
                )
            });
        console.screen.write(&bytes);
        captured.extend_from_slice(&bytes);
    }
}

#[test]
fn queued_source_updates_cannot_repaint_or_retarget_the_destination() {
    let alpha = Core::new();
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let alpha_tabs = marker_tabs(&alpha, "alpha");
    let beta_tabs = marker_tabs(&beta, "beta");
    assert_eq!(
        alpha_tabs[1].0, beta_tabs[1].0,
        "exercise colliding per-Core tab IDs"
    );
    let (alpha_shell, alpha_job) = original_job(&alpha, &alpha_tabs[1].1, "alpha");
    let (beta_shell, beta_job) = original_job(&beta, &beta_tabs[1].1, "beta");
    let identities: Vec<_> = [&alpha_shell, &alpha_job, &beta_shell, &beta_job]
        .into_iter()
        .map(|pid| identity(pid))
        .collect();
    let owner = source_owner(&alpha);
    let mut native = Console::with_cell_size(&alpha, (8, 16));
    native.screen.enable_kitty_graphics().unwrap();
    native.screen.resize(80, 24, 8, 16).unwrap();
    native.wait(|rows| rows.iter().any(|row| row.contains("alpha-2-live")));
    let mut observer = trailboss::Client::attach_graphics(
        &alpha.home.join(format!(
            "trailboss/sessions/{}/trailboss-client.sock",
            alpha.name
        )),
        80,
        24,
        (8, 16),
        true,
        false,
    )
    .unwrap();
    while !has_source_phase(&observer, "SOURCE_INITIAL_MENU")
        || !has_bound_source_action(&observer, &alpha_tabs[1].1)
    {
        observer.receive().unwrap();
    }
    for index in [2, 0, 1] {
        alpha.request("tab.focus", json!({"tab_id":alpha_tabs[index].0}));
        native.wait(|rows| {
            rows.iter()
                .any(|row| row.contains(&format!("alpha-{}-live", index + 1)))
        });
    }
    select_command(&mut native, "Late source action");
    native.text("owned\r");
    wait_file(&owner.join("started"));
    select_command(&mut native, "Switch session");
    native.wait(|rows| rows.iter().any(|row| row.contains(&beta.name)));
    let mut gate = DestinationGate::new(&beta);
    native.text(&format!("{}\r", beta.name));
    gate.wait_preparing();
    std::fs::write(owner.join("phase"), "SOURCE_QUEUED_MENU\n").unwrap();
    alpha.request("pane.send_text", json!({"pane_id":alpha_tabs[1].1,"text":"printf '\\033_Ga=T,f=32,t=d,i=774,p=3,s=1,v=1,c=1,r=1,q=2;/wAA/w==\\033\\\\'; printf 'QUEUED_SOURCE_%s\\n' \"$marker\"\n"}));
    let mut source_image = None;
    loop {
        if let Some(surface) = &observer.primary {
            if let Some(asset) = surface.graphics.assets.iter().find(|asset| {
                matches!(
                    &asset.key.source,
                    trailboss_runtime::protocol::SurfaceGraphicsSource::Terminal {
                        image_id: 774,
                        ..
                    }
                )
            }) {
                assert_eq!(asset.data, [255, 0, 0, 255]);
                source_image = Some(asset.key.clone());
            }
            // Image bytes and later text can arrive in separate frames; a
            // subsequent complete scene legitimately reuses the delivered asset.
            if has_source_phase(&observer, "SOURCE_QUEUED_MENU")
                && surface.text().contains("QUEUED_SOURCE_alpha-2")
                && source_image.as_ref().is_some_and(|key| {
                    surface.graphics.placements.iter().any(|placement| {
                        &placement.asset == key && placement.logical_placement_id == 3
                    })
                })
            {
                break;
            }
        }
        observer.receive().unwrap_or_else(|error| {
            panic!(
                "queued source observation failed: {error}; phase={}; surface={:?}; graphics={:?}",
                has_source_phase(&observer, "SOURCE_QUEUED_MENU"),
                observer.primary.as_ref().map(|surface| surface.text()),
                observer.primary.as_ref().map(|surface| (
                    surface.graphics.assets.len(),
                    surface.graphics.placements.len()
                )),
            )
        });
    }
    gate.release();
    native.wait(|rows| rows.iter().any(|row| row.contains("beta-2-live")));
    std::fs::write(owner.join("phase"), "SOURCE_AFTER_COMMIT_MENU\n").unwrap();
    std::fs::write(owner.join("release"), "release\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let completed: serde_json::Value = loop {
        if let Ok(text) = std::fs::read_to_string(owner.join("completed")) {
            if text.ends_with('\n') {
                break serde_json::from_str(&text).unwrap();
            }
        }
        assert!(
            Instant::now() < deadline,
            "late source action did not complete: {}",
            alpha.request(
                "plugin.log.list",
                json!({"plugin_id":"fixture.source-fence","limit":10})
            ),
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        completed["endpoint"],
        alpha
            .home
            .join(format!("trailboss/sessions/{}/trailboss.sock", alpha.name))
            .to_str()
            .unwrap()
    );
    assert!(
        completed["reply"].get("error").is_none(),
        "late original action failed: {completed}"
    );
    while !observer
        .primary
        .as_ref()
        .is_some_and(|surface| surface.text().contains("SOURCE_ACTION_alpha-2"))
    {
        observer.receive().unwrap();
    }
    native.text("\x1b[17~");
    native.text("printf '%s-history-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("beta-2-history-{beta_shell}");
    native.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    native.text("\x1b[<35;20;24M");
    let rows = native.wait(|rows| rows[23].contains('⋯'));
    native.click(
        rows[23]
            .chars()
            .position(|character| character == '⋯')
            .unwrap(),
        23,
    );
    let rows = native.wait(|rows| rows.iter().any(|row| row.contains("Detach from session")));
    assert!(
        !rows.iter().any(|row| row.contains("SOURCE_")),
        "old source declarations repainted destination: {rows:?}"
    );
    native.text("\x1b");
    native.wait(|rows| !rows.iter().any(|row| row.contains("Detach from session")));
    native.click(40, 10);
    native.text("printf '\\033[2J\\033[H'; printf '    %s-selection\\n' \"$marker\"\r");
    let rows = native.wait(|rows| rows.iter().any(|row| row.contains("beta-2-selection")));
    let y = rows
        .iter()
        .position(|row| row.contains("beta-2-selection"))
        .unwrap();
    let x = rows[y][..rows[y].find("beta-2-selection").unwrap()]
        .chars()
        .count();
    native.text(&format!(
        "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
        x + 1,
        y + 1,
        x + 6,
        y + 1,
        x + 6,
        y + 1
    ));
    assert_eq!(
        selected_clipboard_payload(&mut native),
        b"YmV0YS0y",
        "selection must copy beta-2, not the old source surface"
    );
    assert!(
        native.screen.kitty_image_placements().unwrap().is_empty(),
        "old source graphics survived the attachment switch"
    );
    for (pid, before) in [&alpha_shell, &alpha_job, &beta_shell, &beta_job]
        .into_iter()
        .zip(identities)
    {
        assert_eq!(identity(pid), before);
    }
    alpha.request("ping", json!({}));
    beta.request("ping", json!({}));
    alpha.request(
        "pane.send_text",
        json!({"pane_id":alpha_tabs[1].1,"text":"kill \"$fence_job\"\n"}),
    );
    beta.request(
        "pane.send_text",
        json!({"pane_id":beta_tabs[1].1,"text":"kill \"$fence_job\"\n"}),
    );
}

#[test]
fn pending_destination_keeps_source_responsive_and_escape_cancels_before_commit() {
    let mut alpha = Core::new();
    let mut beta = Core::launch_with_executable(
        alpha.home.clone(),
        "beta-preparation".into(),
        support::executable(),
        false,
    );
    let alpha_tabs = marker_tabs(&alpha, "alpha");
    let beta_tabs = marker_tabs(&beta, "beta");
    let alpha_processes = original_job(&alpha, &alpha_tabs[1].1, "responsive-alpha");
    let beta_processes = original_job(&beta, &beta_tabs[1].1, "responsive-beta");
    let originals = [
        &alpha_processes.0,
        &alpha_processes.1,
        &beta_processes.0,
        &beta_processes.1,
    ]
    .map(|pid| identity(pid));
    let mut native = Console::new(&alpha);
    native.wait(|rows| rows.iter().any(|row| row.contains("alpha-2-live")));
    let mut gate = DestinationGate::new(&beta);
    native.text("\x1bOP");
    native.text("Switch session\r");
    native.wait(|rows| rows.iter().any(|row| row.contains(&beta.name)));
    native.text(&format!("{}\r", beta.name));
    gate.wait_preparing();
    native.click(40, 10);

    native.text("printf '%s-preparing-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("alpha-2-preparing-{}", alpha_processes.0);
    let rows = native.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    assert!(
        !rows.iter().any(|row| row.contains("Cannot switch session")),
        "{rows:?}"
    );
    assert!(
        !gate.worker.as_ref().unwrap().is_finished(),
        "source only resumed after the destination handshake timed out"
    );

    native.text("\x1b");
    gate.wait_cancelled();
    native.text("printf '%s-cancelled-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("alpha-2-cancelled-{}", alpha_processes.0);
    let rows = native.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    assert!(
        !rows.iter().any(|row| row.contains("Cannot switch session")),
        "{rows:?}"
    );
    gate.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gate.worker.as_ref().unwrap().is_finished() {
        assert!(
            Instant::now() < deadline,
            "cancelled destination retained its connection"
        );
        thread::sleep(Duration::from_millis(5));
    }
    native.text("printf '%s-after-cancel-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("alpha-2-after-cancel-{}", alpha_processes.0);
    let rows = native.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    assert!(
        !rows.iter().any(|row| row.contains("Cannot switch session")),
        "{rows:?}"
    );
    for (pid, original) in [
        &alpha_processes.0,
        &alpha_processes.1,
        &beta_processes.0,
        &beta_processes.1,
    ]
    .into_iter()
    .zip(originals)
    {
        assert_eq!(identity(pid), original);
    }
    assert!(alpha.child.try_wait().unwrap().is_none());
    assert!(beta.child.try_wait().unwrap().is_none());
}
