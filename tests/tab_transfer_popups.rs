#![cfg(target_os = "linux")]

mod support;

use serde_json::{json, Value};
use std::{
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use support::Core;
use trailboss_runtime::protocol::{
    self,
    endpoint::{
        EndpointClientHello, BLOB_CODEC_V1, ENDPOINT_HELLO_KIND, INPUT_CODEC_V1, SNAPSHOT_CODEC_V1,
        SURFACE_CODEC_V1,
    },
    ClientMessage, ClientPaneInputEvent, ClientSurfaceSize, PaneSurfaceFrame, ServerMessage,
};

fn wait_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if text.ends_with('\n') {
                return text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "owned process did not finish {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
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
    assert_ne!(fields[4], "0", "owned process lost controlling PTY");
    (
        fields[19].into(),
        fields[4].into(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}

fn surface(
    stream: &mut UnixStream,
    stage: &str,
    ready: impl Fn(&PaneSurfaceFrame) -> bool,
) -> PaneSurfaceFrame {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        assert!(
            Instant::now() < deadline,
            "{stage}: native surface did not arrive"
        );
        let message: ServerMessage =
            protocol::read_message(stream, protocol::MAX_GRAPHICS_FRAME_SIZE)
                .unwrap_or_else(|error| panic!("{stage}: {error}"));
        if let ServerMessage::PaneSurface(frame) = message {
            if ready(&frame) {
                return frame;
            }
        }
    }
}

fn attach(core: &Core) -> UnixStream {
    let mut stream = UnixStream::connect(core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    )))
    .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let hello = EndpointClientHello {
        generation: 1,
        cell_width_px: 0,
        cell_height_px: 0,
        surface_size: ClientSurfaceSize { cols: 80, rows: 24 },
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: true,
        surface_reuse: false,
        surface_delta: false,
        surface_scroll: false,
        session_switch: false,
        tab_transfer: false,
        snapshot_codecs: vec![SNAPSHOT_CODEC_V1.into()],
        surface_codecs: vec![SURFACE_CODEC_V1.into()],
        input_codecs: vec![INPUT_CODEC_V1.into()],
        blob_codecs: vec![BLOB_CODEC_V1.into()],
    };
    protocol::write_message(
        &mut stream,
        &ClientMessage::EndpointControl {
            kind: ENDPOINT_HELLO_KIND.into(),
            data: serde_json::to_string(&hello).unwrap(),
        },
    )
    .unwrap();
    stream
}

fn popup_text(frame: &PaneSurfaceFrame) -> String {
    frame
        .popup
        .as_ref()
        .map(|popup| {
            popup
                .frame
                .cells
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect()
        })
        .unwrap_or_default()
}

struct Popup {
    terminal: String,
    pid: String,
    process: (String, String, PathBuf, PathBuf),
    marker: String,
    reply: PathBuf,
}
impl Popup {
    fn open(core: &Core, stream: &mut UnixStream, marker: &str) -> Self {
        let plugin = core.home.join(format!("{marker}-plugin"));
        std::fs::create_dir(&plugin).unwrap();
        let pid_file = core.home.join(format!("{marker}-popup.pid"));
        let reply = core.home.join(format!("{marker}-popup.reply"));
        let plugin_id = format!("example.{marker}");
        std::fs::write(plugin.join("trailboss-plugin.toml"), format!(r#"
id = "{plugin_id}"
name = "Owned popup"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux"]
[[panes]]
id = "live"
title = "Owned {marker} popup"
placement = "popup"
command = ["sh", "-c", '''popup_local={marker}; printf '%s\n' "$$" > '{}'; printf 'READY-{marker}\n'; while IFS= read -r line; do printf '%s %s %s\n' "$$" "$popup_local" "$line" > '{}'; printf 'REPLY-{marker}-%s\n' "$line"; done''']
"#, pid_file.display(), reply.display())).unwrap();
        core.request("plugin.link", json!({"path":plugin,"enabled":true}));
        core.request(
            "plugin.pane.open",
            json!({"plugin_id":plugin_id,"entrypoint":"live","focus":true}),
        );
        let opened = surface(stream, "owned popup opens", |frame| {
            popup_text(frame).contains(&format!("READY-{marker}"))
        });
        let terminal = opened.popup.unwrap().terminal_id;
        let pid = wait_file(&pid_file).trim().to_owned();
        let process = identity(&pid);
        Self {
            terminal,
            pid,
            process,
            marker: marker.into(),
            reply,
        }
    }

    fn assert_input(&self, stream: &mut UnixStream, stage: &str) {
        protocol::write_message(
            stream,
            &ClientMessage::ClientShellPopupInput {
                terminal_id: self.terminal.clone(),
                events: vec![ClientPaneInputEvent::TextCommit(format!("{stage}\n"))],
            },
        )
        .unwrap();
        let frame = surface(stream, stage, |frame| {
            popup_text(frame).contains(&format!("REPLY-{}-{stage}", self.marker))
        });
        assert_eq!(
            frame.popup.unwrap().terminal_id,
            self.terminal,
            "popup runtime replaced"
        );
        assert_eq!(
            wait_file(&self.reply),
            format!("{} {} {stage}\n", self.pid, self.marker)
        );
        assert_eq!(
            identity(&self.pid),
            self.process,
            "popup PID/start/PTY changed"
        );
    }
}

fn shell(core: &Core, marker: &str) -> (Value, String, (String, String, PathBuf, PathBuf)) {
    let workspace = core.request(
        "workspace.create",
        json!({"label":marker,"cwd":core.home,"focus":true}),
    );
    let mut client = core.attach();
    let deadline = Instant::now() + Duration::from_secs(8);
    while client.focused_pane_id.as_deref() != workspace["root_pane"]["pane_id"].as_str() {
        assert!(
            Instant::now() < deadline,
            "native client did not select the original shell"
        );
        client.receive().unwrap();
    }
    client
        .type_text(&format!(
            "shell_local={marker}; printf '%s\\n' \"$$\" > {marker}-shell.pid\n"
        ))
        .unwrap();
    let pid = wait_file(&core.home.join(format!("{marker}-shell.pid")))
        .trim()
        .to_owned();
    let process = identity(&pid);
    (workspace, pid, process)
}

#[test]
fn tab_owned_popup_keeps_original_process_and_owner_across_cores() {
    let alpha = Core::new();
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let (source, source_pid, source_process) = shell(&alpha, "source");
    let (destination, beta_pid, beta_process) = shell(&beta, "beta");
    let mut alpha_client = attach(&alpha);
    let mut beta_client = attach(&beta);
    surface(&mut alpha_client, "source tab", |frame| {
        frame
            .panes
            .iter()
            .any(|pane| pane.pane_id == source["root_pane"]["pane_id"])
    });
    surface(&mut beta_client, "beta tab", |frame| {
        frame
            .panes
            .iter()
            .any(|pane| pane.pane_id == destination["root_pane"]["pane_id"])
    });
    let alpha_popup = Popup::open(&alpha, &mut alpha_client, "source");
    let beta_popup = Popup::open(&beta, &mut beta_client, "beta");
    alpha_popup.assert_input(&mut alpha_client, "before-source");
    beta_popup.assert_input(&mut beta_client, "before-beta");
    let moved = alpha.request("tab.transfer", json!({"tab_id":source["tab"]["tab_id"],"destination_session":beta.name,
        "workspace_id":destination["workspace"]["workspace_id"],"operation_id":"owned-popup-transfer"}));
    beta.request(
        "tab.focus",
        json!({"tab_id":moved["transfer"]["tab"]["tab_id"]}),
    );
    let moved_surface = surface(&mut beta_client, "moved tab owns original popup", |frame| {
        frame
            .panes
            .iter()
            .any(|pane| pane.pane_id == moved["transfer"]["panes"][0]["pane_id"])
    });
    assert_eq!(
        moved_surface
            .popup
            .as_ref()
            .expect("moved tab lost its running popup")
            .terminal_id,
        alpha_popup.terminal
    );
    alpha_popup.assert_input(&mut beta_client, "after-transfer");
    assert_eq!(identity(&source_pid), source_process);
    beta.request("tab.focus", json!({"tab_id":destination["tab"]["tab_id"]}));
    let unrelated = surface(
        &mut beta_client,
        "beta unrelated tab retains popup",
        |frame| {
            frame
                .panes
                .iter()
                .any(|pane| pane.pane_id == destination["root_pane"]["pane_id"])
        },
    );
    assert_eq!(
        unrelated
            .popup
            .as_ref()
            .expect("unrelated beta popup disappeared")
            .terminal_id,
        beta_popup.terminal
    );
    beta_popup.assert_input(&mut beta_client, "after-unrelated");
    beta.request("pane.send_text", json!({"pane_id":destination["root_pane"]["pane_id"],"text":"printf '%s\\n' \"$shell_local\" > beta-shell-after\n"}));
    assert_eq!(wait_file(&beta.home.join("beta-shell-after")), "beta\n");
    assert_eq!(identity(&beta_pid), beta_process);
    beta.request(
        "tab.focus",
        json!({"tab_id":moved["transfer"]["tab"]["tab_id"]}),
    );
    surface(
        &mut beta_client,
        "moved popup still follows its owner",
        |frame| {
            frame
                .popup
                .as_ref()
                .is_some_and(|popup| popup.terminal_id == alpha_popup.terminal)
        },
    );
    alpha_popup.assert_input(&mut beta_client, "after-refocus");
}

#[test]
fn delayed_moved_shell_exit_closes_only_destination_while_source_popup_and_overlay_survive() {
    let alpha = Core::new();
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let (source, moved_pid, moved_process) = shell(&alpha, "delayed-moved");
    let moved_terminal = alpha.request(
        "pane.get",
        json!({"pane_id":source["root_pane"]["pane_id"]}),
    )["pane"]["terminal_id"]
        .clone();
    let (remaining, remaining_pid, remaining_process) = shell(&alpha, "remaining");
    let (destination, beta_pid, beta_process) = shell(&beta, "delayed-beta");
    let plugin = alpha.home.join("delayed-overlay-plugin");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("trailboss-plugin.toml"), format!(r#"
id = "example.delayed-overlay"
name = "Owned delayed overlay"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux"]
[[panes]]
id = "live"
title = "Remaining tab overlay"
placement = "overlay"
command = ["sh", "-c", '''overlay_local=remaining-overlay; printf '%s\n' "$$" > '{}'; while IFS= read -r line; do printf '%s %s %s\n' "$$" "$overlay_local" "$line" > '{}'; done''']
"#, alpha.home.join("remaining-overlay.pid").display(), alpha.home.join("remaining-overlay.reply").display())).unwrap();
    alpha.request("plugin.link", json!({"path":plugin,"enabled":true}));
    let opened = alpha.request(
        "plugin.pane.open",
        json!({"plugin_id":"example.delayed-overlay","entrypoint":"live","focus":true}),
    );
    let overlay = opened["plugin_pane"]["pane"]["pane_id"].clone();
    let overlay_terminal =
        alpha.request("pane.get", json!({"pane_id":overlay}))["pane"]["terminal_id"].clone();
    let overlay_pid = wait_file(&alpha.home.join("remaining-overlay.pid"))
        .trim()
        .to_owned();
    let overlay_process = identity(&overlay_pid);
    let mut alpha_client = attach(&alpha);
    surface(
        &mut alpha_client,
        "remaining source tab with overlay",
        |frame| frame.panes.iter().any(|pane| pane.pane_id == overlay),
    );
    let popup = Popup::open(&alpha, &mut alpha_client, "remaining");
    popup.assert_input(&mut alpha_client, "before-delayed-exit");
    let moved = alpha.request("tab.transfer", json!({
        "tab_id":source["tab"]["tab_id"],"destination_session":beta.name,
        "workspace_id":destination["workspace"]["workspace_id"],"operation_id":"delayed-source-waiter"
    }));
    let moved_pane = moved["transfer"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| pane["previous_pane_id"] == source["root_pane"]["pane_id"])
        .unwrap()["pane_id"]
        .clone();
    assert_eq!(identity(&moved_pid), moved_process);
    let snapshot = beta.request("session.snapshot", json!({}));
    assert!(snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pane| pane["terminal_id"] == moved_terminal));
    beta.request(
        "pane.send_text",
        json!({"pane_id":moved_pane,
        "text":"printf '%s\\n' \"$shell_local\" > delayed-before-exit; exit\n"}),
    );
    assert_eq!(
        wait_file(&alpha.home.join("delayed-before-exit")),
        "delayed-moved\n"
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = beta.request("session.snapshot", json!({}));
        let gone = !snapshot["snapshot"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pane| pane["terminal_id"] == moved_terminal);
        if gone && std::fs::read_link(format!("/proc/{moved_pid}/exe")).is_err() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "original moved shell exit did not close its destination runtime"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let source_snapshot = alpha.request("session.snapshot", json!({}));
    assert!(
        source_snapshot["snapshot"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pane| pane["terminal_id"] == overlay_terminal),
        "stale source waiter removed the unrelated overlay"
    );
    let overlay_layout = alpha.request("pane.layout", json!({"pane_id":overlay}))["layout"].clone();
    assert_eq!(overlay_layout["panes"].as_array().unwrap().len(), 2);
    alpha.request(
        "pane.send_text",
        json!({"pane_id":overlay,"text":"after-delayed-exit\n"}),
    );
    assert_eq!(
        wait_file(&alpha.home.join("remaining-overlay.reply")),
        format!("{overlay_pid} remaining-overlay after-delayed-exit\n")
    );
    assert_eq!(identity(&overlay_pid), overlay_process);
    popup.assert_input(&mut alpha_client, "after-delayed-exit");
    alpha.request(
        "pane.send_text",
        json!({"pane_id":remaining["root_pane"]["pane_id"],
        "text":"printf '%s\\n' \"$shell_local\" > remaining-after-delayed-exit\n"}),
    );
    assert_eq!(
        wait_file(&alpha.home.join("remaining-after-delayed-exit")),
        "remaining\n"
    );
    assert_eq!(identity(&remaining_pid), remaining_process);
    beta.request(
        "pane.send_text",
        json!({"pane_id":destination["root_pane"]["pane_id"],
        "text":"printf '%s\\n' \"$shell_local\" > beta-after-delayed-exit\n"}),
    );
    assert_eq!(
        wait_file(&beta.home.join("beta-after-delayed-exit")),
        "delayed-beta\n"
    );
    assert_eq!(identity(&beta_pid), beta_process);
}
