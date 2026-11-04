use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;

#[derive(Default, Deserialize)]
struct Settings {
    #[serde(rename = "preferredBackendMode")]
    preferred_backend_mode: Option<String>,
    #[serde(default)]
    env: SettingsEnv,
}

#[derive(Default, Deserialize)]
struct SettingsEnv {
    #[serde(rename = "LETTA_BASE_URL")]
    base_url: Option<String>,
}

#[derive(Deserialize)]
struct Conversation {
    id: String,
    agent_id: Option<String>,
    #[serde(default)]
    agent_free: bool,
}

#[derive(Deserialize)]
struct Manifest {
    schema_version: u64,
    message_format: String,
    provider_stack: String,
}

#[derive(Deserialize)]
struct SessionHeader {
    #[serde(rename = "type")]
    kind: String,
    version: u64,
    id: String,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = File::open(path).map_err(|_| {
        format!(
            "Saved Letta conversation is unavailable: {}",
            path.display()
        )
    })?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(format!(
            "Saved Letta conversation is unavailable: {}",
            path.display()
        ));
    }
    // Unknown fields (including prompts and message bodies) are skipped by serde.
    serde_json::from_reader(BufReader::new(file)).map_err(|_| {
        format!(
            "Saved Letta conversation has invalid metadata: {}",
            path.display()
        )
    })
}

pub(super) fn verify_letta(id: &str, cwd: &Path) -> Result<(), String> {
    let config = match std::env::var("LETTA_CONFIG_DIR") {
        Ok(value) if !value.trim().is_empty() => {
            let path = PathBuf::from(value.trim());
            if !path.is_absolute() {
                return Err(
                    "Saved Letta conversation has invalid LETTA_CONFIG_DIR (must be absolute)"
                        .into(),
                );
            }
            path
        }
        _ => crate::integration::letta_dir().map_err(|error| error.to_string())?,
    };
    let settings_path = config.join("settings.json");
    let settings: Settings = if settings_path
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        read_json(&settings_path)?
    } else {
        Settings::default()
    };
    let default_agent = id.strip_prefix("default:");
    if id.is_empty() || id == "default" || default_agent == Some("") {
        return Err(
            "Saved Letta conversation has invalid identity; default requires its agent ID".into(),
        );
    }
    // Native startup infers the backend from an explicitly selected agent before
    // consulting the environment or saved preference. Ordinary IDs do not prove it.
    let local = if let Some(agent) = default_agent {
        agent.starts_with("agent-local-")
    } else if let Ok(value) = std::env::var("LETTA_LOCAL_BACKEND_EXPERIMENTAL") {
        value == "1" || value.eq_ignore_ascii_case("true")
    } else {
        let base_url = std::env::var("LETTA_BASE_URL")
            .ok()
            .filter(|value| !value.is_empty())
            .or(settings.env.base_url.filter(|value| !value.is_empty()));
        settings.preferred_backend_mode.as_deref() == Some("local")
            && base_url.as_deref().unwrap_or("https://api.letta.com") == "https://api.letta.com"
    };
    if !local {
        return Err("Saved Letta cloud conversation is unavailable: restore requires an authenticated, read-only conversation lookup against the selected Letta backend; no cloud lookup is configured".into());
    }
    let storage = std::env::var_os("LETTA_LOCAL_BACKEND_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.join("lc-local-backend"));
    let storage = if storage.is_absolute() {
        storage
    } else {
        cwd.join(storage)
    };
    let key = if default_agent.is_some() {
        id.to_owned()
    } else {
        format!("conversation:{id}")
    };
    let directory = storage
        .join("conversations")
        .join(URL_SAFE_NO_PAD.encode(key));
    let conversation: Conversation = read_json(&directory.join("conversation.json"))?;
    let expected_id = if default_agent.is_some() {
        "default"
    } else {
        id
    };
    if conversation.id != expected_id
        || default_agent.is_some_and(|agent| conversation.agent_id.as_deref() != Some(agent))
        || (conversation.agent_id.is_none() && !conversation.agent_free)
        || conversation.agent_id.as_deref().is_some_and(str::is_empty)
    {
        return Err("Saved Letta conversation has invalid or mismatched native identity".into());
    }
    let messages_path = directory.join("messages.jsonl");
    let manifest_path = directory.join("manifest.json");
    let messages = match File::open(&messages_path) {
        Ok(file) if file.metadata().is_ok_and(|metadata| metadata.is_file()) => Some(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        _ => {
            return Err(format!(
                "Saved Letta conversation transcript is unavailable: {}",
                messages_path.display()
            ))
        }
    };
    let mut messages = messages.map(BufReader::new);
    let has_messages = match messages.as_mut() {
        Some(reader) => {
            let size = reader
                .get_ref()
                .metadata()
                .map_err(|error| error.to_string())?
                .len();
            let bytes = reader.fill_buf().map_err(|error| error.to_string())?;
            size > 4096 || bytes.iter().any(|byte| !byte.is_ascii_whitespace())
        }
        None => false,
    };
    if !manifest_path
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return if has_messages {
            Err("Saved Letta conversation transcript requires native local-backend migrate-transcripts before restore".into())
        } else {
            Ok(())
        };
    }
    let manifest: Manifest = read_json(&manifest_path)?;
    let current =
        manifest.schema_version == 2 && manifest.message_format == "pi-session-entry-jsonl";
    let legacy = manifest.schema_version == 1 && manifest.message_format == "pi-ai-message-jsonl";
    if (!current && !legacy) || manifest.provider_stack != "pi-ai" {
        return Err(
            "Saved Letta conversation has invalid or unsupported transcript manifest".into(),
        );
    }
    if current && has_messages {
        // Deserialize only the first record; unknown fields are streamed past
        // without allocating their strings or materializing message rows.
        let header = serde_json::Deserializer::from_reader(messages.unwrap())
            .into_iter::<SessionHeader>()
            .next()
            .and_then(Result::ok);
        if !header.is_some_and(|header| {
            header.kind == "session" && header.version == 3 && header.id == expected_id
        }) {
            return Err(
                "Saved Letta conversation transcript has invalid or mismatched native identity"
                    .into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct Fixture {
        root: std::path::PathBuf,
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("trailboss-closed-letta-{}", std::process::id()));
            let previous = [
                "LETTA_CONFIG_DIR",
                "LETTA_LOCAL_BACKEND_DIR",
                "LETTA_LOCAL_BACKEND_EXPERIMENTAL",
                "LETTA_BASE_URL",
            ]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
            std::fs::create_dir_all(root.join("project")).unwrap();
            std::env::set_var("LETTA_CONFIG_DIR", root.join("config"));
            std::env::remove_var("LETTA_LOCAL_BACKEND_DIR");
            std::env::remove_var("LETTA_LOCAL_BACKEND_EXPERIMENTAL");
            std::env::remove_var("LETTA_BASE_URL");
            let fixture = Self { root, previous };
            fixture.settings("local");
            fixture
        }

        fn cwd(&self) -> std::path::PathBuf {
            self.root.join("project")
        }

        fn settings(&self, backend: &str) {
            std::fs::create_dir_all(self.root.join("config")).unwrap();
            std::fs::write(
                self.root.join("config/settings.json"),
                serde_json::json!({"preferredBackendMode":backend}).to_string(),
            )
            .unwrap();
        }

        fn conversation(&self, key_dir: &str, id: &str, agent: &str) -> std::path::PathBuf {
            let dir = self
                .root
                .join("config/lc-local-backend/conversations")
                .join(key_dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("conversation.json"),
                serde_json::json!({
                    "id":id,"agent_id":agent,"in_context_message_ids":[]
                })
                .to_string(),
            )
            .unwrap();
            std::fs::write(dir.join("manifest.json"), r#"{"schema_version":2,"message_format":"pi-session-entry-jsonl","provider_stack":"pi-ai","created_at":"2026-11-04T00:00:00Z"}"#).unwrap();
            std::fs::write(dir.join("messages.jsonl"), format!("{}\n", serde_json::json!({
                "type":"session","version":3,"id":id,"timestamp":"2026-11-04T00:00:00Z","cwd":self.cwd()
            }))).unwrap();
            dir
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for (key, value) in &self.previous {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn local_conversation_requires_exact_record_and_transcript_identity() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        let dir = fixture.conversation(
            "Y29udmVyc2F0aW9uOmxvY2FsLWNvbnYtb25l",
            "local-conv-one",
            "agent-local-one",
        );
        assert_eq!(verify_letta("local-conv-one", &fixture.cwd()), Ok(()));
        assert!(verify_letta("local-conv-missing", &fixture.cwd())
            .unwrap_err()
            .contains("unavailable"));
        std::fs::write(
            dir.join("conversation.json"),
            r#"{"id":"other","agent_id":"agent-local-one"}"#,
        )
        .unwrap();
        assert!(verify_letta("local-conv-one", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
        std::fs::write(
            dir.join("conversation.json"),
            r#"{"id":"local-conv-one","agent_id":"agent-local-one"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("messages.jsonl"),
            r#"{"type":"session","version":3,"id":"other","cwd":"/tmp"}"#,
        )
        .unwrap();
        assert!(verify_letta("local-conv-one", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
    }

    #[test]
    fn default_conversation_is_scoped_to_its_exact_local_agent() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        let dir = fixture.conversation(
            "ZGVmYXVsdDphZ2VudC1sb2NhbC1vbmU",
            "default",
            "agent-local-one",
        );
        assert_eq!(
            verify_letta("default:agent-local-one", &fixture.cwd()),
            Ok(())
        );
        assert!(verify_letta("default:agent-local-two", &fixture.cwd()).is_err());
        std::fs::write(
            dir.join("conversation.json"),
            r#"{"id":"default","agent_id":"agent-local-two"}"#,
        )
        .unwrap();
        assert!(verify_letta("default:agent-local-one", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
    }

    #[test]
    fn cloud_selection_never_uses_a_stale_local_record() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        fixture.conversation(
            "Y29udmVyc2F0aW9uOmxvY2FsLWNvbnYtb25l",
            "local-conv-one",
            "agent-local-one",
        );
        fixture.settings("api");
        assert!(verify_letta("local-conv-one", &fixture.cwd())
            .unwrap_err()
            .contains("authenticated"));
        std::env::set_var("LETTA_LOCAL_BACKEND_EXPERIMENTAL", "true");
        assert_eq!(verify_letta("local-conv-one", &fixture.cwd()), Ok(()));
        fixture.settings("local");
        std::env::set_var("LETTA_LOCAL_BACKEND_EXPERIMENTAL", "0");
        assert!(verify_letta("local-conv-one", &fixture.cwd()).is_err());
        std::env::remove_var("LETTA_LOCAL_BACKEND_EXPERIMENTAL");
        std::env::set_var("LETTA_BASE_URL", "http://localhost:8283");
        assert!(verify_letta("local-conv-one", &fixture.cwd()).is_err());
        assert!(verify_letta("default:agent-cloud-one", &fixture.cwd()).is_err());
    }

    #[test]
    fn relative_storage_override_resolves_from_the_restored_working_directory() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        fixture.conversation(
            "Y29udmVyc2F0aW9uOmxvY2FsLWNvbnYtb25l",
            "local-conv-one",
            "agent-local-one",
        );
        std::fs::rename(
            fixture.root.join("config/lc-local-backend"),
            fixture.cwd().join("native-store"),
        )
        .unwrap();
        std::env::set_var("LETTA_LOCAL_BACKEND_DIR", "native-store");
        assert_eq!(verify_letta("local-conv-one", &fixture.cwd()), Ok(()));
        assert!(verify_letta("local-conv-one", &fixture.root).is_err());
    }

    #[test]
    fn versioned_legacy_and_empty_conversations_remain_supported_but_unversioned_messages_require_migration(
    ) {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        let dir = fixture.conversation(
            "Y29udmVyc2F0aW9uOmxvY2FsLWNvbnYtb25l",
            "local-conv-one",
            "agent-local-one",
        );
        std::fs::write(dir.join("manifest.json"), r#"{"schema_version":1,"message_format":"pi-ai-message-jsonl","provider_stack":"pi-ai","created_at":"2026-11-04T00:00:00Z"}"#).unwrap();
        std::fs::write(dir.join("messages.jsonl"), r#"{"id":"local-msg-one","role":"user","content":[{"type":"text","text":"owned fixture"}]}"#).unwrap();
        assert_eq!(verify_letta("local-conv-one", &fixture.cwd()), Ok(()));
        std::fs::remove_file(dir.join("manifest.json")).unwrap();
        assert!(verify_letta("local-conv-one", &fixture.cwd())
            .unwrap_err()
            .contains("migrate-transcripts"));
        std::fs::remove_file(dir.join("messages.jsonl")).unwrap();
        assert_eq!(verify_letta("local-conv-one", &fixture.cwd()), Ok(()));
    }

    #[test]
    fn explicit_default_local_agent_selects_local_independently_of_saved_cloud_preference() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        fixture.conversation(
            "ZGVmYXVsdDphZ2VudC1sb2NhbC1vbmU",
            "default",
            "agent-local-one",
        );
        fixture.settings("api");
        assert_eq!(
            verify_letta("default:agent-local-one", &fixture.cwd()),
            Ok(())
        );
        assert!(verify_letta("default:agent-cloud-one", &fixture.cwd())
            .unwrap_err()
            .contains("authenticated"));
    }

    #[test]
    fn native_session_header_accepts_large_unknown_fields_and_multibyte_text_across_read_boundaries(
    ) {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new();
        let dir = fixture.conversation(
            "Y29udmVyc2F0aW9uOmxvY2FsLWNvbnYtb25l",
            "local-conv-one",
            "agent-local-one",
        );
        // The first byte of é is at offset 4095: a 4096-byte prefix is not UTF-8.
        // Native JSON parsing nevertheless accepts the complete string, and
        // localTranscriptRowsResult ignores unknown session-header fields.
        let prefix = r#"{"unused":""#;
        let header = format!(
            "{}{}é{}\",\"type\":\"session\",\"version\":3,\"id\":\"local-conv-one\",\"timestamp\":\"2026-11-04T00:00:00Z\",\"cwd\":\"/owned\"}}\n",
            prefix, "x".repeat(4095 - prefix.len()), "x".repeat(128 * 1024)
        );
        std::fs::write(dir.join("messages.jsonl"), &header).unwrap();
        assert_eq!(verify_letta("local-conv-one", &fixture.cwd()), Ok(()));
        std::fs::write(
            dir.join("messages.jsonl"),
            header.replace("\"id\":\"local-conv-one\"", "\"id\":\"different\""),
        )
        .unwrap();
        assert!(verify_letta("local-conv-one", &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
    }
}
