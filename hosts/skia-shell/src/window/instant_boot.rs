//! Frame-0 `NVERELOC` + `MmapOffsetArena` (`NativeInstantBootStore`) prewarm helpers.
//!
//! Opens a pre-baked `frame0.nvereloc` / returning-user `user_frame0.nvereloc` (or legacy bundle
//! `NVERELOC`) and paints Frame 0 into [`SkiaBackend`] prior to loading any app `cdylib`.

use std::path::Path;
use std::time::Instant;

use nve_abi::{ColorRgba, SurfaceId};
use nve_backend_skia::raster::PixelBuffer;
use nve_backend_skia::SkiaBackend;
use nve_engine::{
    MmapReloc, NativeInstantBootStore, RelocBundleView, RelocError, RenderBackend, SceneRelocView,
    StateStoreConfig, VerifyMode, Viewport, RELOC_KIND_BUNDLE, RELOC_KIND_SCENE,
};

use super::{DesktopHostError, DesktopPreviewOptions};

/// Pre-warmed Frame-0 backend state decoded directly from a zero-copy `NVERELOC` scene or
/// [`NativeInstantBootStore`] prior to loading any app `cdylib`.
pub struct PrewarmedSnapshot {
    pub backend: SkiaBackend,
    pub pixel_buffer: PixelBuffer,
    pub frame0_bytes: Vec<u8>,
    pub state_bytes: Vec<u8>,
    pub restore_us: u64,
    pub used_user_frame0: bool,
}

/// Opens an `NVERELOC` file (`SceneRelocView` or `RelocBundleView`) with schema validation,
/// renders Frame 0 straight into a [`SkiaBackend`], and rasterizes before any app `cdylib` loads.
pub fn replay_snapshot_into_backend(
    path: &Path,
    schema_version: u32,
    surface_id: SurfaceId,
    viewport: Viewport,
    clear_color: ColorRgba,
) -> Result<PrewarmedSnapshot, DesktopHostError> {
    let t0 = Instant::now();
    let mapped = MmapReloc::open_with_schema(path, schema_version, VerifyMode::Full)?;
    let view = mapped.view();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(surface_id, viewport);

    let (frame0_bytes, state_bytes) = match view.header().content_kind {
        RELOC_KIND_SCENE => {
            let scene = SceneRelocView::from_view(view)?;
            backend.render_reloc_scene(scene)?;
            (Vec::new(), Vec::new())
        }
        RELOC_KIND_BUNDLE => {
            let bundle = RelocBundleView::from_view(view)?;
            let frame0 = bundle.frame0();
            if !frame0.is_empty() {
                backend.submit_frame(frame0)?;
            }
            (frame0.to_vec(), bundle.state().to_vec())
        }
        other => {
            return Err(DesktopHostError::Reloc(RelocError::ContentKindMismatch {
                expected: RELOC_KIND_SCENE,
                actual: other,
            }));
        }
    };

    let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
    let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
    let mut pixel_buffer = PixelBuffer::new(phys_w, phys_h);
    backend.rasterize(&mut pixel_buffer, clear_color);
    let restore_us = t0.elapsed().as_micros() as u64;

    Ok(PrewarmedSnapshot {
        backend,
        pixel_buffer,
        frame0_bytes,
        state_bytes,
        restore_us,
        used_user_frame0: true,
    })
}

/// Opens a [`NativeInstantBootStore`] (`user_frame0.nvereloc` -> bundled `frame0.nvereloc` +
/// `state.nvemmap`), paints Frame 0 straight into [`SkiaBackend`] with zero layout work, and
/// returns both the rasterized Frame 0 and restored per-user state bytes before `cdylib` boot.
pub fn replay_instant_boot_store_into_backend(
    user_dir: &Path,
    default_frame0_path: Option<&Path>,
    schema_version: u32,
    surface_id: SurfaceId,
    viewport: Viewport,
    clear_color: ColorRgba,
) -> Result<PrewarmedSnapshot, DesktopHostError> {
    let t0 = Instant::now();
    let mut store =
        NativeInstantBootStore::open(user_dir, schema_version, StateStoreConfig::default())?;
    if let Some(default_path) = default_frame0_path {
        store = store.with_default_frame0_path(default_path);
    }

    let mut backend = SkiaBackend::new();
    backend.set_viewport(surface_id, viewport);
    let mut used_user_frame0 = false;
    if let Some(frame0) = store.load_frame0(VerifyMode::Full) {
        used_user_frame0 = frame0.is_user_cached();
        let scene = frame0.scene_view(VerifyMode::Full)?;
        backend.render_reloc_scene(scene)?;
    }

    let state_bytes = store
        .state_bytes()?
        .map(|s| s.to_vec())
        .unwrap_or_default();
    let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
    let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
    let mut pixel_buffer = PixelBuffer::new(phys_w, phys_h);
    backend.rasterize(&mut pixel_buffer, clear_color);
    let restore_us = t0.elapsed().as_micros() as u64;

    Ok(PrewarmedSnapshot {
        backend,
        pixel_buffer,
        frame0_bytes: Vec::new(),
        state_bytes,
        restore_us,
        used_user_frame0,
    })
}

/// Resolves and prewarms Frame-0 from either `options.instant_boot_dir` or `options.snapshot_path`.
pub(crate) fn prewarm_from_options(
    options: &DesktopPreviewOptions,
    viewport: Viewport,
) -> Option<PrewarmedSnapshot> {
    if let Some(boot_dir) = options.instant_boot_dir.as_deref() {
        return match replay_instant_boot_store_into_backend(
            boot_dir,
            options.default_frame0_path.as_deref(),
            options.schema_version,
            options.surface_id,
            viewport,
            options.clear_color,
        ) {
            Ok(p) => {
                let src = if p.used_user_frame0 {
                    "user_frame0.nvereloc"
                } else {
                    "bundled frame0.nvereloc"
                };
                eprintln!(
                    "[nve-skia-shell] Instant-boot Frame-0 NVERELOC restore from '{}' ({src}) in {} us",
                    boot_dir.display(),
                    p.restore_us
                );
                Some(p)
            }
            Err(err) => {
                eprintln!(
                    "[nve-skia-shell] Warning: ignoring instant-boot dir '{}' ({err}); cold-booting app",
                    boot_dir.display()
                );
                None
            }
        };
    }
    let snap_path = options.snapshot_path.as_deref().filter(|p| p.exists())?;
    match replay_snapshot_into_backend(
        snap_path,
        options.schema_version,
        options.surface_id,
        viewport,
        options.clear_color,
    ) {
        Ok(p) => {
            eprintln!(
                "[nve-skia-shell] Instant-boot Frame-0 NVERELOC restore from '{}' in {} us",
                snap_path.display(),
                p.restore_us
            );
            Some(p)
        }
        Err(err) => {
            eprintln!(
                "[nve-skia-shell] Warning: ignoring snapshot '{}' ({err}); cold-booting app",
                snap_path.display()
            );
            None
        }
    }
}
