//! `cargo nve docs dev` — builds static docs (+ optional WASM), serves over a lightweight
//! stdlib HTTP server with directory-index/SPA resolution, and watches `docs/content/` via
//! `notify` for incremental rebuilds.

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crossbeam_channel::unbounded;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::docs::build::{BuildOptions, run_build};

/// Start the local static docs server and incremental `docs/content/` file watcher.
pub(crate) fn run_dev(content_dir: &Path, port: u16, skip_wasm: bool) -> Result<(), String> {
    let out_dir = env::temp_dir().join(format!("nve-docs-dev-{}", std::process::id()));
    let initial_opts = BuildOptions {
        content: content_dir.to_path_buf(),
        out: out_dir.clone(),
        base_path: String::new(),
        site_url: format!("http://127.0.0.1:{port}"),
        skip_wasm,
    };

    run_build(&initial_opts)?;

    let rebuild_lock = Arc::new(RwLock::new(()));
    let watch_content = content_dir.to_path_buf();
    let watch_out = out_dir.clone();
    let watch_lock = Arc::clone(&rebuild_lock);
    let site_url = format!("http://127.0.0.1:{port}");

    thread::spawn(move || {
        watch_and_rebuild(watch_content, watch_out, site_url, watch_lock);
    });

    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("failed to bind 127.0.0.1:{port}: {e}"))?;
    println!(
        "==> Serving NVE docs at http://127.0.0.1:{port}/ (watching {})",
        content_dir.display()
    );
    println!("    Press Ctrl+C to stop.");

    for incoming in listener.incoming() {
        let Ok(stream) = incoming else { continue };
        let root = out_dir.clone();
        let lock = Arc::clone(&rebuild_lock);
        thread::spawn(move || {
            let _ = handle_http_connection(stream, &root, &lock);
        });
    }

    Ok(())
}

fn watch_and_rebuild(
    content_dir: PathBuf,
    out_dir: PathBuf,
    site_url: String,
    rebuild_lock: Arc<RwLock<()>>,
) {
    let (tx, rx) = unbounded::<notify::Result<Event>>();
    let Ok(mut watcher) = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    ) else {
        eprintln!("[docs dev] warning: could not initialize file watcher");
        return;
    };

    if let Err(err) = watcher.watch(&content_dir, RecursiveMode::Recursive) {
        eprintln!("[docs dev] warning: watch failed: {err}");
        return;
    }

    while let Ok(evt_res) = rx.recv() {
        if evt_res.is_err() {
            continue;
        }
        // Coalesce burst saves within 150ms.
        while rx.recv_timeout(Duration::from_millis(150)).is_ok() {}

        println!("[docs dev] change detected -> rebuilding static pages...");
        let _guard = rebuild_lock.write();
        let opts = BuildOptions {
            content: content_dir.clone(),
            out: out_dir.clone(),
            base_path: String::new(),
            site_url: site_url.clone(),
            // Keep previously compiled WASM bundle in assets/ for instant incremental rebuilds.
            skip_wasm: true,
        };
        if let Err(err) = run_build(&opts) {
            eprintln!("[docs dev] rebuild error: {err}");
        }
    }
}

fn handle_http_connection(
    mut stream: TcpStream,
    root: &Path,
    rebuild_lock: &RwLock<()>,
) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let raw_target = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/");
    let path_only = raw_target.split('?').next().unwrap_or("/");

    let _read_guard = rebuild_lock.read();
    let (status, file_path) = resolve_static_file(root, path_only);
    match file_path.and_then(|p| fs::read(&p).ok().map(|body| (p, body))) {
        Some((resolved, body)) => {
            let mime = mime_type_for(&resolved);
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes())?;
            stream.write_all(&body)?;
        }
        None => {
            let body = b"Not Found";
            let header = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes())?;
            stream.write_all(body)?;
        }
    }
    Ok(())
}

fn resolve_static_file(root: &Path, req_path: &str) -> (&'static str, Option<PathBuf>) {
    let clean = req_path.trim_start_matches('/');
    let mut rel = PathBuf::new();
    for seg in clean.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            continue;
        }
        rel.push(seg);
    }

    let direct = root.join(&rel);
    if direct.is_file() {
        return ("200 OK", Some(direct));
    }
    let dir_index = direct.join("index.html");
    if dir_index.is_file() {
        return ("200 OK", Some(dir_index));
    }
    let fallback_404 = root.join("404.html");
    if fallback_404.is_file() {
        return ("404 Not Found", Some(fallback_404));
    }
    ("404 Not Found", None)
}

fn mime_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("nvs1") => "application/octet-stream",
        _ => "application/octet-stream",
    }
}
