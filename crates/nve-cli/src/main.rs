//! `nve` — Best-in-class developer CLI for `native-view-engine`.
//!
//! Subcommands:
//! - `nve doctor`  — verifies Rust targets, caches (kache/ccache), CMake/Ninja, Xcode/Android/Flutter/Swift SDKs, and RN checkout.
//! - `nve inspect` — runs a Dioxus `rsx!` app headlessly, simulates native press events, and dumps ShadowTrees + Flexbox rects.
//! - `nve bench`   — runs 1,000-node initial + incremental multi-backend benchmarks and exports Perfetto/Chrome JSON traces.
//! - `nve dev`     — starts the 2-tier hot-reload daemon (`--skia` opens the live desktop preview window automatically).
//! - `nve codegen` — bi-directional RN <-> NVE + Flutter/SwiftUI/Compose/Web component code generation.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask, EventPayload,
    FlexDirection, LayoutProps, NativeEvent, NativeEventKind, StyleProps, ROOT_NODE_ID,
};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_codegen::ExportTarget;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, RenderBackend, SurfaceRunner, Viewport};

#[derive(Parser)]
#[command(name = "nve", about = "Native View Engine developer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check local toolchain, mobile SDKs, compilation caches, and RN Fabric sources.
    Doctor,
    /// Render a Dioxus rsx! surface headlessly, simulate native taps, and print the layout tree.
    Inspect,
    /// Benchmark 1,000-node initial commit + incremental updates across Headless, Skia, and Web backends.
    Bench {
        /// Number of rows to benchmark in the virtual tree.
        #[arg(long, default_value_t = 1000)]
        rows: usize,
        /// Number of incremental update frames to run.
        #[arg(long, default_value_t = 50)]
        frames: usize,
        /// Optional path to write a Perfetto/Chrome trace JSON file.
        #[arg(long)]
        trace: Option<PathBuf>,
    },
    /// Run the 2-tier hot-reload daemon (WebSocket server + incremental cdylib rebuild on save).
    Dev {
        /// Crate/package name of the cdylib example to watch and hot-rebuild.
        #[arg(long, default_value = "nve-studio-workspace")]
        package: String,
        /// Directory to watch for Rust source changes (defaults to the package's `src/` directory).
        #[arg(long)]
        watch: Option<PathBuf>,
        /// Bind address for the hot-reload WebSocket server (`0.0.0.0` allows LAN/phone clients).
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// WebSocket port for Tier-1/Tier-2 hot-reload clients.
        #[arg(long, default_value_t = 8089)]
        port: u16,
        /// Rebuild the Web WASM bundle (`hosts/web-shell/build.sh --dev`) on save (enabled automatically when `hosts/web-shell/build.sh` exists).
        #[arg(long, default_value_t = false)]
        web: bool,
        /// Skip rebuilding the Web WASM bundle (`hosts/web-shell/build.sh --dev`) on save.
        #[arg(long, default_value_t = false)]
        no_web: bool,
        /// Launch the live cross-platform desktop preview window (`winit` + `softbuffer`) connected to this daemon.
        #[arg(long, default_value_t = false)]
        skia: bool,
    },
    /// Bi-directional React Native <-> NVE and multi-framework (Flutter, SwiftUI, Compose, Web) codegen.
    Codegen {
        #[command(subcommand)]
        action: CodegenAction,
    },
}

#[derive(Subcommand)]
enum CodegenAction {
    /// Parse a React Native `*NativeComponent.{ts,js}` or `Native*.{ts,js}` spec and emit zero-JS Rust + C++20 bindings.
    ImportRn {
        /// Path to the React Native TypeScript/Flow spec file.
        spec: PathBuf,
        /// Output directory for generated Rust and C++20 files.
        #[arg(long, default_value = "generated/import-rn")]
        out: PathBuf,
    },
    /// Export an `NveExportSpec` JSON file into native wrappers for RN, Flutter, SwiftUI, Compose, and/or Web.
    Export {
        /// Path to the `NveExportSpec` JSON file.
        spec: PathBuf,
        /// Target framework(s): `rn`, `flutter`, `swiftui`, `compose`, `web`, or `all` (comma-separated allowed).
        #[arg(long, default_value = "all")]
        target: String,
        /// Output root directory (each target writes into `<out>/<target>/`).
        #[arg(long, default_value = "generated/export")]
        out: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => cmd_doctor(),
        Commands::Inspect => cmd_inspect(),
        Commands::Bench {
            rows,
            frames,
            trace,
        } => cmd_bench(rows, frames, trace),
        Commands::Dev {
            package,
            watch,
            host,
            port,
            web,
            no_web,
            skia,
        } => {
            let watch_dir = watch.unwrap_or_else(|| default_watch_dir_for_package(&package));
            let rebuild_web = !no_web && (web || Path::new("hosts/web-shell/build.sh").exists());
            cmd_dev(&package, &watch_dir, &host, port, rebuild_web, skia);
        }
        Commands::Codegen { action } => cmd_codegen(action),
    }
}

fn probe_tool(cmd: &str, args: &[&str]) -> Option<String> {
    let out = ProcessCommand::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next().unwrap_or("ok").trim().to_string();
    Some(if line.is_empty() { "ok".into() } else { line })
}

fn cmd_doctor() {
    println!("Native View Engine — Environment Doctor\n");
    let tools: &[(&str, &[&str])] = &[
        ("rustc", &["--version"]),
        ("cargo", &["--version"]),
        ("kache", &["--version"]),
        ("clang++", &["--version"]),
        ("cmake", &["--version"]),
        ("ninja", &["--version"]),
        ("ccache", &["--version"]),
        ("ld64.lld", &["--version"]),
        ("xcodebuild", &["-version"]),
        ("swift", &["--version"]),
        ("flutter", &["--version"]),
        ("adb", &["version"]),
        ("wasm-bindgen", &["--version"]),
    ];

    for (name, args) in tools {
        match probe_tool(name, args) {
            Some(ver) => println!("  [OK]   {name:<14} {ver}"),
            None => println!("  [MISS] {name:<14} not found in PATH"),
        }
    }

    if let Ok(out) = ProcessCommand::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    {
        let targets: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        println!(
            "\n  Installed Rust targets ({}): {}",
            targets.len(),
            targets.join(", ")
        );
    }

    let rn_path = Path::new(
        "/Users/user/developer/developer/respond/react-native/react-native/packages/react-native/ReactCommon",
    );
    if rn_path.exists() {
        println!("  [OK]   RN Fabric      {}", rn_path.display());
    } else {
        println!("  [WARN] RN Fabric      local checkout not found at default path");
    }
}

#[component]
fn InspectDemo() -> Element {
    let mut count = use_signal(|| 0i32);
    rsx! {
        view {
            style: "width: 100%; height: 100%; padding: 20px; gap: 12px; background-color: #0f172a;",
            text {
                style: "color: #ffffff; font-size: 24px;",
                "Counter: {count}"
            }
            pressable {
                style: "width: 140px; height: 44px; background-color: #2563eb; border-radius: 10px;",
                onpress: move |_| count += 1,
                text {
                    style: "color: #ffffff; font-size: 16px;",
                    "Tap +1"
                }
            }
        }
    }
}

fn cmd_inspect() {
    let mut surface = DioxusSurface::new(1, InspectDemo);
    let mut runner = SurfaceRunner::new(
        1,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 3.0,
        },
        HeadlessBackend::new(),
    );

    let f1 = runner.next_frame_id();
    let (rec_us, enc_us, bytes) = surface.render_frame(f1);
    let t1 = runner
        .submit_recorded_frame(f1, rec_us, enc_us, &bytes)
        .expect("frame 1");

    println!(
        "=== Frame 1 (initial) | {} bytes | {} ops | total={}us (rec={}us enc={}us dec={}us layout={}us diff={}us) ===",
        bytes.len(),
        t1.op_count,
        t1.total_us(),
        t1.reconcile_us,
        t1.encode_us,
        t1.decode_us,
        t1.layout_us,
        t1.diff_us
    );
    print!("{}", runner.backend().render_ascii_tree());

    let hit = runner
        .backend()
        .hit_test(60.0, 75.0, true)
        .expect("expected pressable button hit at (60, 75)");
    println!("\n--> Simulated native Press at (60, 75) hit Node#{hit}");

    let handled = surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: hit,
        timestamp_us: 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 60.0, y: 75.0 },
    }]);
    assert!(handled, "press event should have been handled");

    let f2 = runner.next_frame_id();
    let (rec2, enc2, bytes2) = surface.render_frame(f2);
    let t2 = runner
        .submit_recorded_frame(f2, rec2, enc2, &bytes2)
        .expect("frame 2");

    println!(
        "\n=== Frame 2 (after tap) | {} bytes | {} ops | total={}us (rec={}us enc={}us dec={}us layout={}us diff={}us) ===",
        bytes2.len(),
        t2.op_count,
        t2.total_us(),
        t2.reconcile_us,
        t2.encode_us,
        t2.decode_us,
        t2.layout_us,
        t2.diff_us
    );
    print!("{}", runner.backend().render_ascii_tree());
}

fn build_initial_feed_cmd(rows: usize) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, 1);
    let mut root_children = Vec::with_capacity(rows);
    let mut next_id = ROOT_NODE_ID + 1;
    let row_layout = LayoutProps {
        width: Dimension::Percent(100.0),
        height: Dimension::Points(56.0),
        flex_direction: FlexDirection::Row,
        padding: EdgeInsets::symmetric(8.0, 16.0),
        gap: 12.0,
        ..Default::default()
    };
    let row_style = StyleProps {
        background: ColorRgba::rgb(30, 41, 59),
        border_radius: 8.0,
        ..Default::default()
    };

    for i in 0..rows {
        let row_id = next_id;
        let text_id = next_id + 1;
        next_id += 2;

        b.create_node(row_id, ComponentKind::View, None);
        b.set_layout(row_id, &row_layout);
        b.set_style(row_id, &row_style);
        b.set_event_mask(row_id, EventMask::PRESS);

        b.create_node(text_id, ComponentKind::Text, None);
        b.set_text(text_id, &format!("Row #{i} — Native View Engine"));
        b.set_children(row_id, &[text_id]);
        root_children.push(row_id);
    }
    b.set_children(ROOT_NODE_ID, &root_children);
    b.commit(true);
    b.into_bytes()
}

fn build_incremental_patch_cmd(frame_id: u64, rows: usize) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    let updates = rows.min(10);
    for i in 0..updates {
        let idx = ((frame_id as usize * 7 + i) % rows) as u32;
        let text_id = ROOT_NODE_ID + 1 + idx * 2 + 1;
        b.set_text(text_id, &format!("Row #{idx} — Frame {frame_id}"));
    }
    b.commit(true);
    b.into_bytes()
}

fn run_backend_bench<B: RenderBackend>(
    backend: B,
    rows: usize,
    frames: usize,
    trace_path: Option<&Path>,
) {
    let mut runner = SurfaceRunner::new(1, Viewport::default(), backend);
    let t_enc0 = Instant::now();
    let init_bytes = build_initial_feed_cmd(rows);
    let enc0_us = t_enc0.elapsed().as_micros() as u64;
    let init = runner
        .backend_mut()
        .submit_frame(&init_bytes)
        .expect("initial bench frame");
    let init_total_us = enc0_us + init.decode_us + init.layout_us + init.diff_us + init.mount_us;

    for f in 2..=(frames as u64 + 1) {
        let t_enc = Instant::now();
        let patch = build_incremental_patch_cmd(f, rows);
        let enc_us = t_enc.elapsed().as_micros() as u64;
        runner
            .submit_recorded_frame(f, 0, enc_us, &patch)
            .expect("incremental bench frame");
    }

    println!(
        "  [{:<16}] init={}us (bytes={}, dec={}us layout={}us diff={}us mount={}us) | incr p50={}us p99={}us",
        runner.backend().name(),
        init_total_us,
        init_bytes.len(),
        init.decode_us,
        init.layout_us,
        init.diff_us,
        init.mount_us,
        runner.profiler().p50_total_us(),
        runner.profiler().p99_total_us(),
    );

    if let Some(path) = trace_path {
        let json = runner.profiler().to_chrome_trace_json();
        std::fs::write(path, json).expect("write Perfetto trace");
        println!("  -> wrote Perfetto/Chrome trace to {}", path.display());
    }
}

fn cmd_bench(rows: usize, frames: usize, trace: Option<PathBuf>) {
    println!("Native View Engine — Multi-Backend Frame Benchmark ({rows} rows, {frames} incremental frames)\n");
    run_backend_bench(HeadlessBackend::new(), rows, frames, trace.as_deref());
    run_backend_bench(SkiaBackend::new(), rows, frames, None);
    run_backend_bench(WebBackend::new(), rows, frames, None);
}

fn dylib_path_for_package(package: &str) -> PathBuf {
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    let prefix = if cfg!(target_os = "windows") { "" } else { "lib" };
    PathBuf::from(format!(
        "target/debug/{prefix}{}.{ext}",
        package.replace('-', "_")
    ))
}

fn default_watch_dir_for_package(package: &str) -> PathBuf {
    match package {
        "nve-studio-workspace" => PathBuf::from("examples/studio-workspace/src"),
        "nve-example-counter" => PathBuf::from("examples/counter/src"),
        other => {
            let stripped = other
                .strip_prefix("nve-example-")
                .or_else(|| other.strip_prefix("nve-"))
                .unwrap_or(other);
            let example_src = PathBuf::from(format!("examples/{stripped}/src"));
            if example_src.exists() {
                example_src
            } else {
                let crate_src = PathBuf::from(format!("crates/{other}/src"));
                if crate_src.exists() {
                    crate_src
                } else {
                    PathBuf::from("examples")
                }
            }
        }
    }
}

fn ws_url_for_port(port: u16) -> String {
    format!("ws://127.0.0.1:{port}")
}

fn ws_bind_addr(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

fn should_trigger_rebuild(kind: &notify::EventKind) -> bool {
    matches!(kind, notify::EventKind::Any) || kind.is_modify() || kind.is_create()
}

fn is_relevant_source_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') || name.ends_with('~') || name.ends_with(".swp") || name.ends_with(".tmp") {
        return false;
    }
    true
}

fn make_dylib_reload_msg(generation: u64, dylib_path: &Path, build_ms: u64) -> HotPatchMsg {
    HotPatchMsg::DylibReload {
        generation,
        dylib_path: dylib_path.to_string_lossy().into_owned(),
        build_ms,
    }
}

fn parse_export_targets(target: &str) -> Result<Vec<ExportTarget>, String> {
    if target.trim().eq_ignore_ascii_case("all") {
        return Ok(ExportTarget::ALL.to_vec());
    }
    let mut out = Vec::new();
    for part in target.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return Err("empty target name in comma-separated list".into());
        }
        let parsed = ExportTarget::from_name(trimmed)
            .map_err(|e| format!("invalid target '{trimmed}': {e}"))?;
        out.push(parsed);
    }
    Ok(out)
}

fn cmd_dev(
    package: &str,
    watch_dir: &Path,
    host: &str,
    port: u16,
    rebuild_web: bool,
    launch_skia_window: bool,
) {
    use crossbeam_channel::unbounded;
    use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

    let bind_addr = ws_bind_addr(host, port);
    let client_ws_url = if host == "0.0.0.0" || host == "::" {
        ws_url_for_port(port)
    } else {
        format!("ws://{host}:{port}")
    };
    println!("nve dev — 2-Tier Hot Reload Daemon");
    println!("  package: {package}");
    println!("  watch:   {}", watch_dir.display());
    println!("  ws:      ws://{bind_addr} (local: {client_ws_url})");
    if rebuild_web {
        println!("  web:     auto-rebuilding hosts/web-shell WASM (--dev) on save (pass --no-web to skip)");
    }
    if launch_skia_window {
        println!("  preview: live desktop window enabled (--skia)");
    }
    println!();

    let clients: Arc<Mutex<Vec<tungstenite::WebSocket<std::net::TcpStream>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let ws_clients = Arc::clone(&clients);
    let bind_addr_thread = bind_addr.clone();

    std::thread::spawn(move || {
        let listener = TcpListener::bind(&bind_addr_thread)
            .unwrap_or_else(|e| panic!("bind hot-reload WS {bind_addr_thread}: {e}"));
        for stream in listener.incoming().flatten() {
            let _ = stream.set_nodelay(true);
            if let Ok(mut ws) = tungstenite::accept(stream) {
                let status = HotPatchMsg::DevStatus {
                    message: "Hot reload: connected".into(),
                    latency_ms: 0,
                };
                let _ = ws.send(tungstenite::Message::Text(status.to_json().into()));
                ws_clients.lock().unwrap().push(ws);
            }
        }
    });

    let dylib_path = dylib_path_for_package(package);
    let build_once = {
        let package = package.to_owned();
        let dylib_for_build = dylib_path.clone();
        let clients = Arc::clone(&clients);
        move |generation: u64| {
            let t0 = Instant::now();
            let status = ProcessCommand::new("cargo")
                .args(["build", "-p", &package])
                .status();
            let web_ok = if rebuild_web && status.as_ref().is_ok_and(|s| s.success()) {
                ProcessCommand::new("./hosts/web-shell/build.sh")
                    .arg("--dev")
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
                    let payload = tungstenite::Message::Text(msg.to_json().into());
                    let mut guard = clients.lock().unwrap();
                    guard.retain_mut(|ws| ws.send(payload.clone()).is_ok());
                }
                _ => eprintln!("[gen {generation}] build failed after {elapsed_ms}ms"),
            }
        }
    };

    build_once(1);

    let watch_owned = watch_dir.to_path_buf();
    let run_watch_loop = move || {
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
        println!("Watching for changes (Ctrl-C to stop)...");
        loop {
            if let Ok(Ok(event)) = rx.recv() {
                let changed_paths: Vec<PathBuf> = event
                    .paths
                    .iter()
                    .filter(|p| is_relevant_source_path(p))
                    .cloned()
                    .collect();
                if should_trigger_rebuild(&event.kind)
                    && (event.paths.is_empty() || !changed_paths.is_empty())
                {
                    if let Some(first) = changed_paths.first() {
                        println!("[watch] changed: {}", first.display());
                    }
                    std::thread::sleep(Duration::from_millis(40));
                    while rx.try_recv().is_ok() {}
                    generation += 1;
                    build_once(generation);
                }
            }
        }
    };

    if launch_skia_window {
        std::thread::spawn(run_watch_loop);
        if let Err(err) = nve_backend_skia::run_desktop_preview(dylib_path, Some(client_ws_url)) {
            eprintln!("Desktop preview exited with error: {err}");
            std::process::exit(1);
        }
    } else {
        run_watch_loop();
    }
}

fn cmd_codegen(action: CodegenAction) {
    match action {
        CodegenAction::ImportRn { spec, out } => {
            let generated = nve_codegen::import_rn_file(&spec).unwrap_or_else(|e| {
                eprintln!("import-rn failed for {}: {e}", spec.display());
                std::process::exit(1);
            });
            let written = generated.write_to_dir(&out).unwrap_or_else(|e| {
                eprintln!("failed writing output to {}: {e}", out.display());
                std::process::exit(1);
            });
            println!("Generated {} files in {}:", written.len(), out.display());
            for path in written {
                println!("  - {}", path.display());
            }
        }
        CodegenAction::Export { spec, target, out } => {
            let targets = parse_export_targets(&target).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let bundles = nve_codegen::export_json_file_for_targets(&spec, &targets)
                .unwrap_or_else(|e| {
                    eprintln!("export failed for {}: {e}", spec.display());
                    std::process::exit(1);
                });
            for bundle in bundles {
                let target_dir = out.join(bundle.target.dir_name());
                let written = bundle.write_to_dir(&target_dir).unwrap_or_else(|e| {
                    eprintln!("failed writing {} output: {e}", bundle.target.dir_name());
                    std::process::exit(1);
                });
                println!(
                    "[{}] wrote {} file(s) -> {}",
                    bundle.target.dir_name(),
                    written.len(),
                    target_dir.display()
                );
                for path in written {
                    println!("  - {}", path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{Command, CommandDecoder};

    #[test]
    fn cli_parses_all_subcommands_defaults_and_flags() {
        assert!(matches!(
            Cli::try_parse_from(["nve", "doctor"]).unwrap().command,
            Commands::Doctor
        ));
        assert!(matches!(
            Cli::try_parse_from(["nve", "inspect"]).unwrap().command,
            Commands::Inspect
        ));

        match Cli::try_parse_from(["nve", "bench"]).unwrap().command {
            Commands::Bench {
                rows,
                frames,
                trace,
            } => {
                assert_eq!(rows, 1000);
                assert_eq!(frames, 50);
                assert!(trace.is_none());
            }
            _ => panic!("expected Bench"),
        }

        match Cli::try_parse_from([
            "nve",
            "bench",
            "--rows",
            "250",
            "--frames",
            "12",
            "--trace",
            "out.trace.json",
        ])
        .unwrap()
        .command
        {
            Commands::Bench {
                rows,
                frames,
                trace,
            } => {
                assert_eq!(rows, 250);
                assert_eq!(frames, 12);
                assert_eq!(trace, Some(PathBuf::from("out.trace.json")));
            }
            _ => panic!("expected Bench"),
        }

        match Cli::try_parse_from([
            "nve",
            "dev",
            "--package",
            "my-app",
            "--watch",
            "src",
            "--port",
            "9001",
            "--skia",
        ])
        .unwrap()
        .command
        {
            Commands::Dev {
                package,
                watch,
                host,
                port,
                web,
                no_web,
                skia,
            } => {
                assert_eq!(package, "my-app");
                assert_eq!(watch, Some(PathBuf::from("src")));
                assert_eq!(host, "0.0.0.0");
                assert_eq!(port, 9001);
                assert!(!web);
                assert!(!no_web);
                assert!(skia);
            }
            _ => panic!("expected Dev"),
        }

        match Cli::try_parse_from([
            "nve",
            "codegen",
            "import-rn",
            "SpecNativeComponent.ts",
            "--out",
            "gen/rn",
        ])
        .unwrap()
        .command
        {
            Commands::Codegen {
                action: CodegenAction::ImportRn { spec, out },
            } => {
                assert_eq!(spec, PathBuf::from("SpecNativeComponent.ts"));
                assert_eq!(out, PathBuf::from("gen/rn"));
            }
            _ => panic!("expected Codegen ImportRn"),
        }

        match Cli::try_parse_from([
            "nve",
            "codegen",
            "export",
            "widget.json",
            "--target",
            "flutter,web",
            "--out",
            "gen/out",
        ])
        .unwrap()
        .command
        {
            Commands::Codegen {
                action: CodegenAction::Export { spec, target, out },
            } => {
                assert_eq!(spec, PathBuf::from("widget.json"));
                assert_eq!(target, "flutter,web");
                assert_eq!(out, PathBuf::from("gen/out"));
            }
            _ => panic!("expected Codegen Export"),
        }

        // Invalid invocations must be rejected cleanly.
        assert!(Cli::try_parse_from(["nve"]).is_err());
        assert!(Cli::try_parse_from(["nve", "unknown"]).is_err());
        assert!(Cli::try_parse_from(["nve", "bench", "--rows", "not_a_num"]).is_err());
        assert!(Cli::try_parse_from(["nve", "dev", "--port", "99999"]).is_err());
        assert!(Cli::try_parse_from(["nve", "codegen", "import-rn"]).is_err());
    }

    #[test]
    fn bench_helpers_generate_valid_initial_and_incremental_buffers_across_edge_cases() {
        // Zero rows produces a valid empty root commit without divide-by-zero in incremental patch.
        let empty_init = build_initial_feed_cmd(0);
        let (hdr0, cmds0) = CommandDecoder::new(&empty_init)
            .unwrap()
            .decode_all()
            .unwrap();
        assert_eq!(hdr0.op_count, 2); // SetChildren(ROOT, []) + Commit
        assert_eq!(cmds0.len(), 2);

        let empty_patch = build_incremental_patch_cmd(2, 0);
        let (_, patch0) = CommandDecoder::new(&empty_patch)
            .unwrap()
            .decode_all()
            .unwrap();
        assert_eq!(patch0, vec![Command::Commit { sync_mount: true }]);

        // Small row count (3 < 10) caps updates at `rows`.
        let init3 = build_initial_feed_cmd(3);
        let mut headless = HeadlessBackend::new();
        let mut skia = SkiaBackend::new();
        let mut web = WebBackend::new();
        headless.set_viewport(1, Viewport::default());
        skia.set_viewport(1, Viewport::default());
        web.set_viewport(1, Viewport::default());

        headless.submit_frame(&init3).unwrap();
        skia.submit_frame(&init3).unwrap();
        web.submit_frame(&init3).unwrap();
        assert_eq!(web.node_count(), 1 + 3 * 2);

        let patch3 = build_incremental_patch_cmd(2, 3);
        let (_, cmds3) = CommandDecoder::new(&patch3).unwrap().decode_all().unwrap();
        let text_updates = cmds3
            .iter()
            .filter(|c| matches!(c, Command::SetText { .. }))
            .count();
        assert_eq!(text_updates, 3);
        headless.submit_frame(&patch3).unwrap();
        web.submit_frame(&patch3).unwrap();
        assert!(web.render_html().contains("Frame 2"));

        // Large row count (50) caps incremental patch at 10 SetText ops + 1 Commit.
        let patch50 = build_incremental_patch_cmd(5, 50);
        let (_, cmds50) = CommandDecoder::new(&patch50).unwrap().decode_all().unwrap();
        assert_eq!(cmds50.len(), 11);
    }

    #[test]
    fn run_backend_bench_writes_valid_chrome_trace_json() {
        let trace_path = std::env::temp_dir().join(format!(
            "nve-cli-bench-test-{}.trace.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&trace_path);

        run_backend_bench(HeadlessBackend::new(), 12, 4, Some(&trace_path));
        let raw = std::fs::read_to_string(&trace_path).expect("trace file written");
        let _ = std::fs::remove_file(&trace_path);

        let events: Vec<serde_json::Value> =
            serde_json::from_str(&raw).expect("valid Chrome trace JSON");
        assert_eq!(events.len(), 4 * 6);
    }

    #[test]
    fn hot_reload_and_codegen_helpers_cover_platform_paths_filters_and_targets() {
        let dylib = dylib_path_for_package("nve-example-counter");
        let s = dylib.to_string_lossy();
        assert!(s.contains("nve_example_counter"), "unexpected path: {s}");
        if cfg!(target_os = "macos") {
            assert_eq!(s, "target/debug/libnve_example_counter.dylib");
        } else if cfg!(target_os = "windows") {
            assert_eq!(s, "target/debug/nve_example_counter.dll");
        } else {
            assert_eq!(s, "target/debug/libnve_example_counter.so");
        }

        assert_eq!(ws_url_for_port(8089), "ws://127.0.0.1:8089");
        assert_eq!(ws_bind_addr("0.0.0.0", 8089), "0.0.0.0:8089");
        assert_eq!(
            default_watch_dir_for_package("nve-studio-workspace"),
            PathBuf::from("examples/studio-workspace/src")
        );
        assert_eq!(
            default_watch_dir_for_package("nve-example-counter"),
            PathBuf::from("examples/counter/src")
        );
        assert!(is_relevant_source_path(Path::new("examples/studio-workspace/src/ui.rs")));
        assert!(!is_relevant_source_path(Path::new("examples/studio-workspace/src/.ui.rs.swp")));

        let msg = make_dylib_reload_msg(3, &dylib, 180);
        let decoded = HotPatchMsg::from_json(&msg.to_json()).unwrap();
        assert_eq!(decoded, msg);

        use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};
        assert!(should_trigger_rebuild(&notify::EventKind::Create(
            CreateKind::File
        )));
        assert!(should_trigger_rebuild(&notify::EventKind::Modify(
            ModifyKind::Any
        )));
        assert!(!should_trigger_rebuild(&notify::EventKind::Access(
            AccessKind::Any
        )));
        assert!(!should_trigger_rebuild(&notify::EventKind::Remove(
            RemoveKind::File
        )));

        assert_eq!(parse_export_targets("all").unwrap(), ExportTarget::ALL);
        assert_eq!(parse_export_targets(" ALL ").unwrap(), ExportTarget::ALL);
        assert_eq!(
            parse_export_targets("rn, flutter , web").unwrap(),
            vec![
                ExportTarget::ReactNative,
                ExportTarget::Flutter,
                ExportTarget::WebComponent,
            ]
        );
        assert!(parse_export_targets("unknown-framework").is_err());
        assert!(parse_export_targets("rn,").is_err());
    }

    #[test]
    fn probe_tool_and_inspect_demo_work_headlessly() {
        assert!(probe_tool("rustc", &["--version"]).is_some());
        assert!(probe_tool("__definitely_missing_nve_binary__", &["--version"]).is_none());
        assert!(probe_tool("rustc", &["--definitely-invalid-flag-xyz"]).is_none());

        // Exercise InspectDemo component end-to-end without printing to stdout.
        let mut surface = DioxusSurface::new(1, InspectDemo);
        let mut runner = SurfaceRunner::new(1, Viewport::default(), HeadlessBackend::new());
        let f1 = runner.next_frame_id();
        let (rec, enc, b1) = surface.render_frame(f1);
        runner.submit_recorded_frame(f1, rec, enc, &b1).unwrap();
        assert!(runner.backend().render_ascii_tree().contains("Counter: 0"));

        let hit = runner.backend().hit_test(60.0, 75.0, true).expect("hit button");
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: hit,
            timestamp_us: 1000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 60.0, y: 75.0 },
        }]));
        let f2 = runner.next_frame_id();
        let (rec2, enc2, b2) = surface.render_frame(f2);
        runner.submit_recorded_frame(f2, rec2, enc2, &b2).unwrap();
        assert!(runner.backend().render_ascii_tree().contains("Counter: 1"));
    }
}
