use std::{
    fs::File,
    io::BufReader,
    path::{Component, Path, PathBuf},
};

#[derive(serde::Deserialize)]
struct SessionStart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "title")]
    _title: String,
    cwd: Option<PathBuf>,
}

fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}

pub(super) fn verify_droid(id: &str, cwd: &Path) -> Result<(), String> {
    let unavailable = || format!("Saved Droid conversation is unavailable: {id}");
    let mut components = Path::new(id).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || id.contains(['/', '\\', '\0'])
    {
        return Err("Saved Droid conversation has invalid identity".into());
    }
    let sessions = crate::integration::home_dir()
        .map_err(|_| unavailable())?
        .join(".factory/sessions");
    // Published @factory/droid-sdk 0.10.0 listSessions uses the canonical cwd
    // for its project bucket, but lexical cwd equality for legacy root records.
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let text = canonical
        .to_str()
        .ok_or("Saved Droid workspace is not UTF-8")?;
    let text = text.trim_end_matches(['/', '\\']);
    let bucket = if cfg!(windows) {
        let text = text.replace('\\', "/");
        let text = if text.as_bytes().get(1) == Some(&b':') {
            format!("{}{}", &text[..1], &text[2..])
        } else {
            text
        };
        let mut bucket = String::with_capacity(text.len() + 1);
        bucket.push('-');
        let mut separator = false;
        for character in text.chars() {
            if character == '/' {
                if !separator {
                    bucket.push('-');
                }
                separator = true;
            } else {
                bucket.push(character);
                separator = false;
            }
        }
        bucket
    } else {
        format!("-{}", text.trim_start_matches('/').replace('/', "-"))
    };
    for (directory, legacy) in [(sessions.join(bucket), false), (sessions, true)] {
        let Ok(file) = File::open(directory.join(format!("{id}.jsonl"))) else {
            continue;
        };
        if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        let header = serde_json::Deserializer::from_reader(BufReader::new(file))
            .into_iter::<SessionStart>()
            .next()
            .and_then(Result::ok);
        let Some(header) = header.filter(|header| header.kind == "session_start") else {
            continue;
        };
        if legacy
            && !header.cwd.is_some_and(|saved| {
                let saved = if saved.is_absolute() {
                    saved
                } else {
                    cwd.join(saved)
                };
                normalized(&saved) == normalized(cwd)
            })
        {
            continue;
        }
        // The native SDK takes identity from the exact filename. Its optional
        // sessionId header field is not an identity requirement.
        return Ok(());
    }
    Err(unavailable())
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    struct Fixture {
        root: PathBuf,
        previous_home: Option<OsString>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            match &self.previous_home {
                Some(home) => std::env::set_var("HOME", home),
                None => std::env::remove_var("HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    #[cfg(unix)]
    fn droid_saved_project_and_legacy_sessions_use_native_filename_identity_and_cwd_scope() {
        let _lock = crate::integration::integration_env_lock();
        let fixture = Fixture {
            root: std::env::temp_dir()
                .join(format!("trailboss-closed-droid-{}", std::process::id())),
            previous_home: std::env::var_os("HOME"),
        };
        std::fs::create_dir_all(fixture.root.join("project.with-dots")).unwrap();
        std::env::set_var("HOME", &fixture.root);
        let cwd = fixture.root.join("project.with-dots");
        let sessions = fixture.root.join(".factory/sessions");
        let bucket = format!(
            "-{}",
            cwd.canonicalize()
                .unwrap()
                .to_str()
                .unwrap()
                .trim_start_matches('/')
                .replace('/', "-")
        );
        let project = sessions.join(bucket);
        std::fs::create_dir_all(&project).unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        let path = project.join(format!("{id}.jsonl"));
        // SDK 0.10.0 takes identity from the filename, not optional sessionId.
        // Project records can omit cwd; legacy root records must match it.
        let record = serde_json::json!({"type":"session_start","title":"owned fixture","sessionId":"legacy-header-value","opaque":"x".repeat(128 * 1024)});
        std::fs::write(&path, format!("{record}\n")).unwrap();
        let verify = || super::super::verify_native_id_conversation("droid", id, &cwd);
        assert_eq!(verify(), Ok(()));
        std::fs::remove_file(path).unwrap();
        let legacy = sessions.join(format!("{id}.jsonl"));
        std::fs::write(
            &legacy,
            serde_json::json!({"type":"session_start","title":"legacy","cwd":cwd}).to_string(),
        )
        .unwrap();
        assert_eq!(verify(), Ok(()));
        std::fs::write(
            &legacy,
            r#"{"type":"session_start","title":"other workspace","cwd":"/not-the-saved-project"}"#,
        )
        .unwrap();
        assert!(verify().is_err());
        std::fs::write(
            &legacy,
            r#"{"type":"session_start","title":"missing workspace"}"#,
        )
        .unwrap();
        assert!(verify().is_err());
        std::fs::write(&legacy, "{corrupt").unwrap();
        assert!(verify().is_err());
        std::fs::remove_file(legacy).unwrap();
        assert!(verify().is_err());
    }
}
