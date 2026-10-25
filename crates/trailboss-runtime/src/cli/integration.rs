use crate::api::schema::IntegrationTarget;

pub(super) fn run_integration_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_integration_help();
        return Ok(2);
    };

    match subcommand {
        "install" => integration_install(&args[1..]),
        "uninstall" => integration_uninstall(&args[1..]),
        "status" => integration_status(&args[1..]),
        "help" | "--help" | "-h" => {
            print_integration_help();
            Ok(0)
        }
        _ => {
            print_integration_help();
            Ok(2)
        }
    }
}

fn integration_status(args: &[String]) -> std::io::Result<i32> {
    let outdated_only = match args {
        [] => false,
        [flag] if flag == "--outdated-only" => true,
        _ => {
            eprintln!("usage: trailboss integration status [--outdated-only]");
            return Ok(2);
        }
    };

    if outdated_only {
        crate::integration::print_outdated_update_notice();
        return Ok(0);
    }

    for status in crate::integration::installed_integration_statuses() {
        let target = crate::integration::integration_target_label(status.target);
        let state = describe_integration_state(
            status.state,
            status.installed_version,
            status.expected_version,
        );
        println!("{target}: {state} ({})", status.path.display());
    }

    for status in crate::integration::experimental_letta_integration_status()
        .into_iter()
        .chain(std::iter::once(
            crate::integration::experimental_shell_integration_status(),
        ))
    {
        let state = describe_integration_state(
            status.state,
            status.installed_version,
            status.expected_version,
        );
        println!(
            "{} (experimental): {state} ({})",
            status.label,
            status.path.display()
        );
    }

    Ok(0)
}

fn describe_integration_state(
    state: crate::integration::IntegrationStatusKind,
    installed_version: Option<u32>,
    expected_version: u32,
) -> String {
    let version = match installed_version {
        Some(version) => format!("v{version}"),
        None => "legacy".to_string(),
    };
    match state {
        crate::integration::IntegrationStatusKind::NotInstalled => "not installed".to_string(),
        crate::integration::IntegrationStatusKind::Current => format!("current ({version})"),
        crate::integration::IntegrationStatusKind::Outdated
            if installed_version.is_some_and(|installed| installed >= expected_version) =>
        {
            format!("needs repair ({version})")
        }
        crate::integration::IntegrationStatusKind::Outdated => {
            format!("outdated ({version} < v{expected_version})")
        }
    }
}

fn integration_install(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = parse_integration_target(args, "install")? else {
        return Ok(2);
    };

    let installed = match target {
        IntegrationCommandTarget::Builtin(target) => crate::integration::install_target(target),
        IntegrationCommandTarget::Letta => crate::integration::install_experimental_letta(),
        IntegrationCommandTarget::Shell => crate::integration::install_experimental_shell(),
    };
    match installed {
        Ok(messages) => {
            print_integration_messages(messages);
            Ok(0)
        }
        Err(err) => {
            eprintln!("{err}");
            Ok(1)
        }
    }
}

fn integration_uninstall(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = parse_integration_target(args, "uninstall")? else {
        return Ok(2);
    };

    let removed = match target {
        IntegrationCommandTarget::Builtin(target) => crate::integration::uninstall_target(target),
        IntegrationCommandTarget::Letta => crate::integration::uninstall_experimental_letta(),
        IntegrationCommandTarget::Shell => crate::integration::uninstall_experimental_shell(),
    };
    match removed {
        Ok(messages) => {
            print_integration_messages(messages);
            Ok(0)
        }
        Err(err) => {
            eprintln!("{err}");
            Ok(1)
        }
    }
}

fn print_integration_messages(messages: Vec<String>) {
    for message in messages {
        println!("{message}");
    }
}

/// Integration target accepted by the CLI. Letta and shell are deliberately
/// kept out of the frozen client endpoint `IntegrationTarget` enum and are
/// handled as experimental CLI-only targets until the agent registry replaces
/// them.
enum IntegrationCommandTarget {
    Builtin(IntegrationTarget),
    Letta,
    Shell,
}

fn parse_integration_target(
    args: &[String],
    action: &str,
) -> std::io::Result<Option<IntegrationCommandTarget>> {
    let Some(target) = args.first().map(|arg| arg.as_str()) else {
        eprintln!(
            "usage: trailboss integration {action} <pi|omp|claude|codex|copilot|devin|droid|kimi|opencode|kilo|hermes|qodercli|qwen|letta|shell|cursor|mastracode|grok>"
        );
        return Ok(None);
    };
    if args.len() != 1 {
        eprintln!(
            "usage: trailboss integration {action} <pi|omp|claude|codex|copilot|devin|droid|kimi|opencode|kilo|hermes|qodercli|qwen|letta|shell|cursor|mastracode|grok>"
        );
        return Ok(None);
    }

    let parsed = match target {
        "pi" => IntegrationCommandTarget::Builtin(IntegrationTarget::Pi),
        "omp" => IntegrationCommandTarget::Builtin(IntegrationTarget::Omp),
        "claude" => IntegrationCommandTarget::Builtin(IntegrationTarget::Claude),
        "codex" => IntegrationCommandTarget::Builtin(IntegrationTarget::Codex),
        "copilot" => IntegrationCommandTarget::Builtin(IntegrationTarget::Copilot),
        "devin" => IntegrationCommandTarget::Builtin(IntegrationTarget::Devin),
        "droid" => IntegrationCommandTarget::Builtin(IntegrationTarget::Droid),
        "kimi" => IntegrationCommandTarget::Builtin(IntegrationTarget::Kimi),
        "opencode" => IntegrationCommandTarget::Builtin(IntegrationTarget::Opencode),
        "kilo" => IntegrationCommandTarget::Builtin(IntegrationTarget::Kilo),
        "hermes" => IntegrationCommandTarget::Builtin(IntegrationTarget::Hermes),
        "qodercli" => IntegrationCommandTarget::Builtin(IntegrationTarget::Qodercli),
        "qwen" => IntegrationCommandTarget::Builtin(IntegrationTarget::Qwen),
        "letta" => IntegrationCommandTarget::Letta,
        "shell" => IntegrationCommandTarget::Shell,
        "cursor" => IntegrationCommandTarget::Builtin(IntegrationTarget::Cursor),
        "mastracode" => IntegrationCommandTarget::Builtin(IntegrationTarget::Mastracode),
        "antigravity-cli" | "antigravity_cli" => {
            IntegrationCommandTarget::Builtin(IntegrationTarget::AntigravityCli)
        }
        "grok" => IntegrationCommandTarget::Builtin(IntegrationTarget::Grok),
        _ => {
            eprintln!("unknown integration target: {target}");
            eprintln!(
                "currently supported: pi, omp, claude, codex, copilot, devin, droid, kimi, opencode, kilo, hermes, qodercli, qwen, letta, shell, cursor, mastracode, antigravity-cli, grok"
            );
            return Ok(None);
        }
    };

    Ok(Some(parsed))
}

fn print_integration_help() {
    eprintln!("trailboss integration commands:");
    eprintln!("  trailboss integration install pi");
    eprintln!("  trailboss integration install omp");
    eprintln!("  trailboss integration install claude");
    eprintln!("  trailboss integration install codex");
    eprintln!("  trailboss integration install copilot");
    eprintln!("  trailboss integration install devin");
    eprintln!("  trailboss integration install droid");
    eprintln!("  trailboss integration install kimi");
    eprintln!("  trailboss integration install opencode");
    eprintln!("  trailboss integration install kilo");
    eprintln!("  trailboss integration install hermes");
    eprintln!("  trailboss integration install qodercli");
    eprintln!("  trailboss integration install qwen");
    eprintln!("  trailboss integration install letta");
    eprintln!("  trailboss integration install shell");
    eprintln!("  trailboss integration install cursor");
    eprintln!("  trailboss integration install mastracode");
    eprintln!("  trailboss integration install antigravity-cli");
    eprintln!("  trailboss integration install grok");
    eprintln!("  trailboss integration uninstall pi");
    eprintln!("  trailboss integration uninstall omp");
    eprintln!("  trailboss integration uninstall claude");
    eprintln!("  trailboss integration uninstall codex");
    eprintln!("  trailboss integration uninstall copilot");
    eprintln!("  trailboss integration uninstall devin");
    eprintln!("  trailboss integration uninstall droid");
    eprintln!("  trailboss integration uninstall kimi");
    eprintln!("  trailboss integration uninstall opencode");
    eprintln!("  trailboss integration uninstall kilo");
    eprintln!("  trailboss integration uninstall hermes");
    eprintln!("  trailboss integration uninstall qodercli");
    eprintln!("  trailboss integration uninstall qwen");
    eprintln!("  trailboss integration uninstall letta");
    eprintln!("  trailboss integration uninstall shell");
    eprintln!("  trailboss integration uninstall cursor");
    eprintln!("  trailboss integration uninstall mastracode");
    eprintln!("  trailboss integration uninstall antigravity-cli");
    eprintln!("  trailboss integration uninstall grok");
    eprintln!("  trailboss integration status [--outdated-only]");
}
