//! `nve bench` — Multi-backend frame benchmark, cold-start/RSS/binary-size profiler,
//! and Perfetto/Chrome trace export.

#[cfg(unix)]
use std::ffi::{c_int, c_long};
use std::path::{Path, PathBuf};
use std::time::Instant;

use nve_abi::{EventSet, FrameId, InteractionEvent, SurfaceId, ROOT_NODE_ID};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_tables::tables::{
    core_realization_order, register_core_tables, FlexDirection, Interaction, Layout, TextContent,
    Visual,
};
use nve_engine::{FrameProfiler, FrameTiming, StoreBackend, StoreWireReplica, Viewport};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

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
#[cfg(any(unix, test))]
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
        #[cfg(target_pointer_width = "64")]
        let raw_maxrss: i64 = usage.ru_maxrss;
        #[cfg(not(target_pointer_width = "64"))]
        let raw_maxrss: i64 = i64::from(usage.ru_maxrss);
        Some(normalize_maxrss_to_bytes(raw_maxrss, is_apple))
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

/// Measures cold backend + `StoreWireReplica` construction and an initial empty-surface commit.
pub(crate) fn measure_cold_init_latencies() -> Vec<ColdInitSample> {
    fn measure_one<B: StoreBackend>(make_backend: impl FnOnce() -> B) -> ColdInitSample {
        register_core_tables();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        {
            let mut tx = store.tx();
            let _ = tx.create_at(NodeId::ROOT);
            let _ = tx.create_at(NodeId::new(ROOT_NODE_ID.raw() + 1, 0));
            tx.set_children(NodeId::ROOT, &[NodeId::new(ROOT_NODE_ID.raw() + 1, 0)]);
            tx.commit_into(&mut changes);
        }
        let mut frame0 = Vec::new();
        WireEncoder::default().encode(&store, &changes, &mut frame0);

        let t0 = Instant::now();
        let mut backend = make_backend();
        backend.set_viewport(SurfaceId::ROOT, Viewport::default());
        let mut replica = StoreWireReplica::new(backend, core_realization_order());
        replica
            .submit_nve2_frame(&frame0)
            .expect("cold-init frame0");
        let cold_init_us = t0.elapsed().as_micros() as u64;
        ColdInitSample {
            backend: replica.backend().name(),
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
        if let Ok(meta) = std::fs::metadata(&path)
            && meta.is_file()
        {
            seen_paths.push(path.clone());
            results.push(ArtifactSize {
                label,
                path,
                bytes: meta.len(),
            });
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
                format!("nve-example-kitchen-sink cdylib ({profile})"),
                dylib_filename("nve_example_kitchen_sink"),
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

    // `hosts/web-shell/build.sh` bundles: one production bundle per app + the dev bundle.
    for bundle in ["studio", "kitchen-sink", "dev"] {
        if let Some(wasm) = web_shell_bundle_wasm(&workspace_root.join("hosts/web-shell/pkg").join(bundle)) {
            push_if_exists(format!("nve-web-shell WASM (pkg/{bundle})"), wasm);
        }
    }
    push_if_exists(
        "nve-web-shell WASM (wasm-release)".to_owned(),
        target_dir.join("wasm32-unknown-unknown/wasm-release/nve_web_shell.wasm"),
    );

    results
}

/// The wasm file a `build.sh` bundle's `manifest.json` names (inside `pkg/<bundle>/<hash>/`).
fn web_shell_bundle_wasm(bundle_dir: &Path) -> Option<PathBuf> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bundle_dir.join("manifest.json")).ok()?).ok()?;
    Some(bundle_dir.join(manifest.get("wasm")?.as_str()?))
}

pub(crate) struct BenchFeedSource {
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    rows: usize,
    initialized: bool,
}

impl BenchFeedSource {
    pub(crate) fn new(rows: usize) -> Self {
        register_core_tables();
        Self {
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            rows,
            initialized: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn build_initial_frame(&mut self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rows.saturating_mul(160).max(4096));
        self.encode_initial(&mut out);
        out
    }

    #[cfg(test)]
    pub(crate) fn build_incremental_frame(&mut self, frame_id: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(1024);
        self.encode_incremental(frame_id, &mut out);
        out
    }

    pub(crate) fn encode_initial(&mut self, out: &mut Vec<u8>) {
        out.clear();
        self.store = NodeStore::new();
        self.encoder = WireEncoder::default();
        let mut root_children = Vec::with_capacity(self.rows);
        let mut next_id = ROOT_NODE_ID.raw() + 1;

        {
            let mut tx = self.store.tx();
            for i in 0..self.rows {
                let row_id = NodeId::new(next_id, 0);
                let text_id = NodeId::new(next_id + 1, 0);
                next_id += 2;

                tx.create_at(row_id);
                tx.set(row_id, Layout::WIDTH, Length::percent(100.0));
                tx.set(row_id, Layout::HEIGHT, Length::points(56.0));
                tx.set(row_id, Layout::FLEX_DIRECTION, FlexDirection::Row);
                tx.set(row_id, Layout::PADDING_TOP, Length::points(8.0));
                tx.set(row_id, Layout::PADDING_BOTTOM, Length::points(8.0));
                tx.set(row_id, Layout::PADDING_LEFT, Length::points(16.0));
                tx.set(row_id, Layout::PADDING_RIGHT, Length::points(16.0));
                tx.set(row_id, Layout::ROW_GAP, Length::points(12.0));
                tx.set(row_id, Layout::COLUMN_GAP, Length::points(12.0));
                tx.set(row_id, Visual::BACKGROUND, Rgba::rgb(30, 41, 59));
                tx.set(row_id, Visual::BORDER_RADIUS, 8.0f32);
                tx.edit::<Interaction>(row_id).listeners(EventSet::one(InteractionEvent::Press));

                tx.create_at(text_id);
                tx.set_str(
                    text_id,
                    TextContent::TEXT,
                    &format!("Row #{i} — Native View Engine"),
                );
                tx.set_children(row_id, &[text_id]);
                root_children.push(row_id);
            }
            tx.set_children(NodeId::ROOT, &root_children);
            tx.commit_into(&mut self.changes);
        }

        self.encoder.encode(&self.store, &self.changes, out);
        self.initialized = true;
    }

    pub(crate) fn encode_incremental(&mut self, frame_id: u64, out: &mut Vec<u8>) {
        if !self.initialized {
            self.encode_initial(out);
            return;
        }
        out.clear();
        let updates = self.rows.min(10);
        {
            let mut tx = self.store.tx();
            for i in 0..updates {
                let idx = ((frame_id as usize * 7 + i) % self.rows) as u32;
                let text_id = NodeId::new(ROOT_NODE_ID.raw() + 1 + idx * 2 + 1, 0);
                tx.set_str(
                    text_id,
                    TextContent::TEXT,
                    &format!("Row #{idx} — Frame {frame_id}"),
                );
            }
            tx.commit_into(&mut self.changes);
        }
        self.encoder.encode(&self.store, &self.changes, out);
    }
}

pub(crate) fn run_backend_bench<B: StoreBackend>(
    backend: B,
    rows: usize,
    frames: usize,
    trace_path: Option<&Path>,
) {
    let mut replica = StoreWireReplica::new(backend, core_realization_order());
    replica
        .backend_mut()
        .set_viewport(SurfaceId::ROOT, Viewport::default());
    let mut profiler = FrameProfiler::new(2048);
    let mut feed = BenchFeedSource::new(rows);
    let mut init_bytes = Vec::with_capacity(rows.saturating_mul(160).max(4096));

    let t_enc0 = Instant::now();
    feed.encode_initial(&mut init_bytes);
    let enc0_us = t_enc0.elapsed().as_micros() as u64;
    let init = replica
        .submit_nve2_frame(&init_bytes)
        .expect("initial bench frame");
    let init_total_us = enc0_us + init.decode_us + init.layout_us + init.diff_us + init.mount_us;

    let mut patch = Vec::with_capacity(1024);
    for f in 2..=(frames as u64 + 1) {
        let t_enc = Instant::now();
        feed.encode_incremental(f, &mut patch);
        let enc_us = t_enc.elapsed().as_micros() as u64;
        let stats = replica
            .submit_nve2_frame(&patch)
            .expect("incremental bench frame");
        profiler.record(FrameTiming {
            frame_id: FrameId(f),
            op_count: stats.op_count,
            mutation_count: stats.mutation_count,
            reconcile_us: 0,
            encode_us: enc_us,
            decode_us: stats.decode_us,
            layout_us: stats.layout_us,
            diff_us: stats.diff_us,
            mount_us: stats.mount_us,
        });
    }

    println!(
        "  [{:<16}] init={}us (bytes={}, dec={}us layout={}us diff={}us mount={}us) | incr p50={}us p99={}us",
        replica.backend().name(),
        init_total_us,
        init_bytes.len(),
        init.decode_us,
        init.layout_us,
        init.diff_us,
        init.mount_us,
        profiler.p50_total_us(),
        profiler.p99_total_us(),
    );

    if let Some(path) = trace_path {
        let json = profiler.to_chrome_trace_json();
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

    #[test]
    fn bench_helpers_generate_valid_initial_and_incremental_buffers_across_edge_cases() {
        let mut feed0 = BenchFeedSource::new(0);
        let empty_init = feed0.build_initial_frame();
        assert_eq!(&empty_init[0..4], b"NVE2");
        let empty_patch = feed0.build_incremental_frame(2);
        assert_eq!(&empty_patch[0..4], b"NVE2");

        let mut feed3 = BenchFeedSource::new(3);
        let init3 = feed3.build_initial_frame();
        let mut headless = StoreWireReplica::new(HeadlessBackend::new(), core_realization_order());
        let mut skia = StoreWireReplica::new(SkiaBackend::new(), core_realization_order());
        let mut web = StoreWireReplica::new(WebBackend::new(), core_realization_order());
        headless
            .backend_mut()
            .set_viewport(SurfaceId::ROOT, Viewport::default());
        skia.backend_mut()
            .set_viewport(SurfaceId::ROOT, Viewport::default());
        web.backend_mut()
            .set_viewport(SurfaceId::ROOT, Viewport::default());

        headless.submit_nve2_frame(&init3).unwrap();
        skia.submit_nve2_frame(&init3).unwrap();
        web.submit_nve2_frame(&init3).unwrap();
        assert_eq!(web.store().live_nodes().count(), 1 + 3 * 2);

        let patch3 = feed3.build_incremental_frame(2);
        headless.submit_nve2_frame(&patch3).unwrap();
        web.submit_nve2_frame(&patch3).unwrap();
        assert!(web.backend().render_html(web.store()).contains("Frame 2"));

        let mut feed50 = BenchFeedSource::new(50);
        let _ = feed50.build_initial_frame();
        let patch50 = feed50.build_incremental_frame(5);
        assert_eq!(&patch50[0..4], b"NVE2");
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
        let bundle_dir = root.join("hosts/web-shell/pkg/studio");
        let wasm_dir = bundle_dir.join("0123456789abcdef");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::create_dir_all(&wasm_dir).unwrap();

        let fake_exe = root.join("nve-fake");
        let fake_dylib = release_dir.join(dylib_filename("nve_example_kitchen_sink"));
        let fake_wasm = wasm_dir.join("nve_web_shell_bg.wasm");
        std::fs::write(&fake_exe, vec![0u8; 4096]).unwrap();
        std::fs::write(&fake_dylib, vec![1u8; 8192]).unwrap();
        std::fs::write(&fake_wasm, vec![2u8; 2048]).unwrap();
        std::fs::write(
            bundle_dir.join("manifest.json"),
            r#"{"js":"0123456789abcdef/nve_web_shell.js","wasm":"0123456789abcdef/nve_web_shell_bg.wasm"}"#,
        )
        .unwrap();
        // A bundle without a manifest is not reported.
        std::fs::create_dir_all(root.join("hosts/web-shell/pkg/counter")).unwrap();
        std::fs::write(root.join("hosts/web-shell/pkg/counter/nve_web_shell_bg.wasm"), vec![3u8; 1024]).unwrap();

        let found = collect_artifact_sizes(&root, Some(&fake_exe));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(found.len(), 3);
        assert_eq!(found[0].bytes, 4096);
        assert!(found.iter().any(|a| a.bytes == 8192));
        assert!(found.iter().any(|a| a.bytes == 2048));
    }
}
