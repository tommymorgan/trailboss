//! OpenAI-compatible loopback model provider shared by restricted-agent tests.

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

/// OpenAI-compatible loopback provider owned by the test. Each completion is
/// held until the test releases it, so work and single-outstanding admission
/// are observed deterministically without contacting a real model.
pub struct StubProvider {
    pub port: u16,
    requests: mpsc::Receiver<String>,
    release: mpsc::Sender<()>,
}

impl StubProvider {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_tx, requests) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let request_tx = request_tx.clone();
                let release_rx = Arc::clone(&release_rx);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                length = value.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let mut body = vec![0; length];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let _ = request_tx.send(String::from_utf8_lossy(&body).into_owned());
                    let released = release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(60));
                    if released.is_err() {
                        return;
                    }
                    let chunk = |delta: Value, finish: Value| {
                        format!(
                            "data: {}\n\n",
                            json!({"id":"owned-stub","object":"chat.completion.chunk","created":0,
                                "model":"stub","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
                        )
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{}{}data: [DONE]\n\n",
                        chunk(json!({"role":"assistant","content":"owned stub reply"}), Value::Null),
                        chunk(json!({}), json!("stop")),
                    );
                    let _ = stream.flush();
                });
            }
        });
        Self {
            port,
            requests,
            release,
        }
    }

    pub fn next_request(&self) -> String {
        self.requests
            .recv_timeout(Duration::from_secs(20))
            .expect("restricted submission never reached the owned model provider")
    }

    pub fn assert_no_request(&self, within: Duration, context: &str) {
        if let Ok(body) = self.requests.recv_timeout(within) {
            panic!("{context}: an unexpected model request was invoked: {body}");
        }
    }

    pub fn release(&self) {
        self.release.send(()).unwrap();
    }
}
