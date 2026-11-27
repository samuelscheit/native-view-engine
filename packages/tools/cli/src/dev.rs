//! `nve dev` — 2-tier hot-reload daemon with optional `--skia` desktop preview and `--pair` live `NVD1` streaming.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use nve_engine::HotPatchMsg;

use crate::pair::{self, PairControlMsg};
use crate::rsx_diff;
use crate::static_http;

pub(crate) struct DevOptions {
    pub package: String,
    pub watch: Option<PathBuf>,
    pub host: String,
    pub port: u16,
    pub web: bool,
    pub web_port: u16,
    pub no_web: bool,
    pub skia: bool,
    pub pair: bool,
    pub pair_udp_port: u16,
    pub pair_ws_port: u16,
    pub pair_token: Option<String>,
}

pub(crate) fn dylib_path_for_package_in(target_dir: &Path, package: &str) -> PathBuf {
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    let prefix = if cfg!(target_os = "windows") { "" } else { "lib" };
    target_dir
        .join("debug")
        .join(format!("{prefix}{}.{ext}", package.replace('-', "_")))
}

pub(crate) fn default_watch_dir_for_package(package: &str) -> PathBuf {
    match package {
        "nve-studio-workspace" => PathBuf::from("examples/studio-workspace/src"),
        "nve-example-kitchen-sink" => PathBuf::from("examples/kitchen-sink/src"),
        other => {
            let stripped = other
                .strip_prefix("nve-example-")
                .or_else(|| other.strip_prefix("nve-"))
                .unwrap_or(other);
            let backend_stripped = stripped.strip_prefix("backend-").unwrap_or(stripped);
            for candidate in [
                format!("examples/{stripped}/src"),
                format!("packages/core/{stripped}/src"),
                format!("packages/backends/{backend_stripped}/src"),
                format!("packages/framework/{stripped}/src"),
                format!("packages/tools/{stripped}/src"),
                format!("packages/modules/{stripped}/src"),
            ] {
                let path = PathBuf::from(candidate);
                if path.exists() {
                    return path;
                }
            }
            PathBuf::from("examples")
        }
    }
}

pub(crate) fn ws_url_for_port(port: u16) -> String {
    format!("ws://127.0.0.1:{port}")
}

pub(crate) fn ws_bind_addr(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// `nve dev` serves `hosts/web-shell`. A bare `/` or `/index.html` (no `ws`, `dev`, `pair` or
/// `bundle` parameter) redirects to itself plus `ws=ws://<request host>:<ws_port>`, which makes
/// `index.html` load the dev bundle connected to this daemon: the dev loop needs no URL flags,
/// and production bundles stay free of hot-reload code.
pub(crate) fn web_dev_redirect(target: &str, host: Option<&str>, ws_port: u16) -> Option<String> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" && path != "/index.html" {
        return None;
    }
    let selects_mode = query.split('&').any(|kv| {
        let key = kv.split_once('=').map_or(kv, |(k, _)| k);
        matches!(key, "ws" | "dev" | "pair" | "bundle")
    });
    if selects_mode {
        return None;
    }
    let hostname = host.map(host_without_port).filter(|h| !h.is_empty()).unwrap_or("127.0.0.1");
    let sep = if query.is_empty() { "" } else { "&" };
    Some(format!("{path}?{query}{sep}ws=ws://{hostname}:{ws_port}"))
}

/// `example.com:8088` -> `example.com`, `[::1]:8088` -> `[::1]`, `host` -> `host`.
fn host_without_port(host: &str) -> &str {
    if host.starts_with('[') {
        return host.find(']').map_or(host, |end| &host[..=end]);
    }
    host.rsplit_once(':').map_or(host, |(h, _)| h)
}

pub(crate) fn should_trigger_rebuild(kind: &notify::EventKind) -> bool {
    matches!(kind, notify::EventKind::Any) || kind.is_modify() || kind.is_create()
}

pub(crate) fn is_relevant_source_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    !(name.starts_with('.') || name.ends_with('~') || name.ends_with(".swp") || name.ends_with(".tmp"))
}

pub(crate) fn make_dylib_reload_msg(generation: u64, dylib_path: &Path, build_ms: u64) -> HotPatchMsg {
    HotPatchMsg::DylibReload {
        generation,
        dylib_path: dylib_path.to_string_lossy().into_owned(),
        build_ms,
    }
}

pub(crate) fn cmd_dev(opts: DevOptions) {
    let watch_dir = opts
        .watch
        .unwrap_or_else(|| default_watch_dir_for_package(&opts.package));
    let rebuild_web = !opts.no_web && (opts.web || Path::new("hosts/web-shell/build.sh").exists());
    let bind_addr = ws_bind_addr(&opts.host, opts.port);
    let client_ws_url = if opts.host == "0.0.0.0" || opts.host == "::" {
        ws_url_for_port(opts.port)
    } else {
        format!("ws://{}:{}", opts.host, opts.port)
    };

    println!("nve dev — 2-Tier Hot Reload Daemon");
    println!("  package: {}", opts.package);
    println!("  watch:   {}", watch_dir.display());
    println!("  ws:      ws://{bind_addr} (local: {client_ws_url})");
    let web_lock = Arc::new(RwLock::new(()));
    if rebuild_web {
        let web_bind = ws_bind_addr(&opts.host, opts.web_port);
        let listener = match TcpListener::bind(&web_bind) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("error: failed to bind web-shell HTTP {web_bind}: {e}");
                std::process::exit(1);
            }
        };
        let ws_port = opts.port;
        let redirect: static_http::Redirect =
            Arc::new(move |target, host| web_dev_redirect(target, host, ws_port));
        let lock = Arc::clone(&web_lock);
        std::thread::spawn(move || {
            static_http::serve(listener, PathBuf::from("hosts/web-shell"), lock, Some(redirect));
        });
        println!(
            "  web:     http://127.0.0.1:{}/ (dev bundle, rebuilt on save; pass --no-web to skip)",
            opts.web_port
        );
    }
    if opts.skia {
        println!("  preview: live desktop window enabled (--skia)");
    }
    if opts.pair {
        println!(
            "  pair:    enabled (udp:{}, tcp/ws:{})",
            opts.pair_udp_port, opts.pair_ws_port
        );
    }
    println!();

    let clients: Arc<Mutex<Vec<tungstenite::WebSocket<std::net::TcpStream>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let ws_clients = Arc::clone(&clients);
    let bind_addr_thread = bind_addr.clone();

    let ws_listener = match TcpListener::bind(&bind_addr_thread) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: failed to bind hot-reload WS {bind_addr_thread}: {e}");
            std::process::exit(1);
        }
    };
    std::thread::spawn(move || {
        for stream in ws_listener.incoming().flatten() {
            let _ = stream.set_nodelay(true);
            if let Ok(mut ws) = tungstenite::accept(stream) {
                let status = HotPatchMsg::DevStatus {
                    message: "Hot reload: connected".into(),
                    latency_ms: 0,
                };
                match status.to_bytes() {
                    Ok(bytes) => {
                        let _ = ws.send(tungstenite::Message::Binary(bytes.into()));
                    }
                    Err(err) => eprintln!("[dev] failed to encode DevStatus: {err}"),
                }
                ws_clients.lock().unwrap_or_else(|e| e.into_inner()).push(ws);
            }
        }
    });

    let broadcast_msg = |clients: &Arc<Mutex<Vec<tungstenite::WebSocket<std::net::TcpStream>>>>,
                         msg: &HotPatchMsg| {
        let bytes = match msg.to_bytes() {
            Ok(b) => b,
            Err(err) => {
                eprintln!("[dev] failed to encode HotPatchMsg: {err}");
                return;
            }
        };
        let payload = tungstenite::Message::Binary(bytes.into());
        let mut guard = clients.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain_mut(|ws| ws.send(payload.clone()).is_ok());
    };

    let target_dir = resolve_cargo_target_dir().unwrap_or_else(|_| PathBuf::from("target"));
    let dylib_path = dylib_path_for_package_in(&target_dir, &opts.package);
    let pair_tx_slot: Arc<Mutex<Option<Sender<PairControlMsg>>>> = Arc::new(Mutex::new(None));
    let (skia_ready_tx, skia_ready_rx) = bounded::<()>(1);

    let build_once = {
        let package = opts.package.clone();
        let dylib_for_build = dylib_path.clone();
        let clients = Arc::clone(&clients);
        let pair_tx_slot = Arc::clone(&pair_tx_slot);
        let web_lock = Arc::clone(&web_lock);
        move |generation: u64| -> bool {
            let t0 = Instant::now();
            let status = ProcessCommand::new("cargo")
                .args(["build", "-p", &package])
                .status();
            let web_ok = if rebuild_web && status.as_ref().is_ok_and(|s| s.success()) {
                // Only the dev bundle carries the NVP1 hot-reload client. The write lock keeps the
                // HTTP server from serving a half-written `pkg/dev`.
                let _serving_paused = web_lock.write();
                ProcessCommand::new("./hosts/web-shell/build.sh")
                    .args(["--dev", "dev"])
                    .status()
                    .is_ok_and(|s| s.success())
            } else {
                true
            };
            let elapsed_ms = t0.elapsed().as_millis() as u64;
            match status {
                Ok(s) if s.success() && web_ok => {
                    let msg = make_dylib_reload_msg(generation, &dylib_for_build, elapsed_ms);
                    println!(
                        "[gen {generation}] rebuilt {package}{} in {elapsed_ms}ms -> {}",
                        if rebuild_web { " + web-wasm" } else { "" },
                        dylib_for_build.display()
                    );
                    broadcast_msg(&clients, &msg);
                    if let Some(tx) = pair_tx_slot.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                        let _ = tx.send(PairControlMsg::DylibReload(dylib_for_build.clone()));
                    }
                    true
                }
                _ => {
                    eprintln!("[gen {generation}] build failed after {elapsed_ms}ms");
                    false
                }
            }
        }
    };

    let start_pair_server = {
        let pair_enabled = opts.pair;
        let pair_host = opts.host.clone();
        let pair_udp_port = opts.pair_udp_port;
        let pair_ws_port = opts.pair_ws_port;
        let pair_token = opts.pair_token.clone();
        let dylib_for_pair = dylib_path.clone();
        let pair_tx_slot = Arc::clone(&pair_tx_slot);
        move || {
            if !pair_enabled
                || pair_tx_slot
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some()
            {
                return;
            }
            let token = pair::resolve_or_generate_token(pair_token.clone());
            match pair::spawn_dylib_pair_thread(
                dylib_for_pair.clone(),
                pair_host.clone(),
                pair_udp_port,
                pair_ws_port,
                token,
                393.0,
                852.0,
                60,
            ) {
                Ok((tx, banner)) => {
                    println!("{banner}");
                    *pair_tx_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
                }
                Err(err) => {
                    eprintln!("[pair] failed to start live pairing server: {err}");
                }
            }
        }
    };

    let initial_ok = build_once(1);
    if initial_ok {
        start_pair_server();
    }

    let watch_owned = watch_dir.clone();
    let clients_for_watch = Arc::clone(&clients);
    let pair_tx_for_watch = Arc::clone(&pair_tx_slot);
    let run_watch_loop = move || {
        let mut has_built_once = initial_ok;
        let mut rsx_cache = rsx_diff::RsxWatchCache::new();
        if has_built_once {
            rsx_cache.seed_dir(&watch_owned);
        }

        let (tx, rx) = unbounded();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .expect("create file watcher");
        watcher
            .watch(&watch_owned, RecursiveMode::Recursive)
            .expect("watch directory");

        let mut generation = 1u64;
        println!(
            "Watching for changes ({} cached .rs file(s) for Tier-1 hot-patch; Ctrl-C to stop)...",
            rsx_cache.len()
        );
        loop {
            let Ok(Ok(event)) = rx.recv() else { continue };
            let mut changed_paths: Vec<PathBuf> = event
                .paths
                .iter()
                .filter(|p| is_relevant_source_path(p))
                .cloned()
                .collect();
            if !(should_trigger_rebuild(&event.kind)
                && (event.paths.is_empty() || !changed_paths.is_empty()))
            {
                continue;
            }
            std::thread::sleep(Duration::from_millis(40));
            while let Ok(Ok(extra)) = rx.try_recv() {
                if should_trigger_rebuild(&extra.kind) {
                    for p in extra.paths {
                        if is_relevant_source_path(&p) && !changed_paths.contains(&p) {
                            changed_paths.push(p);
                        }
                    }
                }
            }

            let t_diff = Instant::now();
            let (tier1_patches, fallback_reason) =
                classify_watch_changes(has_built_once, &changed_paths, &mut rsx_cache, generation + 1);

            if let Some(reason) = fallback_reason {
                if let Some(first) = changed_paths.first() {
                    println!("[tier-2] {} ({reason}) — rebuilding...", first.display());
                }
                generation += 1;
                if build_once(generation) {
                    let first_success = !has_built_once;
                    has_built_once = true;
                    rsx_cache.seed_dir(&watch_owned);
                    if first_success {
                        start_pair_server();
                        let _ = skia_ready_tx.try_send(());
                    }
                }
            } else if tier1_patches.is_empty() {
                if let Some(first) = changed_paths.first() {
                    println!(
                        "[watch] {} — comment/whitespace-only edit (no-op)",
                        first.display()
                    );
                }
            } else {
                let elapsed_ms = t_diff.elapsed().as_millis() as u64;
                for (path, mut msg) in tier1_patches {
                    generation += 1;
                    if let HotPatchMsg::TemplatePatch {
                        generation: ref mut g,
                        ..
                    } = msg
                    {
                        *g = generation;
                    }
                    broadcast_msg(&clients_for_watch, &msg);
                    if let Some(tx) = pair_tx_for_watch
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                    {
                        match msg.to_bytes() {
                            Ok(bytes) => {
                                let _ = tx.send(PairControlMsg::Tier1Patch(bytes));
                            }
                            Err(err) => {
                                eprintln!("[dev] failed to encode Tier-1 HotPatchMsg: {err}");
                            }
                        }
                    }
                    let status = HotPatchMsg::DevStatus {
                        message: format!(
                            "[tier-1] patched {} in {}ms (0 cargo build)",
                            path.display(),
                            elapsed_ms
                        ),
                        latency_ms: elapsed_ms,
                    };
                    broadcast_msg(&clients_for_watch, &status);
                    println!(
                        "[tier-1] patched {} in {elapsed_ms}ms (0 cargo build)",
                        path.display()
                    );
                }
            }
        }
    };

    if opts.skia {
        std::thread::spawn(run_watch_loop);
        if !initial_ok {
            eprintln!(
                "[dev] initial build of '{}' failed; waiting for first successful rebuild to launch `--skia` desktop preview...",
                opts.package
            );
        }
        if let Err(err) = wait_for_skia_launch_ready(initial_ok, &skia_ready_rx) {
            eprintln!("[dev] {err}");
            std::process::exit(1);
        }
        let preview_res = run_skia_shell_subprocess(&dylib_path, &client_ws_url);
        if let Some(tx) = pair_tx_slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = tx.send(PairControlMsg::Shutdown);
        }
        if let Err(err) = preview_res {
            eprintln!("Desktop preview exited with error: {err}");
            std::process::exit(1);
        }
    } else {
        run_watch_loop();
    }
}

fn classify_watch_changes(
    has_built_once: bool,
    changed_paths: &[PathBuf],
    rsx_cache: &mut rsx_diff::RsxWatchCache,
    next_generation: u64,
) -> (Vec<(PathBuf, HotPatchMsg)>, Option<String>) {
    if !has_built_once {
        return (Vec::new(), Some("awaiting first successful build".into()));
    }
    if changed_paths.is_empty() {
        return (Vec::new(), Some("unspecified path event".into()));
    }
    let mut tier1_patches = Vec::new();
    for path in changed_paths {
        match std::fs::read_to_string(path) {
            Ok(new_src) => match rsx_cache.diff_file(path, &new_src, next_generation) {
                rsx_diff::DiffOutcome::NoOp => {}
                rsx_diff::DiffOutcome::Tier1Patch(msg) => {
                    tier1_patches.push((path.clone(), msg));
                }
                rsx_diff::DiffOutcome::RequireTier2Rebuild { reason } => {
                    return (Vec::new(), Some(reason));
                }
            },
            Err(err) => {
                return (
                    Vec::new(),
                    Some(format!("could not read {}: {err}", path.display())),
                );
            }
        }
    }
    (tier1_patches, None)
}

fn wait_for_skia_launch_ready(initial_ok: bool, ready_rx: &Receiver<()>) -> Result<(), &'static str> {
    if initial_ok {
        return Ok(());
    }
    ready_rx
        .recv()
        .map_err(|_| "watch loop terminated before first successful rebuild")
}

fn skia_shell_bin_name() -> &'static str {
    if cfg!(windows) {
        "nve-skia-shell.exe"
    } else {
        "nve-skia-shell"
    }
}

fn parse_cargo_target_dir(stdout: &[u8]) -> Result<PathBuf, String> {
    let meta: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|e| format!("failed to parse `cargo metadata` JSON: {e}"))?;
    meta.get("target_directory")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "`cargo metadata` output missing `target_directory`".to_string())
}

pub(crate) fn resolve_cargo_target_dir() -> Result<PathBuf, String> {
    let out = ProcessCommand::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("failed to run `cargo metadata`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`cargo metadata --no-deps --format-version 1` failed with {}",
            out.status
        ));
    }
    parse_cargo_target_dir(&out.stdout)
}

fn resolve_explicit_skia_shell_bin(explicit: &std::ffi::OsStr) -> Result<PathBuf, String> {
    let p = PathBuf::from(explicit);
    if !p.as_os_str().is_empty() && p.is_file() {
        Ok(p)
    } else {
        Err(format!(
            "NVE_SKIA_SHELL_BIN='{}' does not exist or is not a regular file",
            p.display()
        ))
    }
}

fn resolve_skia_shell_binary() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("NVE_SKIA_SHELL_BIN") {
        return resolve_explicit_skia_shell_bin(&explicit);
    }
    println!("[dev] ensuring binary-only host `nve-skia-shell` is up to date...");
    let status = ProcessCommand::new("cargo")
        .args(["build", "--locked", "-p", "nve-skia-shell"])
        .status()
        .map_err(|e| format!("failed to invoke `cargo build --locked -p nve-skia-shell`: {e}"))?;
    if !status.success() {
        return Err(format!(
            "`cargo build --locked -p nve-skia-shell` failed with {status}"
        ));
    }
    let bin = resolve_cargo_target_dir()?
        .join("debug")
        .join(skia_shell_bin_name());
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!(
            "built `nve-skia-shell` but expected binary at '{}' was not found",
            bin.display()
        ))
    }
}

fn skia_shell_args(dylib_path: &Path, ws_url: &str) -> [std::ffi::OsString; 4] {
    [
        "--dylib".into(),
        dylib_path.as_os_str().to_os_string(),
        "--ws".into(),
        ws_url.into(),
    ]
}

struct ChildProcessGuard(Option<std::process::Child>);

impl ChildProcessGuard {
    fn wait(mut self) -> std::io::Result<std::process::ExitStatus> {
        let res = self.0.as_mut().expect("ChildProcessGuard active").wait();
        if res.is_ok() {
            self.0 = None;
        }
        res
    }
}

impl Drop for ChildProcessGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run_skia_shell_subprocess(dylib_path: &Path, ws_url: &str) -> Result<(), String> {
    let bin = resolve_skia_shell_binary()?;
    let child = ProcessCommand::new(&bin)
        .args(skia_shell_args(dylib_path, ws_url))
        .spawn()
        .map_err(|e| format!("failed to launch '{}': {e}", bin.display()))?;
    let guard = ChildProcessGuard(Some(child));
    let status = guard
        .wait()
        .map_err(|e| format!("failed waiting for '{}': {e}", bin.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`{}` exited with status {status}",
            bin.display()
        ))
    }
}

#[cfg(test)]
#[path = "dev_tests.rs"]
mod tests;
