//! Unix public-listener ownership, independent of accepted connection lifetime.
use std::io::{self, BufRead, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::ListenerNonblockingMode;
use tracing::{debug, error};

use super::{ApiServices, ServerHandle};
use crate::api::schema::Request;
use crate::api::{ApiRequestMessage, ApiRequestSender};
use crate::ipc::LocalListener;
use crate::server::tab_transfer::protocol::RouteAdmission;

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Default)]
struct State {
    pause: bool,
    acknowledged: bool,
    stopped: bool,
}

#[derive(Default)]
struct Control {
    state: Mutex<State>,
    changed: Condvar,
}

pub(super) struct Acceptor {
    listener: Option<Arc<LocalListener>>,
    control: Arc<Control>,
    thread: Option<JoinHandle<()>>,
}

impl Acceptor {
    pub(super) fn start(
        listener: LocalListener,
        services: Arc<ApiServices>,
        running: Arc<AtomicBool>,
        private: bool,
    ) -> io::Result<Self> {
        listener.set_nonblocking(ListenerNonblockingMode::Accept)?;
        let listener = Arc::new(listener);
        let control = Arc::new(Control::default());
        let worker_listener = Arc::clone(&listener);
        let worker_control = Arc::clone(&control);
        let thread = crate::thread_spawn::spawn_named("trailboss-api-accept", move || {
            let mut consecutive_errors = 0_u64;
            loop {
                let mut state = worker_control
                    .state
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                while state.pause && !state.stopped {
                    state.acknowledged = true;
                    worker_control.changed.notify_all();
                    state = worker_control
                        .changed
                        .wait(state)
                        .unwrap_or_else(|err| err.into_inner());
                }
                if state.stopped {
                    break;
                }
                // Accept is nonblocking and serialized with pause requests: an
                // acknowledgement proves no subsequent accept until resume.
                let accepted = worker_listener.accept();
                match accepted {
                    Ok(stream) => {
                        drop(state);
                        consecutive_errors = 0;
                        services.accept(stream, Arc::clone(&running), private);
                    }
                    Err(err) => {
                        let delay = if matches!(
                            err.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) {
                            ACCEPT_POLL_INTERVAL
                        } else {
                            if consecutive_errors == 0 {
                                error!(%err, "api listener accept failed; retrying");
                            }
                            consecutive_errors = consecutive_errors.saturating_add(1);
                            super::ACCEPT_ERROR_BACKOFF
                        };
                        let _ = worker_control
                            .changed
                            .wait_timeout(state, delay)
                            .unwrap_or_else(|err| err.into_inner());
                    }
                }
            }
            debug!("api server thread exiting");
        })?;
        Ok(Self {
            listener: Some(listener),
            control,
            thread: Some(thread),
        })
    }

    fn quiesce(&mut self, timeout: Duration) -> io::Result<OwnedFd> {
        let listener = self.listener.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "public listener already adopted",
            )
        })?;
        let LocalListener::UdSocket(listener) = listener.as_ref();
        let duplicate = listener.as_fd().try_clone_to_owned()?;
        let started = Instant::now();
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if state.stopped {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "public acceptor stopped",
            ));
        }
        state.pause = true;
        self.control.changed.notify_all();
        while !state.acknowledged {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                state.pause = false;
                self.control.changed.notify_all();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "public acceptor quiescence timed out",
                ));
            }
            state = self
                .control
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|err| err.into_inner())
                .0;
        }
        Ok(duplicate)
    }

    fn resume(&mut self) -> io::Result<()> {
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if state.stopped || self.listener.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "public listener already adopted",
            ));
        }
        state.pause = false;
        state.acknowledged = false;
        self.control.changed.notify_all();
        Ok(())
    }

    pub(super) fn stop(&mut self) {
        {
            let mut state = self
                .control
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            state.stopped = true;
            self.control.changed.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.listener.take();
    }
}

impl Drop for Acceptor {
    fn drop(&mut self) {
        self.stop();
    }
}

impl ServerHandle {
    /// Starts a separately owned listener using this Core's exact services.
    /// The caller must supply an existing user-owned mode-0700 directory
    /// within the configuration root. Dropping it cannot stop public clients.
    pub fn start_private_at(&self, path: &Path) -> io::Result<ServerHandle> {
        validate_private_path(path)?;
        super::prepare_socket_path(path)?;
        let listener = crate::ipc::bind_local_listener(path)?;
        let identity = crate::ipc::socket_file_identity(path)?;
        if let Err(err) = super::restrict_socket_permissions(path) {
            let _ = crate::ipc::remove_socket_file_if_owned(path, &identity);
            return Err(err);
        }
        super::start_bound_server(
            path.to_owned(),
            identity,
            listener,
            Arc::clone(&self.services),
            true,
        )
    }

    /// Returns the same listening socket only after the public acceptor has
    /// acknowledged its pause. Existing handlers keep running. A timeout
    /// cancels the pause; the original listener remains owned and accepting.
    pub fn quiesce_public_acceptor(&mut self, timeout: Duration) -> io::Result<OwnedFd> {
        self.acceptor.quiesce(timeout)
    }

    /// Cancels a provisional adoption without rebinding or unlinking.
    pub fn resume_public_acceptor(&mut self) -> io::Result<()> {
        self.acceptor.resume()
    }

    /// Call only after the helper commits ownership of the duplicated socket.
    /// Accepted connections remain attached to this handle's lifetime.
    pub fn commit_public_adoption(&mut self) {
        self.acceptor.stop();
        self.owns_socket = false;
    }
}

pub(super) fn validate_private_path(path: &Path) -> io::Result<()> {
    let denied = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "untrusted private API socket path",
        )
    };
    let parent = path.parent().ok_or_else(denied)?;
    let root = std::fs::canonicalize(crate::config::config_dir())?;
    let resolved_parent = std::fs::canonicalize(parent)?;
    let metadata = std::fs::symlink_metadata(parent)?;
    let uid = unsafe { libc::geteuid() };
    if !resolved_parent.starts_with(&root)
        || !metadata.is_dir()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != 0o700
        || path.file_name().is_none()
        || path
            .file_name()
            .is_some_and(|name| name == "." || name == "..")
    {
        return Err(denied());
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_socket()
                && metadata.uid() == uid
                && metadata.mode() & 0o7777 == 0o600 =>
        {
            Ok(())
        }
        Ok(_) => Err(denied()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Sends exactly once to a protected private endpoint. The ordinary API
/// handler retains responsibility for deferred and outer wait semantics.
pub(crate) fn request_guarded(
    private_endpoint: &Path,
    request: &Request,
    admission: &RouteAdmission,
) -> io::Result<String> {
    validate_private_path(private_endpoint)?;
    #[derive(serde::Serialize)]
    struct GuardedRequest<'a> {
        #[serde(flatten)]
        request: &'a Request,
        #[serde(rename = "_transfer_route")]
        admission: &'a RouteAdmission,
    }
    let encoded = serde_json::to_vec(&GuardedRequest { request, admission })?;
    if encoded.len() > super::MAX_INITIAL_REQUEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "guarded API request is too large",
        ));
    }
    let mut stream = crate::ipc::connect_local_stream(private_endpoint)?;
    stream.set_send_timeout(Some(super::STREAM_WRITE_TIMEOUT))?;
    stream.write_all(&encoded)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    if std::io::BufReader::new(stream).read_line(&mut response)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "private API closed without a response",
        ));
    }
    if response.ends_with('\n') {
        response.pop();
        if response.ends_with('\r') {
            response.pop();
        }
    }
    Ok(response)
}

/// The worker only forwards channel messages (never performs backend I/O).
/// Explicit cancellation and unpark bound cleanup even if a helper retains
/// a clone of the adapted sender.
pub(super) struct GuardedSender {
    pub(super) tx: ApiRequestSender,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for GuardedSender {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

struct WakeThread(std::thread::Thread);

impl std::task::Wake for WakeThread {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

pub(super) fn guarded_sender_for(
    line: &str,
    private: bool,
    backend: &ApiRequestSender,
) -> io::Result<Option<GuardedSender>> {
    if !super::has_route_metadata(line)? {
        return Ok(None);
    }
    if !private {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "route admission is private-only",
        ));
    }
    // Deserialize the original text to reject duplicate metadata keys.
    #[derive(serde::Deserialize)]
    struct AdmissionEnvelope {
        #[serde(rename = "_transfer_route")]
        admission: RouteAdmission,
    }
    let admission = serde_json::from_str::<AdmissionEnvelope>(line)?.admission;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ApiRequestMessage>();
    let backend = backend.clone();
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = Arc::clone(&running);
    let thread = crate::thread_spawn::spawn_named("trailboss-api-route", move || {
        let waker = std::task::Waker::from(Arc::new(WakeThread(std::thread::current())));
        let mut context = std::task::Context::from_waker(&waker);
        while worker_running.load(Ordering::Acquire) {
            match rx.poll_recv(&mut context) {
                std::task::Poll::Ready(Some(mut message)) => {
                    message.route_admission = Some(admission.clone());
                    if backend.send(message).is_err() {
                        break;
                    }
                }
                std::task::Poll::Pending => std::thread::park(),
                std::task::Poll::Ready(None) => break,
            }
        }
    })?;
    Ok(Some(GuardedSender {
        tx,
        running,
        thread: Some(thread),
    }))
}
