use super::{console, support};

use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    sync::mpsc,
    time::Duration,
};

const OWNER: &str = "fixture.argument-lifecycle";
const ORIGINAL: &str = "42  \"review note\" café";
const CORRECTED: &str = "43  \"corrected note\" café";

fn owned_tabs(core: &support::Core) -> Vec<Value> {
    core.request("session.snapshot", json!({}))["snapshot"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tab| {
            tab["label"]
                .as_str()
                .is_some_and(|label| label.starts_with("Attempt[") || label.starts_with("Saved["))
        })
        .cloned()
        .collect()
}

fn owner_logs(core: &support::Core) -> Vec<Value> {
    core.request("plugin.log.list", json!({"plugin_id":OWNER,"limit":20}))["logs"]
        .as_array()
        .unwrap()
        .clone()
}

fn open_arguments(console: &mut console::Console) {
    console.text("\x1bOP");
    console.text("Lifecycle save");
    console.wait(|screen| {
        screen.iter().any(|line| line.contains("Commands"))
            && screen.iter().any(|line| line.contains("Lifecycle save"))
    });
    console.text("\t");
    console.wait(|screen| {
        !screen.iter().any(|line| line.contains("Commands"))
            && screen.iter().any(|line| line.contains("<review> [note]"))
    });
}

fn lifecycle_owner() -> (
    support::Core,
    Value,
    mpsc::Receiver<UnixStream>,
    std::thread::JoinHandle<()>,
) {
    let core = support::Core::new();
    let captured = core.request(
        "workspace.create",
        json!({"label":"Lifecycle","cwd":core.home,"focus":true}),
    );
    let owner = core.home.join("argument-lifecycle-owner");
    std::fs::create_dir_all(&owner).unwrap();
    let listener = UnixListener::bind(owner.join("gate.sock")).unwrap();
    // The first owner cannot finish until the parent releases this socket.
    // A bounded receive observes the actual owner, not a guessed delay.
    let (gate_tx, gate_rx) = mpsc::channel();
    let gate_thread = std::thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        let _ = gate_tx.send(connection);
    });
    std::fs::write(
        owner.join("trailboss-plugin.toml"),
        r#"id = "fixture.argument-lifecycle"
name = "Argument lifecycle owner"
version = "0.1.0"
min_herdr_version = "0.9.0"
[[actions]]
id = "save"
title = "Lifecycle save"
contexts = ["pane"]
argument_input = { kind = "raw_tail" }
argument_hint = "<review> [note]"
command = ["python3", "invoke.py"]
"#,
    )
    .unwrap();
    std::fs::write(
        owner.join("invoke.py"),
        r#"import json, os, socket, sys
from pathlib import Path

tail = json.loads(os.environ["HERDR_PLUGIN_ARGUMENTS_JSON"])
assert isinstance(tail, str)
context = json.loads(os.environ["HERDR_PLUGIN_CONTEXT_JSON"])

def create(label):
    request = {"id": "lifecycle-owner", "method": "tab.create", "params": {
        "workspace_id": context["workspace_id"], "label": label, "focus": False}}
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.connect(os.environ["HERDR_SOCKET_PATH"])
        connection.sendall((json.dumps(request) + "\n").encode())
        response = json.loads(connection.makefile("rb").readline())
        assert "error" not in response, response

# Every invocation has an observable, real Core-owned consequence, including
# failures. A duplicate cannot hide behind an equal receipt or a final success.
create("Attempt[" + tail + "]")
if tail != '43  "corrected note" café':
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as gate:
        gate.connect(str(Path(__file__).with_name("gate.sock")))
        gate.sendall(b"waiting\n")
        assert gate.recv(1) == b"!"
    print("Owner rejected review 42; correct the review number", file=sys.stderr)
    sys.exit(23)
create("Saved[" + tail + "]")
"#,
    )
    .unwrap();
    core.request("plugin.link", json!({"path":owner,"enabled":true}));
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        core.request(
            "plugin.link",
            json!({"path":format!("{}/plugins/{plugin}",env!("CARGO_MANIFEST_DIR")),"enabled":true}),
        );
    }
    (core, captured, gate_rx, gate_thread)
}

#[test]
fn asynchronous_owner_failure_preserves_exact_draft_and_only_explicit_corrected_retry_succeeds() {
    let (core, captured, gate_rx, gate_thread) = lifecycle_owner();
    let mut console = console::Console::new(&core);
    console.wait(|screen| screen.iter().any(|line| line.contains("Lifecycle")));
    open_arguments(&mut console);
    console.text(ORIGINAL);
    console.wait(|screen| screen.iter().any(|line| line.contains(ORIGINAL)));
    assert!(owned_tabs(&core).is_empty(), "editing invoked the owner");
    console.text("\r");

    let mut gate: UnixStream = gate_rx
        .recv_timeout(Duration::from_secs(8))
        .expect("submitted native arguments did not reach the gated owner");
    gate_thread.join().unwrap();
    gate.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    let mut waiting = [0; 8];
    gate.read_exact(&mut waiting).unwrap();
    assert_eq!(&waiting, b"waiting\n");
    let logs = owner_logs(&core);
    assert_eq!(logs.len(), 1, "one submission must start one owner");
    assert_eq!(logs[0]["status"], "running");
    let log_id = logs[0]["log_id"].clone();
    let attempts = owned_tabs(&core);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["label"], format!("Attempt[{ORIGINAL}]"));
    assert_eq!(
        attempts[0]["workspace_id"],
        captured["workspace"]["workspace_id"]
    );

    // The later Core repaint is a visible observation barrier while the owner
    // is still blocked. API acceptance must not dismiss or erase the editor.
    core.request(
        "workspace.rename",
        json!({"workspace_id":captured["workspace"]["workspace_id"],"label":"Blocked"}),
    );
    let screen = console.wait(|screen| screen.iter().any(|line| line.contains("Blocked")));
    assert!(
        screen.iter().any(|line| line.contains(ORIGINAL)),
        "running owner was treated as completed; exact draft disappeared: {screen:?}"
    );
    console.text("\r\r\r");
    console.wait(|screen| screen.iter().any(|line| line.contains(ORIGINAL)));
    assert_eq!(
        owned_tabs(&core),
        attempts,
        "pending Enter duplicated owner operations"
    );
    assert_eq!(
        owner_logs(&core).len(),
        1,
        "pending Enter started another owner"
    );

    gate.write_all(b"!").unwrap();
    drop(gate);
    // Polling the public log inside the existing bounded console observation
    // waits for terminal failure; the gate, not timing, controls completion.
    console.wait(|_| {
        owner_logs(&core).iter().any(|log| {
            log["log_id"] == log_id && log["status"] == "failed" && log["exit_code"] == 23
        })
    });
    console.wait(|screen| {
        screen.iter().any(|line| line.contains(ORIGINAL))
            && screen.iter().any(|line| {
                let lower = line.to_lowercase();
                lower.contains("failed") || lower.contains("rejected") || lower.contains("exit")
            })
    });
    assert_eq!(
        owned_tabs(&core),
        attempts,
        "failure retried without explicit input"
    );
    assert_eq!(owner_logs(&core).len(), 1);

    // Edit the retained draft through the actual native input path. No search,
    // fresh preparation, or direct API invocation substitutes for explicit retry.
    console.text(&"\x7f".repeat(ORIGINAL.chars().count()));
    console.text(CORRECTED);
    console.wait(|screen| screen.iter().any(|line| line.contains(CORRECTED)));
    assert_eq!(
        owned_tabs(&core),
        attempts,
        "editing a failed draft retried it"
    );
    console.text("\r");
    console.wait(|_| {
        let logs = owner_logs(&core);
        logs.len() == 2 && logs.iter().any(|log| log["status"] == "succeeded")
    });
    console.wait(|screen| !screen.iter().any(|line| line.contains("<review> [note]")));
    let tabs = owned_tabs(&core);
    let mut labels: Vec<_> = tabs
        .iter()
        .map(|tab| tab["label"].as_str().unwrap())
        .collect();
    labels.sort_unstable();
    let mut expected = vec![
        format!("Attempt[{ORIGINAL}]"),
        format!("Attempt[{CORRECTED}]"),
        format!("Saved[{CORRECTED}]"),
    ];
    expected.sort_unstable();
    assert_eq!(labels, expected);
    for tab in tabs {
        assert_eq!(tab["workspace_id"], captured["workspace"]["workspace_id"]);
    }
    let logs = owner_logs(&core);
    assert_eq!(
        logs.len(),
        2,
        "only explicit corrected retry may invoke again"
    );
    assert_eq!(
        logs.iter().filter(|log| log["status"] == "failed").count(),
        1
    );
    assert_eq!(
        logs.iter()
            .filter(|log| log["status"] == "succeeded")
            .count(),
        1
    );
}

fn await_owner_failure(core: &support::Core, log_id: &Value) {
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let logs = owner_logs(core);
        if logs.iter().any(|log| {
            log["log_id"] == *log_id && log["status"] == "failed" && log["exit_code"] == 23
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "owner did not fail: {logs:?}"
        );
        // Condition-based public API polling, not a delay used to infer failure.
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn dismissing_dispatched_owner_keeps_its_operation_and_late_failure_cannot_replace_a_new_draft() {
    let (core, captured, gate_rx, gate_thread) = lifecycle_owner();
    let mut console = console::Console::new(&core);
    console.wait(|screen| screen.iter().any(|line| line.contains("Lifecycle")));
    open_arguments(&mut console);
    console.text(ORIGINAL);
    console.text("\r");
    let mut gate = gate_rx.recv_timeout(Duration::from_secs(8)).unwrap();
    gate_thread.join().unwrap();
    gate.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    let mut waiting = [0; 8];
    gate.read_exact(&mut waiting).unwrap();
    assert_eq!(&waiting, b"waiting\n");
    let log_id = owner_logs(&core)[0]["log_id"].clone();
    console.text("\x1b");
    console.wait(|screen| !screen.iter().any(|line| line.contains("<review> [note]")));
    assert_eq!(
        owner_logs(&core)[0]["status"],
        "running",
        "dismissing local input must not claim cancellation of dispatched work"
    );
    open_arguments(&mut console);
    console.text(CORRECTED);
    console.wait(|screen| screen.iter().any(|line| line.contains(CORRECTED)));
    assert_eq!(owned_tabs(&core).len(), 1, "new draft invoked before Enter");
    gate.write_all(b"!").unwrap();
    drop(gate);
    await_owner_failure(&core, &log_id);
    core.request(
        "workspace.rename",
        json!({"workspace_id":captured["workspace"]["workspace_id"],"label":"Late"}),
    );
    let screen = console.wait(|screen| screen.iter().any(|line| line.contains("Late")));
    assert!(
        screen.iter().any(|line| line.contains(CORRECTED)),
        "late completion cleared or restored a different draft: {screen:?}"
    );
    assert!(
        !screen
            .iter()
            .any(|line| line.contains("Owner failed") || line.contains("Owner rejected")),
        "dismissed owner's failure leaked into the new draft: {screen:?}"
    );
    assert_eq!(owned_tabs(&core).len(), 1);
    console.text("\r");
    console.wait(|_| {
        owner_logs(&core)
            .iter()
            .any(|log| log["status"] == "succeeded")
    });
    console.wait(|screen| !screen.iter().any(|line| line.contains("<review> [note]")));
    let tabs = owned_tabs(&core);
    assert_eq!(
        tabs.len(),
        3,
        "dismissal retried or lost an owner operation"
    );
    for label in [
        format!("Attempt[{ORIGINAL}]"),
        format!("Attempt[{CORRECTED}]"),
        format!("Saved[{CORRECTED}]"),
    ] {
        assert_eq!(tabs.iter().filter(|tab| tab["label"] == label).count(), 1);
    }
    assert_eq!(owner_logs(&core).len(), 2);
}

// The imported process is not support::Core.child. Always stop the currently
// serving owner of this isolated session before Core removes its home.
struct StopHandoffOwner(std::path::PathBuf);
impl Drop for StopHandoffOwner {
    fn drop(&mut self) {
        let api = trailboss_runtime::api_client::ApiClient::for_target(
            trailboss_runtime::api_client::ConnectionTarget::SocketPath(self.0.clone()),
        );
        let request = serde_json::from_value::<trailboss_runtime::api_schema::Request>(
            json!({"id":"stop-argument-handoff","method":"server.stop","params":{}}),
        )
        .unwrap();
        let _ = api.request_value_with_timeout(&request, Duration::from_secs(2));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while self.0.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn changed_core_boot_rejects_an_accepted_nonempty_draft_without_invoking_any_owner() {
    use crossterm::event::KeyCode;
    use trailboss::{core::Core as NativeCore, terminal::menus::Response, terminal::FrameRenderer};

    let (core, captured, gate_rx, gate_thread) = lifecycle_owner();
    let unused_gate =
        UnixStream::connect(core.home.join("argument-lifecycle-owner/gate.sock")).unwrap();
    drop(gate_rx.recv_timeout(Duration::from_secs(8)).unwrap());
    gate_thread.join().unwrap();
    drop(unused_gate);
    let _stop_owner = StopHandoffOwner(
        core.home
            .join(format!("trailboss/sessions/{}/trailboss.sock", core.name)),
    );
    let target = super::context(&captured);
    let mut client = core.attach();
    let mut native = NativeCore::default();
    native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        native.command_ui = client.command_ui.clone();
        if client.declarations.as_ref().is_some_and(|declarations| {
            super::available(&native, declarations)
                .iter()
                .any(|action| action.0 == OWNER && action.1 == "save")
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "owner did not publish"
        );
        client.receive().unwrap();
    }
    let declarations = client.declarations.as_ref().unwrap();
    let mut renderer = FrameRenderer::default();
    assert!(renderer
        .controls
        .open_palette(&native, declarations, Some(&target)));
    for character in "Lifecycle save".chars() {
        super::key(
            &mut renderer,
            &native,
            declarations,
            KeyCode::Char(character),
        );
    }
    let Response::Arguments(action, None) =
        super::key(&mut renderer, &native, declarations, KeyCode::Tab)
    else {
        panic!("real issued command did not open argument entry");
    };
    assert_eq!(action.context, target);
    native.argument_editing = true;
    native.argument_context = Some(action.context);
    native.text(ORIGINAL);
    assert!(super::rows(&mut renderer, &native, declarations)
        .iter()
        .any(|line| line.contains(ORIGINAL)));
    let before_boot = client.boot_id.clone();
    drop(client);
    core.request(
        "server.live_handoff",
        json!({"import_exe":support::executable()}),
    );
    let mut replacement = core.attach();
    assert_ne!(before_boot, replacement.boot_id);
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        native.command_ui = replacement.command_ui.clone();
        if replacement
            .declarations
            .as_ref()
            .is_some_and(|declarations| {
                super::available(&native, declarations)
                    .iter()
                    .any(|action| action.0 == OWNER && action.1 == "save")
            })
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "replacement owner did not publish"
        );
        replacement.receive().unwrap();
    }
    let declarations = replacement.declarations.as_ref().unwrap();
    for _ in 0..3 {
        match super::key(&mut renderer, &native, declarations, KeyCode::Enter) {
            Response::Invoke(cancel) => {
                assert_eq!(
                    cancel.action, "host:cancel",
                    "boot change selected another action"
                );
                native.invoke(
                    &cancel.action,
                    &cancel.context,
                    replacement.snapshot.as_ref(),
                );
            }
            Response::Arguments(..) => panic!("old draft survived a different real Core boot"),
            _ => {}
        }
    }
    assert!(owner_logs(&core).is_empty(), "stale Enter reached an owner");
    assert!(
        owned_tabs(&core).is_empty(),
        "stale Enter operated on a replacement target"
    );
}

fn acceptance_chunks(
    client: &mut trailboss::Client,
    request_id: &str,
) -> Vec<trailboss_runtime::protocol::ServerMessage> {
    use trailboss_runtime::protocol::ServerMessage;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut chunks = Vec::new();
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "real Core response {request_id} did not arrive"
        );
        client.receive().unwrap();
        if let Some(message) = client.endpoint_response.take() {
            if let ServerMessage::ClientShellEndpointResponseChunk {
                request_id: received,
                final_chunk,
                ..
            } = &message
            {
                if received == request_id {
                    let done = *final_chunk;
                    chunks.push(message);
                    if done {
                        return chunks;
                    }
                }
            }
        }
    }
}

#[test]
fn unknown_transport_outcome_preserves_visible_draft_blocks_enter_and_ignores_late_responses() {
    use crossterm::event::KeyCode;
    use trailboss::{core::Core as NativeCore, terminal::menus::Response, terminal::FrameRenderer};

    // Drive the public deadline with an explicit Instant. The protocol replies
    // are real Core replies deliberately withheld by this consumer, not
    // invented acceptances or a five-second wall-clock guess.
    for failure in [
        "acceptance-timeout",
        "observation-timeout",
        "observation-failed",
    ] {
        let (core, captured, gate_rx, gate_thread) = lifecycle_owner();
        let target = super::context(&captured);
        let mut client = core.attach();
        let mut native = NativeCore::default();
        native.update_session(target.workspace_id.clone(), target.tab_id.clone(), true);
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            native.command_ui = client.command_ui.clone();
            if client.declarations.as_ref().is_some_and(|declarations| {
                native.fragment(declarations, "command-palette").is_some()
                    && super::available(&native, declarations)
                        .iter()
                        .any(|action| action.0 == OWNER && action.1 == "save")
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "real lifecycle command did not publish"
            );
            client.receive().unwrap();
        }
        let declarations = client.declarations.as_ref().unwrap().clone();
        let mut renderer = FrameRenderer::default();
        let open = |renderer: &mut FrameRenderer, native: &mut NativeCore| {
            assert!(renderer
                .controls
                .open_palette(native, &declarations, Some(&target)));
            for character in "Lifecycle save".chars() {
                super::key(renderer, native, &declarations, KeyCode::Char(character));
            }
            let Response::Arguments(action, None) =
                super::key(renderer, native, &declarations, KeyCode::Tab)
            else {
                panic!("issued lifecycle command did not accept native arguments");
            };
            assert_eq!(action.plugin, OWNER);
            assert_eq!(action.action, "save");
            assert_eq!(action.context, target);
            native.argument_editing = true;
            native.argument_context = Some(action.context);
        };
        open(&mut renderer, &mut native);
        assert!(matches!(
            native.text(ORIGINAL),
            Some(trailboss::core::Effect::Consumed)
        ));
        let Response::Arguments(action, Some(arguments)) =
            super::key(&mut renderer, &native, &declarations, KeyCode::Enter)
        else {
            panic!("accepted native draft did not submit");
        };
        assert_eq!(arguments, ORIGINAL);
        assert_eq!(action.context, target);
        let now = std::time::Instant::now();
        let mut input = client.input().unwrap();
        let invocation = input
            .invoke(
                &client.boot_id,
                &action.plugin,
                &action.action,
                &action.context,
                Some(&arguments),
            )
            .unwrap();
        native.begin_argument_invocation(
            &client.boot_id,
            invocation,
            &action.plugin,
            &action.action,
            now,
        );
        let mut gate = gate_rx.recv_timeout(Duration::from_secs(8)).unwrap();
        gate_thread.join().unwrap();
        gate.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        let mut waiting = [0; 8];
        gate.read_exact(&mut waiting).unwrap();
        assert_eq!(&waiting, b"waiting\n");
        let attempts = owned_tabs(&core);
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0]["label"], format!("Attempt[{ORIGINAL}]"));
        assert_eq!(
            attempts[0]["workspace_id"],
            captured["workspace"]["workspace_id"]
        );
        let log_id = owner_logs(&core)[0]["log_id"].clone();
        let mut late = acceptance_chunks(&mut client, &format!("widget-action-{invocation}"));
        if failure != "acceptance-timeout" {
            for response in &late {
                native.argument_response(response, &client.boot_id, now);
            }
            let params = native
                .argument_log_query(now + Duration::from_secs(1))
                .expect("real acceptance did not schedule owner observation");
            let request = input.request_argument_log(&client.boot_id, params).unwrap();
            native.argument_log_requested(request, now + Duration::from_secs(1));
            late = acceptance_chunks(&mut client, &format!("argument-log-{request}"));
        }
        if failure == "observation-failed" {
            native.argument_observation_failed();
        } else {
            assert!(native
                .argument_log_query(now + Duration::from_secs(7))
                .is_none());
        }
        let screen = super::rows(&mut renderer, &native, &declarations);
        assert!(
            screen.iter().any(|line| line.contains(ORIGINAL)),
            "{failure}: {screen:?}"
        );
        assert!(
            screen
                .iter()
                .any(|line| line.contains("Owner outcome unknown")),
            "{failure}: {screen:?}"
        );
        for _ in 0..3 {
            assert!(
                matches!(
                    super::key(&mut renderer, &native, &declarations, KeyCode::Enter),
                    Response::Consumed
                ),
                "{failure}: unknown outcome permitted unsafe retry"
            );
        }
        assert_eq!(owned_tabs(&core), attempts);
        assert_eq!(owner_logs(&core).len(), 1);
        // Delivering the late real reply cannot turn an unknown outcome into
        // permission to retry, even before the user dismisses the draft.
        for response in &late {
            native.argument_response(response, &client.boot_id, now + Duration::from_secs(8));
        }
        assert!(matches!(
            super::key(&mut renderer, &native, &declarations, KeyCode::Enter),
            Response::Consumed
        ));
        let Response::Invoke(cancel) =
            super::key(&mut renderer, &native, &declarations, KeyCode::Esc)
        else {
            panic!("unknown draft cannot be explicitly dismissed");
        };
        assert!(matches!(
            native.invoke(&cancel.action, &cancel.context, client.snapshot.as_ref()),
            trailboss::core::Effect::Consumed
        ));
        for response in &late {
            native.argument_response(response, &client.boot_id, now + Duration::from_secs(9));
        }
        open(&mut renderer, &mut native);
        assert!(matches!(
            native.text(CORRECTED),
            Some(trailboss::core::Effect::Consumed)
        ));
        for response in &late {
            native.argument_response(response, &client.boot_id, now + Duration::from_secs(10));
        }
        let screen = super::rows(&mut renderer, &native, &declarations);
        assert!(
            screen.iter().any(|line| line.contains(CORRECTED)),
            "{failure}: {screen:?}"
        );
        assert!(
            !screen
                .iter()
                .any(|line| line.contains("Owner outcome unknown")),
            "{failure}: {screen:?}"
        );
        let Response::Arguments(next, Some(next_arguments)) =
            super::key(&mut renderer, &native, &declarations, KeyCode::Enter)
        else {
            panic!("late reply prevented explicit new-draft submission");
        };
        assert_eq!(next_arguments, CORRECTED);
        assert_eq!(next.plugin, OWNER);
        assert_eq!(next.action, "save");
        assert_eq!(next.context, target);
        // Stop at the native submission boundary: do not manufacture a second
        // dispatch to make the preservation assertion look successful.
        gate.write_all(b"!").unwrap();
        drop(gate);
        await_owner_failure(&core, &log_id);
        assert_eq!(owned_tabs(&core), attempts);
        assert_eq!(owner_logs(&core).len(), 1);
    }
}
