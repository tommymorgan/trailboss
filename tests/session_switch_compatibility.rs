#![cfg(target_os = "linux")]

#[path = "support/console.rs"]
mod console;
pub mod support;

use console::Console;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use support::Core;

fn retained_native() -> PathBuf {
    let root = std::env::var_os("TRAILBOSS_COMPAT_BASELINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").expect("HOME for retained release"))
                .join(".local/lib/trailboss-tabs-7c713486")
        });
    let executable = root.join("trailboss");
    let inventory: Value =
        serde_json::from_str(include_str!("fixtures/release-de20c469/inventory.json")).unwrap();
    let digest = Command::new("sha256sum").arg(&executable).output().unwrap();
    assert!(
        digest.status.success(),
        "retained native digest: {:?}",
        digest
    );
    assert_eq!(
        String::from_utf8(digest.stdout)
            .unwrap()
            .split_whitespace()
            .next(),
        inventory["binary_sha256"].as_str(),
        "the older native must be the actual retained release binary"
    );
    executable
}

fn older_console(core: &Core, executable: &Path) -> Console {
    let mut command = portable_pty::CommandBuilder::new(executable);
    command.args(["--session", &core.name]);
    for key in support::HOMES {
        command.env(key, &core.home);
    }
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
        command.env_remove(key);
    }
    command.env("TRAILBOSS_CONFIG_PATH", core.home.join("config.toml"));
    command.env("TERM", "xterm-256color");
    command.env("SHELL", "/bin/sh");
    command.env_remove("NO_COLOR");
    Console::from_command(command)
}

fn process_identity(pid: u32) -> (String, String, PathBuf, PathBuf) {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    (
        fields[19].to_owned(),
        fields[4].to_owned(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}

fn marker_shell(core: &Core, pane: &str, marker: &str) -> (u32, u32) {
    core.request("pane.send_text", json!({
        "pane_id": pane,
        "text": format!("compat_marker={marker}; sleep 300 & compat_job=$!; printf '%s %s\\n' \"$$\" \"$compat_job\" > {marker}.state; printf '%s-original-shell\\n' \"$compat_marker\"\n")
    }));
    let path = core.home.join(format!("{marker}.state"));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if text.ends_with('\n') {
                let pids: Vec<_> = text
                    .split_whitespace()
                    .map(|pid| pid.parse().unwrap())
                    .collect();
                assert_eq!(pids.len(), 2, "shell and background-job witnesses");
                return (pids[0], pids[1]);
            }
        }
        assert!(
            Instant::now() < deadline,
            "shell witness missing: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn current_producers(core: &Core) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins");
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        core.request(
            "plugin.link",
            json!({"path":root.join(plugin), "enabled":true}),
        );
    }
    let registry = core.request("plugin.list", json!({}));
    for plugin in ["stock-ui", "sidebar-menus", "command-palette"] {
        let id = format!("tommymorgan.{plugin}");
        let installed = registry["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["plugin_id"] == id)
            .unwrap_or_else(|| panic!("current producer missing: {id}; {registry}"));
        assert_eq!(installed["enabled"], true, "{id} disabled");
        assert_eq!(
            std::fs::canonicalize(installed["plugin_root"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(root.join(plugin)).unwrap(),
            "old startup must not leave the retained producer installed: {id}"
        );
    }
}

fn session_menu(console: &mut Console) -> Vec<String> {
    console.text("\x1b[<35;20;24M");
    let rows = console.wait(|rows| rows[23].contains('⋯'));
    console.click(
        rows[23]
            .chars()
            .position(|character| character == '⋯')
            .unwrap(),
        23,
    );
    console.wait(|rows| rows.iter().any(|row| row.contains("Detach from session")))
}

fn click_label(console: &mut Console, rows: &[String], label: &str) {
    let y = rows.iter().position(|row| row.contains(label)).unwrap();
    let x = rows[y][..rows[y].find(label).unwrap()].chars().count();
    console.click(x, y);
}

fn palette_has(console: &mut Console, query: &str) -> Vec<String> {
    console.text("\x1bOP");
    console.text(query);
    console.wait(|rows| rows.iter().filter(|row| row.contains(query)).count() >= 2)
}

fn dismiss(console: &mut Console) {
    console.text("\x1b");
    console.wait(|rows| {
        !rows
            .iter()
            .any(|row| row.contains("Enter select") || row.contains("SESSION"))
    });
}

fn old_commands_stay_usable_without_switch(console: &mut Console) {
    let rows = session_menu(console);
    assert!(rows.iter().any(|row| row.contains("New space")));
    assert!(
        !rows.iter().any(|row| row.contains("Switch session")),
        "{rows:?}"
    );
    dismiss(console);
    palette_has(console, "Detach from session");
    dismiss(console);
    console.text("\x1bOP");
    console.text("Switch session");
    let rows = console.wait(|rows| rows.iter().any(|row| row.contains("No matches")));
    assert!(
        rows.iter().any(|row| row.contains("Switch session")),
        "query never reached palette"
    );
    dismiss(console);
}

fn input_reaches_original_shell(console: &mut Console, marker: &str, shell: u32, suffix: &str) {
    console.text(&format!(
        "printf '%s-{suffix}-%s\\n' \"$compat_marker\" \"$$\"\r"
    ));
    let expected = format!("{marker}-{suffix}-{shell}");
    console.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
}

fn detach(console: &mut Console) {
    let rows = session_menu(console);
    click_label(console, &rows, "Detach from session");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = console.child.try_wait().unwrap() {
            assert!(status.success(), "native detach: {status:?}");
            return;
        }
        assert!(Instant::now() < deadline, "native did not detach");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Living scenario: OlderClientsDoNotExposeUnsupportedSessionCommands.
#[test]
fn retained_native_has_no_switch_but_keeps_input_tab_navigation_and_detach() {
    let baseline = retained_native();
    let mut core = Core::new();
    let core_identity = process_identity(core.child.id());
    let snapshot = core.request("session.snapshot", json!({}));
    let first = snapshot["snapshot"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let workspace = snapshot["snapshot"]["workspaces"][0]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (first_shell, first_job) = marker_shell(&core, &first, "compat-first");
    let first_identity = process_identity(first_shell);
    let first_job_identity = process_identity(first_job);
    assert_ne!(
        first_identity.1, "0",
        "original shell has no controlling PTY"
    );

    // Both executables run unchanged. Start the retained native first because its
    // startup links its own stock producers into this shared Core registry.
    let mut old = older_console(&core, &baseline);
    old.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("compat-first-original-shell"))
    });
    let mut current = Console::new(&core);
    current.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("compat-first-original-shell"))
    });
    current_producers(&core);

    let rows = session_menu(&mut current);
    assert!(
        rows.iter().any(|row| row.contains("Switch session")),
        "{rows:?}"
    );
    click_label(&mut current, &rows, "Switch session");
    current.wait(|rows| rows.iter().any(|row| row.contains(&core.name)));
    current.text(&format!("{}\r", core.name));
    input_reaches_original_shell(&mut current, "compat-first", first_shell, "capable-menu");
    palette_has(&mut current, "Switch session");
    current.text("\r");
    current.wait(|rows| rows.iter().any(|row| row.contains(&core.name)));
    current.text(&format!("{}\r", core.name));
    input_reaches_original_shell(&mut current, "compat-first", first_shell, "capable-palette");
    old_commands_stay_usable_without_switch(&mut old);
    input_reaches_original_shell(&mut old, "compat-first", first_shell, "legacy-input");

    // Refresh all declarations after both handshakes. The second pass proves the
    // render loop keeps capability gating, not just the initial welcome seed.
    let second = core.request(
        "tab.create",
        json!({
            "workspace_id":workspace, "label":"Compat second", "cwd":core.home, "focus":false
        }),
    );
    let second_pane = second["root_pane"]["pane_id"].as_str().unwrap().to_owned();
    let (second_shell, second_job) = marker_shell(&core, &second_pane, "compat-second");
    let second_identity = process_identity(second_shell);
    let second_job_identity = process_identity(second_job);
    assert_ne!(
        second_identity.1, "0",
        "second original shell has no controlling PTY"
    );
    let rows = old.wait(|rows| rows.iter().any(|row| row.contains("Compat second")));
    click_label(&mut old, &rows, "Compat second");
    old.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("compat-second-original-shell"))
    });
    input_reaches_original_shell(&mut old, "compat-second", second_shell, "legacy-navigation");
    old_commands_stay_usable_without_switch(&mut old);
    detach(&mut old);
    input_reaches_original_shell(&mut current, "compat-first", first_shell, "upgraded-input");
    detach(&mut current);

    assert!(core.child.try_wait().unwrap().is_none());
    assert_eq!(process_identity(core.child.id()), core_identity);
    assert_eq!(process_identity(first_shell), first_identity);
    assert_eq!(process_identity(first_job), first_job_identity);
    assert_eq!(process_identity(second_shell), second_identity);
    assert_eq!(process_identity(second_job), second_job_identity);
    for pane in [first, second_pane] {
        core.request(
            "pane.send_text",
            json!({"pane_id":pane, "text":"kill \"$compat_job\"\n"}),
        );
    }
}

#[test]
fn a_retained_native_attaching_later_cannot_remove_the_current_switch_command() {
    let baseline = retained_native();
    let mut core = Core::new();
    let snapshot = core.request("session.snapshot", json!({}));
    let pane = snapshot["snapshot"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (shell, job) = marker_shell(&core, &pane, "compat-late");
    let shell_identity = process_identity(shell);
    let job_identity = process_identity(job);
    let mut current = Console::new(&core);
    current.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("compat-late-original-shell"))
    });
    let rows = session_menu(&mut current);
    assert!(
        rows.iter().any(|row| row.contains("Switch session")),
        "{rows:?}"
    );
    dismiss(&mut current);

    let mut old = older_console(&core, &baseline);
    old.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("compat-late-original-shell"))
    });
    old_commands_stay_usable_without_switch(&mut old);
    // Do not relink current producers: this is the retained client's actual
    // startup, including its normal installation of its own stock producers.
    core.request(
        "pane.send_text",
        json!({
            "pane_id":pane,
            "text":"printf 'LATE_CLIENT_%s\\n' \"$compat_marker\"\n"
        }),
    );
    current.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("LATE_CLIENT_compat-late"))
    });
    let rows = session_menu(&mut current);
    assert!(
        rows.iter().any(|row| row.contains("Switch session")),
        "retained startup removed the capable window's command: {rows:?}",
    );
    click_label(&mut current, &rows, "Switch session");
    current.wait(|rows| rows.iter().any(|row| row.contains(&core.name)));
    current.text(&format!("{}\r", core.name));
    input_reaches_original_shell(&mut current, "compat-late", shell, "after-legacy-startup");
    palette_has(&mut current, "Switch session");
    current.text("\r");
    current.wait(|rows| rows.iter().any(|row| row.contains(&core.name)));
    current.text(&format!("{}\r", core.name));
    input_reaches_original_shell(&mut old, "compat-late", shell, "legacy-still-live");
    detach(&mut old);
    detach(&mut current);
    assert!(core.child.try_wait().unwrap().is_none());
    assert_eq!(process_identity(shell), shell_identity);
    assert_eq!(process_identity(job), job_identity);
    core.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":"kill \"$compat_job\"\n"}),
    );
}
