#[path = "support/console.rs"]
mod console;
pub mod support;

use console::Console;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use support::Core;

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

fn marker_shell(core: &Core, marker: &str) -> (String, String, String) {
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
    let mut fields = state.split_whitespace();
    (
        pane,
        fields.next().unwrap().to_owned(),
        fields.next().unwrap().to_owned(),
    )
}

enum Entry {
    SessionMenu,
    CommandPalette,
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

fn switch_changes_only_the_invoking_native_window(entry: Entry) {
    let mut alpha = Core::new();
    let mut beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let alpha_core = alpha.child.id();
    let beta_core = beta.child.id();
    let (alpha_pane, alpha_shell, alpha_job) = marker_shell(&alpha, "alpha");
    let (beta_pane, beta_shell, beta_job) = marker_shell(&beta, "beta");
    let alpha_shell_identity = process_identity(&alpha_shell);
    let alpha_job_identity = process_identity(&alpha_job);
    let beta_shell_identity = process_identity(&beta_shell);
    let beta_job_identity = process_identity(&beta_job);
    let mut switching = Console::new(&alpha);
    let mut alpha_spectator = Console::new(&alpha);
    let mut beta_spectator = Console::new(&beta);
    switching.wait(|rows| rows.iter().any(|row| row.contains("alpha-original-shell")));
    alpha_spectator.wait(|rows| rows.iter().any(|row| row.contains("alpha-original-shell")));
    beta_spectator.wait(|rows| rows.iter().any(|row| row.contains("beta-original-shell")));

    match entry {
        Entry::SessionMenu => open_session_menu_picker(&mut switching),
        Entry::CommandPalette => {
            switching.text("\x1bOP");
            switching.wait(|rows| rows.iter().any(|row| row.contains("Commands")));
            switching.text("Switch session");
            switching.wait(|rows| rows.iter().any(|row| row.contains("Switch session")));
            switching.text("\r");
        }
    }
    switching.wait(|rows| rows.iter().any(|row| row.contains(&beta.name)));
    switching.text(&beta.name);
    switching.text("\r");
    switching.wait(|rows| rows.iter().any(|row| row.contains("beta-original-shell")));
    switching.text("printf '%s-switch-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("beta-switch-{beta_shell}");
    switching.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    beta_spectator.wait(|rows| rows.iter().any(|row| row.contains(&expected)));

    alpha_spectator.text("printf 'spectator-%s-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("spectator-alpha-{alpha_shell}");
    alpha_spectator.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    beta_spectator.text("printf 'spectator-%s-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("spectator-beta-{beta_shell}");
    beta_spectator.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    switching.wait(|rows| rows.iter().any(|row| row.contains(&expected)));

    assert_eq!(alpha.child.id(), alpha_core);
    assert_eq!(beta.child.id(), beta_core);
    assert!(alpha.child.try_wait().unwrap().is_none());
    assert!(beta.child.try_wait().unwrap().is_none());
    assert_eq!(process_identity(&alpha_shell), alpha_shell_identity);
    assert_eq!(process_identity(&alpha_job), alpha_job_identity);
    assert_eq!(process_identity(&beta_shell), beta_shell_identity);
    assert_eq!(process_identity(&beta_job), beta_job_identity);
    let alpha_snapshot = alpha.request("session.snapshot", json!({}));
    let beta_snapshot = beta.request("session.snapshot", json!({}));
    assert_eq!(alpha_snapshot["snapshot"]["focused_pane_id"], alpha_pane);
    assert_eq!(beta_snapshot["snapshot"]["focused_pane_id"], beta_pane);
    alpha.request(
        "pane.send_text",
        json!({"pane_id":alpha_pane,"text":"kill \"$switch_job\"\n"}),
    );
    beta.request(
        "pane.send_text",
        json!({"pane_id":beta_pane,"text":"kill \"$switch_job\"\n"}),
    );
}

#[test]
fn session_menu_switch_changes_only_the_invoking_native_window() {
    switch_changes_only_the_invoking_native_window(Entry::SessionMenu);
}

#[test]
fn command_palette_switch_changes_only_the_invoking_native_window() {
    switch_changes_only_the_invoking_native_window(Entry::CommandPalette);
}

#[test]
fn cancelling_the_session_picker_keeps_the_original_shell_and_attachment() {
    let mut alpha = Core::new();
    let mut beta = Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-beta", alpha.name),
        support::executable(),
        false,
    );
    let (pane, shell, job) = marker_shell(&alpha, "alpha");
    let shell_identity = process_identity(&shell);
    let job_identity = process_identity(&job);
    let mut console = Console::new(&alpha);
    console.wait(|rows| rows.iter().any(|row| row.contains("alpha-original-shell")));
    open_session_menu_picker(&mut console);
    console.wait(|rows| rows.iter().any(|row| row.contains(&beta.name)));
    console.text("\x1b");
    console.wait(|rows| !rows.iter().any(|row| row.contains("Switch session on")));
    console.text("printf '%s-cancelled-%s\\n' \"$marker\" \"$$\"\r");
    let expected = format!("alpha-cancelled-{shell}");
    let rows = console.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    eprintln!("native cancelled-switch screen: {rows:?}");
    assert_eq!(process_identity(&shell), shell_identity);
    assert_eq!(process_identity(&job), job_identity);
    assert!(alpha.child.try_wait().unwrap().is_none());
    assert!(beta.child.try_wait().unwrap().is_none());
    assert_eq!(
        alpha.request("session.snapshot", json!({}))["snapshot"]["focused_pane_id"],
        pane
    );
    alpha.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":"kill \"$switch_job\"\n"}),
    );
}
