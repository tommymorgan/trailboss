#[path = "support/console.rs"]
mod console;
mod support;
use console::Console;
use serde_json::json;
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use support::{Core, HOMES};

// A complete 1x1 GIF89a, not a signature-only image or private clipboard content.
const IMAGE: &[u8] = b"GIF89a\x01\0\x01\0\x80\0\0\xff\0\0\0\0\0\x21\xf9\x04\x01\0\0\0\0\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0\x3b";
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn checked(c: &mut Command) -> String {
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "{c:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
struct Clipboard {
    remote: bool,
    root: String,
    display: String,
    owner: Option<Child>,
    server: Option<Child>,
}
impl Clipboard {
    fn new(remote: bool) -> Self {
        if remote {
            let root = checked(Command::new("ssh").args([
                "-o",
                "BatchMode=yes",
                "achy",
                "mktemp -d /tmp/trailboss-task52.XXXXXXXX",
            ]))
            .trim()
            .to_owned();
            let mut cb = Self {
                remote,
                root,
                display: String::new(),
                owner: None,
                server: None,
            };
            checked(Command::new("scp").args([
                "-q",
                env!("CARGO_BIN_EXE_trailboss"),
                &format!("achy:{}/trailboss", cb.root),
            ]));
            cb.server = Some(
                Command::new("ssh")
                    .args([
                        "-o",
                        "BatchMode=yes",
                        "achy",
                        &format!(
                            "/bin/sh -c {}",
                            quote(&format!("Xvfb -displayfd 1 -screen 0 640x480x24 -nolisten tcp >{root}/display & pid=$!; echo $pid >{root}/xvfb.pid; trap 'kill $pid; wait $pid' TERM INT EXIT; wait $pid", root=quote(&cb.root)))
                        ),
                    ])
                    .spawn()
                    .unwrap(),
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                cb.display = checked(Command::new("ssh").args([
                    "-o",
                    "BatchMode=yes",
                    "achy",
                    &format!(
                        "/bin/sh -c {}",
                        quote(&format!(
                            "test ! -f {root}/display || cat {root}/display",
                            root = quote(&cb.root)
                        ))
                    ),
                ]))
                .trim()
                .to_owned();
                if !cb.display.is_empty() {
                    break;
                }
                assert!(Instant::now() < deadline, "isolated achy Xvfb startup");
                std::thread::sleep(Duration::from_millis(20));
            }
            cb.display = format!(":{}", cb.display);
            cb
        } else {
            let root =
                std::env::temp_dir().join(format!("trailboss-task52-{}", std::process::id()));
            fs::create_dir(&root).unwrap();
            let file = fs::File::create(root.join("display")).unwrap();
            let executable = std::env::var("TRAILBOSS_TEST_XVFB").unwrap_or_else(|_| "Xvfb".into());
            let mut cb = Self {
                remote,
                root: root.to_str().unwrap().into(),
                display: String::new(),
                owner: None,
                server: Some(
                    Command::new(executable)
                        .args([
                            "-displayfd",
                            "1",
                            "-screen",
                            "0",
                            "640x480x24",
                            "-nolisten",
                            "tcp",
                        ])
                        .stdout(file)
                        .spawn()
                        .unwrap(),
                ),
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let n = fs::read_to_string(root.join("display")).unwrap();
                if !n.trim().is_empty() {
                    cb.display = format!(":{}", n.trim());
                    break;
                }
                assert!(Instant::now() < deadline, "isolated local Xvfb startup");
                std::thread::sleep(Duration::from_millis(20));
            }
            cb
        }
    }
    fn publish(&mut self, mime: &str, bytes: &[u8]) {
        if let Some(mut owner) = self.owner.take() {
            let _ = owner.kill();
            let _ = owner.wait();
        }
        let mut cmd = if self.remote {
            let mut c = Command::new("ssh");
            c.args([
                "-o",
                "BatchMode=yes",
                "achy",
                &format!(
                    "DISPLAY={} /usr/bin/xclip -selection clipboard -t {} -i -quiet",
                    quote(&self.display),
                    quote(mime)
                ),
            ]);
            c
        } else {
            let mut c = Command::new("/usr/bin/xclip");
            c.args(["-selection", "clipboard", "-t", mime, "-i", "-quiet"])
                .env("DISPLAY", &self.display);
            c
        };
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        self.owner = Some(child);
        std::thread::sleep(Duration::from_millis(80));
    }
    fn console(&self, core: &Core) -> Console {
        if !self.remote {
            let mut c = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_trailboss"));
            c.args(["--session", &core.name]);
            for key in HOMES {
                c.env(key, &core.home);
            }
            for (key, _) in std::env::vars_os().filter(|(k, _)| {
                let name = k.to_string_lossy();
                name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
            }) {
                c.env_remove(key);
            }
            c.env("TRAILBOSS_CONFIG_PATH", core.home.join("config.toml"));
            c.env("DISPLAY", &self.display);
            c.env_remove("WAYLAND_DISPLAY");
            c.env("PATH", "/usr/bin:/bin");
            c.env("TERM", "xterm-256color");
            Console::from_command(c)
        } else {
            let wrapper = core.home.join("remote-core");
            let mut script = String::from("#!/bin/sh\n");
            for key in HOMES {
                script.push_str(&format!(
                    "export {key}={}\n",
                    quote(core.home.to_str().unwrap())
                ));
            }
            script.push_str(&format!(
                "export TRAILBOSS_CONFIG_PATH={}\nexec {} \"$@\"\n",
                quote(core.home.join("config.toml").to_str().unwrap()),
                quote(env!("CARGO_BIN_EXE_trailboss"))
            ));
            fs::write(&wrapper, script).unwrap();
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
            let mut c = portable_pty::CommandBuilder::new("ssh");
            c.args(["-tt", "-o", "BatchMode=yes", "achy"]);
            c.arg(format!("env -u WAYLAND_DISPLAY DISPLAY={display} PATH=/usr/bin:/bin TERM=xterm-256color XDG_CONFIG_HOME={root} XDG_STATE_HOME={root} XDG_DATA_HOME={root} XDG_CACHE_HOME={root} XDG_RUNTIME_DIR={root} TRAILBOSS_CONFIG_PATH={root}/config.toml {root}/trailboss --remote junkman --remote-executable {wrapper} --session {name}",display=quote(&self.display),root=quote(&self.root),wrapper=quote(wrapper.to_str().unwrap()),name=quote(&core.name)));
            Console::from_command(c)
        }
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        if let Some(mut p) = self.owner.take() {
            let _ = p.kill();
            let _ = p.wait();
        }
        if let Some(mut p) = self.server.take() {
            let _ = p.kill();
            let _ = p.wait();
        }
        if self.remote {
            let _ = Command::new("ssh")
                .args([
                    "-o",
                    "BatchMode=yes",
                    "achy",
                    &format!("/bin/sh -c {}", quote(&format!("test ! -f {root}/xvfb.pid || kill $(cat {root}/xvfb.pid); rm -rf -- {root}",root=quote(&self.root)))), 
                ])
                .status();
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
struct Proof {
    clipboard: Clipboard,
    core: Core,
    console: Console,
    panes: Vec<String>,
    files: Vec<PathBuf>,
}
impl Proof {
    fn given(remote: bool) -> Self {
        let core = Core::new();
        let first = core.request("session.snapshot", json!({}))["snapshot"]["panes"][0]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second = core.request(
            "pane.split",
            json!({"target_pane_id":first,"direction":"right","focus":false}),
        )["pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut clipboard = Clipboard::new(remote);
        clipboard.publish("image/gif", IMAGE);
        let console = clipboard.console(&core);
        Self {
            clipboard,
            core,
            console,
            panes: vec![first, second],
            files: vec![],
        }
    }
    fn when(&mut self) {
        self.when_pasting("\x1b[200~\x1b[201~");
    }
    fn when_pasting(&mut self, trigger: &str) {
        for pane in self.panes.clone() {
            self.core.request("pane.focus", json!({"pane_id":pane}));
            let marker = format!("ready-{pane}");
            self.core.request("pane.send_text",json!({"pane_id":pane,"text":format!("printf '\\033[2J\\033[H{marker}\\n'\n"),"submit":false}));
            self.console
                .wait(|rows| rows.iter().any(|r| r.contains(&marker)));
            let other_pane = self.panes.iter().find(|p| *p != &pane).unwrap();
            let other_before = self.core.request(
                "pane.read",
                json!({"pane_id":other_pane,"source":"visible"}),
            )["read"]["text"]
                .as_str()
                .unwrap()
                .to_owned();
            self.console.text(trigger);
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                let text = self
                    .core
                    .request("pane.read", json!({"pane_id":pane,"source":"visible"}))["read"]
                    ["text"]
                    .as_str()
                    .unwrap()
                    .replace('\n', "");
                if let Some(start) = text.find("/tmp/trailboss-clipboard-images-") {
                    let path = text[start..].split_whitespace().next().unwrap();
                    self.files.push(PathBuf::from(path));
                    let other = self.core.request(
                        "pane.read",
                        json!({"pane_id":other_pane,"source":"visible"}),
                    )["read"]["text"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                    assert_eq!(
                        other, other_before,
                        "nonfocused pane changed during image paste"
                    );
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "image paste must inject staged server path, got {text:?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            self.console.text("\x15");
        }
    }
    fn assert_ctrl_v_passthrough(&mut self) {
        self.console.text("saved=$(stty -g); stty raw -echo; printf '\\033[2J\\033[Hctrl-%s\\n' ready; byte=$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d '[:space:]'); stty \"$saved\"; printf '\\nbyte-%s\\n' \"$byte\"\r");
        self.console
            .wait(|rows| rows.iter().any(|r| r.contains("ctrl-ready")));
        self.console.text("\x16");
        self.console
            .wait(|rows| rows.iter().any(|r| r.contains("byte-16")));
    }
    fn then(&mut self) {
        let uid = checked(Command::new("id").arg("-u"))
            .trim()
            .parse::<u32>()
            .unwrap();
        assert_eq!(self.files.len(), 2);
        for (i, path) in self.files.iter().enumerate() {
            assert_eq!(
                path.parent().unwrap(),
                PathBuf::from(format!("/tmp/trailboss-clipboard-images-{uid}"))
            );
            assert_eq!(
                fs::read(path).unwrap(),
                IMAGE,
                "actual system clipboard bytes staged on server"
            );
            let meta = fs::metadata(path).unwrap();
            assert_eq!(meta.uid(), uid);
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            let other = self.core.request(
                "pane.read",
                json!({"pane_id":self.panes[1-i],"source":"visible"}),
            )["read"]["text"]
                .as_str()
                .unwrap()
                .replace('\n', "");
            assert!(
                !other.contains(path.to_str().unwrap()),
                "image injected into other pane"
            );
            println!(
                "mode={} pane={} path={} uid={uid} bytes={} checksum={}",
                if self.clipboard.remote {
                    "remote from achy"
                } else {
                    "local"
                },
                self.panes[i],
                path.display(),
                IMAGE.len(),
                checked(Command::new("sha256sum").arg(path)).trim()
            );
        }
        // Text supplied by the terminal wins over stale clipboard images.
        self.console.text("\x1b[200~literal-text-paste\x1b[201~");
        self.console
            .wait(|rows| rows.iter().any(|r| r.contains("literal-text-paste")));
        self.console.text("\x15");
        // A mislabeled image cannot cross the retained signature boundary.
        self.clipboard.publish("image/gif", b"not-an-image");
        self.console.text("\x1b[200~\x1b[201~");
        self.console
            .text("printf '\\033[2J\\033[Hfallback-%s\\n' okay\r");
        self.console
            .wait(|rows| rows.iter().any(|r| r.contains("fallback-okay")));
        self.clipboard
            .publish("UTF8_STRING", b"plain clipboard text");
        self.assert_ctrl_v_passthrough();
        self.console.text("\x02q");
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.console.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "normal detach");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Proof {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = fs::remove_file(file);
        }
    }
}
#[test]
#[ignore = "requires isolated Xvfb and native xclip; never accesses human clipboard"]
fn actual_local_clipboard_paste_stages_only_focused_pane() {
    let mut p = Proof::given(false);
    p.assert_ctrl_v_passthrough();
    p.when();
    p.then();
}
#[test]
#[ignore = "requires actual achy clipboard and achy -> junkman SSH"]
fn actual_achy_clipboard_paste_stages_only_focused_pane() {
    let mut p = Proof::given(true);
    p.when();
    p.then();
}

#[test]
#[ignore = "requires isolated achy Xvfb clipboard and achy -> junkman SSH"]
fn actual_achy_ctrl_v_stages_exact_image_only_in_focused_pane() {
    let mut proof = Proof::given(true);
    proof.when_pasting("\x16");
    proof.then();
}
fn living_example(mode: &str) {
    let lines: Vec<_> = include_str!("../features/clipboard-images.feature")
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(lines[0], "Feature: Clipboard images");
    assert_eq!(
        lines[1],
        "Scenario Outline: Paste clipboard images into the focused pane"
    );
    assert_eq!(
        &lines[5..],
        &["Examples:", "| mode |", "| local |", "| remote from achy |"]
    );
    {
        let mut proof = None;
        for (index, line) in lines[2..5].iter().enumerate() {
            match (index,*line) {
                (0,"Given I am attached in <mode> mode and an image is on the client clipboard") => proof=Some(Proof::given(mode=="remote from achy")),
                (1,"When I paste into the focused pane") => proof.as_mut().unwrap().when(),
                (2,"Then the client intercepts the image, the session core stages it in /tmp/trailboss-clipboard-images-UID/ on the server host where UID is the server user’s UID, and the staged path is injected only into that focused pane") => proof.as_mut().unwrap().then(),
                _ => panic!("unbound clipboard living step: {line}"),
            }
        }
    }
}
#[test]
#[ignore = "requires isolated local clipboard"]
fn paste_clipboard_images_living_local() {
    living_example("local");
}
#[test]
#[ignore = "requires actual achy clipboard and achy -> junkman SSH"]
fn paste_clipboard_images_living_achy() {
    living_example("remote from achy");
}

#[test]
#[ignore = "requires an isolated Xvfb clipboard; subprocess keeps display variables private"]
fn clipboard_input_seam_stages_bytes_and_captures_pane_target() {
    if std::env::var_os("TRAILBOSS_CLIPBOARD_WORKER").is_none() {
        let mut clipboard = Clipboard::new(false);
        clipboard.publish("image/gif", IMAGE);
        checked(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "clipboard_input_seam_stages_bytes_and_captures_pane_target",
                    "--ignored",
                    "--nocapture",
                ])
                .env("TRAILBOSS_CLIPBOARD_WORKER", "1")
                .env("DISPLAY", &clipboard.display)
                .env_remove("WAYLAND_DISPLAY")
                .env("PATH", "/usr/bin:/bin"),
        );
        return;
    }
    let core = Core::new();
    let first = core.request("session.snapshot", json!({}))["snapshot"]["focused_pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = core.request(
        "pane.split",
        json!({"target_pane_id":first,"direction":"right","focus":false}),
    )["pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut client = core.attach();
    while client.focused_pane_id.as_deref() != Some(first.as_str()) {
        client.receive().unwrap();
    }
    let mut input = client.input().unwrap();
    let mut state = trailboss::core::Core::default();
    assert!(
        !input.paste_image(&state).unwrap(),
        "no pane must preserve key dispatch"
    );
    state.update_session(
        client.workspace_id.clone(),
        client.focused_tab_id.clone(),
        true,
    );
    let context = trailboss_runtime::protocol::widgets::Context {
        workspace_id: client.workspace_id.clone(),
        tab_id: client.focused_tab_id.clone(),
        ..Default::default()
    };
    for action in ["host:rename-space", "host:rename-tab", "host:new"] {
        state.invoke(action, &context, None);
        assert!(
            !input.paste_image(&state).unwrap(),
            "host interactions must preserve key dispatch"
        );
        state.invoke("host:cancel", &context, None);
    }
    input.focus_pane_target("");
    assert!(
        !input.paste_image(&state).unwrap(),
        "missing target must preserve key dispatch"
    );
    for pane in [&first, &second] {
        core.request("pane.focus", json!({"pane_id":pane}));
        while client.focused_pane_id.as_deref() != Some(pane) {
            client.receive().unwrap();
        }
        input.focus_pane_target(pane);
        input.paste(&mut state, &client.boot_id, "").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let path = loop {
            let text = core.request("pane.read", json!({"pane_id":pane,"source":"visible"}))
                ["read"]["text"]
                .as_str()
                .unwrap()
                .replace('\n', "");
            if let Some(start) = text.find("/tmp/trailboss-clipboard-images-") {
                break PathBuf::from(text[start..].split_whitespace().next().unwrap());
            }
            assert!(
                Instant::now() < deadline,
                "input paste did not stage/inject image: {text:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(fs::read(&path).unwrap(), IMAGE);
        let other = if pane == &first { &second } else { &first };
        let other_text = core.request("pane.read", json!({"pane_id":other,"source":"visible"}))
            ["read"]["text"]
            .as_str()
            .unwrap()
            .replace('\n', "");
        assert!(!other_text.contains(path.to_str().unwrap()));
        fs::remove_file(path).unwrap();
        input.type_text("\x15").unwrap();
    }
    input.detach().unwrap();
}

#[test]
#[ignore = "requires isolated achy clipboard, SSH, and TRAILBOSS_TEST_OMP executable"]
fn actual_achy_ctrl_v_attaches_image_in_isolated_omp() {
    let executable =
        std::env::var("TRAILBOSS_TEST_OMP").expect("set TRAILBOSS_TEST_OMP to the omp executable");
    let mut proof = Proof::given(true);
    let pane = proof.panes[0].clone();
    for setting in ["startup.setupWizard", "startup.checkUpdate"] {
        checked(
            Command::new(&executable)
                .env_clear()
                .env("HOME", &proof.core.home)
                .env("PATH", "/usr/bin:/bin")
                .args(["config", "set", setting, "false"]),
        );
    }
    let command = format!(
        "env -i HOME={home} XDG_CONFIG_HOME={home} XDG_STATE_HOME={home} XDG_DATA_HOME={home} XDG_CACHE_HOME={home} PATH=/usr/bin:/bin TERM=xterm-256color {executable} --cwd {home} --no-session --no-tools --no-lsp --no-pty --no-extensions --no-skills --no-rules --no-title",
        home = quote(proof.core.home.to_str().unwrap()),
        executable = quote(&executable),
    );
    proof.core.request(
        "pane.send_text",
        json!({"pane_id":pane,"text":format!("{command}\n"),"submit":false}),
    );
    proof
        .console
        .wait(|rows| rows.iter().any(|r| r.contains("No default model")));
    let other_before = proof.core.request(
        "pane.read",
        json!({"pane_id":proof.panes[1],"source":"visible"}),
    )["read"]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    let uid = checked(Command::new("id").arg("-u")).trim().to_owned();
    let directory = PathBuf::from(format!("/tmp/trailboss-clipboard-images-{uid}"));
    let staged_paths = || -> std::collections::BTreeSet<PathBuf> {
        match fs::read_dir(&directory) {
            Ok(entries) => entries.map(|entry| entry.unwrap().path()).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(error) => panic!("cannot inspect staging directory: {error}"),
        }
    };
    let before = staged_paths();
    proof.console.text("\x16");
    proof
        .console
        .wait(|rows| rows.iter().any(|r| r.contains("🖼 #1")));
    let added: Vec<_> = staged_paths().difference(&before).cloned().collect();
    assert_eq!(added.len(), 1, "one Ctrl-V must stage one image");
    proof.files.push(added[0].clone());
    assert_eq!(fs::read(&added[0]).unwrap(), IMAGE);
    let received = proof
        .core
        .request("pane.read", json!({"pane_id":pane,"source":"visible"}))["read"]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        received.contains("🖼 #1"),
        "omp must recognize the attachment"
    );
    assert!(!received.contains("Clipboard is empty"));
    let other_after = proof.core.request(
        "pane.read",
        json!({"pane_id":proof.panes[1],"source":"visible"}),
    )["read"]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(other_after, other_before);
    println!("actual omp received attachment 🖼 #1; staged {} exact bytes at {}; nonfocused pane unchanged", IMAGE.len(), added[0].display());
}
