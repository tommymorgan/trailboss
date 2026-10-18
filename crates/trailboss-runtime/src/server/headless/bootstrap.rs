use super::*;

/// Run the headless server. This is the entry point called from main.rs.
pub fn run_server() -> io::Result<()> {
    // Restore the launch environment before logging/API/runtime worker threads start.
    #[cfg(unix)]
    let shell_restore = restore_missing_shell();
    crate::platform::ignore_server_hangup();
    let args: Vec<String> = std::env::args().collect();
    let handoff_import = args.get(2).map(String::as_str) == Some("--handoff-import");
    let process_context = crate::platform::prepare_server_process(handoff_import);
    init_logging();
    #[cfg(unix)]
    if let Err(err) = shell_restore {
        warn!(%err, "could not restore user login shell; retaining platform fallback");
    }
    match process_context {
        Ok(true) => info!("server using persistent user service context"),
        Ok(false) => {}
        Err(err) => {
            warn!(%err, "could not select persistent user service context; retaining inherited context")
        }
    }
    crate::platform::raise_server_nofile_limit();

    if handoff_import {
        let socket_path = args
            .get(3)
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing handoff socket"))?;
        let token = args
            .get(4)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing handoff token"))?;
        return run_handoff_import_server(&socket_path, token);
    }

    let loaded_config = config::Config::load();
    #[cfg(windows)]
    if loaded_config.config.server.allow_unelevated_clients {
        crate::platform::allow_unelevated_clients();
    }
    let (api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let event_hub = api::EventHub::default();
    let server_stop = crate::server::shutdown::ServerStop::default();

    // Start the JSON API socket server.
    let _api_server = match api::start_server_with_stop_control(
        api_tx.clone(),
        event_hub.clone(),
        server_stop.clone(),
    ) {
        Ok(server) => server,
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            eprintln!("error: trailboss server is already running");
            eprintln!("api socket: {}", api::socket_path().display());
            std::process::exit(1);
        }
        Err(err) => return Err(err),
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;

    let result = rt.block_on(async {
        // Create the App (with AppState, event channels, etc.).
        let mut app = app::App::try_new(
            &loaded_config.config,
            app::AppPolicy::PRODUCTION,
            config::config_diagnostic_summary(&loaded_config.diagnostics),
            api_rx,
            event_hub,
        )?;
        seed_startup_workspace_if_empty(&mut app);

        // Create the headless server.
        let mut server = match HeadlessServer::new(
            app,
            &loaded_config.diagnostics,
            Some(api_tx.clone()),
            Some(_api_server),
            server_stop,
        ) {
            Ok(server) => server,
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                eprintln!("error: trailboss server is already running");
                eprintln!("client socket: {}", client_socket_path().display());
                std::process::exit(1);
            }
            Err(err) => return Err(err),
        };

        info!(
            api_socket = %api::socket_path().display(),
            client_socket = %client_socket_path().display(),
            "trailboss server started"
        );
        print_ready_message(&api::socket_path(), &client_socket_path());
        server.app.run_plugin_startup_hooks();

        server.run().await
    });

    rt.shutdown_timeout(Duration::from_millis(100));
    crate::logging::shutdown("server");
    result
}

/// Supply only a missing launch environment; the pane chooser still owns precedence.
#[cfg(unix)]
fn restore_missing_shell() -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    if std::env::var_os("SHELL")
        .is_some_and(|shell| !shell.as_bytes().iter().all(u8::is_ascii_whitespace))
    {
        return Ok(());
    }
    // A blank value must not prevent the unchanged chooser from reaching its fallback
    // when NSS has no entry or the lookup fails. This runs before server threads exist.
    std::env::remove_var("SHELL");
    if let Some(shell) = effective_user_login_shell()? {
        std::env::set_var("SHELL", shell);
    }
    Ok(())
}

#[cfg(unix)]
fn effective_user_login_shell() -> io::Result<Option<std::ffi::OsString>> {
    use std::{ffi::CStr, os::unix::ffi::OsStringExt};

    // SAFETY: sysconf and geteuid have no pointer arguments or shared result buffer.
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let uid = unsafe { libc::geteuid() };
    let mut size = usize::try_from(suggested)
        .ok()
        .filter(|size| *size > 0)
        .unwrap_or(1024);
    let mut buffer = Vec::<libc::c_char>::new();
    loop {
        buffer
            .try_reserve_exact(size - buffer.len())
            .map_err(io::Error::other)?;
        buffer.resize(size, 0);
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: entry and result are writable, and buffer owns size bytes for the
        // entire call and until pw_shell is copied. getpwuid_r is reentrant.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE {
            size = size
                .checked_mul(2)
                .ok_or_else(|| io::Error::other("passwd buffer size overflow"))?;
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if result.is_null() {
            return Ok(None);
        }
        // SAFETY: a successful lookup with a non-null result initializes entry.
        let shell = unsafe { entry.assume_init() }.pw_shell;
        if shell.is_null() {
            return Ok(None);
        }
        // SAFETY: successful getpwuid_r supplies a NUL-terminated pw_shell in buffer.
        let shell = unsafe { CStr::from_ptr(shell) }.to_bytes();
        return Ok((!shell.iter().all(u8::is_ascii_whitespace))
            .then(|| std::ffi::OsString::from_vec(shell.to_vec())));
    }
}
fn seed_startup_workspace_if_empty(app: &mut app::App) {
    let Some(cwd) = take_startup_cwd() else {
        return;
    };

    if !app.state.workspaces.is_empty() {
        info!(
            cwd = %cwd.display(),
            "restored session already has workspaces; ignoring startup cwd"
        );
        return;
    }

    match app.create_workspace_with_options(cwd.clone(), true) {
        Ok(_) => {
            info!(cwd = %cwd.display(), "created startup workspace");
        }
        Err(err) => {
            warn!(cwd = %cwd.display(), err = %err, "failed to create startup workspace");
            app.state.mode = app::Mode::Navigate;
        }
    }
}

fn take_startup_cwd() -> Option<PathBuf> {
    let cwd = std::env::var_os(crate::server::autodetect::STARTUP_CWD_ENV_VAR)?;
    std::env::remove_var(crate::server::autodetect::STARTUP_CWD_ENV_VAR);
    (!cwd.is_empty()).then(|| PathBuf::from(cwd))
}

#[cfg(unix)]
fn run_handoff_import_server(socket_path: &Path, token: &str) -> io::Result<()> {
    // The import child can inherit a stale session or socket override from
    // whoever spawned the old core (a Trailboss pane exports its own session,
    // a pre-rename core exports the old names). The handoff socket path is
    // the one authority for which session is being transferred, so derive
    // the session from it and drop any overrides before anything resolves
    // paths.
    if let Some(session_dir) = socket_path.parent() {
        if let Some(name) = session_dir.file_name().and_then(|n| n.to_str()) {
            std::env::set_var(crate::session::SESSION_ENV_VAR, name);
        }
    }
    std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
    std::env::remove_var(crate::server::socket_paths::CLIENT_SOCKET_PATH_ENV_VAR);
    let loaded_config = config::Config::load();
    let mut received = crate::server::handoff::receive(socket_path, token)?;
    crate::server::handoff::log_import_result(received.manifest.panes.len());

    let (api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let event_hub = api::EventHub::default();
    let server_stop = crate::server::shutdown::ServerStop::default();

    let mut imports = HashMap::new();
    for (pane, fd) in received.manifest.panes.into_iter().zip(received.fds) {
        let pane_id = pane.pane_id;
        imports.insert(
            pane_id,
            crate::handoff_runtime::ImportedHandoffRuntime {
                master_fd: fd,
                state: pane,
            },
        );
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;

    let result = rt.block_on(async {
        let app = app::App::new_from_handoff(
            &loaded_config.config,
            config::config_diagnostic_summary(&loaded_config.diagnostics),
            api_rx,
            event_hub.clone(),
            &received.manifest.snapshot,
            &mut imports,
        )?;
        crate::server::handoff::report_restored(&mut received.stream)?;
        if std::env::var("TRAILBOSS_TEST_HANDOFF_IMPORT_FAIL").as_deref() == Ok("after_restored") {
            return Err(io::Error::other(
                "test handoff import failure after restored",
            ));
        }
        wait_for_old_public_sockets_to_close(Duration::from_secs(5))?;

        let api_server = api::start_server_with_stop_control(
            api_tx.clone(),
            event_hub.clone(),
            server_stop.clone(),
        )?;
        // Report ready as soon as the public API socket is bound: a
        // pre-rename source core has a shorter ready timeout than this
        // build's construction (plugin + server assembly) takes, and rolls
        // back if "ready" arrives late. The commit line is buffered by the
        // stream, so reading it after construction stays safe.
        crate::server::handoff::report_ready(&mut received.stream)?;
        let mut server = HeadlessServer::new(
            app,
            &loaded_config.diagnostics,
            Some(api_tx.clone()),
            Some(api_server),
            server_stop,
        )?;
        // Carried across before any client attaches, so the first title sent is
        // the override rather than the configured one it replaced.
        server.api_window_title = received.manifest.api_window_title.take();
        crate::server::handoff::wait_committed(&mut received.stream)?;
        server.app.assume_handoff_ownership();
        server.app.unpause_handoff_readers();
        server.pending_handoff_repaint_nudge = true;
        if let Err(err) = crate::server::handoff::report_owned(&mut received.stream) {
            warn!(err = %err, "failed to report handoff ownership; continuing as owner");
        }
        info!("handoff import server started");
        print_ready_message(&api::socket_path(), &client_socket_path());
        server.app.run_plugin_startup_hooks();
        server.run().await
    });

    rt.shutdown_timeout(Duration::from_millis(100));
    crate::logging::shutdown("server");
    result
}

#[cfg(not(unix))]
fn run_handoff_import_server(_socket_path: &Path, _token: &str) -> io::Result<()> {
    Err(io::Error::other("live handoff is only supported on Unix"))
}

fn print_ready_message(api_socket: &Path, client_socket: &Path) {
    let message = format!(
        "trailboss server running; you can use any trailboss CLI command in another terminal.\n\
         api socket: {}\n\
         client socket: {}\n\
         logs: {}\n\
         did you mean to open the Trailboss TUI? run `trailboss`; you do not need `trailboss server`.\n",
        api_socket.display(),
        client_socket.display(),
        crate::session::data_dir()
            .join("trailboss-server.log")
            .display()
    );
    // The launching terminal may already be gone; the server keeps running.
    let _ = io::Write::write_all(&mut io::stderr(), message.as_bytes());
}

/// Initialize logging for the server process.
fn init_logging() {
    crate::logging::init_file_logging("trailboss-server.log");
}
