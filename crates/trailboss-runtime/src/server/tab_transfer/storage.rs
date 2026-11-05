use super::protocol::CoreRegistration;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
pub(super) fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "invalid protected transfer identity",
    )
}
pub(super) fn root() -> io::Result<PathBuf> {
    fs::create_dir_all(crate::config::config_dir())?;
    let root = fs::canonicalize(crate::config::config_dir())?.join("tab-transfer");
    match fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    validate(&root, false)?;
    Ok(root)
}
pub(super) fn validate(path: &Path, socket: bool) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || (socket && !m.file_type().is_socket())
    {
        return Err(denied());
    }
    Ok(())
}
pub(super) fn key(s: &str) -> String {
    s.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    validate(path, false)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(64 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
pub(super) fn atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let pid = std::process::id();
    let tmp = path.with_extension(format!(
        "{pid}-{}-{}.tmp",
        process_start(pid)?,
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    let result = (|| {
        serde_json::to_writer(&mut file, value).map_err(io::Error::other)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(path.parent().ok_or_else(denied)?)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub(super) fn process_start(pid: u32) -> io::Result<u64> {
    crate::platform::process_start_token(pid).ok_or_else(denied)
}
pub(super) fn alive(pid: u32, start: u64) -> bool {
    crate::platform::live_pane_process_group(pid, pid, start).is_some()
}
pub(super) fn registration_path(session: &str) -> io::Result<PathBuf> {
    Ok(root()?.join(format!("core-{}.json", key(session))))
}
pub(super) fn registration(session: &str) -> io::Result<CoreRegistration> {
    let r: CoreRegistration = read(&registration_path(session)?)?;
    if r.session != session {
        return Err(denied());
    }
    validate_registration(&r)?;
    Ok(r)
}
pub(super) fn validate_registration(r: &CoreRegistration) -> io::Result<()> {
    let config = fs::canonicalize(crate::config::config_dir())?;
    if r.version != super::protocol::COORDINATION_VERSION
        || r.config_root != config
        || !alive(r.pid, r.start)
    {
        return Err(denied());
    }
    for path in [&r.control_endpoint, &r.private_endpoint] {
        if path.parent() != Some(root()?.as_path()) {
            return Err(denied());
        }
    }
    validate(&r.control_endpoint, true)?;
    let expected =
        crate::session::api_socket_path_for(if r.session == crate::session::DEFAULT_SESSION_NAME {
            None
        } else {
            Some(&r.session)
        });
    if r.public_endpoint != expected {
        return Err(denied());
    }
    Ok(())
}
pub(super) fn lock(path: &Path) -> io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    validate(path, false)?;
    if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&f), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(f)
}
pub(super) fn try_lock(path: &Path) -> io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    validate(path, false)?;
    if unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(&f),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    } != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another Core already owns this session",
        ));
    }
    Ok(f)
}
pub(super) fn next_generation() -> io::Result<u64> {
    let root = root()?;
    let _lock = lock(&root.join("generation.lock"))?;
    let path = root.join("generation.json");
    let previous: u64 = match read(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let generation = previous
        .checked_add(1)
        .ok_or_else(|| io::Error::other("ownership generation exhausted"))?;
    atomic(&path, &generation)?;
    Ok(generation)
}
