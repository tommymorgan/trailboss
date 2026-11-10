use super::{protocol::*, storage, transport};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
    os::{
        fd::OwnedFd,
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Route {
    pub origin: CoreRegistration,
    pub origin_pane: String,
    pub owner: CoreRegistration,
    pub target: RouteTarget,
    pub generation: u64,
    pub operation_id: String,
}
#[derive(Serialize, Deserialize)]
struct HelperIdentity {
    pid: u32,
    start: u64,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RetainedListener {
    endpoint: PathBuf,
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    listen_pid: u32,
    listen_start: u64,
}
impl RetainedListener {
    fn validate_path(&self) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        storage::validate(&self.endpoint, true)?;
        let metadata = std::fs::symlink_metadata(&self.endpoint)?;
        if (
            metadata.dev(),
            metadata.ino(),
            metadata.uid(),
            metadata.mode(),
        ) != (self.dev, self.ino, self.uid, self.mode)
        {
            return Err(storage::denied());
        }
        Ok(())
    }
}
fn retained_listeners() -> io::Result<Vec<RetainedListener>> {
    match storage::read(&storage::root()?.join("retained.json")) {
        Ok(records) => Ok(records),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}
#[derive(Serialize, Deserialize)]
enum HelperRequest {
    Ping,
    Stage(CoreRegistration),
    Activate(PathBuf),
    Cancel(PathBuf),
    Release(PathBuf),
    Validate(RetainedListener),
}
fn endpoint() -> io::Result<PathBuf> {
    Ok(storage::root()?.join("helper.sock"))
}
pub(super) fn routes() -> io::Result<Vec<Route>> {
    let path = storage::root()?.join("routes.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    storage::read(&path)
}
pub(super) fn save_routes(routes: &[Route]) -> io::Result<()> {
    storage::atomic(&storage::root()?.join("routes.json"), &routes)
}
fn live(route: &Route) -> bool {
    storage::alive(route.target.process.pid, route.target.process.start)
}
pub(super) fn has_live_routes(path: &Path) -> io::Result<bool> {
    Ok(routes()?
        .iter()
        .any(|r| r.origin.public_endpoint == path && live(r)))
}
pub(super) fn retained(path: &Path) -> io::Result<bool> {
    Ok(retained_listeners()?
        .iter()
        .any(|record| record.endpoint == path))
}
pub(super) fn validate_public_peer(
    stream: &UnixStream,
    core: &CoreRegistration,
    timeout: std::time::Duration,
) -> io::Result<()> {
    let Some(record) = retained_listeners()?
        .into_iter()
        .find(|record| record.endpoint == core.public_endpoint)
    else {
        if transport::peer(stream)? != (core.pid, core.start) {
            return Err(storage::denied());
        }
        return Ok(());
    };
    record.validate_path()?;
    validate_listener_peer(stream, &record)?;
    // The original listen-time PID/start is admitted during Stage. It may now
    // be dead or reused; SO_PEERCRED does not identify the current acceptor.
    // Only the authenticated live helper can confirm this record is active.
    request_timeout(HelperRequest::Validate(record.clone()), Vec::new(), timeout)?;
    record.validate_path()
}
#[cfg(target_os = "linux")]
fn validate_listener_peer(stream: &UnixStream, record: &RetainedListener) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&credentials) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut _ as *mut _,
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if credentials.pid <= 0
        || credentials.pid as u32 != record.listen_pid
        || credentials.uid != record.uid
        || credentials.uid != unsafe { libc::geteuid() }
    {
        return Err(storage::denied());
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn validate_listener_peer(stream: &UnixStream, record: &RetainedListener) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let mut uid = 0;
    let mut gid = 0;
    let mut pid: libc::pid_t = 0;
    let mut size = std::mem::size_of_val(&pid) as libc::socklen_t;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
        || unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                &mut pid as *mut _ as *mut _,
                &mut size,
            )
        } != 0
        || pid <= 0
        || pid as u32 != record.listen_pid
        || uid != record.uid
        || uid != unsafe { libc::geteuid() }
    {
        return Err(storage::denied());
    }
    Ok(())
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn validate_listener_peer(_stream: &UnixStream, _record: &RetainedListener) -> io::Result<()> {
    Err(storage::denied())
}
pub(super) fn inherited_handles() -> io::Result<Vec<u32>> {
    let path = crate::session::api_socket_path_for(crate::session::active_name().as_deref());
    Ok(routes()?
        .iter()
        .filter(|r| r.origin.public_endpoint == path && live(r))
        .filter_map(|r| {
            r.origin_pane
                .rsplit_once(":p")
                .map(|(_, p)| p)
                .unwrap_or(&r.origin_pane)
                .parse()
                .ok()
        })
        .collect())
}
pub(super) fn inherited_context() -> io::Result<Option<(PathBuf, RouteAdmission)>> {
    if crate::session::explicit_session_requested() {
        return Ok(None);
    }
    let Some(pane) = std::env::var("HERDR_PANE_ID").ok() else {
        return Ok(None);
    };
    let endpoint = crate::session::active_api_socket_path();
    let mut matches = routes()?
        .into_iter()
        .filter(|r| r.origin.public_endpoint == endpoint && r.origin_pane == pane && live(r));
    let Some(route) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(storage::denied());
    }
    // Reuse Core's process/session admission; matching environment text alone grants nothing.
    let pid = std::process::id();
    let start = storage::process_start(pid)?;
    if crate::platform::live_pane_process_group(route.target.process.pid, pid, start).is_none() {
        return Err(storage::denied());
    }
    storage::validate_registration(&route.owner)?;
    Ok(Some((
        route.owner.private_endpoint,
        RouteAdmission {
            terminal_id: route.target.terminal_id,
            process: route.target.process,
            generation: route.generation,
        },
    )))
}
fn request(request: HelperRequest, fds: Vec<OwnedFd>) -> io::Result<()> {
    request_timeout(request, fds, std::time::Duration::from_secs(15))
}
fn request_timeout(
    request: HelperRequest,
    fds: Vec<OwnedFd>,
    timeout: std::time::Duration,
) -> io::Result<()> {
    let mut stream = transport::connect_timeout(&endpoint()?, timeout)?;
    let identity: HelperIdentity = storage::read(&storage::root()?.join("helper.json"))?;
    if transport::peer(&stream)? != (identity.pid, identity.start) {
        return Err(storage::denied());
    }
    transport::send(&mut stream, "helper-v1", request, &fds)?;
    let (reply, _): (Result<(), String>, _) = transport::receive(&mut stream, "helper-v1")?;
    reply.map_err(io::Error::other)
}
pub(super) fn ensure() -> io::Result<()> {
    let _lock = storage::lock(&storage::root()?.join("startup.lock"))?;
    if request(HelperRequest::Ping, Vec::new()).is_ok() {
        return Ok(());
    }
    if let Ok(identity) = storage::read::<HelperIdentity>(&storage::root()?.join("helper.json")) {
        if storage::alive(identity.pid, identity.start) {
            return Err(io::Error::other("live helper cannot be reached"));
        }
    }
    let socket = endpoint()?;
    if socket.exists() {
        storage::validate(&socket, true)?;
        std::fs::remove_file(&socket)?
    }
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("__tab-transfer-helper")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if request(HelperRequest::Ping, Vec::new()).is_ok() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            return Err(io::Error::other("routing helper exited during startup"));
        }
        std::thread::sleep(std::time::Duration::from_millis(10))
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "routing helper startup timed out",
    ))
}
pub(super) fn release(path: &Path) -> io::Result<()> {
    if !retained(path)? {
        return Ok(());
    }
    request(HelperRequest::Release(path.to_path_buf()), Vec::new())
}
pub(super) fn adopt(core: &CoreRegistration) -> io::Result<()> {
    let _adoption = storage::lock(
        &storage::root()?.join(format!("front-{}.lock", storage::key(&core.session))),
    )?;
    if retained(&core.public_endpoint)? {
        return Ok(());
    }
    let helper: HelperIdentity = storage::read(&storage::root()?.join("helper.json"))?;
    // A previous adopter may have lost its Activate reply before retained.json
    // was published. The helper serializes control requests: an acknowledged
    // Cancel drains that Activate and certifies this endpoint is not active.
    // Refuse without touching the source acceptor if ownership is uncertain.
    request(
        HelperRequest::Cancel(core.public_endpoint.clone()),
        Vec::new(),
    )?;
    let packet = match transport::call(core, CoreRequest::PrepareFrontDoor, Vec::new()) {
        Ok(packet) => packet,
        Err(error) => {
            let _ = transport::call(core, CoreRequest::ResumeFrontDoor, Vec::new());
            return Err(error);
        }
    };
    if !matches!(packet.reply, CoreReply::FrontDoorReady(_)) || packet.descriptors.len() != 1 {
        let _ = transport::call(core, CoreRequest::ResumeFrontDoor, Vec::new());
        return Err(storage::denied());
    }
    if let Err(error) = request(HelperRequest::Stage(core.clone()), packet.descriptors) {
        // The preflight fenced earlier adopters, and this adopter has not
        // Activated. Stage only retains a passive descriptor, so resumption
        // is safe even if Cancel fails or EOF precedes observable helper death.
        let _ = request(
            HelperRequest::Cancel(core.public_endpoint.clone()),
            Vec::new(),
        );
        let _ = transport::call(core, CoreRequest::ResumeFrontDoor, Vec::new());
        return Err(error);
    }
    // A live helper with a lost Activate acknowledgement may already accept;
    // only verified death of that exact helper permits source resumption.
    if let Err(error) = request(
        HelperRequest::Activate(core.public_endpoint.clone()),
        Vec::new(),
    ) {
        if !storage::alive(helper.pid, helper.start) {
            let _ = transport::call(core, CoreRequest::ResumeFrontDoor, Vec::new());
        }
        return Err(error);
    }
    transport::call(core, CoreRequest::CommitFrontDoor, Vec::new())?;
    Ok(())
}
pub(super) fn run() -> io::Result<()> {
    let _singleton = storage::lock(&storage::root()?.join("helper.lock"))?;
    let listener = UnixListener::bind(endpoint()?)?;
    std::fs::set_permissions(endpoint()?, std::fs::Permissions::from_mode(0o600))?;
    storage::atomic(
        &storage::root()?.join("helper.json"),
        &HelperIdentity {
            pid: std::process::id(),
            start: storage::process_start(std::process::id())?,
        },
    )?;
    let mut staged: HashMap<PathBuf, (CoreRegistration, UnixListener, RetainedListener)> =
        HashMap::new();
    let mut active: HashMap<
        PathBuf,
        (
            RetainedListener,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
    > = HashMap::new();
    for connection in listener.incoming() {
        let mut stream = connection?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
        let result = (|| -> io::Result<()> {
            transport::peer(&stream)?;
            let (request, mut fds) = transport::receive(&mut stream, "helper-v1")?;
            match request {
                HelperRequest::Ping => {}
                HelperRequest::Validate(record) => {
                    let (owned, _) = active.get(&record.endpoint).ok_or_else(storage::denied)?;
                    if owned != &record {
                        return Err(storage::denied());
                    }
                    owned.validate_path()?;
                }
                HelperRequest::Stage(core) => {
                    storage::validate_registration(&core)?;
                    if fds.len() != 1 || active.contains_key(&core.public_endpoint) {
                        return Err(storage::denied());
                    }
                    let listener = UnixListener::from(fds.remove(0));
                    let address = listener
                        .local_addr()?
                        .as_pathname()
                        .ok_or_else(storage::denied)?
                        .to_path_buf();
                    if address != core.public_endpoint {
                        return Err(storage::denied());
                    }
                    storage::validate(&address, true)?;
                    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
                    let mut accepting: libc::c_int = 0;
                    let mut size = std::mem::size_of_val(&accepting) as libc::socklen_t;
                    if unsafe {
                        libc::getsockopt(
                            listener.as_raw_fd(),
                            libc::SOL_SOCKET,
                            libc::SO_ACCEPTCONN,
                            &mut accepting as *mut _ as *mut _,
                            &mut size,
                        )
                    } != 0
                        || accepting != 1
                    {
                        return Err(storage::denied());
                    }
                    let metadata = std::fs::symlink_metadata(&address)?;
                    // Authenticate the original listener while its creating
                    // Core is live, before retaining its immutable provenance.
                    let public = transport::connect(&address)?;
                    if transport::peer(&public)? != (core.pid, core.start) {
                        return Err(storage::denied());
                    }
                    let record = RetainedListener {
                        endpoint: address.clone(),
                        dev: metadata.dev(),
                        ino: metadata.ino(),
                        uid: metadata.uid(),
                        mode: metadata.mode(),
                        listen_pid: core.pid,
                        listen_start: core.start,
                    };
                    record.validate_path()?;
                    staged.insert(address, (core, listener, record));
                }
                HelperRequest::Cancel(path) => {
                    if active.contains_key(&path) {
                        return Err(io::Error::other("front door already active"));
                    }
                    staged.remove(&path);
                }
                HelperRequest::Release(path) => {
                    if has_live_routes(&path)? {
                        return Err(io::Error::other(
                            "live moved processes still require this endpoint",
                        ));
                    }
                    if let Some((_, stop)) = active.remove(&path) {
                        stop.store(true, std::sync::atomic::Ordering::Release);
                    }
                    staged.remove(&path);
                    let retained: Vec<_> = active.values().map(|(record, _)| record).collect();
                    storage::atomic(&storage::root()?.join("retained.json"), &retained)?;
                    let _lock = storage::lock(&storage::root()?.join("routes.lock"))?;
                    let mut routes = routes()?;
                    routes.retain(live);
                    save_routes(&routes)?;
                }
                HelperRequest::Activate(path) => {
                    if !active.contains_key(&path) {
                        let (core, listener, identity) =
                            staged.remove(&path).ok_or_else(storage::denied)?;
                        identity.validate_path()?;
                        listener.set_nonblocking(true)?;
                        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        active.insert(path.clone(), (identity.clone(), stop.clone()));
                        let retained: Vec<_> = active.values().map(|(record, _)| record).collect();
                        storage::atomic(&storage::root()?.join("retained.json"), &retained)?;
                        std::thread::spawn(move || {
                            while !stop.load(std::sync::atomic::Ordering::Acquire) {
                                match listener.accept() {
                                    Ok((stream, _)) => {
                                        let core = core.clone();
                                        std::thread::spawn(move || {
                                            let _ =
                                                proxy(stream, &core, (identity.dev, identity.ino));
                                        });
                                    }
                                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                        std::thread::sleep(std::time::Duration::from_millis(10))
                                    }
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                }
            }
            Ok(())
        })();
        let _ = transport::send(
            &mut stream,
            "helper-v1",
            result.map_err(|e| e.to_string()),
            &[],
        );
    }
    Ok(())
}
fn proxy(stream: UnixStream, origin: &CoreRegistration, identity: (u64, u64)) -> io::Result<()> {
    transport::peer(&stream)?;
    use std::os::unix::fs::MetadataExt;
    storage::validate(&origin.public_endpoint, true)?;
    let metadata = std::fs::symlink_metadata(&origin.public_endpoint)?;
    if (metadata.dev(), metadata.ino()) != identity {
        return Err(storage::denied());
    }
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut reader = io::BufReader::new(stream.try_clone()?);
    let mut output = stream;
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    if line.len() > 64 * 1024 * 1024 {
        return Err(storage::denied());
    }
    let value: serde_json::Value = serde_json::from_str(&line).map_err(io::Error::other)?;
    let id = value.get("id").cloned().unwrap_or_default();
    let admitted = (|| -> io::Result<Option<String>> {
        if value.get("method").and_then(|v| v.as_str()) == Some("tab.transfer.status") {
            let operation_id = value
                .pointer("/params/operation_id")
                .and_then(|v| v.as_str())
                .ok_or_else(storage::denied)?;
            let status = super::transfer_status(operation_id)?;
            return Ok(Some(serde_json::json!({"id":id,"result":crate::api::schema::ResponseResult::TabTransferStatus{status}}).to_string()));
        }
        let params = value.get("params").ok_or_else(storage::denied)?;
        let pane = params
            .get("pane_id")
            .or_else(|| params.get("target"))
            .and_then(|p| p.as_str());
        let terminal = params
            .get("terminal_id")
            .or_else(|| params.get("binding").and_then(|b| b.get("terminal_id")))
            .and_then(|p| p.as_str());
        let routes = routes()?;
        let found: Vec<_> = routes
            .iter()
            .filter(|r| {
                r.origin.public_endpoint == origin.public_endpoint
                    && (pane == Some(r.origin_pane.as_str())
                        || terminal == Some(r.target.terminal_id.as_str()))
                    && live(r)
            })
            .collect();
        let Some(route) = found.first() else {
            return Ok(None);
        };
        if found.iter().any(|other| {
            other.owner.boot != route.owner.boot
                || other.generation != route.generation
                || other.target.terminal_id != route.target.terminal_id
        }) {
            return Err(storage::denied());
        }
        let mut request = value.clone();
        for field in ["pane_id", "target"] {
            if request["params"].get(field).and_then(|p| p.as_str())
                == Some(route.origin_pane.as_str())
            {
                request["params"][field] = serde_json::Value::String(route.target.pane_id.clone());
            }
        }
        let request = serde_json::from_value(request).map_err(io::Error::other)?;
        let packet = transport::call(
            &route.owner,
            CoreRequest::Forward {
                terminal_id: route.target.terminal_id.clone(),
                process: route.target.process.clone(),
                generation: route.generation,
                request,
            },
            Vec::new(),
        )?;
        let CoreReply::Response(response) = packet.reply else {
            return Err(storage::denied());
        };
        Ok(Some(response))
    })();
    match admitted {
        Ok(Some(response)) => {
            output.write_all(response.as_bytes())?;
            if !response.ends_with('\n') {
                output.write_all(b"\n")?
            }
            output.flush()
        }
        Err(error) => {
            serde_json::to_writer(&mut output,&serde_json::json!({"id":id,"error":{"code":"transfer_route_unavailable","message":error.to_string()}})).map_err(io::Error::other)?;
            output.write_all(b"\n")
        }
        Ok(None) => {
            let current = match storage::registration(&origin.session) {
                Ok(current) => current,
                Err(error) => {
                    // The retained endpoint is still a real routing service after
                    // its original Core exits. CLI protocol probes precede pane
                    // requests, so answer only that service's own handshake.
                    // A live replacement Core's invalid registration must remain
                    // an error, never be hidden by this stopped-source fallback.
                    let stopped = match storage::read::<CoreRegistration>(
                        &storage::registration_path(&origin.session)?,
                    ) {
                        Ok(current) => !storage::alive(current.pid, current.start),
                        Err(e) if e.kind() == io::ErrorKind::NotFound => true,
                        Err(e) => return Err(e),
                    };
                    if value.get("method").and_then(|v| v.as_str()) == Some("ping")
                        && !storage::alive(origin.pid, origin.start)
                        && stopped
                        && has_live_routes(&origin.public_endpoint)?
                    {
                        serde_json::to_writer(
                            &mut output,
                            &serde_json::json!({
                                "id":id,"result":crate::api::schema::ResponseResult::Pong{
                                    version:crate::build_info::version(),
                                    protocol:crate::protocol::PROTOCOL_VERSION,
                                    capabilities:None,
                                }
                            }),
                        )
                        .map_err(io::Error::other)?;
                        output.write_all(b"\n")?;
                        return output.flush();
                    }
                    return Err(error);
                }
            };
            let mut upstream = transport::connect(&current.private_endpoint)?;
            if transport::peer(&upstream)? != (current.pid, current.start) {
                return Err(storage::denied());
            }
            upstream.set_read_timeout(None)?;
            upstream.set_write_timeout(None)?;
            output.set_read_timeout(None)?;
            upstream.write_all(line.as_bytes())?;
            upstream.flush()?;
            // Preserve normal streaming and SSH-agent lease lifetimes without replay.
            let mut reverse = upstream.try_clone()?;
            let mut client = output.try_clone()?;
            std::thread::spawn(move || {
                let _ = io::copy(&mut reader, &mut upstream);
                let _ = upstream.shutdown(std::net::Shutdown::Write);
            });
            io::copy(&mut reverse, &mut client)?;
            Ok(())
        }
    }
}
