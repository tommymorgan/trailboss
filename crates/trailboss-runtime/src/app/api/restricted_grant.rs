//! Carries a restricted-app opt-in grant (ADR 0007) across a restart-idle
//! restart. The grant marker is bound to one process incarnation, so a restart
//! would otherwise silently revoke the paired app's access.

use super::restart_idle::ProcessIdentity;
use std::path::{Path, PathBuf};

const MARKER_LIMIT: u64 = 4096;

/// A validated marker of the process being restarted.
#[derive(Debug)]
pub(super) struct HeldGrant {
    path: PathBuf,
    identity: (u64, u64),
    grant: serde_json::Value,
}

/// Marker directory the agent's reporter reads: `/tmp/trailboss-<kind>-control-<uid>`.
pub(super) fn grant_directory(kind: &str, uid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/trailboss-{kind}-control-{uid}"))
}

fn marker_path(directory: &Path, pid: u32) -> PathBuf {
    directory.join(format!("{pid}.restricted-app.json"))
}

fn safe_directory(directory: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(directory)
        .is_ok_and(|meta| meta.is_dir() && meta.uid() == uid && meta.mode() & 0o777 == 0o700)
}

/// Reads the old process's marker with the reporter's own checks, and requires
/// it to bind exactly this terminal and the session being resumed.
pub(super) fn read_held_grant(
    directory: &Path,
    uid: u32,
    old: &ProcessIdentity,
    terminal_id: &str,
    session_path: &str,
) -> Option<HeldGrant> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    if !safe_directory(directory, uid) {
        return None;
    }
    let path = marker_path(directory, old.pid);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file()
        || meta.uid() != uid
        || meta.mode() & 0o777 != 0o600
        || meta.nlink() != 1
        || meta.len() == 0
        || meta.len() > MARKER_LIMIT
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MARKER_LIMIT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let grant: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let bound = grant["version"] == 1
        && grant["pid"] == old.pid
        && grant["proc_start_ticks"].as_str() == Some(old.started.to_string().as_str())
        && grant["terminal_id"] == terminal_id
        && grant["session_path"] == session_path
        && grant["conversation_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty());
    bound.then(|| HeldGrant {
        path,
        identity: (meta.dev(), meta.ino()),
        grant,
    })
}

/// Issues the same binding to the new process, then removes the old marker.
/// Call only after the new process is verified to have resumed the same session.
pub(super) fn carry_grant(
    directory: &Path,
    uid: u32,
    held: &HeldGrant,
    new: &ProcessIdentity,
) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    if !safe_directory(directory, uid) {
        return Err("unsafe_grant_directory".into());
    }
    let mut grant = held.grant.clone();
    grant["pid"] = new.pid.into();
    grant["proc_start_ticks"] = new.started.to_string().into();
    let target = marker_path(directory, new.pid);
    let staging = directory.join(format!(
        ".{}.restricted-app.json.{}",
        new.pid,
        std::process::id()
    ));
    let staged = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&staging)?;
        file.write_all(grant.to_string().as_bytes())?;
        file.sync_all()?;
        // Never replace a marker someone else issued for the new process.
        std::fs::hard_link(&staging, &target)
    })();
    let _ = std::fs::remove_file(&staging);
    staged.map_err(|err| format!("grant_write_failed: {err}"))?;
    // Remove the old marker only if it is still the file that was validated.
    if std::fs::symlink_metadata(&held.path)
        .is_ok_and(|meta| (meta.dev(), meta.ino()) == held.identity)
    {
        let _ = std::fs::remove_file(&held.path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    const TERMINAL: &str = "term-review";
    const SESSION: &str = "/home/user/.pi/agent/sessions/x/2026_conv-1.jsonl";

    fn directory() -> (PathBuf, u32) {
        let uid = unsafe { libc::geteuid() };
        let dir = std::env::temp_dir().join(format!(
            "trailboss-grant-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir, uid)
    }

    fn old() -> ProcessIdentity {
        ProcessIdentity {
            pid: 4100,
            started: 777,
        }
    }

    fn new() -> ProcessIdentity {
        ProcessIdentity {
            pid: 4200,
            started: 888,
        }
    }

    fn marker(pid: u32, start: &str, terminal: &str, session: &str) -> serde_json::Value {
        serde_json::json!({"version":1,"pid":pid,"proc_start_ticks":start,
            "terminal_id":terminal,"conversation_id":"conv-1","session_path":session})
    }

    fn write(dir: &Path, name: &str, body: &serde_json::Value, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&path)
            .unwrap();
        std::fs::write(&path, body.to_string()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    fn read(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn restart_carries_the_grant_to_the_new_process_and_removes_the_old_marker() {
        let (dir, uid) = directory();
        let old_path = write(
            &dir,
            "4100.restricted-app.json",
            &marker(4100, "777", TERMINAL, SESSION),
            0o600,
        );
        let held = read_held_grant(&dir, uid, &old(), TERMINAL, SESSION).unwrap();
        carry_grant(&dir, uid, &held, &new()).unwrap();
        let new_path = dir.join("4200.restricted-app.json");
        assert_eq!(read(&new_path), marker(4200, "888", TERMINAL, SESSION));
        let meta = std::fs::metadata(&new_path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::os::unix::fs::MetadataExt::nlink(&meta), 1);
        assert!(!old_path.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn grants_for_another_process_terminal_or_conversation_are_not_carried() {
        for (case, body, mode) in [
            (
                "other start ticks",
                marker(4100, "999", TERMINAL, SESSION),
                0o600,
            ),
            ("other pid", marker(4999, "777", TERMINAL, SESSION), 0o600),
            (
                "other terminal",
                marker(4100, "777", "term-other", SESSION),
                0o600,
            ),
            (
                "other session",
                marker(4100, "777", TERMINAL, "/other.jsonl"),
                0o600,
            ),
            (
                "readable by others",
                marker(4100, "777", TERMINAL, SESSION),
                0o644,
            ),
        ] {
            let (dir, uid) = directory();
            write(&dir, "4100.restricted-app.json", &body, mode);
            assert!(
                read_held_grant(&dir, uid, &old(), TERMINAL, SESSION).is_none(),
                "{case}"
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
        let (dir, uid) = directory();
        assert!(read_held_grant(&dir, uid, &old(), TERMINAL, SESSION).is_none());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        write(
            &dir,
            "4100.restricted-app.json",
            &marker(4100, "777", TERMINAL, SESSION),
            0o600,
        );
        assert!(
            read_held_grant(&dir, uid, &old(), TERMINAL, SESSION).is_none(),
            "unsafe directory"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_existing_marker_for_the_new_process_is_never_replaced() {
        let (dir, uid) = directory();
        let old_path = write(
            &dir,
            "4100.restricted-app.json",
            &marker(4100, "777", TERMINAL, SESSION),
            0o600,
        );
        let existing = marker(4200, "888", "term-other", SESSION);
        let new_path = write(&dir, "4200.restricted-app.json", &existing, 0o600);
        let held = read_held_grant(&dir, uid, &old(), TERMINAL, SESSION).unwrap();
        assert!(carry_grant(&dir, uid, &held, &new()).is_err());
        assert_eq!(read(&new_path), existing);
        assert!(old_path.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
