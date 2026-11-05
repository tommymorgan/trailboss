use std::{
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
};
use trailboss::{Client, Input, Surface};
use trailboss_runtime::{
    protocol::{
        endpoint::EndpointPluginUi, widgets::Declaration, ClientShellSnapshot, ServerMessage,
    },
    remote::RemoteConnection,
};

pub struct Update {
    pub surface: Option<Surface>,
    pub declarations: Option<Vec<(String, Declaration)>>,
    pub focus: Option<(Option<String>, Option<String>, Option<String>)>,
    pub boot: String,
    pub snapshot: Option<ClientShellSnapshot>,
    pub graphics: Vec<ServerMessage>,
    pub surface_changed: bool,
    pub response: Option<ServerMessage>,
    pub commands: Option<EndpointPluginUi>,
}

pub struct Receiver {
    updates: Option<mpsc::Receiver<io::Result<Update>>>,
    worker: Option<thread::JoinHandle<()>>,
    shutdown: Input,
}
impl Receiver {
    pub fn start(mut client: Client) -> io::Result<Self> {
        let shutdown = client.input()?;
        let (tx, rx) = mpsc::sync_channel(1);
        tracing::info!(
            event = "native_client_receiver_start",
            "starting native receive worker"
        );
        let worker = thread::spawn(move || {
            let mut last_focus = (
                client.workspace_id.clone(),
                client.focused_tab_id.clone(),
                client.focused_pane_id.clone(),
            );
            loop {
                if let Err(error) = client.receive() {
                    tracing::error!(
                        event = "native_client_receiver_end",
                        reason = "receive_failure",
                        "native receive worker failed"
                    );
                    let _ = tx.send(Err(error));
                    break;
                }
                let changed_focus = if client.workspace_id != last_focus.0
                    || client.focused_tab_id != last_focus.1
                    || client.focused_pane_id != last_focus.2
                {
                    let focus = (
                        client.workspace_id.clone(),
                        client.focused_tab_id.clone(),
                        client.focused_pane_id.clone(),
                    );
                    last_focus.clone_from(&focus);
                    Some(focus)
                } else {
                    None
                };
                if client.surface_changed
                    || client.declarations.is_some()
                    || changed_focus.is_some()
                    || client.snapshot.is_some()
                    || client.endpoint_response.is_some()
                    || client.command_ui.is_some()
                    || !client.graphics_messages.is_empty()
                {
                    let update = Update {
                        surface: client.primary.take(),
                        declarations: client.declarations.take(),
                        focus: changed_focus,
                        boot: client.boot_id.clone(),
                        snapshot: client.snapshot.take(),
                        graphics: std::mem::take(&mut client.graphics_messages),
                        surface_changed: std::mem::take(&mut client.surface_changed),
                        response: client.endpoint_response.take(),
                        commands: client.command_ui.take(),
                    };
                    trailboss_runtime::render_prof::trace(
                        "client.update.send_start",
                        format_args!("pane={:?}", client.focused_pane_id),
                    );
                    if tx.send(Ok(update)).is_err() {
                        tracing::info!(
                            event = "native_client_receiver_end",
                            reason = "consumer_closed",
                            "native receive worker stopped after consumer closed"
                        );
                        break;
                    }
                    trailboss_runtime::render_prof::trace(
                        "client.update.send_end",
                        format_args!("pane={:?}", client.focused_pane_id),
                    );
                }
            }
        });
        Ok(Self {
            updates: Some(rx),
            worker: Some(worker),
            shutdown,
        })
    }
    pub fn try_recv(&self) -> Result<io::Result<Update>, mpsc::TryRecvError> {
        self.updates
            .as_ref()
            .expect("live attachment receiver")
            .try_recv()
    }
    pub fn stop(&mut self) {
        let _ = self.shutdown.shutdown();
        // A full one-slot channel can block the worker after the socket closes.
        self.updates.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct SessionHost {
    pub current: String,
    pub remote: Option<String>,
    pub bridge: Option<RemoteConnection>,
}
fn selected_name(name: &str) -> Option<&str> {
    (name != trailboss_runtime::session::DEFAULT_SESSION_NAME).then_some(name)
}

impl SessionHost {
    pub fn path(&self) -> PathBuf {
        self.bridge.as_ref().map_or_else(
            || trailboss_runtime::session::client_socket_path_for(selected_name(&self.current)),
            |bridge| bridge.socket_path().to_owned(),
        )
    }
    pub fn connect_existing(&self, name: &str) -> io::Result<(PathBuf, Option<RemoteConnection>)> {
        trailboss_runtime::session::validate_name(name).map_err(io::Error::other)?;
        if let Some(bridge) = &self.bridge {
            let bridge = bridge.connect_session_existing(name)?;
            Ok((bridge.socket_path().to_owned(), Some(bridge)))
        } else if self.remote.is_some() {
            Err(io::Error::other(
                "remote attachment transport is unavailable",
            ))
        } else {
            Ok((
                trailboss_runtime::session::client_socket_path_for(selected_name(name)),
                None,
            ))
        }
    }
}

pub struct Geometry {
    pub size: (u16, u16),
    pub cell: (u32, u32),
    pub exact_cell_size: bool,
    pub direct_graphics: bool,
}

#[derive(Default)]
struct Cancellation {
    requested: AtomicBool,
    input: Mutex<Option<Input>>,
}
impl Cancellation {
    fn check(&self) -> io::Result<()> {
        if self.requested.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "session switch cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

pub struct Preparation {
    cancellation: Arc<Cancellation>,
    worker: Option<thread::JoinHandle<io::Result<(String, Prepared)>>>,
}
impl Preparation {
    pub fn start(host: &SessionHost, name: String, geometry: Geometry) -> io::Result<Self> {
        let (path, bridge) = host.connect_existing(&name)?;
        let local_api = host
            .remote
            .is_none()
            .then(|| trailboss_runtime::session::api_socket_path_for(selected_name(&name)));
        let cancellation = Arc::new(Cancellation::default());
        let worker_cancellation = cancellation.clone();
        let worker = thread::spawn(move || {
            let prepared =
                Prepared::connect(path, bridge, local_api, geometry, &worker_cancellation)?;
            Ok((name, prepared))
        });
        Ok(Self {
            cancellation,
            worker: Some(worker),
        })
    }
    pub fn finished(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
    }
    pub fn cancel(&self) {
        self.cancellation.requested.store(true, Ordering::Release);
        if let Some(input) = self
            .cancellation
            .input
            .lock()
            .expect("preparation shutdown lock")
            .as_ref()
        {
            let _ = input.shutdown();
        }
    }
    pub fn finish(mut self) -> Option<io::Result<(String, Prepared)>> {
        let result = self
            .worker
            .take()
            .expect("finished preparation")
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("session preparation worker failed")));
        if self.cancellation.requested.load(Ordering::Acquire) {
            None
        } else {
            Some(result)
        }
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.cancel();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

pub struct Prepared {
    pub input: Input,
    pub receiver: Receiver,
    pub initial: Update,
    pub bridge: Option<RemoteConnection>,
}
impl Prepared {
    fn connect(
        path: PathBuf,
        bridge: Option<RemoteConnection>,
        local_api: Option<PathBuf>,
        geometry: Geometry,
        cancellation: &Cancellation,
    ) -> io::Result<Self> {
        cancellation.check()?;
        let mut client = Client::attach_graphics(
            &path,
            geometry.size.0,
            geometry.size.1,
            geometry.cell,
            geometry.exact_cell_size,
            geometry.direct_graphics,
        )?;
        *cancellation
            .input
            .lock()
            .expect("preparation shutdown lock") = Some(client.input()?);
        cancellation.check()?;
        client.wait_ready()?;
        cancellation.check()?;
        if let Some(path) = local_api {
            crate::enable_default_producer_at(path)?;
        }
        let input = client.input()?;
        client.blocking()?;
        let initial = Update {
            surface: client.primary.take(),
            declarations: client.declarations.take(),
            focus: Some((
                client.workspace_id.clone(),
                client.focused_tab_id.clone(),
                client.focused_pane_id.clone(),
            )),
            boot: client.boot_id.clone(),
            snapshot: client.snapshot.take(),
            graphics: std::mem::take(&mut client.graphics_messages),
            surface_changed: true,
            response: None,
            commands: client.command_ui.take(),
        };
        let receiver = Receiver::start(client)?;
        Ok(Self {
            input,
            receiver,
            initial,
            bridge,
        })
    }
}
