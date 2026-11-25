//! `nve prerender` — Build-time / cold Frame-0 `frame0.nvereloc` pre-baker for native NVE apps.
//!
//! Executes Frame 0 of the selected `NveApp` (`studio` or `counter`) at the requested logical
//! viewport + scale factor using `CosmicTextEngine` + `TaffyLayoutEngine`, validates the emitted
//! `SceneRelocView` under `VerifyMode::Full`, and writes it crash-safely via atomic rename.

use std::sync::Arc;
use std::time::Instant;

use nve_abi::ColorRgba;
use nve_backend_headless::prerender_native_frame0_with_text_measurer;
use nve_backend_skia::CosmicTextEngine;
use nve_engine::{write_reloc_atomic, NveApp, SceneRelocView, TextMeasurer, VerifyMode, Viewport};
use nve_example_counter::{CounterModule, COUNTER_STATE_BIN_VERSION};
use nve_studio_workspace::{StudioWorkspaceModule, STUDIO_SNAPSHOT_SCHEMA_VERSION};

/// Options passed to `nve prerender`.
#[derive(Debug, Clone)]
pub struct PrerenderOptions {
    pub app: String,
    pub out: std::path::PathBuf,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

/// Summary returned after pre-baking a `frame0.nvereloc` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrerenderReport {
    pub app: String,
    pub schema_version: u32,
    pub node_count: usize,
    pub byte_len: usize,
    pub elapsed_us: u64,
}

/// Prerenders Frame 0 for `opts.app` (`studio` | `counter`), validates it with `VerifyMode::Full`,
/// and atomically writes `frame0.nvereloc` to `opts.out`.
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

    let (slug, schema_version, buf) = match opts.app.trim().to_ascii_lowercase().as_str() {
        "studio" | "studio-workspace" | "nve-studio-workspace" => {
            let mut app = StudioWorkspaceModule::create(1);
            let buf = prerender_native_frame0_with_text_measurer(
                &mut app,
                1,
                viewport,
                STUDIO_SNAPSHOT_SCHEMA_VERSION,
                "studio",
                ColorRgba::rgb(11, 17, 32),
                measurer,
            )
            .map_err(|e| format!("failed to prerender studio Frame-0: {e}"))?;
            ("studio".to_string(), STUDIO_SNAPSHOT_SCHEMA_VERSION, buf)
        }
        "counter" | "nve-example-counter" => {
            let mut app = CounterModule::create(1);
            let buf = prerender_native_frame0_with_text_measurer(
                &mut app,
                1,
                viewport,
                COUNTER_STATE_BIN_VERSION,
                "counter",
                ColorRgba::rgb(15, 23, 42),
                measurer,
            )
            .map_err(|e| format!("failed to prerender counter Frame-0: {e}"))?;
            ("counter".to_string(), COUNTER_STATE_BIN_VERSION, buf)
        }
        other => {
            return Err(format!(
                "unsupported --app '{other}' (expected 'studio' or 'counter')"
            ));
        }
    };

    let scene = SceneRelocView::from_bytes(&buf, VerifyMode::Full)
        .map_err(|e| format!("generated frame0.nvereloc failed Full validation: {e}"))?;
    let node_count = scene.nodes().len();

    write_reloc_atomic(&opts.out, &buf)
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
                "Prerendered {} Frame-0 NVERELOC -> {} ({} nodes, {} bytes, schema v{}, {} us)",
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
    use nve_engine::MmapReloc;

    fn temp_out(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nve_cli_prerender_{name}_{}.nvereloc",
            std::process::id()
        ))
    }

    #[test]
    fn prerender_studio_and_counter_roundtrip_into_skia_backend() {
        for (app_name, w, h, expected_text) in [
            ("studio", 1280.0, 800.0, "AETHER"),
            ("counter", 640.0, 420.0, "Count: 0"),
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

            let mapped = MmapReloc::open_with_schema(&out, report.schema_version, VerifyMode::Full)
                .expect("mmap open");
            let scene = SceneRelocView::from_view(mapped.view()).expect("scene view");
            assert_eq!(scene.nodes().len(), report.node_count);
            assert_eq!(scene.viewport().width, w);
            assert_eq!(scene.viewport().height, h);

            let mut skia = SkiaBackend::new();
            let loaded = skia
                .render_reloc_scene(scene)
                .expect("skia render_reloc_scene");
            assert_eq!(loaded, report.node_count);
            assert!(!skia.display_list().ops.is_empty());

            let has_text = skia.display_list().ops.iter().any(|op| match op {
                nve_backend_skia::PaintOp::DrawText { text, .. } => text.contains(expected_text),
                _ => false,
            });
            assert!(
                has_text,
                "expected '{expected_text}' in prerendered {app_name} DisplayList"
            );

            let _ = std::fs::remove_file(Path::new(&out));
        }

        // Invalid app name rejected cleanly.
        assert!(prerender_app_to_file(&PrerenderOptions {
            app: "unknown-app".into(),
            out: temp_out("bad"),
            width: 800.0,
            height: 600.0,
            scale: 1.0,
        })
        .is_err());
    }
}
