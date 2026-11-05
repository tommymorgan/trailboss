mod coordinator;
mod discovery;
mod helper;
pub(crate) mod protocol;
mod storage;
mod transport;
pub(crate) use coordinator::{begin_transfer, stored_transfer_status, transfer_status};
pub(crate) use discovery::{list_destinations, TransferDestination};
pub(crate) use protocol::RouteAdmission;
use protocol::*;
use std::{
    io,
    os::{
        fd::RawFd,
        unix::{fs::PermissionsExt, net::UnixListener},
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
pub(crate) struct ControlServer {
    registration: CoreRegistration,
    stop: Arc<AtomicBool>,
    previous: Option<CoreRegistration>,
}
pub(crate) fn private_api_path(_boot: &str) -> io::Result<PathBuf> {
    let pid = std::process::id();
    Ok(storage::root()?.join(format!("api-{pid}-{}.sock", storage::process_start(pid)?)))
}
pub(crate) fn live_process(pid: u32, fd: RawFd) -> io::Result<LiveProcess> {
    live_process_on_pty(pid, crate::pty::fd::pty_identity(fd)?)
}
pub(crate) fn live_process_on_pty(pid: u32, pty: u64) -> io::Result<LiveProcess> {
    Ok(LiveProcess {
        pid,
        start: storage::process_start(pid)?,
        pty,
    })
}
impl ControlServer {
    pub(crate) fn start(
        boot: String,
        events: tokio::sync::mpsc::Sender<crate::server::client_transport::ServerEvent>,
    ) -> io::Result<Self> {
        let root = storage::root()?;
        let pid = std::process::id();
        let session = crate::session::active_name()
            .unwrap_or_else(|| crate::session::DEFAULT_SESSION_NAME.to_string());
        let _lease =
            storage::try_lock(&root.join(format!("core-{}.lock", storage::key(&session))))?;
        let importing = std::env::args().nth(2).as_deref() == Some("--handoff-import");
        let previous =
            match storage::read::<CoreRegistration>(&storage::registration_path(&session)?) {
                Ok(previous) => {
                    if previous.session != session {
                        return Err(storage::denied());
                    }
                    if storage::alive(previous.pid, previous.start) {
                        storage::validate_registration(&previous)?;
                        Some(previous)
                    } else {
                        None
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
        if !importing && previous.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another Core already owns this session",
            ));
        }
        let registration = CoreRegistration {
            version: COORDINATION_VERSION,
            session,
            boot: boot.clone(),
            pid,
            start: storage::process_start(pid)?,
            config_root: std::fs::canonicalize(crate::config::config_dir())?,
            public_endpoint: crate::session::api_socket_path_for(
                crate::session::active_name().as_deref(),
            ),
            private_endpoint: private_api_path(&boot)?,
            control_endpoint: root.join(format!("ctl-{pid}-{}.sock", storage::process_start(pid)?)),
        };
        let listener = UnixListener::bind(&registration.control_endpoint)?;
        std::fs::set_permissions(
            &registration.control_endpoint,
            std::fs::Permissions::from_mode(0o600),
        )?;
        listener.set_nonblocking(true)?;
        storage::atomic(
            &storage::registration_path(&registration.session)?,
            &registration,
        )?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let events = events.clone();
                        let boot = boot.clone();
                        std::thread::spawn(move || {
                            if let Err(error) = serve_control(stream, boot, events) {
                                tracing::warn!(%error,"private transfer control rejected");
                            }
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            registration,
            stop,
            previous,
        })
    }
    pub(crate) fn registration(&self) -> &CoreRegistration {
        &self.registration
    }
    pub(crate) fn commit_import(&mut self) {
        self.previous = None;
    }
}
fn serve_control(
    mut stream: std::os::unix::net::UnixStream,
    boot: String,
    events: tokio::sync::mpsc::Sender<crate::server::client_transport::ServerEvent>,
) -> io::Result<()> {
    transport::peer(&stream)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(15)))?;
    let (request, descriptors) = transport::receive(&mut stream, &boot)?;
    let forwarded = matches!(&request, CoreRequest::Forward { .. });
    let (tx, rx) = std::sync::mpsc::channel();
    events
        .blocking_send(crate::server::client_transport::ServerEvent::TabTransfer(
            Box::new(CoreEnvelope {
                request,
                descriptors,
                respond_to: tx,
            }),
        ))
        .map_err(|_| io::Error::other("Core event loop closed"))?;
    let packet = if forwarded {
        rx.recv().map_err(io::Error::other)?
    } else {
        rx.recv_timeout(std::time::Duration::from_secs(15))
            .map_err(io::Error::other)?
    };
    match packet {
        Ok(packet) => transport::send(
            &mut stream,
            &boot,
            Ok::<_, String>(packet.reply),
            &packet.descriptors,
        ),
        Err(error) => transport::send(&mut stream, &boot, Err::<CoreReply, _>(error), &[]),
    }
}
impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(path) = storage::registration_path(&self.registration.session) {
            let Ok(root) = storage::root() else { return };
            let Ok(_lease) = storage::lock(&root.join(format!(
                "core-{}.lock",
                storage::key(&self.registration.session)
            ))) else {
                return;
            };
            if storage::registration(&self.registration.session)
                .is_ok_and(|r| r.boot == self.registration.boot)
            {
                if let Some(previous) = self
                    .previous
                    .as_ref()
                    .filter(|r| storage::validate_registration(r).is_ok())
                {
                    let _ = storage::atomic(&path, previous);
                } else {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        let _ = std::fs::remove_file(&self.registration.control_endpoint);
    }
}
pub(crate) fn run_helper() -> io::Result<()> {
    helper::run()
}
pub(crate) fn core_running(session: &str) -> Option<bool> {
    let path = storage::registration_path(session).ok()?;
    if !path.exists() {
        let public = crate::session::api_socket_path_for(
            if session == crate::session::DEFAULT_SESSION_NAME {
                None
            } else {
                Some(session)
            },
        );
        return if retained_endpoint(&public) {
            Some(false)
        } else {
            None
        };
    }
    Some(storage::registration(session).is_ok())
}
pub(crate) fn core_endpoint(session: &str) -> io::Result<PathBuf> {
    Ok(storage::registration(session)?.private_endpoint)
}
pub(crate) fn retained_endpoint(path: &std::path::Path) -> bool {
    helper::retained(path).unwrap_or(false)
}
pub(crate) fn inherited_context() -> io::Result<Option<(PathBuf, RouteAdmission)>> {
    helper::inherited_context()
}
pub(crate) fn inherited_handles() -> io::Result<Vec<u32>> {
    helper::inherited_handles()
}
pub(crate) fn has_live_routes(path: &std::path::Path) -> io::Result<bool> {
    helper::has_live_routes(path)
}
pub(crate) fn release_retained_endpoint(path: &std::path::Path) -> io::Result<()> {
    helper::release(path)
}
pub(crate) fn reserve_inherited_handles() -> io::Result<()> {
    if let Some(max) = inherited_handles()?.into_iter().max() {
        crate::layout::PaneId::reserve_through(max)
    }
    let endpoint = crate::session::api_socket_path_for(crate::session::active_name().as_deref());
    let routes = helper::routes()?;
    crate::workspace::reserve_workspace_ids(
        routes
            .iter()
            .filter(|r| {
                r.origin.public_endpoint == endpoint
                    && storage::alive(r.target.process.pid, r.target.process.start)
            })
            .filter_map(|r| r.origin_pane.split_once(':').map(|(w, _)| w)),
    );
    Ok(())
}
pub(crate) fn refresh_owned_routes(
    owner: &CoreRegistration,
    owned: &[(RouteTarget, u64)],
) -> io::Result<()> {
    storage::validate_registration(owner)?;
    if storage::registration(&owner.session)?.boot != owner.boot {
        return Err(storage::denied());
    }
    let _lock = storage::lock(&storage::root()?.join("routes.lock"))?;
    let mut routes = helper::routes()?;
    for route in &mut routes {
        if route.owner.session != owner.session {
            continue;
        }
        if let Some((target, _)) = owned.iter().find(|(target, generation)| {
            target.terminal_id == route.target.terminal_id
                && target.process == route.target.process
                && *generation == route.generation
        }) {
            if !storage::alive(target.process.pid, target.process.start) {
                return Err(storage::denied());
            }
            route.owner = owner.clone();
            route.target = target.clone();
        }
    }
    helper::save_routes(&routes)
}
