use std::{
    fs::File,
    io::BufReader,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

fn unavailable(agent: &str, id: &str, detail: &str) -> String {
    format!("Saved {agent} conversation is unavailable: {id} ({detail})")
}

fn invalid(agent: &str, id: &str, detail: &str) -> String {
    format!("Saved {agent} conversation is invalid: {id} ({detail})")
}

fn check_id(agent: &str, id: &str) -> Result<(), String> {
    let mut parts = Path::new(id).components();
    if id.is_empty()
        || !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || id.contains(['/', '\\'])
    {
        return Err(invalid(agent, id, "invalid session ID"));
    }
    Ok(())
}

fn open(agent: &str, id: &str, path: &Path) -> Result<BufReader<File>, String> {
    let file = File::open(path)
        .map_err(|_| unavailable(agent, id, &format!("cannot read {}", path.display())))?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(unavailable(
            agent,
            id,
            "session record is not a regular file",
        ));
    }
    Ok(BufReader::new(file))
}

fn same_directory(saved: &Path, cwd: &Path) -> bool {
    saved == cwd
        || saved
            .canonicalize()
            .ok()
            .zip(cwd.canonicalize().ok())
            .is_some_and(|(saved, cwd)| saved == cwd)
}

#[derive(Deserialize)]
struct CopilotContext {
    cwd: PathBuf,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopilotData {
    session_id: Option<String>,
    context: Option<CopilotContext>,
    cwd: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopilotEvent {
    #[serde(rename = "type")]
    kind: String,
    agent_id: Option<String>,
    #[serde(default)]
    data: CopilotData,
}

pub(super) fn verify_copilot(id: &str, cwd: &Path) -> Result<(), String> {
    check_id("copilot", id)?;
    let root = crate::integration::copilot_dir()
        .map_err(|_| unavailable("copilot", id, "cannot locate Copilot home"))?;
    let path = root.join("session-state").join(id).join("events.jsonl");
    let reader = open("copilot", id, &path)?;
    let mut events = serde_json::Deserializer::from_reader(reader).into_iter::<CopilotEvent>();
    let first = events
        .next()
        .transpose()
        .map_err(|_| invalid("copilot", id, "malformed event log"))?
        .ok_or_else(|| invalid("copilot", id, "empty event log"))?;
    if first.kind != "session.start" || first.data.session_id.as_deref() != Some(id) {
        return Err(invalid(
            "copilot",
            id,
            "session.start identity does not match",
        ));
    }
    let mut saved_cwd = first.data.context.map(|context| context.cwd);
    // SDK event schema makes start/resume context optional. Do not invent a mandatory
    // workspace sidecar. Track subsequent main-agent directory changes when recorded.
    for event in events {
        let event = event.map_err(|_| invalid("copilot", id, "malformed event log"))?;
        if event.agent_id.is_some() {
            continue;
        }
        match event.kind.as_str() {
            "session.start" => {
                if event.data.session_id.as_deref() != Some(id) {
                    return Err(invalid("copilot", id, "conflicting session identity"));
                }
                if let Some(context) = event.data.context {
                    saved_cwd = Some(context.cwd);
                }
            }
            "session.resume" => {
                if let Some(context) = event.data.context {
                    saved_cwd = Some(context.cwd);
                }
            }
            "session.context_changed" => saved_cwd = event.data.cwd,
            _ => {}
        }
    }
    if saved_cwd.is_some_and(|saved| !same_directory(&saved, cwd)) {
        return Err(invalid("copilot", id, "working directory does not match"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KimiIndexEntry {
    session_id: String,
    session_dir: PathBuf,
    work_dir: PathBuf,
}

#[derive(Default, Deserialize)]
struct KimiCustom {
    cwd: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KimiState {
    id: Option<String>,
    cwd: Option<PathBuf>,
    work_dir: Option<PathBuf>,
    custom: Option<KimiCustom>,
}

impl KimiState {
    fn cwd(&self) -> Option<&Path> {
        self.cwd
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(|| {
                self.work_dir
                    .as_deref()
                    .filter(|path| !path.as_os_str().is_empty())
            })
            .or_else(|| {
                self.custom
                    .as_ref()
                    .and_then(|custom| custom.cwd.as_deref())
            })
    }
}

#[derive(Deserialize)]
struct KimiWireRecord {
    #[serde(rename = "type")]
    kind: String,
    protocol_version: Option<String>,
    created_at: Option<f64>,
}

fn verify_kimi_record(id: &str, cwd: &Path, directory: &Path, indexed: bool) -> Result<(), String> {
    if directory.file_name().and_then(|name| name.to_str()) != Some(id) {
        return Err(invalid(
            "kimi",
            id,
            "indexed session directory identity does not match",
        ));
    }
    // Stable 2.4.1 reads state.json first, then the older session-meta/state.json.
    let state_path = directory.join("state.json");
    let state_path = if !state_path.exists() {
        directory.join("session-meta/state.json")
    } else {
        state_path
    };
    let state: KimiState = serde_json::from_reader(open("kimi", id, &state_path)?)
        .map_err(|_| invalid("kimi", id, "malformed session state"))?;
    if state.id.as_deref().is_some_and(|saved| saved != id) {
        return Err(invalid("kimi", id, "session state identity does not match"));
    }
    match state.cwd() {
        Some(saved) if !same_directory(saved, cwd) => {
            return Err(invalid("kimi", id, "working directory does not match"));
        }
        None if !indexed => {
            return Err(invalid(
                "kimi",
                id,
                "session state has no working directory",
            ));
        }
        _ => {}
    }
    let reader = open("kimi", id, &directory.join("agents/main/wire.jsonl"))?;
    // Upstream isWireRecord requires an object with a string type. Other fields,
    // including potentially large message bodies, are skipped by serde.
    let mut any = false;
    for record in serde_json::Deserializer::from_reader(reader).into_iter::<KimiWireRecord>() {
        let record = record.map_err(|_| invalid("kimi", id, "malformed main-agent wire record"))?;
        if !any
            && record.kind == "metadata"
            && (record.protocol_version.is_none() || record.created_at.is_none())
        {
            return Err(invalid("kimi", id, "malformed main-agent wire metadata"));
        }
        any = true;
    }
    if !any {
        return Err(invalid("kimi", id, "empty main-agent wire log"));
    }
    Ok(())
}

pub(super) fn verify_kimi(id: &str, cwd: &Path) -> Result<(), String> {
    check_id("kimi", id)?;
    let root = crate::integration::kimi_dir()
        .map_err(|_| unavailable("kimi", id, "cannot locate Kimi home"))?;
    let native = find_kimi_native_directory(&root, id)?;
    let native_error = if let Some(directory) = &native {
        match verify_kimi_record(id, cwd, directory, false) {
            Ok(()) => return Ok(()),
            Err(error) => Some(error),
        }
    } else {
        None
    };
    // Stable 2.4.1 uses the old index only as a workspace hint; a stale or
    // malformed hint must not hide an exact, valid native filesystem session.
    let index_path = root.join("session_index.jsonl");
    let mut indexed = None;
    if index_path.exists() {
        let reader = open("kimi", id, &index_path)?;
        for entry in serde_json::Deserializer::from_reader(reader).into_iter::<KimiIndexEntry>() {
            let entry = entry.map_err(|_| invalid("kimi", id, "malformed session index"))?;
            if entry.session_id != id {
                continue;
            }
            if !same_directory(&entry.work_dir, cwd) {
                return Err(invalid(
                    "kimi",
                    id,
                    "indexed working directory does not match",
                ));
            }
            if !entry.session_dir.is_absolute() {
                return Err(invalid(
                    "kimi",
                    id,
                    "indexed session directory is not absolute",
                ));
            }
            if indexed
                .as_ref()
                .is_some_and(|saved| saved != &entry.session_dir)
            {
                return Err(invalid("kimi", id, "ambiguous indexed session identity"));
            }
            indexed = Some(entry.session_dir);
        }
    }
    if let Some(directory) = indexed {
        return verify_kimi_record(id, cwd, &directory, true);
    }
    Err(native_error.unwrap_or_else(|| unavailable("kimi", id, "no matching native session")))
}

fn find_kimi_native_directory(root: &Path, id: &str) -> Result<Option<PathBuf>, String> {
    // Only inspect the exact requested ID in each native workspace bucket.
    let buckets = match std::fs::read_dir(root.join("sessions")) {
        Ok(buckets) => buckets,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unavailable("kimi", id, "cannot read native sessions")),
    };
    let mut found = None;
    for bucket in buckets {
        let bucket =
            bucket.map_err(|_| unavailable("kimi", id, "cannot read session directory"))?;
        if !bucket
            .file_type()
            .map_err(|_| unavailable("kimi", id, "cannot read workspace directory"))?
            .is_dir()
        {
            continue;
        }
        let directory = bucket.path().join(id);
        if !directory.is_dir() {
            continue;
        }
        if found.is_some() {
            return Err(invalid("kimi", id, "ambiguous native session identity"));
        }
        found = Some(directory);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct Fixture {
        root: PathBuf,
        variable: &'static str,
        previous: Option<OsString>,
    }

    impl Fixture {
        fn new(variable: &'static str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "trailboss-closed-local-{}-{}",
                std::process::id(),
                variable
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("project")).unwrap();
            let previous = std::env::var_os(variable);
            std::env::set_var(variable, &root);
            Self {
                root,
                variable,
                previous,
            }
        }

        fn cwd(&self) -> PathBuf {
            self.root.join("project")
        }

        fn copilot(&self, id: &str, saved_id: &str) -> PathBuf {
            let directory = self.root.join("session-state").join(id);
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("events.jsonl");
            std::fs::write(
                &path,
                format!(
                    "{}\n",
                    serde_json::json!({
                        "type":"session.start", "id":"event-id", "parentId":null,
                        "timestamp":"2026-11-04T00:00:00Z",
                        "data":{"sessionId":saved_id,"version":1,"producer":"copilot-agent",
                            "copilotVersion":"1.0.0","startTime":"2026-11-04T00:00:00Z",
                            "context":{"cwd":self.cwd()}}
                    })
                ),
            )
            .unwrap();
            path
        }

        fn kimi(&self, id: &str, indexed: bool) -> PathBuf {
            let directory = self.root.join("sessions/wm_fixture").join(id);
            std::fs::create_dir_all(directory.join("agents/main")).unwrap();
            std::fs::write(
                directory.join("state.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id":id,"version":2,"cwd":self.cwd(),"title":"Fixture",
                    "createdAt":1,"updatedAt":1,"archived":false
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(
                directory.join("agents/main/wire.jsonl"),
                "{\"type\":\"metadata\",\"protocol_version\":\"1.5\",\"created_at\":1}\n",
            )
            .unwrap();
            if indexed {
                std::fs::write(
                    self.root.join("session_index.jsonl"),
                    format!(
                        "{}\n",
                        serde_json::json!({
                            "sessionId":id,"sessionDir":directory,"workDir":self.cwd()
                        })
                    ),
                )
                .unwrap();
            }
            directory
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.variable, value),
                None => std::env::remove_var(self.variable),
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn copilot_accepts_exact_native_identity_without_modifying_events() {
        let _guard = crate::integration::integration_env_lock();
        let fixture = Fixture::new("COPILOT_HOME");
        let path = fixture.copilot("saved", "saved");
        let before = std::fs::read(&path).unwrap();
        assert_eq!(verify_copilot("saved", &fixture.cwd()), Ok(()));
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn copilot_rejects_missing_wrong_identity_wrong_cwd_and_corrupt_logs() {
        let _guard = crate::integration::integration_env_lock();
        let fixture = Fixture::new("COPILOT_HOME");
        assert!(verify_copilot("missing", &fixture.cwd())
            .unwrap_err()
            .contains("unavailable"));
        let path = fixture.copilot("saved", "different");
        assert!(verify_copilot("saved", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
        fixture.copilot("saved", "saved");
        assert!(verify_copilot("saved", &fixture.root)
            .unwrap_err()
            .contains("working directory"));
        std::fs::write(path, "not json\n").unwrap();
        assert!(verify_copilot("saved", &fixture.cwd())
            .unwrap_err()
            .contains("invalid"));
    }

    #[test]
    fn copilot_uses_latest_recorded_main_directory_and_accepts_optional_start_context() {
        let _guard = crate::integration::integration_env_lock();
        let fixture = Fixture::new("COPILOT_HOME");
        let path = fixture.copilot("saved", "saved");
        let without_context = "{\"type\":\"session.start\",\"data\":{\"sessionId\":\"saved\"}}\n";
        std::fs::write(&path, without_context).unwrap();
        assert_eq!(verify_copilot("saved", &fixture.cwd()), Ok(()));
        let records = format!(
            "{}{}\n{}\n",
            without_context,
            serde_json::json!({"type":"assistant.message","data":{"content":"x".repeat(64 * 1024)}}),
            serde_json::json!({"type":"session.context_changed","data":{"cwd":fixture.cwd()}})
        );
        std::fs::write(&path, records).unwrap();
        assert_eq!(verify_copilot("saved", &fixture.cwd()), Ok(()));
        assert!(verify_copilot("saved", &fixture.root)
            .unwrap_err()
            .contains("working directory"));
    }

    #[test]
    fn kimi_accepts_documented_index_without_invented_workspace_or_state_identity_fields() {
        let _guard = crate::integration::integration_env_lock();
        let fixture = Fixture::new("KIMI_CODE_HOME");
        let directory = fixture.kimi("saved", true);
        std::fs::write(
            directory.join("state.json"),
            "{\"title\":\"Fixture\",\"createdAt\":1}",
        )
        .unwrap();
        let index = std::fs::read(fixture.root.join("session_index.jsonl")).unwrap();
        let state = std::fs::read(directory.join("state.json")).unwrap();
        let wire = std::fs::read(directory.join("agents/main/wire.jsonl")).unwrap();
        assert_eq!(verify_kimi("saved", &fixture.cwd()), Ok(()));
        assert_eq!(
            std::fs::read(fixture.root.join("session_index.jsonl")).unwrap(),
            index
        );
        assert_eq!(std::fs::read(directory.join("state.json")).unwrap(), state);
        assert_eq!(
            std::fs::read(directory.join("agents/main/wire.jsonl")).unwrap(),
            wire
        );
    }

    #[test]
    fn kimi_accepts_stable_native_filesystem_sessions_without_a_jsonl_index() {
        let _guard = crate::integration::integration_env_lock();
        let fixture = Fixture::new("KIMI_CODE_HOME");
        let directory = fixture.kimi("saved", false);
        assert_eq!(verify_kimi("saved", &fixture.cwd()), Ok(()));
        assert!(verify_kimi("saved", &fixture.root)
            .unwrap_err()
            .contains("working directory"));
        std::fs::write(directory.join("state.json"), "{\"id\":\"different\"}").unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
        std::fs::create_dir_all(directory.join("session-meta")).unwrap();
        std::fs::remove_file(directory.join("state.json")).unwrap();
        std::fs::write(
            directory.join("session-meta/state.json"),
            serde_json::to_vec(
                &serde_json::json!({"custom":{"cwd":fixture.cwd()},"title":"Legacy metadata"}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(verify_kimi("saved", &fixture.cwd()), Ok(()));
        std::fs::write(directory.join("session-meta/state.json"), "{}").unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("no working directory"));
    }

    #[test]
    fn kimi_native_session_survives_stale_or_corrupt_legacy_index_hints() {
        let _guard = crate::integration::integration_env_lock();
        let fixture = Fixture::new("KIMI_CODE_HOME");
        let directory = fixture.kimi("saved", false);
        let index_path = fixture.root.join("session_index.jsonl");
        std::fs::write(
            &index_path,
            format!(
                "{}\n",
                serde_json::json!({
                    "sessionId":"saved",
                    "sessionDir":fixture.root.join("sessions/old-workspace/saved"),
                    "workDir":fixture.root.join("old-project")
                })
            ),
        )
        .unwrap();
        assert_eq!(verify_kimi("saved", &fixture.cwd()), Ok(()));
        std::fs::write(&index_path, "not-json\n").unwrap();
        assert_eq!(verify_kimi("saved", &fixture.cwd()), Ok(()));
        assert!(directory.join("state.json").is_file());
    }

    #[test]
    fn kimi_rejects_wrong_index_identity_cwd_missing_records_and_malformed_data() {
        let _guard = crate::integration::integration_env_lock();
        let fixture = Fixture::new("KIMI_CODE_HOME");
        assert!(verify_kimi("missing", &fixture.cwd())
            .unwrap_err()
            .contains("unavailable"));
        let directory = fixture.kimi("saved", true);
        assert!(verify_kimi("other", &fixture.cwd())
            .unwrap_err()
            .contains("unavailable"));
        assert!(verify_kimi("saved", &fixture.root)
            .unwrap_err()
            .contains("working directory"));
        std::fs::write(fixture.root.join("session_index.jsonl"), format!("{}\n", serde_json::json!({
            "sessionId":"saved","sessionDir":directory.with_file_name("different"),"workDir":fixture.cwd()
        }))).unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
        fixture.kimi("saved", true);
        std::fs::remove_file(directory.join("state.json")).unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("unavailable"));
        fixture.kimi("saved", true);
        std::fs::remove_file(directory.join("agents/main/wire.jsonl")).unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("unavailable"));
        fixture.kimi("saved", true);
        std::fs::write(directory.join("state.json"), "not-json").unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("invalid"));
        fixture.kimi("saved", true);
        std::fs::write(directory.join("agents/main/wire.jsonl"), "not-json\n").unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("invalid"));
        std::fs::write(
            directory.join("agents/main/wire.jsonl"),
            "{\"type\":\"metadata\"}\n",
        )
        .unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("wire metadata"));
        std::fs::remove_dir_all(&directory).unwrap();
        std::fs::write(fixture.root.join("session_index.jsonl"), "not-json\n").unwrap();
        assert!(verify_kimi("saved", &fixture.cwd())
            .unwrap_err()
            .contains("index"));
    }
}
