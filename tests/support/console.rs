use super::support::{Core, HOMES};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};
pub struct Console {
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    pub(crate) writer: Box<dyn Write + Send>,
    pub(crate) rx: std::sync::mpsc::Receiver<Vec<u8>>,
    pub screen: ghostty_vt::Terminal,
}
impl Console {
    #[allow(dead_code)] // Shared harness: remote-only callers supply their own command.
    pub fn new(core: &Core) -> Self {
        let mut command = portable_pty::CommandBuilder::new(super::support::executable());
        command.args(["--session", &core.name]);
        for key in HOMES {
            command.env(key, &core.home);
        }
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            let name = key.to_string_lossy();
            name.starts_with("TRAILBOSS_") || name.starts_with("HERDR_")
        }) {
            command.env_remove(key);
        }
        command.env("TRAILBOSS_CONFIG_PATH", core.home.join("config.toml"));
        command.env("TERM", "xterm-256color");
        command.env("SHELL", "/bin/sh");
        command.env_remove("NO_COLOR");
        Self::from_command(command)
    }
    pub fn from_command(command: portable_pty::CommandBuilder) -> Self {
        Self::sized(command, 80, 24)
    }
    pub fn sized(command: portable_pty::CommandBuilder, cols: u16, rows: u16) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0; 16384];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || tx.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            _master: pair.master,
            writer,
            rx,
            screen: ghostty_vt::Terminal::new(cols, rows, 0).unwrap(),
        }
    }
    pub(crate) fn rows(&self) -> Vec<String> {
        self.screen
            .screen_text_rows()
            .unwrap()
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        cell.graphemes
                            .first()
                            .and_then(|cp| char::from_u32(*cp))
                            .unwrap_or(' ')
                    })
                    .collect()
            })
            .collect()
    }
    pub fn wait(&mut self, predicate: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let rows = self.rows();
            if predicate(&rows) {
                match self.rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(bytes) => {
                        self.screen.write(&bytes);
                        continue;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return rows,
                    Err(error) => panic!("executable output closed: {error}"),
                }
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "executable exited: {rows:?}"
            );
            let bytes = self
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!("executable composition observation: {error}; {rows:?}")
                });
            self.screen.write(&bytes);
        }
    }
    #[allow(dead_code)] // Shared harness: keyboard-only scenarios do not click.
    pub fn click(&mut self, x: usize, y: usize) {
        self.writer
            .write_all(
                format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes(),
            )
            .unwrap();
        self.writer.flush().unwrap();
    }
    pub fn text(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.writer.write_all(b"\x02q");
        let _ = self.writer.flush();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().is_ok_and(|status| status.is_none())
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
