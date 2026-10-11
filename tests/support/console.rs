use super::support::{Core, HOMES};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};
pub struct Console {
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    pub screen: ghostty_vt::Terminal,
}
impl Console {
    pub fn new(core: &Core) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_herdr-fork"));
        command.args(["--session", &core.name]);
        for key in HOMES {
            command.env(key, &core.home);
        }
        for (key, _) in
            std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("HERDR_"))
        {
            command.env_remove(key);
        }
        command.env("HERDR_CONFIG_PATH", core.home.join("config.toml"));
        command.env("TERM", "xterm-256color");
        command.env("SHELL", "/bin/sh");
        command.env_remove("NO_COLOR");
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
            screen: ghostty_vt::Terminal::new(80, 24, 0).unwrap(),
        }
    }
    fn rows(&self) -> Vec<String> {
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
        let _ = self.writer.write_all(b"\x11");
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
