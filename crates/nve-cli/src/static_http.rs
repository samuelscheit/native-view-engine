//! Minimal stdlib static HTTP/1.1 file server shared by `cargo nve docs dev` and `nve dev`
//! (directory-index resolution, optional `404.html`, `..`-safe paths, `no-store` caching and an
//! optional redirect hook evaluated per request).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

/// Maps a request target (`/path?query`) and its `Host` header to a `302` location, or `None` to
/// serve the file.
pub(crate) type RedirectFn = dyn Fn(&str, Option<&str>) -> Option<String> + Send + Sync;
pub(crate) type Redirect = Arc<RedirectFn>;

/// Serves `root` on `listener` forever, one thread per connection. Requests hold a read lock on
/// `rebuild_lock` so a writer can swap files atomically.
pub(crate) fn serve(
    listener: TcpListener,
    root: PathBuf,
    rebuild_lock: Arc<RwLock<()>>,
    redirect: Option<Redirect>,
) {
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else { continue };
        let root = root.clone();
        let lock = Arc::clone(&rebuild_lock);
        let redirect = redirect.clone();
        thread::spawn(move || {
            let _ = handle_connection(stream, &root, &lock, redirect.as_deref());
        });
    }
}

fn handle_connection(
    mut stream: TcpStream,
    root: &Path,
    rebuild_lock: &RwLock<()>,
    redirect: Option<&RedirectFn>,
) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut host = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim_end().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("host")
        {
            host = Some(value.trim().to_owned());
        }
    }
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");

    if let Some(location) = redirect.and_then(|r| r(target, host.as_deref())) {
        let header = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
        );
        return stream.write_all(header.as_bytes());
    }

    let path_only = target.split('?').next().unwrap_or("/");
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
            stream.write_all(&body)
        }
        None => {
            let body = b"Not Found";
            let header = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes())?;
            stream.write_all(body)
        }
    }
}

/// Resolves `req_path` under `root` (ignoring `.`/`..` segments), falling back to the directory's
/// `index.html` and then `root/404.html`.
pub(crate) fn resolve_static_file(root: &Path, req_path: &str) -> (&'static str, Option<PathBuf>) {
    let mut rel = PathBuf::new();
    for seg in req_path.trim_start_matches('/').split('/') {
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

pub(crate) fn mime_type_for(path: &Path) -> &'static str {
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
        _ => "application/octet-stream",
    }
}
