#[path = "support/console.rs"]
mod console;
pub mod support;

use console::Console;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use support::{Core, HOMES};

fn default_core() -> Core {
    let home = std::env::temp_dir().join(format!(
        "at-default-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();
    Core::launch(home, "default".into())
}

fn native_default(core: &Core, explicit: bool) -> Console {
    let mut command = portable_pty::CommandBuilder::new(support::executable());
    if explicit {
        command.args(["--session", "default"]);
    }
    for key in HOMES {
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
    for key in ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY"] {
        command.env_remove(key);
    }
    Console::from_command(command)
}

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
        std::thread::sleep(Duration::from_millis(10));
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
            "text": format!("marker={marker}; sleep 120 & switch_job=$!; printf '%s %s %s\\n' \"$$\" \"$switch_job\" \"$(tty)\" > {marker}.state; printf '%s-original-shell\\n' \"$marker\"\n")
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
            boot: snapshot["snapshot"]["boot_id"].clone(),
        }
    }
    fn assert_unchanged(&self, core: &Core) {
        assert_eq!(process_identity(&self.shell), self.shell_identity);
        assert_eq!(process_identity(&self.job), self.job_identity);
        let snapshot = core.request("session.snapshot", json!({}));
        assert_eq!(snapshot["snapshot"]["boot_id"], self.boot);
        assert_eq!(snapshot["snapshot"]["focused_pane_id"], self.pane);
        assert_eq!(snapshot["snapshot"]["panes"].as_array().unwrap().len(), 1);
    }
    fn stop_job(&self, core: &Core) {
        core.request(
            "pane.send_text",
            json!({"pane_id": self.pane, "text": "kill \"$switch_job\"\n"}),
        );
    }
}

fn default_attach_preserves_the_running_shell(explicit: bool) {
    let mut core = default_core();
    let core_pid = core.child.id();
    let original = OriginalShell::seed(&core, "default");
    let mut native = native_default(&core, explicit);
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("default-original-shell"))
    });
    native.text("printf 'attached-%s-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("attached-default-{}", original.shell);
    native.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    original.assert_unchanged(&core);
    assert_eq!(core.child.id(), core_pid);
    assert!(core.child.try_wait().unwrap().is_none());
    original.stop_job(&core);
}

#[test]
fn ordinary_native_default_attach_preserves_the_running_shell() {
    default_attach_preserves_the_running_shell(false);
}

#[test]
fn explicit_native_default_attach_preserves_the_running_shell() {
    default_attach_preserves_the_running_shell(true);
}

#[derive(Clone, Copy)]
enum Entry {
    SessionMenu,
    CommandPalette,
}

fn switch_to(console: &mut Console, entry: Entry, name: &str) {
    match entry {
        Entry::SessionMenu => {
            console.text("\x1b[<35;20;24M");
            let rows = console.wait(|rows| rows[23].contains('⋯'));
            let x = rows[23]
                .chars()
                .position(|character| character == '⋯')
                .unwrap();
            console.click(x, 23);
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
        Entry::CommandPalette => {
            console.text("\x1bOP");
            console.wait(|rows| rows.iter().any(|row| row.contains("Commands")));
            console.text("Switch session");
            console.wait(|rows| rows.iter().any(|row| row.contains("Switch session")));
            console.text("\r");
        }
    }
    console.wait(|rows| rows.iter().any(|row| row.contains(name)));
    console.text(name);
    console.text("\r");
}

fn shell_reply(console: &mut Console, marker: &str, shell: &str, observation: &str) -> String {
    console.text(&format!(
        "printf '{observation}-%s-%s\\n' \"$marker\" \"$$\"\r"
    ));
    let expected = format!("{observation}-{marker}-{shell}");
    console.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    expected
}

fn named_default_round_trip_changes_only_the_invoking_window(entry: Entry) {
    let mut named = Core::new();
    let mut default = Core::launch_with_executable(
        named.home.clone(),
        "default".into(),
        support::executable(),
        false,
    );
    let named_pid = named.child.id();
    let default_pid = default.child.id();
    let named_shell = OriginalShell::seed(&named, "named");
    let default_shell = OriginalShell::seed(&default, "default");
    let mut switching = Console::new(&named);
    let mut named_spectator = Console::new(&named);
    let mut default_spectator = native_default(&default, false);
    switching.wait(|rows| rows.iter().any(|row| row.contains("named-original-shell")));
    named_spectator.wait(|rows| rows.iter().any(|row| row.contains("named-original-shell")));
    default_spectator.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("default-original-shell"))
    });
    let switching_pid = switching.child.process_id();
    let named_spectator_pid = named_spectator.child.process_id();
    let default_spectator_pid = default_spectator.child.process_id();

    switch_to(&mut switching, entry, "default");
    switching.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("default-original-shell"))
    });
    let reply = shell_reply(&mut switching, "default", &default_shell.shell, "switched");
    default_spectator.wait(|rows| rows.iter().any(|row| row.contains(&reply)));
    shell_reply(
        &mut named_spectator,
        "named",
        &named_shell.shell,
        "stayed-named",
    );
    let reply = shell_reply(
        &mut default_spectator,
        "default",
        &default_shell.shell,
        "stayed-default",
    );
    switching.wait(|rows| rows.iter().any(|row| row.contains(&reply)));
    named_shell.assert_unchanged(&named);
    default_shell.assert_unchanged(&default);

    switch_to(&mut switching, entry, &named.name);
    switching.wait(|rows| rows.iter().any(|row| row.contains("named-original-shell")));
    let reply = shell_reply(&mut switching, "named", &named_shell.shell, "returned");
    named_spectator.wait(|rows| rows.iter().any(|row| row.contains(&reply)));
    shell_reply(
        &mut default_spectator,
        "default",
        &default_shell.shell,
        "still-default",
    );
    let reply = shell_reply(
        &mut named_spectator,
        "named",
        &named_shell.shell,
        "still-named",
    );
    switching.wait(|rows| rows.iter().any(|row| row.contains(&reply)));
    named_shell.assert_unchanged(&named);
    default_shell.assert_unchanged(&default);
    assert_eq!(switching.child.process_id(), switching_pid);
    assert_eq!(named_spectator.child.process_id(), named_spectator_pid);
    assert_eq!(default_spectator.child.process_id(), default_spectator_pid);
    assert!(switching.child.try_wait().unwrap().is_none());
    assert!(named_spectator.child.try_wait().unwrap().is_none());
    assert!(default_spectator.child.try_wait().unwrap().is_none());
    assert_eq!(named.child.id(), named_pid);
    assert_eq!(default.child.id(), default_pid);
    assert!(named.child.try_wait().unwrap().is_none());
    assert!(default.child.try_wait().unwrap().is_none());
    named_shell.stop_job(&named);
    default_shell.stop_job(&default);
}

#[test]
fn session_menu_default_round_trip_preserves_both_sessions_and_other_windows() {
    named_default_round_trip_changes_only_the_invoking_window(Entry::SessionMenu);
}

#[test]
fn command_palette_default_round_trip_preserves_both_sessions_and_other_windows() {
    named_default_round_trip_changes_only_the_invoking_window(Entry::CommandPalette);
}
