#[path = "support/console.rs"]
mod console;
pub mod support;

use console::Console;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use support::{Core, HOMES};

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn wait_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = fs::read_to_string(path) {
            if text.ends_with('\n') {
                return text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "owned shell did not write {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn process_identity(pid: &str) -> (String, String, PathBuf, PathBuf) {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    (
        fields[19].to_owned(),
        fields[4].to_owned(),
        fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}

struct MarkerShell<'a> {
    core: &'a Core,
    pane: String,
    shell: String,
    job: String,
    shell_identity: (String, String, PathBuf, PathBuf),
    job_identity: (String, String, PathBuf, PathBuf),
}
impl<'a> MarkerShell<'a> {
    fn new(core: &'a Core, marker: &str) -> Self {
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
        assert_eq!(fields.len(), 3, "shell/job/TTY identity: {state}");
        let shell_identity = process_identity(fields[0]);
        let job_identity = process_identity(fields[1]);
        assert_ne!(shell_identity.1, "0", "shell has no controlling PTY");
        assert_ne!(job_identity.1, "0", "job has no controlling PTY");
        assert_eq!(shell_identity.2, PathBuf::from(fields[2]));
        Self {
            core,
            pane,
            shell: fields[0].into(),
            job: fields[1].into(),
            shell_identity,
            job_identity,
        }
    }
    fn assert_preserved(&self) {
        assert_eq!(process_identity(&self.shell), self.shell_identity);
        assert_eq!(process_identity(&self.job), self.job_identity);
        let snapshot = self.core.request("session.snapshot", json!({}));
        assert_eq!(snapshot["snapshot"]["focused_pane_id"], self.pane);
    }
}
impl Drop for MarkerShell<'_> {
    fn drop(&mut self) {
        // Kill only the fixture-owned background job, including during an assertion unwind.
        // The Core fixture tears down the shell after this borrowed guard is dropped.
        if process_identity_if_alive(&self.job).as_ref() == Some(&self.job_identity) {
            let _ = Command::new("kill").args(["--", &self.job]).status();
        }
    }
}

fn process_identity_if_alive(pid: &str) -> Option<(String, String, PathBuf, PathBuf)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    Some((
        fields.get(19)?.to_string(),
        fields.get(4)?.to_string(),
        fs::read_link(format!("/proc/{pid}/fd/0")).ok()?,
        fs::read_link(format!("/proc/{pid}/exe")).ok()?,
    ))
}

fn remote_wrapper(core: &Core) -> (PathBuf, PathBuf) {
    let path = core.home.join("remote-core");
    let log = core.home.join("remote-wrapper.log");
    let executable = support::executable();
    // This is the actual SSH-side executable boundary, not a replacement transport.
    // Keep stdout exclusively for the real candidate's bridge protocol.
    let mut script = format!(
        "#!/bin/sh\nrecord=$(printf '%s\\t' \"$(hostname)\" \"$0\" {} \"${{SSH_CONNECTION:-}}\" \"$@\")\nprintf '%s\\n' \"$record\" >> {}\nexec env -i PATH=/usr/bin:/bin",
        quote(executable.to_str().unwrap()),
        quote(log.to_str().unwrap()),
    );
    for key in HOMES {
        script.push_str(&format!(" {key}={}", quote(core.home.to_str().unwrap())));
    }
    script.push_str(&format!(
        " TRAILBOSS_CONFIG_PATH={} {} \"$@\"\n",
        quote(core.home.join("config.toml").to_str().unwrap()),
        quote(executable.to_str().unwrap()),
    ));
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    (path, log)
}

fn require_real_ssh(core: &Core, wrapper: &Path) {
    // This fixture deliberately targets the same machine through real SSH, as in
    // remote_attach.rs: the owned logical HOME is shared, but the display HOME is not.
    let check = format!(
        "test \"$(hostname)\" = junkman && test -x {} && test -x {} && test -S {} && printf '%s\\n' fixture-ready",
        quote(wrapper.to_str().unwrap()),
        quote(support::executable().to_str().unwrap()),
        quote(core.home.join(format!("trailboss/sessions/{}/trailboss-client.sock", core.name)).to_str().unwrap()),
    );
    let result = Command::new("timeout")
        .args([
            "20s",
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "junkman",
        ])
        .arg(format!("/bin/sh -c {}", quote(&check)))
        .output()
        .expect("real SSH prerequisite: timeout and OpenSSH must be executable");
    assert!(
        result.status.success() && String::from_utf8_lossy(&result.stdout).trim() == "fixture-ready",
        "real SSH prerequisite: noninteractive SSH to junkman must expose this owned HOME/socket and candidate executable; status {}; stdout {}; stderr {}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
}

fn assert_remote_bridge(log: &Path, wrapper: &Path, session: &str) {
    let text = wait_file(log);
    let executable = support::executable();
    let records: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split_terminator('\t').collect())
        .collect();
    assert!(
        records.iter().any(|record| {
            record.len() >= 7
                && record[0] == "junkman"
                && record[1] == wrapper.to_str().unwrap()
                && record[2] == executable.to_str().unwrap()
                && record[3].split_whitespace().count() == 4
                && record[4..]
                    .windows(2)
                    .any(|args| args == ["--session", session])
                && record[4..].contains(&"remote-client-bridge")
        }),
        "no real SSH bridge through the selected executable for {session}: {text}"
    );
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

#[test]
#[ignore = "requires real SSH to junkman; no remote installation"]
fn remote_switch_discovers_sessions_on_the_attached_host() {
    let alpha = Core::new();
    let beta = Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let display_home = alpha.home.with_file_name(format!("{}-display", alpha.name));
    fs::create_dir_all(&display_home).unwrap();
    let local = Core::launch(display_home, "local-only".into());
    assert_ne!(alpha.home, local.home);
    let core_identities: Vec<_> = [&alpha, &beta, &local]
        .iter()
        .map(|core| process_identity(&core.child.id().to_string()))
        .collect();
    let alpha_shell = MarkerShell::new(&alpha, "alpha");
    let beta_shell = MarkerShell::new(&beta, "beta");
    let local_shell = MarkerShell::new(&local, "local");
    let (wrapper, log) = remote_wrapper(&alpha);
    require_real_ssh(&alpha, &wrapper);

    let mut command = portable_pty::CommandBuilder::new(support::executable());
    command.args([
        "--remote",
        "junkman",
        "--remote-executable",
        wrapper.to_str().unwrap(),
        "--session",
        &alpha.name,
    ]);
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
        command.env_remove(key);
    }
    for key in HOMES {
        command.env(key, &local.home);
    }
    command.env("TRAILBOSS_CONFIG_PATH", local.home.join("config.toml"));
    command.env("TERM", "xterm-256color");
    command.env("SHELL", "/bin/sh");
    command.env_remove("NO_COLOR");
    let mut native = Console::from_command(command);
    native.wait(|rows| rows.iter().any(|row| row.contains("alpha-original-shell")));
    assert_remote_bridge(&log, &wrapper, &alpha.name);
    open_session_menu_picker(&mut native);
    let picker = native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("Switch session on junkman"))
            && rows.iter().any(|row| row.contains(&beta.name))
            && rows.iter().any(|row| row.contains(&alpha.name))
    });
    assert!(
        !picker.iter().any(|row| row.contains("local-only")),
        "display-only session leaked into remote picker: {picker:?}"
    );
    native.text(&beta.name);
    native.text("\r");
    native.wait(|rows| rows.iter().any(|row| row.contains("beta-original-shell")));
    native.text("printf '%s-remote-switch-%s-%s\\n' \"$marker\" \"$$\" \"$(hostname)\"\r");
    let expected = format!("beta-remote-switch-{}-junkman", beta_shell.shell);
    native.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    assert_remote_bridge(&log, &wrapper, &beta.name);
    let snapshot = beta.request("session.snapshot", json!({}));
    assert_eq!(snapshot["snapshot"]["focused_pane_id"], beta_shell.pane);

    // Detach the owned native client first. Both SSH-side Cores and the unrelated
    // local Core must still answer, with their original shells, jobs and PTYs.
    drop(native);
    for (core, identity) in [&alpha, &beta, &local].iter().zip(core_identities) {
        assert_eq!(process_identity(&core.child.id().to_string()), identity);
    }
    alpha_shell.assert_preserved();
    beta_shell.assert_preserved();
    local_shell.assert_preserved();
    eprintln!(
        "real SSH native switch: {expected}; picker {picker:?}; wrapper {}",
        wrapper.display()
    );
    // Borrowed marker guards drop before their Cores; beta drops before alpha's
    // owning HOME, and local owns only the distinct display HOME.
}

#[test]
#[ignore = "requires real SSH to junkman; no remote installation"]
fn remote_switch_rejects_destination_stopped_after_discovery() {
    let alpha = Core::new();
    let mut beta = Core::launch_with_executable(
        alpha.home.clone(),
        "beta".into(),
        support::executable(),
        false,
    );
    let alpha_shell = MarkerShell::new(&alpha, "alpha");
    let (wrapper, log) = remote_wrapper(&alpha);
    require_real_ssh(&alpha, &wrapper);
    let mut command = portable_pty::CommandBuilder::new(support::executable());
    command.args([
        "--remote",
        "junkman",
        "--remote-executable",
        wrapper.to_str().unwrap(),
        "--session",
        &alpha.name,
    ]);
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
        command.env_remove(key);
    }
    for key in HOMES {
        command.env(key, &alpha.home);
    }
    command.env("TRAILBOSS_CONFIG_PATH", alpha.home.join("config.toml"));
    command.env("TERM", "xterm-256color");
    command.env("SHELL", "/bin/sh");
    command.env_remove("NO_COLOR");
    let mut native = Console::from_command(command);
    eprintln!(
        "remote disappearance: awaiting initial Alpha render; wrapper {}",
        log.display()
    );
    native.wait(|rows| rows.iter().any(|row| row.contains("alpha-original-shell")));
    eprintln!("remote disappearance: Alpha attached; discovering Beta");
    assert_remote_bridge(&log, &wrapper, &alpha.name);
    open_session_menu_picker(&mut native);
    native.wait(|rows| rows.iter().any(|row| row.contains(&beta.name)));
    eprintln!("remote disappearance: Beta discovered; stopping owned destination");

    // Stop only the owned destination after its picker row has been discovered.
    beta.request("server.stop", json!({}));
    let deadline = Instant::now() + Duration::from_secs(5);
    while beta.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "owned destination did not stop");
        std::thread::sleep(Duration::from_millis(10));
    }
    let destination = beta.home.join(format!("trailboss/sessions/{}", beta.name));
    assert!(!destination.join("trailboss-client.sock").exists());
    native.text(&beta.name);
    native.text("\r");
    eprintln!("remote disappearance: stopped Beta selected; awaiting native error");
    native.wait(|rows| {
        rows.iter()
            .any(|row| row.contains("Cannot switch session:"))
    });
    eprintln!("remote disappearance: native error visible; checking original Alpha shell");
    native.text("printf 'source-still-usable-%s\\n' \"$$\"\r");
    let expected = format!("source-still-usable-{}", alpha_shell.shell);
    native.wait(|rows| rows.iter().any(|row| row.contains(&expected)));
    assert_remote_bridge(&log, &wrapper, &beta.name);
    alpha_shell.assert_preserved();
    assert!(
        !destination.join("trailboss-client.sock").exists(),
        "switch spawned a replacement Core"
    );
    let snapshot = alpha.request("session.snapshot", json!({}));
    assert_eq!(snapshot["snapshot"]["panes"].as_array().unwrap().len(), 1);
    drop(native);
}
