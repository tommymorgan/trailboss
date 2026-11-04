use std::fs::File;
use std::io::BufReader;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    const ID: &str = "11111111-2222-3333-4444-555555555555";

    struct Fixture {
        root: PathBuf,
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "trailboss-project-sessions-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("home")).unwrap();
            std::fs::create_dir_all(root.join("project.with-dots")).unwrap();
            let names = [
                "HOME",
                "USERPROFILE",
                "QWEN_HOME",
                "QWEN_RUNTIME_DIR",
                "QWEN_CODE_SYSTEM_SETTINGS_PATH",
                "QWEN_CODE_SYSTEM_DEFAULTS_PATH",
            ];
            let saved = names
                .into_iter()
                .map(|name| (name, std::env::var_os(name)))
                .collect();
            std::env::set_var("HOME", root.join("home"));
            std::env::set_var("USERPROFILE", root.join("home"));
            std::env::remove_var("QWEN_HOME");
            std::env::remove_var("QWEN_RUNTIME_DIR");
            std::env::set_var("QWEN_CODE_SYSTEM_SETTINGS_PATH", root.join("system.json"));
            std::env::set_var("QWEN_CODE_SYSTEM_DEFAULTS_PATH", root.join("defaults.json"));
            Self { root, saved }
        }

        fn cwd(&self) -> PathBuf {
            self.root.join("project.with-dots")
        }

        fn transcript(&self, runtime: &Path, id: &str, cwd: &Path) {
            let slug = qwen_project_name(&self.cwd()).unwrap();
            let chats = runtime.join("projects").join(slug).join("chats");
            std::fs::create_dir_all(&chats).unwrap();
            let record = serde_json::json!({"type":"user", "sessionId":id,
                "cwd":cwd, "timestamp":"2026-11-04T00:00:00Z",
                "message":{"role":"user","parts":[{"text":"fixture"}]}});
            std::fs::write(chats.join(format!("{ID}.jsonl")), format!("{record}\n")).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn qwen_restores_only_the_saved_identity_and_workspace_in_its_effective_store() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new("identity");
        let runtime = fixture.root.join("home/.qwen");
        fixture.transcript(&runtime, ID, &fixture.cwd());
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        assert!(verify_qwen("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", &fixture.cwd()).is_err());
        fixture.transcript(
            &runtime,
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            &fixture.cwd(),
        );
        assert!(verify_qwen(ID, &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
        fixture.transcript(&runtime, ID, &fixture.root.join("other"));
        assert!(verify_qwen(ID, &fixture.cwd())
            .unwrap_err()
            .contains("workspace"));
    }

    #[test]
    fn qwen_restores_exact_identity_after_a_large_initial_message() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new("large-head");
        let runtime = fixture.root.join("home/.qwen");
        fixture.transcript(&runtime, ID, &fixture.cwd());
        let path = runtime
            .join("projects")
            .join(qwen_project_name(&fixture.cwd()).unwrap())
            .join("chats")
            .join(format!("{ID}.jsonl"));
        // Put the ignored body before the identity fields so the reader must
        // traverse it rather than stopping as soon as the identity is seen.
        let record = format!(
            "{{\"message\":{{\"parts\":[{{\"text\":{}}}]}},\"sessionId\":{},\"cwd\":{}}}\n",
            serde_json::to_string(&"x".repeat(2 * 1024 * 1024)).unwrap(),
            serde_json::to_string(ID).unwrap(),
            serde_json::to_string(&fixture.cwd()).unwrap(),
        );
        std::fs::write(&path, &record).unwrap();
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        std::fs::write(
            &path,
            record.replace(ID, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
        )
        .unwrap();
        assert!(verify_qwen(ID, &fixture.cwd())
            .unwrap_err()
            .contains("identity"));
    }

    #[test]
    fn qwen_runtime_environment_overrides_home_and_settings_without_fallback() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new("override");
        let home = fixture.root.join("custom-home");
        std::env::set_var("QWEN_HOME", &home);
        fixture.transcript(&home, ID, &fixture.cwd());
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        std::env::set_var("QWEN_RUNTIME_DIR", "runtime");
        assert!(verify_qwen(ID, &fixture.cwd()).is_err());
        fixture.transcript(&fixture.cwd().join("runtime"), ID, &fixture.cwd());
        std::fs::write(
            home.join("settings.json"),
            r#"{"advanced":{"runtimeOutputDir":"elsewhere"}}"#,
        )
        .unwrap();
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        std::env::remove_var("QWEN_RUNTIME_DIR");
        assert!(verify_qwen(ID, &fixture.cwd()).is_err());
    }

    #[test]
    fn qwen_selects_system_over_user_over_defaults_runtime_settings() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new("settings-precedence");
        let home = fixture.root.join("home/.qwen");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            fixture.root.join("defaults.json"),
            "{/* default runtime */\"advanced\":{\"runtimeOutputDir\":\"defaults\"}}",
        )
        .unwrap();
        fixture.transcript(&fixture.cwd().join("defaults"), ID, &fixture.cwd());
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        std::fs::write(
            home.join("settings.json"),
            r#"{"advanced":{"runtimeOutputDir":"user"}}"#,
        )
        .unwrap();
        assert!(verify_qwen(ID, &fixture.cwd()).is_err());
        fixture.transcript(&fixture.cwd().join("user"), ID, &fixture.cwd());
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        std::fs::write(
            fixture.root.join("system.json"),
            r#"{"advanced":{"runtimeOutputDir":"system"}}"#,
        )
        .unwrap();
        assert!(verify_qwen(ID, &fixture.cwd()).is_err());
        fixture.transcript(&fixture.cwd().join("system"), ID, &fixture.cwd());
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        std::fs::create_dir_all(fixture.cwd().join(".qwen")).unwrap();
        std::fs::write(
            fixture.cwd().join(".qwen/settings.json"),
            r#"{"advanced":{"runtimeOutputDir":"workspace"}}"#,
        )
        .unwrap();
        // System wins regardless of whether workspace settings are trusted.
        assert!(verify_qwen(ID, &fixture.cwd()).is_ok());
        std::fs::remove_file(fixture.root.join("system.json")).unwrap();
        assert!(verify_qwen(ID, &fixture.cwd())
            .unwrap_err()
            .contains("trust"));
        std::fs::remove_file(fixture.cwd().join(".qwen/settings.json")).unwrap();
        std::fs::write(
            home.join("settings.json"),
            r#"{"advanced":{"runtimeOutputDir":"${RUNTIME_ROOT}"}}"#,
        )
        .unwrap();
        assert!(verify_qwen(ID, &fixture.cwd())
            .unwrap_err()
            .contains("interpolation"));
    }

    #[test]
    fn qwen_rejects_sanitizer_collisions_and_unresolved_runtime_configuration() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture::new("configuration");
        let runtime = fixture.root.join("home/.qwen");
        let collision = fixture.root.join("project-with-dots");
        fixture.transcript(&runtime, ID, &collision);
        assert!(verify_qwen(ID, &fixture.cwd()).is_err());
        fixture.transcript(&runtime, ID, &fixture.cwd());
        std::fs::create_dir_all(fixture.cwd().join(".qwen")).unwrap();
        std::fs::write(
            fixture.cwd().join(".qwen/settings.json"),
            r#"{"advanced":{"runtimeOutputDir":"custom"}}"#,
        )
        .unwrap();
        assert!(verify_qwen(ID, &fixture.cwd()).is_err());
        std::fs::remove_file(fixture.cwd().join(".qwen/settings.json")).unwrap();
        std::fs::write(fixture.root.join("home/.env"), "QWEN_RUNTIME_DIR=custom\n").unwrap();
        assert!(verify_qwen(ID, &fixture.cwd())
            .unwrap_err()
            .contains("environment"));
    }
}

// Qwen's first JSONL record carries the session and project identity. Unknown
// fields (including message bodies) are skipped by serde without building a Value.
#[derive(Deserialize)]
struct QwenHead {
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
}

pub(super) fn verify_qwen(id: &str, cwd: &Path) -> Result<(), String> {
    if !(32..=36).contains(&id.len())
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("Qwen saved conversation identity is invalid".into());
    }
    let global = crate::integration::qwen_dir()
        .map_err(|error| format!("Qwen saved conversation unavailable: {error}"))?;
    let global = resolve_path(&global, cwd)?;
    let runtime = match std::env::var_os("QWEN_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        Some(value) => resolve_path(Path::new(&value), cwd)?,
        None => configured_qwen_runtime(&global, cwd)?,
    };
    let path = runtime
        .join("projects")
        .join(qwen_project_name(cwd)?)
        .join("chats")
        .join(format!("{id}.jsonl"));
    let file = File::open(&path)
        .map_err(|error| format!("Qwen saved conversation unavailable: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Qwen saved conversation is not a regular file".into());
    }
    let head = serde_json::Deserializer::from_reader(BufReader::new(file))
        .into_iter::<QwenHead>()
        .next()
        .ok_or("Qwen saved conversation identity record is missing")?
        .map_err(|error| format!("Qwen saved conversation identity is invalid: {error}"))?;
    if head.session_id != id {
        return Err("Qwen saved conversation identity does not match".into());
    }
    let expected = cwd
        .to_str()
        .ok_or("Qwen saved workspace path is not UTF-8")?;
    let matches = if cfg!(windows) {
        head.cwd.to_lowercase() == expected.to_lowercase()
    } else {
        head.cwd == expected
    };
    if !matches {
        return Err("Qwen saved conversation belongs to a different workspace".into());
    }
    Ok(())
}

fn qwen_project_name(cwd: &Path) -> Result<String, String> {
    let text = cwd
        .to_str()
        .ok_or("Qwen saved workspace path is not UTF-8")?;
    let text = if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.to_owned()
    };
    // JavaScript's non-Unicode regex replaces each UTF-16 code unit.
    Ok(text
        .encode_utf16()
        .map(|unit| {
            if unit <= 127 && (unit as u8).is_ascii_alphanumeric() {
                unit as u8 as char
            } else {
                '-'
            }
        })
        .collect())
}

fn resolve_path(path: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("Qwen runtime path is not UTF-8")?;
    let expanded = if text == "~" || text.starts_with("~/") || text.starts_with("~\\") {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or("Qwen home directory is unavailable")?;
        let mut path = PathBuf::from(home);
        for segment in text
            .get(2..)
            .unwrap_or("")
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
        {
            path.push(segment);
        }
        path
    } else {
        path.to_path_buf()
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn configured_qwen_runtime(global: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let system = std::env::var_os("QWEN_CODE_SYSTEM_SETTINGS_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from(r"C:\ProgramData\qwen-code\settings.json")
            } else if cfg!(target_os = "macos") {
                PathBuf::from("/Library/Application Support/QwenCode/settings.json")
            } else {
                PathBuf::from("/etc/qwen-code/settings.json")
            }
        });
    let defaults = std::env::var_os("QWEN_CODE_SYSTEM_DEFAULTS_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| system.with_file_name("system-defaults.json"));
    // Native loadSettings merges System Defaults -> User -> trusted Workspace
    // -> System. Only admit a workspace override if trust cannot change the
    // resulting root; the system override is authoritative in either case.
    let defaults = runtime_setting(&resolve_path(&defaults, cwd)?)?;
    let user = runtime_setting(&global.join("settings.json"))?;
    let workspace = runtime_setting(&cwd.join(".qwen/settings.json"))?;
    let system = runtime_setting(&resolve_path(&system, cwd)?)?;
    let lower = user.or(defaults);
    let selected = if let Some(system) = system {
        Some(system)
    } else if let Some(workspace) = workspace {
        if lower.as_deref().unwrap_or("") != workspace {
            return Err("Qwen saved conversation unavailable: runtimeOutputDir depends on native workspace trust resolution".into());
        }
        Some(workspace)
    } else {
        lower
    };
    // Native settings bootstrap can obtain QWEN_HOME/QWEN_RUNTIME_DIR from
    // dotenv files. Do not silently use the default when that input is present.
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let mut environment_files = vec![global.join(".env")];
    if let Some(home) = home {
        environment_files.push(PathBuf::from(home).join(".env"));
    }
    for ancestor in cwd.ancestors() {
        environment_files.push(ancestor.join(".env"));
        environment_files.push(ancestor.join(".qwen/.env"));
    }
    if environment_files.iter().any(|path| path.exists()) {
        return Err("Qwen saved conversation unavailable: native environment bootstrap may redirect runtime storage".into());
    }
    match selected.filter(|value| !value.is_empty()) {
        Some(value) => resolve_path(Path::new(&value), cwd),
        None => Ok(global.to_path_buf()),
    }
}

fn runtime_setting(path: &Path) -> Result<Option<String>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Qwen runtime settings unavailable: {error}")),
    };
    // Qwen strips comments, but does not permit trailing commas or other
    // JavaScript extensions before JSON.parse.
    let options = jsonc_parser::ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: false,
        allow_trailing_commas: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    };
    let root =
        jsonc_parser::cst::CstRootNode::parse(content.trim_start_matches('\u{feff}'), &options)
            .map_err(|error| format!("Qwen runtime settings invalid: {error}"))?;
    let value = root
        .value()
        .and_then(|value| value.to_serde_value())
        .ok_or("Qwen runtime settings are missing")?;
    if !value.is_object() || value.get("runtimeOutputDir").is_some() {
        return Err("Qwen runtime settings require native migration resolution".into());
    }
    if let Some(advanced) = value.get("advanced") {
        if !advanced.is_object() {
            return Err("Qwen advanced runtime settings are invalid".into());
        }
    }
    let Some(value) = value.pointer("/advanced/runtimeOutputDir") else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or("Qwen runtimeOutputDir must be a string")?;
    if value.contains('$') {
        return Err("Qwen runtimeOutputDir requires native environment interpolation".into());
    }
    Ok(Some(value.to_owned()))
}
