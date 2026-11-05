#![cfg(target_os = "linux")]

mod support;

use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use support::Core;

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
            "shell did not finish {}",
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
    assert_ne!(fields[4], "0", "process lost controlling PTY");
    (
        fields[19].into(),
        fields[4].into(),
        std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap(),
        std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
    )
}

fn peer(alpha: &Core, suffix: &str) -> Core {
    Core::launch_with_executable(
        alpha.home.clone(),
        format!("{}-{suffix}", alpha.name),
        support::executable(),
        false,
    )
}

fn pane_for(core: &Core, terminal: &Value) -> Value {
    core.request("session.snapshot", json!({}))["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pane| &pane["terminal_id"] == terminal)
        .expect("original terminal must remain present")["pane_id"]
        .clone()
}

struct Shell {
    workspace: Value,
    terminal: Value,
    state: String,
    shell_identity: (String, String, PathBuf, PathBuf),
    job_identity: (String, String, PathBuf, PathBuf),
    environment: Vec<u8>,
}
impl Shell {
    fn start(core: &Core, marker: &str) -> Self {
        let workspace = core.request(
            "workspace.create",
            json!({"label":"Identical label","cwd":core.home,"focus":true}),
        );
        let pane = &workspace["root_pane"]["pane_id"];
        let terminal =
            core.request("pane.get", json!({"pane_id":pane}))["pane"]["terminal_id"].clone();
        core.request("pane.send_text", json!({"pane_id":pane,"text":format!(
            "route_marker={marker}; sleep 300 & route_job=$!; printf '%s\\n' \"$$\" \"$route_job\" \"$route_marker\" \"$HERDR_SOCKET_PATH\" \"$HERDR_PANE_ID\" \"$HERDR_PANE_RUNTIME_ID\" > {marker}-before\n"
        )}));
        let state = wait_file(&core.home.join(format!("{marker}-before")));
        let fields: Vec<_> = state.lines().collect();
        let shell_identity = identity(fields[0]);
        // The background job inherits the same controlling PTY, but its exec may
        // still be in flight when the shell writes the readiness file.
        let deadline = Instant::now() + Duration::from_secs(5);
        let job_identity = loop {
            let value = identity(fields[1]);
            if value.3.file_name().is_some_and(|name| name == "sleep") {
                break value;
            }
            assert!(Instant::now() < deadline, "background sleep did not exec");
            std::thread::sleep(Duration::from_millis(10));
        };
        let environment = std::fs::read(format!("/proc/{}/environ", fields[0])).unwrap();
        Self {
            workspace,
            terminal,
            state,
            shell_identity,
            job_identity,
            environment,
        }
    }

    fn assert_inherited(&self, core: &Core, stage: &str) {
        let pane = pane_for(core, &self.terminal);
        core.request("pane.send_text", json!({"pane_id":pane,"text":format!(
            "kill -0 \"$route_job\" && printf '%s\\n' \"$$\" \"$route_job\" \"$route_marker\" \"$HERDR_SOCKET_PATH\" \"$HERDR_PANE_ID\" \"$HERDR_PANE_RUNTIME_ID\" > {stage}-state; '{}' pane current > {stage}-current.json 2> {stage}-error; printf '%s\\n' \"$?\" > {stage}-done\n",
            support::executable().display()
        )}));
        assert_eq!(
            wait_file(&core.home.join(format!("{stage}-done"))),
            "0\n",
            "inherited pane.current failed: {}",
            std::fs::read_to_string(core.home.join(format!("{stage}-error"))).unwrap()
        );
        assert_eq!(
            wait_file(&core.home.join(format!("{stage}-state"))),
            self.state,
            "shell locals or inherited context changed"
        );
        let current: Value =
            serde_json::from_str(&wait_file(&core.home.join(format!("{stage}-current.json"))))
                .unwrap();
        assert_eq!(current["result"]["pane"]["terminal_id"], self.terminal);
        assert_eq!(current["result"]["pane"]["pane_id"], pane);
        let fields: Vec<_> = self.state.lines().collect();
        assert_eq!(identity(fields[0]), self.shell_identity);
        assert_eq!(identity(fields[1]), self.job_identity);
        assert!(
            std::fs::read(format!("/proc/{}/environ", fields[0])).unwrap() == self.environment,
            "owned shell environment changed"
        );
    }

    fn inherited_marker(&self, core: &Core, stage: &str) {
        // Run the real CLI inside the moved shell. It inherits the original
        // endpoint and locator unchanged, and queues input back to that shell.
        let pane = pane_for(core, &self.terminal);
        core.request("pane.focus", json!({"pane_id":pane}));
        let mut client = core.attach();
        let deadline = Instant::now() + Duration::from_secs(8);
        while client.focused_pane_id.as_deref() != pane.as_str() {
            assert!(
                Instant::now() < deadline,
                "native client did not select the retained shell"
            );
            client.receive().unwrap();
        }
        client.type_text(&format!(
            "'{}' pane send-text \"$HERDR_PANE_ID\" 'printf \"%s\\n\" \"$route_marker\" > {stage}-marker\n' > {stage}-send.json 2> {stage}-send.error; printf '%s\\n' \"$?\" > {stage}-send.done\n",
            support::executable().display()
        )).unwrap();
        assert_eq!(
            wait_file(&core.home.join(format!("{stage}-send.done"))),
            "0\n",
            "inherited input failed: {}",
            std::fs::read_to_string(core.home.join(format!("{stage}-send.error"))).unwrap()
        );
        assert_eq!(
            wait_file(&core.home.join(format!("{stage}-marker"))).trim(),
            self.state.lines().nth(2).unwrap()
        );
    }
}

fn transfer(source: &Core, shell: &Shell, destination: &Core, workspace: &Value, operation: &str) {
    let current = source.request(
        "pane.get",
        json!({"pane_id":pane_for(source, &shell.terminal)}),
    );
    source.request(
        "tab.transfer",
        json!({
            "tab_id":current["pane"]["tab_id"],"destination_session":destination.name,
            "workspace_id":workspace["workspace"]["workspace_id"],"operation_id":operation
        }),
    );
}

#[test]
fn independent_core_locators_keep_inherited_shell_requests_on_their_original_processes() {
    let alpha = Core::new();
    let beta = peer(&alpha, "beta");
    let moved = Shell::start(&alpha, "alpha");
    let unrelated = Shell::start(&beta, "beta");
    assert_eq!(
        moved.workspace["root_pane"]["pane_id"], unrelated.workspace["root_pane"]["pane_id"],
        "fixture must create colliding public locators in independent Cores"
    );
    assert_ne!(moved.terminal, unrelated.terminal);
    moved.assert_inherited(&alpha, "alpha-before");
    unrelated.assert_inherited(&beta, "beta-before");
    transfer(
        &alpha,
        &moved,
        &beta,
        &unrelated.workspace,
        "independent-locators",
    );
    beta.request(
        "pane.focus",
        json!({"pane_id":pane_for(&beta, &unrelated.terminal)}),
    );
    moved.assert_inherited(&beta, "alpha-moved");
    moved.inherited_marker(&beta, "alpha-moved");
    unrelated.assert_inherited(&beta, "beta-unrelated");
    unrelated.inherited_marker(&beta, "beta-unrelated");
}

fn stop(core: &mut Core) {
    core.request("server.stop", json!({}));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = core.child.try_wait().unwrap() {
            assert!(status.success(), "owned source Core exited unsuccessfully");
            return;
        }
        assert!(Instant::now() < deadline, "owned source Core did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn returning_to_alpha_keeps_original_shell_context_after_beta_exits() {
    let alpha = Core::new();
    let mut beta = peer(&alpha, "beta");
    let moved = Shell::start(&alpha, "returning");
    let bystander = Shell::start(&beta, "bystander");
    let returning_workspace = alpha.request(
        "workspace.create",
        json!({"label":"Return destination","cwd":alpha.home}),
    );
    transfer(
        &alpha,
        &moved,
        &beta,
        &bystander.workspace,
        "return-outward",
    );
    moved.assert_inherited(&beta, "return-in-beta");
    transfer(&beta, &moved, &alpha, &returning_workspace, "return-home");
    moved.assert_inherited(&alpha, "return-in-alpha");
    moved.inherited_marker(&alpha, "return-in-alpha");
    stop(&mut beta);
    moved.assert_inherited(&alpha, "return-after-beta-exit");
    moved.inherited_marker(&alpha, "return-after-beta-exit");
}

#[test]
fn third_owner_gamma_keeps_original_shell_context_after_both_previous_cores_exit() {
    let mut alpha = Core::new();
    let mut beta = peer(&alpha, "beta");
    let gamma = peer(&alpha, "gamma");
    let moved = Shell::start(&alpha, "third-owner");
    let beta_shell = Shell::start(&beta, "beta-stays");
    let gamma_shell = Shell::start(&gamma, "gamma-stays");
    transfer(
        &alpha,
        &moved,
        &beta,
        &beta_shell.workspace,
        "third-outward",
    );
    moved.assert_inherited(&beta, "third-in-beta");
    transfer(
        &beta,
        &moved,
        &gamma,
        &gamma_shell.workspace,
        "third-to-gamma",
    );
    beta_shell.assert_inherited(&beta, "third-beta-unrelated-after-second-move");
    beta_shell.inherited_marker(&beta, "third-beta-unrelated-after-second-move");
    moved.assert_inherited(&gamma, "third-in-gamma");
    moved.inherited_marker(&gamma, "third-in-gamma");
    stop(&mut alpha);
    stop(&mut beta);
    moved.assert_inherited(&gamma, "third-after-previous-exit");
    moved.inherited_marker(&gamma, "third-after-previous-exit");
    gamma_shell.assert_inherited(&gamma, "third-gamma-unrelated");
    let inventory = gamma.request("session.list", json!({}));
    for stopped in [&alpha.name, &beta.name] {
        assert_eq!(
            inventory["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|session| session["name"].as_str() == Some(stopped.as_str()))
                .unwrap()["running"],
            false,
            "retained routing service must not make a stopped Core appear running"
        );
    }
}

#[test]
fn restarted_alpha_cannot_steal_the_moved_shell_inherited_context() {
    let mut alpha = Core::new();
    let beta = peer(&alpha, "beta");
    let moved = Shell::start(&alpha, "restart-moved");
    let beta_shell = Shell::start(&beta, "restart-beta");
    transfer(
        &alpha,
        &moved,
        &beta,
        &beta_shell.workspace,
        "restart-outward",
    );
    stop(&mut alpha);
    moved.assert_inherited(&beta, "restart-source-stopped");
    let restarted = Core::launch_with_executable(
        alpha.home.clone(),
        alpha.name.clone(),
        support::executable(),
        false,
    );
    let fresh = Shell::start(&restarted, "restart-fresh");
    assert_ne!(fresh.terminal, moved.terminal);
    assert_ne!(fresh.state.lines().next(), moved.state.lines().next());
    moved.assert_inherited(&beta, "restart-old-context");
    moved.inherited_marker(&beta, "restart-old-context");
    fresh.assert_inherited(&restarted, "restart-fresh-context");
    fresh.inherited_marker(&restarted, "restart-fresh-context");
    beta_shell.assert_inherited(&beta, "restart-beta-unrelated");
}

fn cli(core: &Core, args: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(support::executable());
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let key = key.to_string_lossy();
        key.starts_with("HERDR_") || key.starts_with("TRAILBOSS_")
    }) {
        command.env_remove(key);
    }
    for key in support::HOMES {
        command.env(key, &core.home);
    }
    command.args(args).current_dir(&core.home).output().unwrap()
}

#[test]
fn deleting_stopped_source_preserves_live_routes_until_original_processes_end() {
    let mut alpha = Core::new();
    let beta = peer(&alpha, "beta");
    let moved = Shell::start(&alpha, "delete-moved");
    let bystander = Shell::start(&beta, "delete-bystander");
    transfer(
        &alpha,
        &moved,
        &beta,
        &bystander.workspace,
        "delete-outward",
    );
    stop(&mut alpha);
    let endpoint = PathBuf::from(moved.state.lines().nth(3).unwrap());
    let refused = cli(&alpha, &["session", "delete", &alpha.name, "--json"]);
    assert!(
        !refused.status.success(),
        "deletion must reject a live moved process route"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("live moved process"),
        "deletion must explain that live routing retains the source endpoint"
    );
    assert!(
        endpoint.exists(),
        "refused deletion removed inherited endpoint"
    );
    moved.assert_inherited(&beta, "delete-refused");
    moved.inherited_marker(&beta, "delete-refused");
    bystander.assert_inherited(&beta, "delete-bystander-live");
    beta.request("pane.send_text", json!({
        "pane_id":pane_for(&beta, &moved.terminal),
        "text":"kill \"$route_job\"; wait \"$route_job\"; printf 'exiting\\n' > delete-processes-ending; exit\n"
    }));
    assert_eq!(
        wait_file(&alpha.home.join("delete-processes-ending")),
        "exiting\n"
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let processes_ended = moved
            .state
            .lines()
            .take(2)
            .all(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).is_err());
        let snapshot = beta.request("session.snapshot", json!({}));
        let pane_removed = !snapshot["snapshot"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pane| pane["terminal_id"] == moved.terminal);
        if processes_ended && pane_removed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "original moved shell/job did not end and close their pane"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let deleted = cli(&alpha, &["session", "delete", &alpha.name, "--json"]);
    assert!(
        deleted.status.success(),
        "ordinary deletion failed after process exit: {}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    assert!(
        !endpoint.exists(),
        "successful deletion retained the original endpoint"
    );
    assert!(
        std::os::unix::net::UnixStream::connect(&endpoint).is_err(),
        "deleted endpoint still accepts connections"
    );
    assert!(!alpha
        .home
        .join("trailboss/sessions")
        .join(&alpha.name)
        .exists());
    let inventory = beta.request("session.list", json!({}));
    assert!(!inventory["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|session| session["name"] == alpha.name));
    bystander.assert_inherited(&beta, "delete-bystander-after");
    bystander.inherited_marker(&beta, "delete-bystander-after");
}
