//! Background WebSocket client subscribing to `nve dev` (`HotPatchMsg` JSON frames) and waking
//! the `winit` event loop whenever a hot-reload patch arrives.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver};
use nve_engine::HotPatchMsg;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

pub(crate) fn resolve_dylib_path(raw_path: &str) -> PathBuf {
    let p = PathBuf::from(raw_path);
    if p.is_absolute() || p.exists() {
        return p;
    }
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join(&p);
        if candidate.exists() {
            return candidate;
        }
    }
    p
}

/// Background WebSocket client subscribing to `nve dev` (`HotPatchMsg` JSON frames) and waking
/// the UI event loop whenever a patch arrives.
pub struct HotReloadClient {
    rx: Receiver<HotPatchMsg>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HotReloadClient {
    pub fn spawn<F>(ws_url: String, on_wake: F) -> Self
    where
        F: Fn() + Send + 'static,
    {
        let (tx, rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let worker = thread::Builder::new()
            .name("nve-hot-reload-ws".to_string())
            .spawn(move || {
                let mut backoff = Duration::from_millis(150);
                let max_backoff = Duration::from_secs(2);

                while !stop_flag.load(Ordering::Relaxed) {
                    match tungstenite::connect(ws_url.as_str()) {
                        Ok((mut ws, _response)) => {
                            backoff = Duration::from_millis(150);
                            if let MaybeTlsStream::Plain(stream) = ws.get_ref() {
                                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                            }
                            while !stop_flag.load(Ordering::Relaxed) {
                                match ws.read() {
                                    Ok(Message::Binary(bytes)) => {
                                        if let Ok(msg) = HotPatchMsg::from_bytes(&bytes) {
                                            if tx.send(msg).is_err() {
                                                return;
                                            }
                                            on_wake();
                                        }
                                    }
                                    Ok(Message::Ping(payload)) => {
                                        let _ = ws.send(Message::Pong(payload));
                                    }
                                    Ok(Message::Close(_)) => break,
                                    Ok(_) => {}
                                    Err(tungstenite::Error::Io(ref err))
                                        if err.kind() == ErrorKind::WouldBlock
                                            || err.kind() == ErrorKind::TimedOut =>
                                    {
                                        continue;
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                        Err(_) => {
                            let deadline = Instant::now() + backoff;
                            while Instant::now() < deadline && !stop_flag.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(50));
                            }
                            backoff = (backoff * 2).min(max_backoff);
                        }
                    }
                }
            })
            .ok();

        Self { rx, stop, worker }
    }

    pub fn try_drain(&self) -> Vec<HotPatchMsg> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            out.push(msg);
        }
        out
    }

    pub fn receiver(&self) -> &Receiver<HotPatchMsg> {
        &self.rx
    }
}

impl Drop for HotReloadClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
