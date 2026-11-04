use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use serde::Deserialize;

// Native contract pinned to xai-org/grok-build 2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8:
// crates/codegen/xai-dirs/src/lib.rs (GROK_HOME, not GROK_CONFIG_DIR);
// crates/codegen/xai-grok-config/src/paths.rs (cwd encoding);
// crates/codegen/xai-grok-shell/src/session/persistence.rs (session_exists_for_cwd);
// crates/codegen/xai-grok-shared/src/session/info.rs (persisted identity).
// summary.json is the local availability marker. Neither transcript file is
// required by that predicate; this is not a full restore/credential preflight.
#[derive(Deserialize)]
struct Summary {
    info: Info,
}

#[derive(Deserialize)]
struct Info {
    id: String,
    cwd: String,
}

fn encoded_cwd(cwd: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut result = String::with_capacity(cwd.len());
    for byte in cwd.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            result.push(char::from(byte));
        } else {
            result.push('%');
            result.push(char::from(HEX[usize::from(byte >> 4)]));
            result.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    result
}

pub(super) fn verify_grok(id: &str, cwd: &Path) -> Result<(), String> {
    if id.is_empty() || matches!(id, "." | "..") || id.contains(['/', '\\', '\0']) {
        return Err("Saved Grok conversation has invalid native identity".into());
    }
    let cwd = cwd
        .to_str()
        .ok_or("Saved Grok conversation cwd is not UTF-8")?;
    let encoded = encoded_cwd(cwd);
    if encoded.len() > 255 {
        return Err("Saved Grok conversation cwd requires the native BLAKE3 long-path directory encoder; no approved encoder is available".into());
    }
    let home = match std::env::var_os("GROK_HOME").filter(|value| !value.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => {
            let home = crate::integration::home_dir().map_err(|error| error.to_string())?;
            std::fs::canonicalize(&home).unwrap_or(home).join(".grok")
        }
    };
    let path = home
        .join("sessions")
        .join(encoded)
        .join(id)
        .join("summary.json");
    let file = File::open(&path)
        .map_err(|_| format!("Saved Grok conversation is unavailable: {}", path.display()))?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(format!(
            "Saved Grok conversation is unavailable: {}",
            path.display()
        ));
    }
    // Serde skips all non-identity fields without allocating their potentially
    // large summaries, prompts, or message bodies. No arbitrary byte ceiling.
    let summary: Summary = serde_json::from_reader(BufReader::new(file))
        .map_err(|_| "Saved Grok conversation has invalid native metadata".to_string())?;
    if summary.info.id != id {
        return Err("Saved Grok conversation has mismatched native identity".into());
    }
    if summary.info.cwd != cwd {
        return Err("Saved Grok conversation has mismatched native cwd".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct Fixture {
        root: PathBuf,
        previous: Option<OsString>,
    }

    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("trailboss-closed-grok-{}", std::process::id()));
            std::fs::create_dir_all(root.join("project")).unwrap();
            let previous = std::env::var_os("GROK_HOME");
            std::env::set_var("GROK_HOME", root.join("grok"));
            Self { root, previous }
        }

        fn cwd(&self) -> PathBuf {
            self.root.join("project")
        }

        fn summary(&self, id: &str, info_id: &str, cwd: &Path) -> PathBuf {
            let directory = self
                .root
                .join("grok/sessions")
                .join(encoded_cwd(cwd.to_str().unwrap()))
                .join(id);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join("summary.json"),
                serde_json::json!({
                    "info": {"id": info_id, "cwd": cwd}, "session_summary": "fixture",
                    "created_at": "2026-11-04T00:00:00Z", "updated_at": "2026-11-04T00:00:00Z",
                    "num_messages": 0, "current_model_id": "grok-code"
                })
                .to_string(),
            )
            .unwrap();
            directory
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("GROK_HOME", value),
                None => std::env::remove_var("GROK_HOME"),
            }
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn persisted_session_requires_exact_native_record_identity_and_cwd() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        assert!(verify_grok("missing", &fixture.cwd())
            .unwrap_err()
            .contains("unavailable"));
        let directory = fixture.summary("session-one", "session-one", &fixture.cwd());
        // Native local availability does not require updates.jsonl or chat_history.jsonl.
        assert_eq!(verify_grok("session-one", &fixture.cwd()), Ok(()));
        assert!(verify_grok("session-two", &fixture.cwd()).is_err());
        assert!(verify_grok("session-one", &fixture.root.join("other")).is_err());
        fixture.summary("session-one", "other", &fixture.cwd());
        assert!(verify_grok("session-one", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
        std::fs::write(
            directory.join("summary.json"),
            r#"{"info":{"id":"session-one","cwd":"/wrong"}}"#,
        )
        .unwrap();
        assert!(verify_grok("session-one", &fixture.cwd())
            .unwrap_err()
            .contains("cwd"));
        for invalid in ["", ".", "..", "../session-one", "a/b", "a\\b"] {
            assert!(verify_grok(invalid, &fixture.cwd())
                .unwrap_err()
                .contains("identity"));
        }
    }

    #[test]
    fn native_cwd_encoding_preserves_unicode_spaces_and_reserved_characters() {
        assert_eq!(
            encoded_cwd("/tmp/a b+中~._-"),
            "%2Ftmp%2Fa%20b%2B%E4%B8%AD~._-"
        );
    }

    #[test]
    fn long_cwd_does_not_accept_an_unverified_slug_hash_bucket() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        let cwd = PathBuf::from(format!("/tmp/{}", "中".repeat(30)));
        let bucket = fixture
            .root
            .join("grok/sessions/workspace-0000000000000000");
        std::fs::create_dir_all(bucket.join("long")).unwrap();
        std::fs::write(bucket.join(".cwd"), cwd.to_str().unwrap()).unwrap();
        std::fs::write(
            bucket.join("long/summary.json"),
            serde_json::json!({
                "info": {"id":"long", "cwd":cwd}
            })
            .to_string(),
        )
        .unwrap();
        assert!(verify_grok("long", &cwd).unwrap_err().contains("BLAKE3"));
    }

    #[test]
    fn large_native_message_fields_do_not_prevent_metadata_availability() {
        use std::io::Write;
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        let directory = fixture.summary("large", "large", &fixture.cwd());
        let mut file = File::create(directory.join("summary.json")).unwrap();
        write!(
            file,
            "{{\"info\":{{\"id\":\"large\",\"cwd\":{}}},\"session_summary\":\"",
            serde_json::to_string(&fixture.cwd()).unwrap()
        )
        .unwrap();
        for _ in 0..4096 {
            file.write_all(&[b'x'; 4096]).unwrap();
        }
        file.write_all(b"\"}").unwrap();
        drop(file);
        assert_eq!(verify_grok("large", &fixture.cwd()), Ok(()));
    }
}
