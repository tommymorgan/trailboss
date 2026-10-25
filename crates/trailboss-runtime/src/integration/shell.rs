//! Bash shell integration: OSC 133 markers for ordinary interactive commands.
//!
//! The hook loads AFTER normal user startup on both delivery paths, so user
//! rc content cannot leave a Trailboss pane uninstrumented:
//!
//! - Interactive non-login Bash panes read a Trailboss rcfile wrapper that
//!   sources `~/.bashrc` and then loads the hook. Bash still loads any
//!   compiled-in system startup file itself. No user file is modified.
//! - `trailboss integration install shell` writes a managed standalone hook
//!   file and appends a minimal guarded source block to the user's Bash
//!   startup files — `.bashrc` plus the login file chosen by Bash's own
//!   precedence — so the block always runs after existing user content. This
//!   is the path for hosts reached through SSH (a fresh SSH login is a login
//!   shell and never assumes `.bash_profile` sources `.bashrc`) and the
//!   authoritative path for login panes, which keep native login startup and
//!   are never replayed as non-login shells. The command only alters the
//!   host it runs on; Trailboss never pushes it onto a remote host implicitly.
//!
//! The hook only prepends: one guarded first `PROMPT_COMMAND` entry (string or
//! array form preserved) and a `PS0` prefix. It captures the previous
//! command's exit status before any user hook can observe it and returns that
//! status unchanged, never touches `PS1`, history, or user commands, and emits
//! markers only in interactive shells attached to a terminal. The first
//! prompt emits A without D so a fresh prompt can never falsely complete a
//! still-running outer command with exit 0. zsh and fish are unsupported and
//! stay uninstrumented; `sh` is never treated as Bash.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::config_file::write_config;
use super::file_ops::remove_file_if_exists;
use super::types::{ShellInstallPaths, ShellUninstallResult};

pub(crate) const SHELL_HOOK_INSTALL_NAME: &str = "shell-integration.bash";
pub(crate) const SHELL_INTEGRATION_VERSION: u32 = 1;
/// Rcfile Trailboss passes to interactive non-login Bash panes.
pub(crate) const PANE_BASH_RCFILE_NAME: &str = "pane-bash-wrapper.bash";

const SHELL_RC_BLOCK_BEGIN: &str = "# >>> trailboss shell integration >>>";
const SHELL_RC_BLOCK_END: &str = "# <<< trailboss shell integration <<<";
/// Records startup files the installer created, so uninstall can remove them
/// again instead of leaving empty rc files behind.
const SHELL_CREATED_FILES_STATE_NAME: &str = "shell-integration.created";
const BASH_RC_NAME: &str = ".bashrc";
/// Bash login-startup precedence. A higher-precedence file is never created
/// when a lower-precedence one exists, because it would shadow it.
const BASH_LOGIN_RC_NAMES: [&str; 3] = [".bash_profile", ".bash_login", ".profile"];

/// The marker function invoked as the first `PROMPT_COMMAND` entry.
///
/// It reads the status captured by the prelude instead of `$?` at function
/// entry, and returns the captured status so later user entries (and
/// `$?`-aware prompts) observe the original exit status.
const PROMPT_MARK_FUNCTION: &str = r#"__trailboss_prompt_mark() {
    if [[ -v __TRAILBOSS_COMMAND_STARTED ]]; then
        printf '\033]133;D;%d\007\033]133;A\007' "$__TRAILBOSS_MARK_STATUS"
    else
        printf '\033]133;A\007'
    fi
    return "$__TRAILBOSS_MARK_STATUS"
}"#;

/// First `PROMPT_COMMAND` entry: capture the previous command's exit status
/// before anything else can clobber it, then run the marker function.
const PROMPT_MARK_PRELUDE: &str = r"__TRAILBOSS_MARK_STATUS=$?; __trailboss_prompt_mark";

/// True only for real Bash. `sh` is deliberately not instrumented even where
/// it links to Bash, and zsh/fish are unsupported.
pub(crate) fn is_bash_shell(shell: &str) -> bool {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    name == "bash"
}

/// Contents of the managed hook file written by `integration install shell`
/// and reused verbatim by the pane rcfile wrapper.
///
/// One `$(declare -p ...)` subshell runs at load time to tell scalar from
/// array `PROMPT_COMMAND`; nothing forks per prompt or per command. `PS0`
/// expands after a command line is read, before it executes: the `${...:=}`
/// expansion arms D emission for the next prompt (so the first prompt cannot
/// emit a D for an exit that never happened), and the B/C markers separate
/// user input from command execution. A user PS0 keeps expanding after the
/// markers.
pub(crate) fn hook_file_contents() -> &'static str {
    static CONTENTS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let mut out = String::with_capacity(2048);
        out.push_str(
            "# trailboss shell integration for interactive Bash (OSC 133 status markers).\n",
        );
        out.push_str("# managed by trailboss; reinstalling the integration replaces this file.\n");
        out.push_str("# TRAILBOSS_INTEGRATION_ID=shell\n");
        out.push_str("# TRAILBOSS_INTEGRATION_VERSION=");
        out.push_str(&SHELL_INTEGRATION_VERSION.to_string());
        out.push_str("\n\n");
        out.push_str("# Loading again repairs rather than duplicates: re-running the user's\n");
        out.push_str("# rc files can overwrite PROMPT_COMMAND and PS0 after this hook\n");
        out.push_str("# installed its entries, so the checks below re-install only what is\n");
        out.push_str("# missing and never touch the command-armed marker state.\n");
        out.push_str("# Markers are only for interactive shells attached to a terminal.\n");
        out.push_str("case $- in\n    *i*) ;;\n    *) return 0 ;;\nesac\n");
        out.push_str("[ -t 1 ] || return 0\n\n");
        out.push_str(PROMPT_MARK_FUNCTION);
        out.push_str("\n\n");
        out.push_str("# PS0 expands after a command line is read, before it executes: the\n");
        out.push_str("# markers separate user input from execution and arm D for the next\n");
        out.push_str("# prompt; a user PS0, if any, keeps expanding after them.\n");
        out.push_str("__trailboss_ps0_markers=$'\\033]133;B\\007\\033]133;C\\007'\n");
        out.push_str("if [[ ${PS0:-} != *\"${__trailboss_ps0_markers}\"* ]]; then\n");
        out.push_str(
            "    PS0=\"\\${__TRAILBOSS_COMMAND_STARTED:=}${__trailboss_ps0_markers}${PS0:-}\"\n",
        );
        out.push_str("fi\n\n");
        out.push_str("# Hook first so the previous exit status is captured before any user\n");
        out.push_str("# entry can observe it, and returned unchanged afterwards. An entry\n");
        out.push_str("# that survived a reload is left exactly where it is, so re-sourcing\n");
        out.push_str("# repairs an overwritten hook without ever hooking twice.\n");
        out.push_str("if [[ ${PROMPT_COMMAND[*]:-} != *__trailboss_prompt_mark* ]]; then\n");
        out.push_str("    case \"$(declare -p PROMPT_COMMAND 2>/dev/null)\" in\n");
        out.push_str("        'declare -a'*)\n");
        out.push_str("            PROMPT_COMMAND=('");
        out.push_str(PROMPT_MARK_PRELUDE);
        out.push_str("' \"${PROMPT_COMMAND[@]}\")\n");
        out.push_str("            ;;\n");
        out.push_str("        'declare'*)\n");
        out.push_str("            PROMPT_COMMAND='");
        out.push_str(PROMPT_MARK_PRELUDE);
        out.push_str("; '\"${PROMPT_COMMAND}\"\n");
        out.push_str("            ;;\n");
        out.push_str("        *)\n");
        out.push_str("            PROMPT_COMMAND='");
        out.push_str(PROMPT_MARK_PRELUDE);
        out.push_str("'\n");
        out.push_str("            ;;\n");
        out.push_str("    esac\n");
        out.push_str("fi\n");
        out
    });
    CONTENTS.as_str()
}

/// Sources the user's normal non-login rcfile before loading the hook.
/// Bash loads its system rcfile itself, including when `--rcfile` is used.
/// Login panes keep native login startup and use the managed installation.
pub(crate) fn pane_bash_rcfile_contents() -> &'static str {
    static CONTENTS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let hook = hook_file_contents();
        let mut out = String::with_capacity(hook.len() + 320);
        out.push_str(
            "# trailboss pane rcfile for interactive non-login Bash; regenerated as needed.\n",
        );
        out.push_str("# Replays Bash's own non-login interactive startup, then loads the\n");
        out.push_str("# shell integration hook after user configuration.\n");
        out.push_str("[ -r \"${HOME:-}/.bashrc\" ] && . \"${HOME:-}/.bashrc\"\n\n");
        out.push_str(hook);
        out
    });
    CONTENTS.as_str()
}

/// Path to the pane rcfile wrapper, creating or refreshing it first. Returns
/// `None` for shells that are not Bash (left explicitly uninstrumented) and
/// on Windows, which is not supported.
pub(crate) fn pane_bash_rcfile(shell: &str) -> io::Result<Option<PathBuf>> {
    if !is_bash_shell(shell) || cfg!(windows) {
        return Ok(None);
    }
    let path = crate::config::config_dir().join(PANE_BASH_RCFILE_NAME);
    let desired = pane_bash_rcfile_contents();
    if !fs::read_to_string(&path).is_ok_and(|content| content == desired) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_config(&path, desired)?;
    }
    Ok(Some(path))
}

/// Managed source block added to Bash startup files. The hook location is
/// resolved at session time through the same default the installer used, so a
/// moved or shared HOME keeps working. `${BASH_VERSION-}` (never
/// `$BASH_VERSION`) keeps the shared `.profile` safe under `set -u` in
/// sh-family shells that also read it; `[[ ... ]]` and `source` only evaluate
/// after that guard passes.
pub(crate) fn shell_rc_block() -> String {
    let hook = "${XDG_CONFIG_HOME:-$HOME/.config}/trailboss/shell-integration.bash";
    format!(
        "{SHELL_RC_BLOCK_BEGIN}\nif [ -n \"${{BASH_VERSION-}}\" ] && [[ $- == *i* && -r \"{hook}\" ]]; then\n    source \"{hook}\"\nfi\n{SHELL_RC_BLOCK_END}"
    )
}

/// Appends (or replaces, on reinstall) the managed block after existing
/// content, preserving every unrelated line.
pub(crate) fn build_rc_with_shell_block(content: &str) -> String {
    let mut result = remove_shell_block(content);
    // This separator belongs to the managed block, including when the
    // original file did not end with a newline.
    result.push('\n');
    result.push_str(&shell_rc_block());
    result.push('\n');
    result
}

/// Removes the managed block if present; any other content is returned
/// unchanged, byte for byte.
pub(crate) fn remove_shell_block(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut block_start = None;
    let mut copied_through = 0;
    let mut offset = 0;

    for line in content.split_inclusive('\n') {
        if block_start.is_none() && line.trim() == SHELL_RC_BLOCK_BEGIN {
            let start = if offset > copied_through && content.as_bytes()[offset - 1] == b'\n' {
                offset - 1
            } else {
                offset
            };
            block_start = Some(start);
        } else if line.trim() == SHELL_RC_BLOCK_END {
            if let Some(start) = block_start.take() {
                result.push_str(&content[copied_through..start]);
                copied_through = offset + line.len();
            }
        }
        offset += line.len();
    }

    // An incomplete block is left untouched rather than deleting user data.
    result.push_str(&content[copied_through..]);
    result
}

pub(crate) fn rc_file_contains_shell_block(content: &str) -> bool {
    content
        .lines()
        .any(|line| line.trim() == SHELL_RC_BLOCK_BEGIN)
}

pub(crate) fn shell_hook_path() -> PathBuf {
    crate::config::config_dir().join(SHELL_HOOK_INSTALL_NAME)
}

/// First existing file in Bash's login-startup precedence. When none exists,
/// `.bash_profile` is the created candidate — and only then, so a created
/// file can never shadow a preexisting lower-precedence profile.
fn login_startup_file(home: &Path) -> PathBuf {
    for name in BASH_LOGIN_RC_NAMES {
        let candidate = home.join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    home.join(BASH_LOGIN_RC_NAMES[0])
}

fn created_files_state_path(hook_path: &Path) -> PathBuf {
    hook_path.with_file_name(SHELL_CREATED_FILES_STATE_NAME)
}

fn write_created_files_state(hook_path: &Path, created: &[PathBuf]) -> io::Result<()> {
    let mut contents = String::new();
    for path in created {
        contents.push_str(&path.to_string_lossy());
        contents.push('\n');
    }
    fs::write(created_files_state_path(hook_path), contents)
}

fn read_created_files_state(hook_path: &Path) -> Vec<PathBuf> {
    fs::read_to_string(created_files_state_path(hook_path))
        .map(|contents| {
            contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Installs the hook file plus managed source blocks in `.bashrc` (interactive
/// non-login shells) and the Bash login startup file chosen by precedence.
pub(crate) fn install_shell() -> io::Result<ShellInstallPaths> {
    if cfg!(windows) {
        return Err(io::Error::other(
            "shell integration is not supported on Windows yet",
        ));
    }

    let home = super::env::home_dir()?;
    let login_shell = std::env::var("SHELL").unwrap_or_default();
    if !is_bash_shell(&login_shell) {
        return Err(io::Error::other(format!(
            "shell integration currently supports interactive Bash only; login shell {login_shell:?} stays uninstrumented"
        )));
    }

    let hook_path = shell_hook_path();
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&hook_path, hook_file_contents())?;

    let mut rc_paths = Vec::new();
    let mut created_rc_files = read_created_files_state(&hook_path);
    for path in [home.join(BASH_RC_NAME), login_startup_file(&home)] {
        let existed = path.is_file();
        let existing = if existed {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };
        let updated = build_rc_with_shell_block(&existing);
        if updated != existing {
            write_config(&path, &updated)?;
        }
        if !existed && !created_rc_files.contains(&path) {
            created_rc_files.push(path.clone());
        }
        rc_paths.push(path);
    }

    write_created_files_state(&hook_path, &created_rc_files)?;

    Ok(ShellInstallPaths {
        hook_path,
        rc_paths,
        created_rc_files,
    })
}

/// Removes the managed blocks from every Bash startup file, deletes the hook
/// file, and removes startup files the installer created when nothing but the
/// block remains. User-created files are never deleted.
pub(crate) fn uninstall_shell() -> io::Result<ShellUninstallResult> {
    if cfg!(windows) {
        return Err(io::Error::other(
            "shell integration is not supported on Windows yet",
        ));
    }

    let home = super::env::home_dir()?;
    let hook_path = shell_hook_path();
    let created = read_created_files_state(&hook_path);

    let mut names: Vec<&str> = vec![BASH_RC_NAME];
    names.extend(BASH_LOGIN_RC_NAMES);
    let mut updated_rc_files = Vec::new();
    let mut removed_created_rc_files = Vec::new();
    for name in names {
        let path = home.join(name);
        let Ok(existing) = fs::read_to_string(&path) else {
            continue;
        };
        let updated = remove_shell_block(&existing);
        if updated == existing {
            continue;
        }
        if created.contains(&path) && updated.trim().is_empty() {
            fs::remove_file(&path)?;
            removed_created_rc_files.push(path);
        } else {
            write_config(&path, &updated)?;
            updated_rc_files.push(path);
        }
    }

    let removed_hook_file = remove_file_if_exists(&hook_path)?;
    let _ = remove_file_if_exists(&created_files_state_path(&hook_path));

    Ok(ShellUninstallResult {
        hook_path,
        removed_hook_file,
        updated_rc_files,
        removed_created_rc_files,
    })
}

/// True when any Bash startup file still carries the managed source block; a
/// hook file without it is nonfunctional.
pub(crate) fn shell_block_is_installed() -> bool {
    let Some(home) = super::env::home_dir().ok() else {
        return false;
    };
    let mut names: Vec<&str> = vec![BASH_RC_NAME];
    names.extend(BASH_LOGIN_RC_NAMES);
    names.into_iter().any(|name| {
        fs::read_to_string(home.join(name))
            .map(|content| rc_file_contains_shell_block(&content))
            .unwrap_or(false)
    })
}
