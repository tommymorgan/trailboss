use super::{protocol::*, storage};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    path::Path,
};
const MAX_PACKET: usize = 64 * 1024 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
struct Frame<T> {
    boot: String,
    descriptors: usize,
    value: T,
}
#[cfg(target_os = "linux")]
pub(super) fn peer(stream: &UnixStream) -> io::Result<(u32, u64)> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&cred) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut _,
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if cred.uid != unsafe { libc::geteuid() } || cred.pid <= 0 {
        return Err(storage::denied());
    }
    let pid = cred.pid as u32;
    Ok((pid, storage::process_start(pid)?))
}
#[cfg(target_os = "macos")]
pub(super) fn peer(stream: &UnixStream) -> io::Result<(u32, u64)> {
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
        || uid != unsafe { libc::geteuid() }
    {
        return Err(storage::denied());
    }
    let mut pid: libc::pid_t = 0;
    let mut size = std::mem::size_of_val(&pid) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut pid as *mut _ as *mut _,
            &mut size,
        )
    } != 0
        || pid <= 0
    {
        return Err(storage::denied());
    }
    Ok((pid as u32, storage::process_start(pid as u32)?))
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn peer(_stream: &UnixStream) -> io::Result<(u32, u64)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "protected transfer peer identity unavailable",
    ))
}
pub(super) fn send<T: Serialize>(
    stream: &mut UnixStream,
    boot: &str,
    value: T,
    fds: &[OwnedFd],
) -> io::Result<()> {
    let bytes = serde_json::to_vec(&Frame {
        boot: boot.to_string(),
        descriptors: fds.len(),
        value,
    })
    .map_err(io::Error::other)?;
    if bytes.len() > MAX_PACKET {
        return Err(storage::denied());
    }
    stream.write_all(&(bytes.len() as u64).to_be_bytes())?;
    stream.write_all(&bytes)?;
    super::super::handoff::send_fds(
        stream,
        &fds.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>(),
    )
}
pub(super) fn receive<T: DeserializeOwned>(
    stream: &mut UnixStream,
    boot: &str,
) -> io::Result<(T, Vec<OwnedFd>)> {
    let mut len = [0; 8];
    stream.read_exact(&mut len)?;
    let len = u64::from_be_bytes(len) as usize;
    if len > MAX_PACKET {
        return Err(storage::denied());
    }
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes)?;
    let frame: Frame<T> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if frame.boot != boot || frame.descriptors > 65536 {
        return Err(storage::denied());
    }
    let fds: Vec<OwnedFd> = super::super::handoff::recv_fds(stream, frame.descriptors)?
        .into_iter()
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
        .collect();
    for fd in &fds {
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((frame.value, fds))
}
pub(crate) fn call(
    core: &CoreRegistration,
    request: CoreRequest,
    descriptors: Vec<OwnedFd>,
) -> io::Result<CorePacket> {
    call_timeout(
        core,
        request,
        descriptors,
        std::time::Duration::from_secs(15),
    )
}
pub(super) fn call_timeout(
    core: &CoreRegistration,
    request: CoreRequest,
    descriptors: Vec<OwnedFd>,
    timeout: std::time::Duration,
) -> io::Result<CorePacket> {
    storage::validate_registration(core)?;
    let mut stream = connect_timeout(&core.control_endpoint, timeout)?;
    let (pid, start) = peer(&stream)?;
    if pid != core.pid || start != core.start {
        return Err(storage::denied());
    }
    if matches!(&request, CoreRequest::Forward { .. }) {
        stream.set_read_timeout(None)?;
    }
    send(&mut stream, &core.boot, request, &descriptors)?;
    let (reply, descriptors): (Result<CoreReply, String>, _) = receive(&mut stream, &core.boot)?;
    Ok(CorePacket {
        reply: reply.map_err(io::Error::other)?,
        descriptors,
    })
}
pub(super) fn connect(path: &Path) -> io::Result<UnixStream> {
    connect_timeout(path, std::time::Duration::from_secs(15))
}
/// Connect to a protected, unchanged socket; the caller must authenticate its peer.
/// Adopted listeners retain listen-time credentials even after that Core exits.
pub(super) fn connect_timeout(path: &Path, timeout: std::time::Duration) -> io::Result<UnixStream> {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let deadline = std::time::Instant::now() + timeout;
    storage::validate(path, true)?;
    let before = std::fs::symlink_metadata(path)?;
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix coordination socket path is too long",
        ));
    }
    for (target, byte) in address.sun_path.iter_mut().zip(bytes) {
        *target = *byte as _;
    }
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of_val(&address) as _;
    }
    #[cfg(target_os = "linux")]
    let kind = libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK;
    #[cfg(not(target_os = "linux"))]
    let kind = libc::SOCK_STREAM;
    let raw = unsafe { libc::socket(libc::AF_UNIX, kind, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe { libc::fcntl(raw, libc::F_SETFL, libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "coordination socket connect timed out",
            ));
        }
        let result = unsafe {
            libc::connect(
                raw,
                &address as *const _ as *const libc::sockaddr,
                std::mem::size_of_val(&address) as _,
            )
        };
        if result == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EISCONN) => break,
            Some(libc::EAGAIN) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            Some(libc::EINTR) => continue,
            Some(libc::EINPROGRESS) | Some(libc::EALREADY) => {}
            _ => return Err(error),
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let mut poll = libc::pollfd {
            fd: raw,
            events: libc::POLLOUT,
            revents: 0,
        };
        let ready = unsafe {
            libc::poll(
                &mut poll,
                1,
                remaining.as_millis().max(1).min(i32::MAX as u128) as i32,
            )
        };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "coordination socket connect timed out",
            ));
        }
        let mut error: libc::c_int = 0;
        let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut error as *mut _ as *mut _,
                &mut size,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        break;
    }
    if unsafe { libc::fcntl(raw, libc::F_SETFL, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = UnixStream::from(fd);
    let after = std::fs::symlink_metadata(path)?;
    if before.ino() != after.ino() || before.dev() != after.dev() {
        return Err(storage::denied());
    }
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "coordination socket connect timed out",
        ));
    }
    stream.set_read_timeout(Some(remaining))?;
    stream.set_write_timeout(Some(remaining))?;
    Ok(stream)
}
