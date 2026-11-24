//! `nve` — Best-in-class developer CLI for `native-view-engine`.
//!
//! Subcommands:
//! - `nve doctor`  — verifies Rust targets, caches (kache/ccache), CMake/Ninja, Xcode/Android SDKs, and RN checkout.
//! - `nve inspect` — runs a Dioxus `rsx!` app headlessly, simulates native press events, and dumps ShadowTrees + Flexbox rects.
//! - `nve bench`   — runs 1,000-node initial + incremental multi-backend benchmarks and exports Perfetto/Chrome JSON traces.
//! - `nve dev`     — starts the 2-tier hot-reload daemon (WebSocket server + file watcher + incremental cdylib builder).

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
        #[arg(long, default_value = "nve-example-counter")]
        package: String,
        /// Directory to watch for Rust source changes.
        #[arg(long, default_value = "examples/counter/src")]
        watch: PathBuf,
        /// WebSocket port for Tier-1/Tier-2 hot-reload clients.
        #[arg(long, default_value_t = 8089)]
        port: u16,
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
            port,
        } => cmd_dev(&package, &watch, port),
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
        ("adb", &["version"]),
    ];

    for (name, args) in tools {
        match probe_tool(name, args) {
            Some(ver) => println!("  [OK]   {name:<12} {ver}"),
            None => println!("  [MISS] {name:<12} not found in PATH"),
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
        println!("\n  Installed Rust targets ({}): {}", targets.len(), targets.join(", "));
    }

    let rn_path = Path::new(
        "/Users/user/developer/developer/respond/react-native/react-native/packages/react-native/ReactCommon",
    );
    if rn_path.exists() {
        println!("  [OK]   RN Fabric    {}", rn_path.display());
    } else {
        println!("  [WARN] RN Fabric    local checkout not found at default path");
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

    // Hit-test the pressable button at (60, 75) and dispatch a simulated native press!
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

fn cmd_dev(package: &str, watch_dir: &Path, port: u16) {
    use crossbeam_channel::unbounded;
    use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

    println!("nve dev — 2-Tier Hot Reload Daemon");
    println!("  package: {package}");
    println!("  watch:   {}", watch_dir.display());
    println!("  ws:      ws://127.0.0.1:{port}\n");

    let clients: Arc<Mutex<Vec<tungstenite::WebSocket<std::net::TcpStream>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let ws_clients = Arc::clone(&clients);

    std::thread::spawn(move || {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind hot-reload WS port");
        for stream in listener.incoming().flatten() {
            if let Ok(ws) = tungstenite::accept(stream) {
                ws_clients.lock().unwrap().push(ws);
            }
        }
    });

    let (tx, rx) = unbounded();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .expect("create file watcher");
    watcher
        .watch(watch_dir, RecursiveMode::Recursive)
        .expect("watch directory");

    let mut generation = 1u64;
    let build_and_broadcast = |generation: u64| {
        let t0 = Instant::now();
        let status = ProcessCommand::new("cargo")
            .args(["build", "-p", package])
            .status();
        let elapsed_ms = t0.elapsed().as_millis() as u64;
        match status {
            Ok(s) if s.success() => {
                let dylib = format!("target/debug/lib{}.dylib", package.replace('-', "_"));
                let msg = HotPatchMsg::DylibReload {
                    generation,
                    dylib_path: dylib.clone(),
                    build_ms: elapsed_ms,
                };
                println!("[gen {generation}] rebuilt {package} in {elapsed_ms}ms -> {dylib}");
                let payload = tungstenite::Message::Text(msg.to_json().into());
                let mut guard = clients.lock().unwrap();
                guard.retain_mut(|ws| ws.send(payload.clone()).is_ok());
            }
            _ => eprintln!("[gen {generation}] build failed after {elapsed_ms}ms"),
        }
    };

    build_and_broadcast(generation);
    println!("Watching for changes (Ctrl-C to stop)...");

    loop {
        if let Ok(Ok(event)) = rx.recv() {
            if event.kind.is_modify() || event.kind.is_create() {
                // Debounce rapid editor saves.
                std::thread::sleep(Duration::from_millis(40));
                while rx.try_recv().is_ok() {}
                generation += 1;
                build_and_broadcast(generation);
            }
        }
    }
}
