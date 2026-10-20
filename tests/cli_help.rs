use std::process::{Command, Stdio};

#[test]
fn help_is_available_without_a_terminal_or_running_core() {
    for args in [
        &["--help"][..],
        &["-h"][..],
        &["help"][..],
        &["help", "agent"][..],
        &["agent", "--help"][..],
        &["completion", "--help"][..],
        &["config", "--help"][..],
        &["terminal", "--help"][..],
        &["agent", "restart-idle", "--help"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_trailboss"))
            .args(args)
            .env("HERDR_SOCKET_PATH", "/nonexistent/trailboss-help-test.sock")
            .env(
                "TRAILBOSS_CONFIG_PATH",
                "/nonexistent/trailboss-help-test.toml",
            )
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{args:?} emitted an error");
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Usage: trailboss"), "{args:?}: {help}");
    }
}

#[test]
fn machine_help_rejects_conflicting_launch_selectors() {
    for selector in ["--session", "--remote"] {
        for args in [
            vec![
                "--machine",
                "unused-test-machine",
                selector,
                "other",
                "agent",
                "--help",
            ],
            vec![
                selector,
                "other",
                "--machine",
                "unused-test-machine",
                "agent",
                "--help",
            ],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_trailboss"))
                .args(&args)
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{args:?}");
            assert!(
                output.stdout.is_empty(),
                "{args:?} ran despite conflicting targets"
            );
        }
    }
}

#[test]
fn inherited_upstream_channel_command_is_unavailable() {
    let output = Command::new(env!("CARGO_BIN_EXE_trailboss"))
        .args(["channel", "show"])
        .env(
            "TRAILBOSS_CONFIG_PATH",
            "/nonexistent/trailboss-help-test.toml",
        )
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "the upstream channel route is exposed"
    );
    assert!(output.stdout.is_empty());
}
