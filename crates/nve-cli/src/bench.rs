//! `nve bench` — Multi-backend frame benchmark, cold-start/RSS/binary-size profiler,
//! and Perfetto/Chrome trace export.

#[cfg(unix)]
use std::ffi::{c_int, c_long};
use std::path::{Path, PathBuf};
use std::time::Instant;

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
    FlexDirection, LayoutProps, StyleProps, ROOT_NODE_ID,
};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_engine::{RenderBackend, SurfaceRunner, Viewport};

#[cfg(unix)]
const RUSAGE_SELF: c_int = 0;

#[cfg(unix)]
#[repr(C)]
struct Timeval {
    tv_sec: c_long,
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    tv_usec: i32,
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )))]
    tv_usec: c_long,
}

#[cfg(unix)]
#[repr(C)]
struct Rusage {
    ru_utime: Timeval,
    ru_stime: Timeval,
    ru_maxrss: c_long,
    ru_ixrss: c_long,
    ru_idrss: c_long,
    ru_isrss: c_long,
    ru_minflt: c_long,
    ru_majflt: c_long,
    ru_nswap: c_long,
    ru_inblock: c_long,
    ru_oublock: c_long,
    ru_msgsnd: c_long,
    ru_msgrcv: c_long,
    ru_nsignals: c_long,
    ru_nvcsw: c_long,
    ru_nivcsw: c_long,
}

#[cfg(unix)]
unsafe extern "C" {
    fn getrusage(who: c_int, usage: *mut Rusage) -> c_int;
}

/// Normalizes `rusage.ru_maxrss` to bytes: Apple platforms (macOS/iOS) report bytes directly,
/// whereas Linux and other BSDs report kibibytes (KiB).
#[inline]
pub(crate) fn normalize_maxrss_to_bytes(raw_maxrss: i64, is_apple_os: bool) -> u64 {
    if raw_maxrss <= 0 {
        return 0;
    }
    let v = raw_maxrss as u64;
    if is_apple_os {
        v
    } else {
        v.saturating_mul(1024)
    }
}

/// Returns the process peak resident set size (Peak RSS) in bytes via `getrusage(RUSAGE_SELF)`
/// with zero external crate dependencies.
pub(crate) fn peak_rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<Rusage>::zeroed();
        // SAFETY: `usage` is a valid, zero-initialized `Rusage` buffer matching the platform ABI.
        let rc = unsafe { getrusage(RUSAGE_SELF, usage.as_mut_ptr()) };
        if rc != 0 {
            return None;
        }
        // SAFETY: `getrusage` returned 0 and initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        let is_apple = cfg!(any(target_os = "macos", target_os = "ios"));
        Some(normalize_maxrss_to_bytes(usage.ru_maxrss as i64, is_apple))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Formats a byte count cleanly in both KiB and MiB.
pub(crate) fn format_bytes_kib_mib(bytes: u64) -> String {
    let kib = bytes as f64 / 1024.0;
    let mib = bytes as f64 / (1024.0 * 1024.0);
    format!("{mib:.2} MiB ({kib:.1} KiB, {bytes} B)")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColdInitSample {
    pub backend: &'static str,
    pub cold_init_us: u64,
    pub frame0_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactSize {
    pub label: String,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Measures cold backend + `SurfaceRunner` construction and an initial empty-surface commit.
pub(crate) fn measure_cold_init_latencies() -> Vec<ColdInitSample> {
    fn measure_one<B: RenderBackend>(make_backend: impl FnOnce() -> B) -> ColdInitSample {
        let mut builder = CommandBufferBuilder::with_capacity(1, 0, 128);
        builder.create_node(ROOT_NODE_ID + 1, ComponentKind::View, None);
        builder.set_children(ROOT_NODE_ID, &[ROOT_NODE_ID + 1]);
        builder.commit(true);
        let frame0 = builder.into_bytes();

        let t0 = Instant::now();
        let backend = make_backend();
        let mut runner = SurfaceRunner::new(1, Viewport::default(), backend);
        runner
            .backend_mut()
            .submit_frame(&frame0)
            .expect("cold-init frame0");
        let cold_init_us = t0.elapsed().as_micros() as u64;
        ColdInitSample {
            backend: runner.backend().name(),
            cold_init_us,
            frame0_bytes: frame0.len(),
        }
    }

    vec![
        measure_one(HeadlessBackend::new),
        measure_one(SkiaBackend::new),
        measure_one(WebBackend::new),
    ]
}

fn dylib_filename(stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(target_os = "windows") {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    }
}

fn exe_filename(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

/// Collects on-disk byte sizes for the running CLI binary and any built workspace cdylibs / WASM
/// bundles discovered under `workspace_root`.
pub(crate) fn collect_artifact_sizes(
    workspace_root: &Path,
    current_exe: Option<&Path>,
) -> Vec<ArtifactSize> {
    let mut results = Vec::new();
    let mut seen_paths = Vec::new();

    let mut push_if_exists = |label: String, path: PathBuf| {
        if seen_paths.iter().any(|p: &PathBuf| p == &path) {
            return;
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file() {
                seen_paths.push(path.clone());
                results.push(ArtifactSize {
                    label,
                    path,
                    bytes: meta.len(),
                });
            }
        }
    };

    if let Some(exe) = current_exe {
        push_if_exists("nve-cli (current exe)".to_owned(), exe.to_path_buf());
    }

    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                workspace_root.join(p)
            }
        })
        .unwrap_or_else(|| workspace_root.join("target"));

    for profile in ["release", "debug"] {
        for (label, fname) in [
            (
                format!("nve-example-counter cdylib ({profile})"),
                dylib_filename("nve_example_counter"),
            ),
            (
                format!("nve-studio-workspace cdylib ({profile})"),
                dylib_filename("nve_studio_workspace"),
            ),
            (
                format!("nve-skia-shell ({profile})"),
                exe_filename("nve-skia-shell"),
            ),
        ] {
            push_if_exists(label, target_dir.join(profile).join(fname));
        }
    }

    push_if_exists(
        "nve-web-shell WASM (pkg)".to_owned(),
        workspace_root.join("hosts/web-shell/pkg/nve_web_shell_bg.wasm"),
    );
    push_if_exists(
        "nve-web-shell WASM (wasm-release)".to_owned(),
        target_dir.join("wasm32-unknown-unknown/wasm-release/nve_web_shell.wasm"),
    );

    results
}

pub(crate) fn build_initial_feed_cmd(rows: usize) -> Vec<u8> {
    let mut b = CommandBufferBuilder::with_capacity(1, 1, rows.saturating_mul(220).max(4096));
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

pub(crate) fn build_incremental_patch_cmd(frame_id: u64, rows: usize) -> Vec<u8> {
    let mut b = CommandBufferBuilder::with_capacity(1, frame_id, 1024);
    let updates = rows.min(10);
    for i in 0..updates {
        let idx = ((frame_id as usize * 7 + i) % rows) as u32;
        let text_id = ROOT_NODE_ID + 1 + idx * 2 + 1;
        b.set_text(text_id, &format!("Row #{idx} — Frame {frame_id}"));
    }
    b.commit(true);
    b.into_bytes()
}

pub(crate) fn run_backend_bench<B: RenderBackend>(
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

pub(crate) fn cmd_bench(rows: usize, frames: usize, trace: Option<PathBuf>) {
    println!(
        "Native View Engine — Multi-Backend Frame Benchmark ({rows} rows, {frames} incremental frames)\n"
    );

    let rss_before = peak_rss_bytes();
    let cold_samples = measure_cold_init_latencies();

    println!("Cold-Start Init (SurfaceRunner + Frame-0):");
    for sample in &cold_samples {
        println!(
            "  [{:<16}] cold_init={}us (frame0={}B)",
            sample.backend, sample.cold_init_us, sample.frame0_bytes
        );
    }
    println!();

    println!("Frame Benchmarks:");
    run_backend_bench(HeadlessBackend::new(), rows, frames, trace.as_deref());
    run_backend_bench(SkiaBackend::new(), rows, frames, None);
    run_backend_bench(WebBackend::new(), rows, frames, None);

    let rss_after = peak_rss_bytes();
    println!("\nMemory Footprint (Peak RSS via getrusage):");
    match (rss_before, rss_after) {
        (Some(before), Some(after)) => {
            let delta = after.saturating_sub(before);
            println!("  before bench: {}", format_bytes_kib_mib(before));
            println!("  after bench:  {}", format_bytes_kib_mib(after));
            println!("  peak delta:   +{}", format_bytes_kib_mib(delta));
        }
        (_, Some(after)) => {
            println!("  peak RSS:     {}", format_bytes_kib_mib(after));
        }
        _ => {
            println!("  peak RSS:     unavailable on this platform");
        }
    }

    let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let current_exe = std::env::current_exe().ok();
    let artifacts = collect_artifact_sizes(&workspace_root, current_exe.as_deref());
    println!("\nMeasured Binary / Artifact Sizes:");
    if artifacts.is_empty() {
        println!("  (no workspace binaries or cdylibs found)");
    } else {
        for art in &artifacts {
            println!(
                "  {:<38} {}  [{}]",
                art.label,
                format_bytes_kib_mib(art.bytes),
                art.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{Command, CommandDecoder};

    #[test]
    fn bench_helpers_generate_valid_initial_and_incremental_buffers_across_edge_cases() {
        let empty_init = build_initial_feed_cmd(0);
        let (hdr0, cmds0) = CommandDecoder::new(&empty_init)
            .unwrap()
            .decode_all()
            .unwrap();
        assert_eq!(hdr0.op_count, 2);
        assert_eq!(cmds0.len(), 2);

        let empty_patch = build_incremental_patch_cmd(2, 0);
        let (_, patch0) = CommandDecoder::new(&empty_patch)
            .unwrap()
            .decode_all()
            .unwrap();
        assert_eq!(patch0, vec![Command::Commit { sync_mount: true }]);

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
    fn maxrss_normalization_handles_apple_bytes_linux_kib_and_edge_cases() {
        // macOS/iOS reports bytes directly.
        assert_eq!(normalize_maxrss_to_bytes(16_777_216, true), 16_777_216);
        // Linux reports KiB -> multiplied by 1024.
        assert_eq!(normalize_maxrss_to_bytes(16_384, false), 16_777_216);
        // Zero and negative values clamp cleanly to 0.
        assert_eq!(normalize_maxrss_to_bytes(0, true), 0);
        assert_eq!(normalize_maxrss_to_bytes(0, false), 0);
        assert_eq!(normalize_maxrss_to_bytes(-42, true), 0);
        assert_eq!(normalize_maxrss_to_bytes(-42, false), 0);
        // Saturation prevents overflow on extreme Linux values.
        assert_eq!(normalize_maxrss_to_bytes(i64::MAX, false), u64::MAX);

        assert_eq!(
            format_bytes_kib_mib(1_048_576),
            "1.00 MiB (1024.0 KiB, 1048576 B)"
        );
    }

    #[test]
    fn peak_rss_and_cold_init_measurements_succeed_on_host_platform() {
        #[cfg(unix)]
        {
            let rss = peak_rss_bytes().expect("getrusage must succeed on unix");
            assert!(rss > 0, "peak RSS must be non-zero");
            assert!(
                rss < 64 * 1024 * 1024 * 1024,
                "peak RSS ({rss} B) looks unnormalized"
            );
        }

        let samples = measure_cold_init_latencies();
        assert_eq!(samples.len(), 3);
        assert!(samples.iter().all(|s| s.frame0_bytes > 0));
    }

    #[test]
    fn collect_artifact_sizes_discovers_existing_binaries_and_ignores_missing() {
        let root = std::env::temp_dir().join(format!(
            "nve-cli-artifacts-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let release_dir = root.join("target/release");
        let wasm_dir = root.join("hosts/web-shell/pkg");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::create_dir_all(&wasm_dir).unwrap();

        let fake_exe = root.join("nve-fake");
        let fake_dylib = release_dir.join(dylib_filename("nve_example_counter"));
        let fake_wasm = wasm_dir.join("nve_web_shell_bg.wasm");
        std::fs::write(&fake_exe, vec![0u8; 4096]).unwrap();
        std::fs::write(&fake_dylib, vec![1u8; 8192]).unwrap();
        std::fs::write(&fake_wasm, vec![2u8; 2048]).unwrap();

        let found = collect_artifact_sizes(&root, Some(&fake_exe));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(found.len(), 3);
        assert_eq!(found[0].bytes, 4096);
        assert!(found.iter().any(|a| a.bytes == 8192));
        assert!(found.iter().any(|a| a.bytes == 2048));
    }
}
