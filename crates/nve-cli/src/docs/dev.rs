//! `cargo nve docs dev` — builds static docs (+ optional WASM), serves over a lightweight
//! stdlib HTTP server with directory-index/SPA resolution, and watches `docs/content/` via
//! `notify` for incremental rebuilds.

use std::env;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crossbeam_channel::unbounded;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::docs::build::{BuildOptions, run_build};
use crate::static_http;

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

    static_http::serve(listener, out_dir, rebuild_lock, None);
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
