use crate::{
    console::Console,
    support::{Core, HOMES},
};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const END: &[u8] = b"\x1b[?25h\x1b[0m\x1b[39m\x1b[49m";
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn checked(c: &mut Command) -> String {
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "{c:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
fn epoch() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
struct Remote(String);
impl Drop for Remote {
    fn drop(&mut self) {
        let _ = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "achy",
                &format!("rm -rf -- {}", quote(&self.0)),
            ])
            .status();
    }
}
fn wait_population(
    core: &Core,
    population: &Value,
    deadline: Instant,
    probe: &str,
    responded: bool,
) -> String {
    let label = population["label"].as_str().unwrap();
    let response = format!("{label}:{probe}");
    loop {
        let read = core.request(
            "pane.read",
            json!({"pane_id":population["pane"],"source":"visible"}),
        );
        let text = read["read"]["text"].as_str().unwrap();
        let populated = (0..4).all(|row| {
            text.lines()
                .any(|line| line.trim_end() == format!("{label}-row{row}"))
        });
        if populated && (probe.is_empty() || text.contains(&response) == responded) {
            return text.to_owned();
        }
        assert!(Instant::now() < deadline, "population readiness failed for {label}: populated={populated}, expected input response={responded}; visible={text:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}
pub(crate) fn seed(core: &Core) -> Value {
    seed_population(core, None)
}
pub(crate) fn seed_population(core: &Core, empty: Option<&str>) -> Value {
    let initial = core.request("session.snapshot", json!({}));
    for workspace in initial["snapshot"]["workspaces"].as_array().unwrap() {
        core.request(
            "workspace.close",
            json!({"workspace_id":workspace["workspace_id"]}),
        );
    }
    let mut populations = Vec::new();
    for w in 0..3 {
        let created = core.request(
            "workspace.create",
            json!({"label":format!("Work{w}"),"cwd":core.home,"focus":true}),
        );
        let workspace = created["workspace"]["workspace_id"].clone();
        for t in 0..2 {
            let tab = if t == 0 {
                created.clone()
            } else {
                core.request("tab.create", json!({"workspace_id":workspace,"label":format!("Tab{w}{t}"),"cwd":core.home,"focus":true}))
            };
            let tab_id = tab["tab"]["tab_id"].clone();
            core.request(
                "tab.rename",
                json!({"tab_id":tab_id,"label":format!("Tab{w}{t}")}),
            );
            let root = tab["root_pane"]["pane_id"].clone();
            let mut panes = vec![root.clone()];
            for _ in 1..if t == 0 { 3 } else { 2 } {
                let split = core.request(
                    "pane.split",
                    json!({"target_pane_id":root,"direction":"right","focus":false,"cwd":core.home}),
                );
                panes.push(split["pane"]["pane_id"].clone());
            }
            for (p, pane) in panes.iter().enumerate() {
                let label = format!("Pane{w}{t}{p}");
                core.request("pane.rename", json!({"pane_id":pane,"label":label}));
                let script = core.home.join(format!("population-{label}.sh"));
                fs::write(&script, format!("#!/bin/sh\npaint() {{ printf '\\033[2J\\033[H'; printf '{label}-row0\\n{label}-row1\\n{label}-row2\\n{label}-row3\\n'; }}\npaint\nwhile IFS= read -r line; do paint; if [ -n \"$line\" ]; then printf '{label}:%s\\n' \"$line\"; fi; done\n")).unwrap();
                if empty == Some(label.as_str()) {
                    fs::write(&script, "#!/bin/sh\nwhile IFS= read -r line; do :; done\n").unwrap();
                }
                core.request("pane.send_text", json!({"pane_id":pane,"text":format!("exec /bin/sh {}\n",quote(script.to_str().unwrap()))}));
                populations.push(json!({"workspace":workspace,"workspace_label":format!("Work{w}"),"tab":tab_id,"tab_label":format!("Tab{w}{t}"),"pane":pane,"label":label,"process":"/bin/sh population script; four populated identity rows; live input loop"}));
            }
        }
    }
    assert_eq!(populations.len(), 15);
    let readiness_deadline = Instant::now() + Duration::from_secs(5);
    for population in &mut populations {
        let label = population["label"].as_str().unwrap().to_owned();
        let initial = wait_population(core, population, readiness_deadline, "", false);
        let probe = format!("ready-{label}");
        core.request(
            "pane.send_text",
            json!({"pane_id":population["pane"],"text":format!("{probe}\n")}),
        );
        wait_population(core, population, readiness_deadline, &probe, true);
        core.request(
            "pane.send_text",
            json!({"pane_id":population["pane"],"text":"\n"}),
        );
        let restored = wait_population(core, population, readiness_deadline, &probe, false);
        population["readiness"] = json!({"completed_epoch_ms":epoch(),"populated_visible":initial,"input_response":format!("{label}:{probe}"),"restored_visible":restored});
    }
    for index in [13, 10, 8, 5, 3, 0] {
        core.request("pane.focus", json!({"pane_id":populations[index]["pane"]}));
    }
    for plugin in ["agent-tree", "stock-ui"] {
        core.request("plugin.link", json!({"path":format!("{}/plugins/{plugin}",env!("CARGO_MANIFEST_DIR")),"enabled":true}));
    }
    json!(populations)
}
// Read through the renderer's final cursor + palette reset, not a partial cell or bold row.
fn frame(console: &mut Console, pending: &mut Vec<u8>, deadline: Instant) -> bool {
    loop {
        if let Some(index) = pending.windows(END.len()).position(|part| part == END) {
            let end = index + END.len();
            console.screen.write(&pending[..end]);
            pending.drain(..end);
            return true;
        }
        match console
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(bytes) => pending.extend(bytes),
            Err(_) => return false,
        }
    }
}
fn visible(console: &Console, target: &Value) -> bool {
    let label = target["label"].as_str().unwrap();
    let rows = console.rows();
    // Every visible Surface cell is checked, including blank padding; the last row
    // is part of the completed frame, not an early text-prefix observation.
    (0..45).all(|y| {
        let actual: String = rows[y].chars().skip(25).collect();
        let expected = if y < 4 {
            format!("{label}-row{y}")
        } else {
            String::new()
        };
        actual.trim_end() == expected
    })
}
fn observe(config: &Value) -> Value {
    let mut command = portable_pty::CommandBuilder::new(config["executable"].as_str().unwrap());
    command.args(["--session", config["session"].as_str().unwrap()]);
    if config["mode"] == "remote from achy" {
        command.args([
            "--remote",
            "junkman",
            "--remote-executable",
            config["wrapper"].as_str().unwrap(),
        ]);
    }
    for key in HOMES {
        command.env(key, config["home"].as_str().unwrap());
    }
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        let name = key.to_string_lossy();
        name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
    }) {
        command.env_remove(key);
    }
    let trace_path = format!("{}/client-timing.log", config["home"].as_str().unwrap());
    if config["timing"] == true {
        command.env("TRAILBOSS_RENDER_PROF_TRACE", &trace_path);
    }
    command.env(
        "TRAILBOSS_CONFIG_PATH",
        format!("{}/config.toml", config["home"].as_str().unwrap()),
    );
    command.env("TERM", "xterm-256color");
    command.env("SHELL", "/bin/sh");
    command.env_remove("NO_COLOR");
    let mut console = Console::sized(command, 100, 45);
    let client_pid = console.child.process_id();
    let populations = config["populations"].as_array().unwrap();
    let mut pending = Vec::new();
    console.wait(|rows| rows.iter().any(|row| row.contains("Pane000-row3")));
    assert!(
        visible(&console, &populations[0]),
        "startup frame: {:?}",
        console.rows()
    );
    let mut samples = Vec::new();
    let mut selected = 0;
    // Each group changes workspace, tab, then pane within the same tab. No
    // warmups are discarded and no sample is removed from the acceptance set.
    let mut remembered = [0, 5, 10];
    let mut tab_panes = [[0, 3], [5, 8], [10, 13]];
    for round in 0..4 {
        for w in [1, 2, 0] {
            let tab = if remembered[w] - w * 5 < 3 { 1 } else { 0 };
            let tab_target = tab_panes[w][tab];
            let root = w * 5 + if tab == 0 { 0 } else { 3 };
            let pane_target = if tab_target == root { root + 1 } else { root };
            for (kind, target_index, needle) in [
                ("workspace", remembered[w], format!("Work{w}")),
                ("tab", tab_target, format!("Tab{w}{tab}")),
                (
                    "pane",
                    pane_target,
                    populations[pane_target]["label"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                ),
            ] {
                let target = &populations[target_index];
                let rows = console.rows();
                let y = rows
                    .iter()
                    .position(|row| row.chars().take(25).collect::<String>().contains(&needle))
                    .expect("sidebar click row absent");
                let x = 4;
                let dispatch_epoch = epoch();
                let started = Instant::now();
                console.click(x, y);
                let deadline = started + Duration::from_secs(2);
                let mut matched = false;
                let mut completed_frames_ms = Vec::new();
                while frame(&mut console, &mut pending, deadline) {
                    let rows = console.rows();
                    let selected_tab = rows
                        .iter()
                        .find(|row| row.starts_with('▏'))
                        .map(|row| row.chars().take(25).collect::<String>())
                        .unwrap_or_default();
                    completed_frames_ms.push(json!({"elapsed_ms":started.elapsed().as_secs_f64() * 1000.0,"surface_first_row":rows[0].chars().skip(25).collect::<String>(),"selected_tab":selected_tab}));
                    if visible(&console, target) {
                        matched = true;
                        break;
                    }
                }
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                // Prove the displayed selection also owns input, independently of style.
                let probe = format!("p{}", samples.len());
                console.text(&format!("{probe}\r"));
                let probe_deadline = Instant::now() + Duration::from_secs(2);
                let expected = format!("{}:{probe}", target["label"].as_str().unwrap());
                let mut focus = false;
                while frame(&mut console, &mut pending, probe_deadline) {
                    if console.rows().iter().any(|row| row.contains(&expected)) {
                        focus = true;
                        break;
                    }
                }
                // Restore the population's clean four-row display through real PTY input.
                console.text("\r");
                while frame(
                    &mut console,
                    &mut pending,
                    Instant::now() + Duration::from_secs(2),
                ) {
                    if visible(&console, target) {
                        break;
                    }
                }
                samples.push(json!({"kind":kind,"round":round,"input_dispatch_epoch_ms":dispatch_epoch,"elapsed_ms":elapsed,"completed_frames_ms":completed_frames_ms,"complete_visible_frame":matched,"input_focus_matches":focus,"target":target,"previous_index":selected,"click":{"x":x,"y":y},"pass":matched&&focus&&elapsed<=100.0}));
                selected = target_index;
                // Keep probe output out of the next latency criterion by matching it explicitly.
                std::thread::sleep(Duration::from_millis(80));
            }
            remembered[w] = pane_target;
            tab_panes[w][tab] = pane_target;
        }
    }
    let timing_trace = if config["timing"] == true {
        Some(fs::read_to_string(trace_path).unwrap())
    } else {
        None
    };
    json!({"machine":checked(&mut Command::new("hostname")).trim(),"observer_pid":std::process::id(),"client_pid":client_pid,"display":{"cols":100,"rows":45},"timestamp_ms":epoch(),"samples":samples,"timing_trace":timing_trace})
}
#[allow(dead_code)] // Called by the deployed benchmark observer, not the test orchestrator.
pub fn observer_file(path: &Path) {
    let config: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    println!("{}", observe(&config));
}
pub fn run(remote: bool, observer: &Path, output: &Path) {
    let core = Core::new();
    let populations = seed(&core);
    let mut identity = core.attach();
    while identity.boot_id.is_empty() {
        identity.receive().unwrap();
    }
    let boot_id = identity.boot_id.clone();
    drop(identity);
    let binary = crate::support::executable();
    let wrapper = core.home.join("remote-core");
    let mut script = String::from("#!/bin/sh\n");
    for key in HOMES {
        script.push_str(&format!(
            "export {key}={}\n",
            quote(core.home.to_str().unwrap())
        ));
    }
    script.push_str(&format!(
        "export TRAILBOSS_CONFIG_PATH={}\nexec {} \"$@\"\n",
        quote(core.home.join("config.toml").to_str().unwrap()),
        quote(binary.to_str().unwrap())
    ));
    fs::write(&wrapper, script).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let deployment = remote.then(|| {
        Remote(
            checked(Command::new("ssh").args([
                "-o",
                "BatchMode=yes",
                "achy",
                "mktemp -d /tmp/trailboss-latency.XXXXXXXX",
            ]))
            .trim()
            .to_owned(),
        )
    });
    let home = deployment
        .as_ref()
        .map_or_else(|| core.home.to_string_lossy().into_owned(), |r| r.0.clone());
    let executable = deployment.as_ref().map_or_else(
        || binary.to_string_lossy().into_owned(),
        |r| format!("{}/trailboss", r.0),
    );
    let config = json!({"mode":if remote {"remote from achy"} else {"local"},"timing":std::env::var_os("TRAILBOSS_LATENCY_CAPTURE").is_some(),"session":core.name,"home":home,"executable":executable,"wrapper":wrapper,"populations":populations,"core_pid":core.child.id(),"commit":checked(Command::new("jj").args(["log","-r","@","--no-graph","-T","commit_id"]))});
    let config_path = core.home.join("latency.json");
    fs::write(&config_path, config.to_string()).unwrap();
    let result = if let Some(deployment) = &deployment {
        checked(Command::new("scp").args([
            "-q",
            binary.to_str().unwrap(),
            &format!("achy:{}/trailboss", deployment.0),
        ]));
        checked(Command::new("scp").args([
            "-q",
            observer.to_str().unwrap(),
            &format!("achy:{}/observe", deployment.0),
        ]));
        checked(Command::new("scp").args([
            "-q",
            config_path.to_str().unwrap(),
            &format!("achy:{}/config.json", deployment.0),
        ]));
        let text = checked(Command::new("ssh").args([
            "-o",
            "BatchMode=yes",
            "achy",
            &format!(
                "{} --observe {}",
                quote(&format!("{}/observe", deployment.0)),
                quote(&format!("{}/config.json", deployment.0))
            ),
        ]));
        serde_json::from_str::<Value>(&text).unwrap()
    } else {
        observe(&config)
    };
    let perf_log = fs::read_to_string(
        core.home
            .join("trailboss/plugins/tommymorgan.agent-tree/perf/perf.log"),
    )
    .unwrap();
    let core_timing_trace = if config["timing"] == true {
        Some(fs::read_to_string(core.home.join("core-timing.log")).unwrap())
    } else {
        None
    };
    let report = json!({"setup":config,"boot_id":boot_id,"observation":result,"perf_log":perf_log,"core_timing_trace":core_timing_trace,"core_machine":checked(&mut Command::new("hostname")).trim(),"producer_processes":checked(Command::new("ps").args(["--ppid",&core.child.id().to_string(),"-o","pid,ppid,args"]))});
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let samples = report["observation"]["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 36);
    assert!(
        samples.iter().all(|sample| sample["pass"] == true),
        "latency/focus acceptance failed; raw samples: {}",
        output.display()
    );
}
