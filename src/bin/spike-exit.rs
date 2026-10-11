use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::Path,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

const DAY: u64 = 86_400_000;
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn nonempty(v: &Value, key: &str) -> bool {
    !text(v, key).trim().is_empty()
}
fn load(path: &Path) -> Result<Value, String> {
    let bytes =
        fs::read(path).map_err(|_| format!("{}: evidence absent or unreadable", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|_| format!("{}: invalid JSON evidence", path.display()))
}
fn require(reasons: &mut Vec<String>, valid: bool, reason: &str) {
    if !valid {
        reasons.push(reason.to_owned());
    }
}
fn within(t: u64, start: u64, now: u64) -> bool {
    start > 0 && t >= start && t <= now && t - start <= 21 * DAY
}
fn same_nonempty(a: &Value, ak: &str, b: &Value, bk: &str) -> bool {
    nonempty(a, ak) && text(a, ak) == text(b, bk)
}
fn readiness(p: &Value) -> bool {
    let r = &p["readiness"];
    nonempty(p, "pane")
        && nonempty(p, "process")
        && r["completed_epoch_ms"].as_u64().is_some_and(|n| n > 0)
        && nonempty(r, "input_response")
        && nonempty(r, "populated_visible")
        && same_nonempty(r, "populated_visible", r, "restored_visible")
}
fn latency(v: &Value, mode: &str, start: u64, now: u64) -> Result<f64, String> {
    let setup = &v["setup"];
    let o = &v["observation"];
    let fail = || {
        format!(
            "{mode}: missing/failing raw 15-pane latency, Surface/focus, setup or window evidence"
        )
    };
    let populations = setup["populations"].as_array().ok_or_else(fail)?;
    let samples = o["samples"].as_array().ok_or_else(fail)?;
    let ids: HashSet<_> = populations
        .iter()
        .filter_map(|p| p["pane"].as_str())
        .collect();
    let remote = mode == "remote from achy";
    if text(setup, "mode") != mode
        || text(v, "core_machine") != "junkman"
        || text(o, "machine") != if remote { "achy" } else { "junkman" }
        || !nonempty(setup, "commit")
        || !nonempty(setup, "executable")
        || !nonempty(v, "boot_id")
        || o["client_pid"].as_u64().unwrap_or(0) == 0
        || o["observer_pid"].as_u64().unwrap_or(0) == 0
        || o["display"]["cols"].as_u64().unwrap_or(0) == 0
        || o["display"]["rows"].as_u64().unwrap_or(0) == 0
        || !within(o["timestamp_ms"].as_u64().unwrap_or(0), start, now)
        || populations.len() != 15
        || ids.len() != 15
        || !populations.iter().all(|p| {
            readiness(p)
                && within(
                    p["readiness"]["completed_epoch_ms"].as_u64().unwrap_or(0),
                    start,
                    o["timestamp_ms"].as_u64().unwrap_or(0),
                )
        })
        || samples.len() < 6
    {
        return Err(fail());
    }
    let mut max: f64 = 0.0;
    for kind in ["workspace", "tab", "pane"] {
        if samples.iter().filter(|s| text(s, "kind") == kind).count() < 2 {
            return Err(fail());
        }
    }
    for s in samples {
        let elapsed = s["elapsed_ms"].as_f64().ok_or_else(fail)?;
        let target = &s["target"];
        let frames = s["completed_frames_ms"].as_array().ok_or_else(fail)?;
        let last = frames.last().ok_or_else(fail)?;
        let visible = text(last, "surface_first_row").trim_end();
        if !elapsed.is_finite()
            || !(0.0..=100.0).contains(&elapsed)
            || !["workspace", "tab", "pane"].contains(&text(s, "kind"))
            || s["complete_visible_frame"] != true
            || s["input_focus_matches"] != true
            || !within(
                s["input_dispatch_epoch_ms"].as_u64().unwrap_or(0),
                target["readiness"]["completed_epoch_ms"]
                    .as_u64()
                    .unwrap_or(0),
                o["timestamp_ms"].as_u64().unwrap_or(0),
            )
            || !populations.iter().any(|p| p == target)
            || !nonempty(target, "label")
            || visible != format!("{}-row0", text(target, "label"))
            || !nonempty(target, "tab_label")
            || !text(last, "tab_strip").contains(text(target, "tab_label"))
            || frames.iter().any(|f| {
                f["elapsed_ms"]
                    .as_f64()
                    .is_none_or(|n| !n.is_finite() || n < 0.0 || n > elapsed)
            })
        {
            return Err(fail());
        }
        max = max.max(elapsed);
    }
    Ok(max)
}
fn actual(v: &Value, start: u64, now: u64, reasons: &mut Vec<String>) {
    require(reasons, text(v,"source") == "actual-session-exercise" && nonempty(v,"session_id")
        && nonempty(v,"authorization_reference") && nonempty(v,"command") && nonempty(v,"revision")
        && nonempty(v,"boot_id") && within(v["timestamp_ms"].as_u64().unwrap_or(0),start,now),
        "actual session: missing identity, explicit exercise authorization, command, revision, boot or current machine-observed provenance");
    let stock = &v["stock_ui"];
    require(
        reasons,
        text(stock, "plugin_id") == "stock-ui"
            && text(stock, "composition_source") == "public-plugin-declarations"
            && stock["privileged_paths"]
                .as_array()
                .is_some_and(Vec::is_empty),
        "actual session: stock UI public declarations not demonstrated",
    );
    let tree = &v["agent_tree"];
    require(
        reasons,
        text(tree, "plugin_id") == "agent-tree"
            && text(tree, "transport") == "stdin-envelope"
            && text(tree, "store") == "tanstack-db"
            && same_nonempty(tree, "requested_tab", tree, "focused_tab"),
        "actual session: ported agent tree and focus action not demonstrated",
    );
    let attach = &v["attach"];
    require(
        reasons,
        text(attach, "client_machine") == "achy"
            && text(attach, "core_machine") == "junkman"
            && text(attach, "transport") == "ssh"
            && same_nonempty(attach, "session_id", v, "session_id"),
        "actual session: achy SSH attach not demonstrated",
    );
    let image = &v["clipboard_image"];
    let hash = text(image, "original_sha256");
    require(
        reasons,
        text(image, "source_machine") == "achy"
            && image["bytes"].as_u64().unwrap_or(0) > 0
            && hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
            && same_nonempty(image, "original_sha256", image, "received_sha256")
            && same_nonempty(image, "target_pane", image, "focused_pane"),
        "actual session: clipboard image into focused remote pane not demonstrated",
    );
    let continuity = &v["continuity"];
    require(
        reasons,
        same_nonempty(continuity, "before_session", v, "session_id")
            && same_nonempty(continuity, "after_session", v, "session_id")
            && continuity["before_panes"]
                .as_array()
                .is_some_and(|p| !p.is_empty())
            && continuity["before_panes"] == continuity["after_panes"],
        "actual session: continuity not demonstrated",
    );
}
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        eprintln!("Usage: spike-exit MANIFEST.json (read-only; exit 0 only for complete evidence)");
        return ExitCode::from(2);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut reasons = Vec::new();
    let manifest = match load(Path::new(&args[0])) {
        Ok(v) => v,
        Err(e) => {
            reasons.push(e);
            Value::Null
        }
    };
    let start = manifest["spike_started_ms"].as_u64().unwrap_or(0);
    require(
        &mut reasons,
        within(now, start, now),
        "spike: missing start or outside the 2–3 week spike's 21-day maximum",
    );
    let evidence = match load(Path::new(text(&manifest, "actual_session"))) {
        Ok(v) => v,
        Err(e) => {
            reasons.push(e);
            Value::Null
        }
    };
    actual(&evidence, start, now, &mut reasons);
    for (key, mode, default) in [
        ("local", "local", "/tmp/herdr-latency-288945-local.json"),
        (
            "remote",
            "remote from achy",
            "/tmp/herdr-latency-288945-remote.json",
        ),
        (
            "smoke_local",
            "local",
            "/tmp/herdr-task61-final-smoke-local.json",
        ),
        (
            "smoke_remote",
            "remote from achy",
            "/tmp/herdr-task61-final-smoke-remote.json",
        ),
    ] {
        let path = manifest[key].as_str().unwrap_or(default);
        match load(Path::new(path)) {
            Ok(v) => {
                match latency(&v, mode, start, now) {
                    Ok(max) => println!("{key}: raw latency passed, max {max:.6}ms"),
                    Err(e) => reasons.push(format!("{key}: {e}")),
                }
                require(&mut reasons, same_nonempty(&v["setup"],"session_id",&evidence,"session_id")
                    && same_nonempty(&v["setup"],"commit",&evidence,"revision"),
                    &format!("{key}: latency setup not bound to demonstrated actual session/revision; isolated fixtures cannot establish daily-driver acceptance"));
            }
            Err(e) => reasons.push(e),
        }
    }
    let passed = reasons.is_empty();
    println!("Spike {}", if passed { "PASSED" } else { "FAILED" });
    for reason in reasons {
        println!("- {reason}");
    }
    if !passed {
        println!(
            "Continue plugin development on the existing herdr fork; keep it as the daily driver."
        );
    }
    println!("No sessions, PTYs or configuration were changed.");
    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
