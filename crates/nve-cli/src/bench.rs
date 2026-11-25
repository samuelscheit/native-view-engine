//! `nve bench` — Multi-backend frame benchmark and Perfetto/Chrome trace export.

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

pub(crate) fn build_initial_feed_cmd(rows: usize) -> Vec<u8> {
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

pub(crate) fn build_incremental_patch_cmd(frame_id: u64, rows: usize) -> Vec<u8> {
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
    run_backend_bench(HeadlessBackend::new(), rows, frames, trace.as_deref());
    run_backend_bench(SkiaBackend::new(), rows, frames, None);
    run_backend_bench(WebBackend::new(), rows, frames, None);
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
}
