use serde_json::{json, Value};
use std::{fs, process::Command};

fn run(evidence: &Value) -> (bool, String) {
    let path = std::env::temp_dir().join(format!(
        "trailboss-exit-test-{}-{}.json",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, serde_json::to_vec(evidence).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_spike-exit"))
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

#[test]
fn absent_daily_driver_results_keep_existing_fork() {
    let (passed, report) = run(&json!({}));
    assert!(!passed);
    assert!(report.contains("FAILED"));
    assert!(report.contains("actual session"));
    assert!(report.contains("plugin development on the existing original Herdr fork"));
    assert!(report.contains("No sessions, PTYs or configuration were changed"));
}

struct Bundle {
    dir: std::path::PathBuf,
    manifest: Value,
    actual: Value,
    latency: Value,
}
impl Bundle {
    fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let dir = std::env::temp_dir().join(format!(
            "trailboss-report-fixture-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap()
        ));
        fs::create_dir_all(&dir).unwrap();
        let actual = json!({"source":"actual-session-exercise","session_id":"opaque-session","authorization_reference":"test-only-not-human-evidence","command":"test observer","revision":"test-revision","boot_id":"test-boot","timestamp_ms":now,
          "stock_ui":{"plugin_id":"stock-ui","composition_source":"public-plugin-declarations","privileged_paths":[]},
          "agent_tree":{"plugin_id":"agent-tree","transport":"stdin-envelope","store":"tanstack-db","requested_tab":"t1","focused_tab":"t1"},
          "attach":{"client_machine":"achy","core_machine":"junkman","transport":"ssh","session_id":"opaque-session"},
          "clipboard_image":{"source_machine":"achy","bytes":32,"original_sha256":"a".repeat(64),"received_sha256":"a".repeat(64),"target_pane":"p1","focused_pane":"p1"},
          "continuity":{"before_session":"opaque-session","after_session":"opaque-session","before_panes":["p1"],"after_panes":["p1"]}});
        let populations: Vec<_> = (0..15).map(|i| json!({"pane":format!("p{i}"),"label":format!("Pane{i}"),"tab_label":"Tab","process":"test workload", "readiness":{"completed_epoch_ms":now,"input_response":"ready","populated_visible":"rows","restored_visible":"rows"}})).collect();
        let samples: Vec<_> = ["workspace","tab","pane","workspace","tab","pane"].iter().map(|kind| json!({"kind":kind,"target":populations[0],"elapsed_ms":100.0,"complete_visible_frame":true,"input_focus_matches":true,"input_dispatch_epoch_ms":now,"completed_frames_ms":[{"elapsed_ms":99.0,"surface_first_row":"Pane0-row0","selected_tab":"Tab"}]})).collect();
        let latency = json!({"boot_id":"test-boot","core_machine":"junkman","setup":{"mode":"local","commit":"test-revision","session_id":"opaque-session","executable":"test-client","populations":populations},"observation":{"machine":"junkman","client_pid":1,"observer_pid":2,"display":{"cols":100,"rows":45},"timestamp_ms":now,"samples":samples}});
        let manifest = json!({"spike_started_ms":now-86400000,"actual_session":dir.join("actual.json"),"local":dir.join("local.json"),"remote":dir.join("remote.json"),"smoke_local":dir.join("local.json"),"smoke_remote":dir.join("remote.json")});
        Self {
            dir,
            manifest,
            actual,
            latency,
        }
    }
    fn report(&self) -> (bool, String) {
        fs::write(
            self.dir.join("actual.json"),
            serde_json::to_vec(&self.actual).unwrap(),
        )
        .unwrap();
        fs::write(
            self.dir.join("local.json"),
            serde_json::to_vec(&self.latency).unwrap(),
        )
        .unwrap();
        let mut remote = self.latency.clone();
        remote["setup"]["mode"] = json!("remote from achy");
        remote["observation"]["machine"] = json!("achy");
        fs::write(
            self.dir.join("remote.json"),
            serde_json::to_vec(&remote).unwrap(),
        )
        .unwrap();
        run(&self.manifest)
    }
}
impl Drop for Bundle {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn report_requires_every_result_and_independent_raw_mode_evidence() {
    // Production-shaped deterministic contract data, NOT Tommy’s session or acceptance evidence.
    let mut bundle = Bundle::new();
    assert!(bundle.report().0);
    for key in [
        "source",
        "session_id",
        "authorization_reference",
        "command",
        "revision",
        "boot_id",
        "timestamp_ms",
        "stock_ui",
        "agent_tree",
        "attach",
        "clipboard_image",
        "continuity",
    ] {
        let original = bundle.actual[key].take();
        assert!(!bundle.report().0, "accepted missing {key}");
        bundle.actual[key] = original;
    }
    for pointer in [
        "/stock_ui/composition_source",
        "/agent_tree/focused_tab",
        "/attach/client_machine",
        "/clipboard_image/received_sha256",
        "/continuity/after_session",
    ] {
        let original = bundle.actual.pointer_mut(pointer).unwrap().take();
        *bundle.actual.pointer_mut(pointer).unwrap() = json!("failing");
        assert!(!bundle.report().0, "accepted failing {pointer}");
        *bundle.actual.pointer_mut(pointer).unwrap() = original;
    }
    for pointer in [
        "/setup/populations/14/readiness",
        "/observation/samples/0/elapsed_ms",
        "/observation/samples/0/completed_frames_ms",
        "/observation/samples/0/completed_frames_ms/0/selected_tab",
        "/observation/samples/0/input_focus_matches",
        "/observation/samples/0/target/pane",
        "/observation/timestamp_ms",
        "/setup/session_id",
        "/setup/commit",
        "/setup/mode",
    ] {
        let original = bundle.latency.pointer_mut(pointer).unwrap().take();
        assert!(!bundle.report().0, "accepted missing {pointer}");
        *bundle.latency.pointer_mut(pointer).unwrap() = original;
    }
    for before_readiness in [true, false] {
        let saved = bundle.latency.clone();
        invert_chronology(&mut bundle.latency, before_readiness);
        assert!(
            !bundle.report().0,
            "accepted causal inversion before_readiness={before_readiness}"
        );
        bundle.latency = saved;
    }
    bundle.latency["observation"]["samples"][0]["elapsed_ms"] = json!(100.001);
    assert!(!bundle.report().0);
    bundle.latency["observation"]["samples"][0]["elapsed_ms"] = json!(100);
    for key in ["local", "remote", "smoke_local", "smoke_remote"] {
        let original = bundle.manifest[key].take();
        bundle.manifest[key] = json!(bundle.dir.join("absent.json"));
        assert!(!bundle.report().0, "accepted missing {key}");
        bundle.manifest[key] = original;
    }
    bundle.manifest["remote"] = bundle.manifest["local"].clone();
    assert!(!bundle.report().0);
    bundle.manifest["spike_started_ms"] = json!(1);
    assert!(!bundle.report().0);
}

#[test]
fn living_keep_fork_when_one_required_exit_result_is_absent() {
    let spec = include_str!("../features/spike-exit.feature");
    assert!(
        spec.contains("Given at least one required daily-driver exit result is failing or absent")
    );
    assert!(spec.contains("rather than switching daily drivers"));
    let mut bundle = Bundle::new();
    let stock = bundle.actual["stock_ui"].take();
    let (passed, report) = bundle.report();
    assert!(!passed);
    assert!(report.contains("stock UI"));
    assert!(report.contains("plugin development on the existing original Herdr fork"));
    bundle.actual["stock_ui"] = stock;
    for before_readiness in [true, false] {
        let saved = bundle.latency.clone();
        invert_chronology(&mut bundle.latency, before_readiness);
        let (passed, report) = bundle.report();
        assert!(
            !passed,
            "accepted living causal inversion before_readiness={before_readiness}"
        );
        assert!(report.contains("plugin development on the existing original Herdr fork"));
        bundle.latency = saved;
    }
}

fn invert_chronology(latency: &mut Value, before_readiness: bool) {
    let finished = latency["observation"]["timestamp_ms"].as_u64().unwrap() - 10;
    latency["observation"]["timestamp_ms"] = json!(finished);
    for population in latency["setup"]["populations"].as_array_mut().unwrap() {
        population["readiness"]["completed_epoch_ms"] = json!(finished - 10);
    }
    let target = latency["setup"]["populations"][0].clone();
    for sample in latency["observation"]["samples"].as_array_mut().unwrap() {
        sample["target"] = target.clone();
        sample["input_dispatch_epoch_ms"] = json!(finished - 5);
    }
    latency["observation"]["samples"][0]["input_dispatch_epoch_ms"] = json!(if before_readiness {
        finished - 11
    } else {
        finished + 1
    });
}
