//! Remote-host side of the SSH stdio bridge.

use std::io;
use std::time::Duration;

pub fn run_remote_client_bridge(
    args: &[String],
    initialize: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let mut idle_timeout = false;
    let mut existing_only = false;
    for option in args {
        match option.as_str() {
            "--idle-timeout-v1"
                if !idle_timeout && crate::platform::REMOTE_BRIDGE_IDLE_TIMEOUT_SUPPORTED =>
            {
                idle_timeout = true;
            }
            "--existing-only-v1" if !existing_only => existing_only = true,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported remote client bridge option",
                ));
            }
        }
    }
    // Picker targets are existing sessions, never permission to recreate a Core.
    // Connect below without the daemon-starting fallback, including if it stops
    // between discovery and this SSH command.
    if !existing_only {
        ensure_remote_server_running()?;
    }
    initialize()?;
    #[cfg(unix)]
    let _ssh_agent = super::ssh_agent::Registration::start();

    let socket_path = crate::server::socket_paths::client_socket_path();
    let stream = crate::ipc::connect_local_stream(&socket_path).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "failed to connect to remote Trailboss client socket {}: {err}",
                socket_path.display()
            ),
        )
    })?;

    crate::platform::forward_remote_bridge_stdio(stream, idle_timeout)
}

fn ensure_remote_server_running() -> io::Result<()> {
    let socket_path = crate::server::socket_paths::client_socket_path();
    if crate::server::autodetect::is_server_listening()? {
        let status = crate::api::read_runtime_status_at(
            &crate::api::socket_path(),
            Duration::from_millis(500),
        )?
        .ok_or_else(|| io::Error::other("remote server status API is unavailable"))?;
        if status
            .capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.endpoint_protocol_generation)
            == Some(crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION)
        {
            return Ok(());
        }
        return Err(io::Error::other(
            "remote trailboss server needs one final update before this bridge can attach; rerun `trailboss --remote` from an interactive terminal to approve it",
        ));
    }

    crate::server::autodetect::spawn_server_daemon()?;
    crate::server::autodetect::wait_for_server_socket(&socket_path, Duration::from_secs(5))
}
