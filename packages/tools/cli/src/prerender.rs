//! `nve prerender` — Build-time / cold Frame-0 `frame0.nve2` pre-baker for native NVE apps.
//!
//! Executes Frame 0 of the selected `NveApp` (`studio` or `kitchen-sink`), encodes its `NodeStore`
//! into a `Frame0File` (`frame0.nve2` = `NVE2` keyframe + 40-byte footer), validates it by booting
//! a cold `StoreWireReplica<HeadlessBackend>` at the requested viewport, and writes it atomically.

use std::sync::Arc;
use std::time::Instant;

use nve_abi::{FrameId, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::CosmicTextEngine;
use nve_tables::tables::{core_realization_order, register_core_tables};
use nve_engine::{Frame0File, Frame0Meta, NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_layout::TextMeasurer;
use nve_example_kitchen_sink::{KITCHEN_SINK_STATE_VERSION, KitchenSinkModule};
use nve_studio_workspace::{STUDIO_SNAPSHOT_SCHEMA_VERSION, StudioWorkspaceModule};

/// Options passed to `nve prerender`.
#[derive(Debug, Clone)]
pub struct PrerenderOptions {
    pub app: String,
    pub out: std::path::PathBuf,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

/// Summary returned after pre-baking a `frame0.nve2` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrerenderReport {
    pub app: String,
    pub schema_version: u32,
    pub node_count: usize,
    pub byte_len: usize,
    pub elapsed_us: u64,
}

fn prerender_module<A: NveApp>(
    app: &mut A,
    viewport: Viewport,
    measurer: Arc<dyn TextMeasurer>,
) -> Result<(Vec<u8>, usize), String> {
    register_core_tables();
    let mut backend = HeadlessBackend::new();
    backend.set_text_measurer(measurer);
    backend.set_viewport(SurfaceId::ROOT, viewport);
    let mut replica = StoreWireReplica::new(backend, core_realization_order());

    let frame1 = app.render(FrameId(1));
    replica
        .submit_nve2_frame(frame1)
        .map_err(|e| format!("failed to render Frame-0 NVE2: {e}"))?;

    let meta = Frame0Meta::default_core();
    let buf = Frame0File::encode(replica.store(), meta)
        .map_err(|e| format!("failed to encode Frame-0 NVE2: {e}"))?;

    // Round-trip validate into a cold replica before writing to disk.
    let mut verify_backend = HeadlessBackend::new();
    verify_backend.set_viewport(SurfaceId::ROOT, viewport);
    let mut verify_replica = StoreWireReplica::new(verify_backend, core_realization_order());
    let (footer, _) = Frame0File::boot_bytes_into_replica(&buf, meta, &mut verify_replica)
        .map_err(|e| format!("generated frame0.nve2 failed validation: {e}"))?;

    Ok((buf, footer.node_count as usize))
}

/// Prerenders Frame 0 for `opts.app` (`studio` | `kitchen-sink`), validates it via a cold
/// `Frame0File::boot_bytes_into_replica`, and atomically writes `frame0.nve2` to `opts.out`.
pub fn prerender_app_to_file(opts: &PrerenderOptions) -> Result<PrerenderReport, String> {
    if !opts.width.is_finite() || opts.width <= 0.0 {
        return Err(format!("invalid --width: {}", opts.width));
    }
    if !opts.height.is_finite() || opts.height <= 0.0 {
        return Err(format!("invalid --height: {}", opts.height));
    }
    if !opts.scale.is_finite() || opts.scale <= 0.0 {
        return Err(format!("invalid --scale: {}", opts.scale));
    }

    let viewport = Viewport {
        width: opts.width,
        height: opts.height,
        scale_factor: opts.scale,
    };
    let measurer: Arc<dyn TextMeasurer> = CosmicTextEngine::shared();
    let t0 = Instant::now();

    let app_arg = opts.app.trim();
    let (slug, schema_version, (buf, node_count)) = if app_arg.eq_ignore_ascii_case("studio")
        || app_arg.eq_ignore_ascii_case("studio-workspace")
        || app_arg.eq_ignore_ascii_case("nve-studio-workspace")
    {
        let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
        let res = prerender_module(&mut app, viewport, measurer)?;
        ("studio".to_string(), STUDIO_SNAPSHOT_SCHEMA_VERSION, res)
    } else if app_arg.eq_ignore_ascii_case("kitchen-sink")
        || app_arg.eq_ignore_ascii_case("nve-example-kitchen-sink")
    {
        let mut app = KitchenSinkModule::create(SurfaceId::ROOT);
        let res = prerender_module(&mut app, viewport, measurer)?;
        ("kitchen-sink".to_string(), KITCHEN_SINK_STATE_VERSION, res)
    } else {
        return Err(format!(
            "unsupported --app '{app_arg}' (expected 'studio' or 'kitchen-sink')"
        ));
    };

    Frame0File::save_bytes(&opts.out, &buf)
        .map_err(|e| format!("failed to write '{}': {e}", opts.out.display()))?;

    Ok(PrerenderReport {
        app: slug,
        schema_version,
        node_count,
        byte_len: buf.len(),
        elapsed_us: t0.elapsed().as_micros() as u64,
    })
}

pub fn cmd_prerender(opts: PrerenderOptions) {
    match prerender_app_to_file(&opts) {
        Ok(report) => {
            println!(
                "Prerendered {} Frame-0 NVE2 -> {} ({} nodes, {} bytes, schema v{}, {} us)",
                report.app,
                opts.out.display(),
                report.node_count,
                report.byte_len,
                report.schema_version,
                report.elapsed_us
            );
        }
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use nve_backend_skia::SkiaBackend;

    fn temp_out(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nve_cli_prerender_{name}_{}.nve2",
            std::process::id()
        ))
    }

    #[test]
    fn prerender_studio_and_kitchen_sink_roundtrip_into_skia_backend() {
        for (app_name, w, h, expected_text) in [
            ("studio", 1280.0, 800.0, "AETHER"),
            ("kitchen-sink", 1280.0, 800.0, "NVE Kitchen Sink"),
        ] {
            let out = temp_out(app_name);
            let report = prerender_app_to_file(&PrerenderOptions {
                app: app_name.into(),
                out: out.clone(),
                width: w,
                height: h,
                scale: 1.0,
            })
            .expect("prerender succeeds");
            assert!(report.node_count >= 5);
            assert!(report.byte_len >= 64);

            let meta = Frame0Meta::default_core();
            let mut skia = SkiaBackend::new();
            skia.set_viewport(
                SurfaceId::ROOT,
                Viewport {
                    width: w,
                    height: h,
                    scale_factor: 1.0,
                },
            );
            let mut replica = StoreWireReplica::new(skia, core_realization_order());
            let (footer, _) = Frame0File::boot_into_replica(&out, meta, &mut replica)
                .expect("skia boot_into_replica");
            assert_eq!(footer.node_count as usize, report.node_count);
            assert!(!replica.backend().display_list().ops.is_empty());

            let has_text = replica
                .backend()
                .display_list()
                .ops
                .iter()
                .any(|op| match op {
                    nve_backend_skia::PaintOp::DrawText { text, .. } => {
                        text.contains(expected_text)
                    }
                    _ => false,
                });
            assert!(
                has_text,
                "expected '{expected_text}' in prerendered {app_name} DisplayList"
            );

            let _ = std::fs::remove_file(Path::new(&out));
        }

        assert!(
            prerender_app_to_file(&PrerenderOptions {
                app: "unknown-app".into(),
                out: temp_out("bad"),
                width: 800.0,
                height: 600.0,
                scale: 1.0,
            })
            .is_err()
        );
    }
}
