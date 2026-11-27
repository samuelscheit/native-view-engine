//! Frame-0 `NVERELOC` + `MmapOffsetArena` (`NativeInstantBootStore`) prewarm helpers.
//!
//! Opens a pre-baked `frame0.nvereloc` / returning-user `user_frame0.nvereloc` (or legacy bundle
//! `NVERELOC`) and paints Frame 0 into [`SkiaBackend`] prior to loading any app `cdylib`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use nve_abi::{ColorRgba, SurfaceId};
use nve_backend_skia::raster::PixelBuffer;
use nve_backend_skia::SkiaBackend;
use nve_engine::{
    default_instant_boot_dir, is_snapshot_disabled, is_snapshot_reset_requested,
    reset_instant_boot_dir, resolve_sidecar_frame0_path, MmapReloc, NativeInstantBootStore,
    RelocBundleView, RelocError, SceneRelocView, StateStoreConfig, StoreWireReplica, VerifyMode,
    Viewport, RELOC_KIND_BUNDLE, RELOC_KIND_SCENE,
};

use super::session::new_shell_skia_replica;
use super::{DesktopHostError, DesktopPreviewOptions};

/// Pre-warmed Frame-0 backend state decoded directly from a zero-copy `NVERELOC` scene or
/// [`NativeInstantBootStore`] prior to loading any app `cdylib`.
pub struct PrewarmedSnapshot {
    pub replica: StoreWireReplica<SkiaBackend>,
    pub pixel_buffer: PixelBuffer,
    pub frame0_bytes: Vec<u8>,
    pub state_bytes: Vec<u8>,
    pub restore_us: u64,
    pub used_user_frame0: bool,
    pub has_frame0: bool,
}

/// Resolves default-on per-app `instant_boot_dir` and sidecar `frame0.nvereloc` for `dylib_path`,
/// honoring `--no-snapshot`, `--reset-state`, `NVE_SNAPSHOT_DISABLE`, `NVE_SNAPSHOT_PATH`, and
/// `NVE_RESET_STATE`.
pub(crate) fn resolve_default_instant_boot_options(
    dylib_path: &Path,
    options: &mut DesktopPreviewOptions,
) {
    if options.disable_instant_boot || is_snapshot_disabled() {
        options.instant_boot_dir = None;
        options.snapshot_path = None;
        return;
    }

    if options.instant_boot_dir.is_none() && options.snapshot_path.is_none() {
        if let Ok(env_path) = std::env::var("NVE_SNAPSHOT_PATH")
            && !env_path.trim().is_empty()
        {
            options.snapshot_path = Some(PathBuf::from(env_path.trim()));
        } else {
            let app_id = dylib_path.to_string_lossy();
            options.instant_boot_dir = default_instant_boot_dir(&app_id);
        }
    }

    if options.default_frame0_path.is_none() {
        options.default_frame0_path = resolve_sidecar_frame0_path(dylib_path);
    }

    if options.reset_state || is_snapshot_reset_requested() {
        if let Some(dir) = options.instant_boot_dir.as_deref() {
            reset_instant_boot_dir(dir);
            eprintln!(
                "[nve-skia-shell] Cleared cached instant-boot state in '{}' (--reset-state / NVE_RESET_STATE)",
                dir.display()
            );
        }
        if let Some(snap) = options.snapshot_path.as_deref() {
            let _ = std::fs::remove_file(snap);
            eprintln!(
                "[nve-skia-shell] Cleared cached snapshot at '{}' (--reset-state / NVE_RESET_STATE)",
                snap.display()
            );
        }
    }
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
    let mut replica = new_shell_skia_replica(surface_id, viewport);

    let (frame0_bytes, state_bytes) = match view.header().content_kind {
        RELOC_KIND_SCENE => {
            let scene = SceneRelocView::from_view(view)?;
            replica.backend_mut().load_scene_reloc(&scene)?;
            (Vec::new(), Vec::new())
        }
        RELOC_KIND_BUNDLE => {
            let bundle = RelocBundleView::from_view(view)?;
            let frame0 = bundle.frame0();
            if !frame0.is_empty() {
                replica.submit_nve2_frame(frame0)?;
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
    replica.backend().rasterize(&mut pixel_buffer, clear_color);
    let restore_us = t0.elapsed().as_micros() as u64;

    Ok(PrewarmedSnapshot {
        replica,
        pixel_buffer,
        frame0_bytes,
        state_bytes,
        restore_us,
        used_user_frame0: true,
        has_frame0: true,
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

    let mut replica = new_shell_skia_replica(surface_id, viewport);
    let mut used_user_frame0 = false;
    let mut has_frame0 = false;
    if let Some(frame0) = store.load_frame0(VerifyMode::Full) {
        used_user_frame0 = frame0.is_user_cached();
        let scene = frame0.scene_view(VerifyMode::Full)?;
        replica.backend_mut().load_scene_reloc(&scene)?;
        has_frame0 = true;
    }

    let state_bytes = store
        .state_bytes()?
        .map(|s| s.to_vec())
        .unwrap_or_default();
    let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
    let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
    let mut pixel_buffer = PixelBuffer::new(phys_w, phys_h);
    replica.backend().rasterize(&mut pixel_buffer, clear_color);
    let restore_us = t0.elapsed().as_micros() as u64;

    Ok(PrewarmedSnapshot {
        replica,
        pixel_buffer,
        frame0_bytes: Vec::new(),
        state_bytes,
        restore_us,
        used_user_frame0,
        has_frame0,
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
                if !p.has_frame0 && p.state_bytes.is_empty() {
                    return None;
                }
                if p.has_frame0 {
                    let src = if p.used_user_frame0 {
                        "user_frame0.nvereloc"
                    } else {
                        "bundled frame0.nvereloc"
                    };
                    eprintln!(
                        "[nve-skia-shell] Instant-boot Frame-0 NVERELOC restore from '{}' ({src}) in {} us (state={} bytes)",
                        boot_dir.display(),
                        p.restore_us,
                        p.state_bytes.len()
                    );
                } else {
                    eprintln!(
                        "[nve-skia-shell] Instant-boot state restore from '{}' ({} bytes) in {} us",
                        boot_dir.display(),
                        p.state_bytes.len(),
                        p.restore_us
                    );
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::{DesktopApp, DesktopSession, DesktopWindowHost};
    use nve_abi::FrameId;
    use nve_components::tables::{register_core_tables, Layout, Visual};
    use nve_store::wire::WireEncoder;
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

    struct CountingTestApp {
        count: u32,
        buf: Vec<u8>,
        state_buf: Vec<u8>,
    }

    impl CountingTestApp {
        fn new() -> Self {
            Self {
                count: 0,
                buf: Vec::new(),
                state_buf: Vec::new(),
            }
        }
    }

    impl DesktopApp for CountingTestApp {
        fn generation(&self) -> u64 {
            1
        }
        fn render(&mut self, _frame_id: FrameId) -> &[u8] {
            register_core_tables();
            let mut store = NodeStore::new();
            let mut changes = ChangeSet::default();
            {
                let mut tx = store.tx();
                let root = NodeId::ROOT;
                let n2 = NodeId::new(2, 0);
                let _ = tx.create_at(root);
                let _ = tx.create_at(n2);
                tx.set(n2, Layout::WIDTH, Length::points(200.0));
                tx.set(n2, Layout::HEIGHT, Length::points(100.0));
                tx.set(n2, Visual::BACKGROUND, Rgba::rgb(37, 99, 235));
                tx.set_children(root, &[n2]);
                tx.commit_into(&mut changes);
            }
            self.buf.clear();
            WireEncoder::default().encode(&store, &changes, &mut self.buf);
            &self.buf
        }
        fn dispatch_events(&mut self, _events: nve_abi::ValidatedEvents<'_>) -> bool {
            self.count += 1;
            true
        }
        fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
            false
        }
        fn reload(&mut self, _p: &Path, _sid: SurfaceId) -> Result<u64, DesktopHostError> {
            Ok(1)
        }
        fn snapshot_state(&mut self) -> &[u8] {
            self.state_buf = format!("count={}", self.count).into_bytes();
            &self.state_buf
        }
    }

    #[test]
    fn default_instant_boot_options_resolve_and_allow_repeated_saves() {
        let tmp_dir = std::env::temp_dir().join(format!(
            "nve_skia_default_boot_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let fake_dylib = tmp_dir.join("libdemo_default_boot.dylib");
        std::fs::write(&fake_dylib, b"stub").unwrap();
        let mut opts = DesktopPreviewOptions::default();
        resolve_default_instant_boot_options(&fake_dylib, &mut opts);
        assert!(opts.instant_boot_dir.is_some());
        assert!(opts.default_frame0_path.is_none());

        let sidecar = tmp_dir.join("frame0.nvereloc");
        std::fs::write(&sidecar, b"stub-frame0").unwrap();
        let mut opts_sidecar = DesktopPreviewOptions::default();
        resolve_default_instant_boot_options(&fake_dylib, &mut opts_sidecar);
        assert_eq!(opts_sidecar.default_frame0_path.as_deref(), Some(sidecar.as_path()));

        let mut opts_disabled = DesktopPreviewOptions {
            disable_instant_boot: true,
            snapshot_path: Some(tmp_dir.join("ignored.nvereloc")),
            ..Default::default()
        };
        resolve_default_instant_boot_options(&fake_dylib, &mut opts_disabled);
        assert!(opts_disabled.instant_boot_dir.is_none());
        assert!(opts_disabled.snapshot_path.is_none());

        let boot_dir = tmp_dir.join("repeated_save_store");
        let viewport = Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        };
        let mut session = DesktopSession::new(
            CountingTestApp::new(),
            SurfaceId::ROOT,
            viewport,
            ColorRgba::BLACK,
            false,
        );
        session.render_and_rasterize().unwrap();
        session.app_mut().count = 1;

        let host_opts = DesktopPreviewOptions {
            instant_boot_dir: Some(boot_dir.clone()),
            schema_version: 1,
            ..Default::default()
        };
        let mut host = DesktopWindowHost::from_primary_session(host_opts, session);
        host.save_snapshot_if_configured();

        let first = replay_instant_boot_store_into_backend(
            &boot_dir,
            None,
            1,
            SurfaceId::ROOT,
            viewport,
            ColorRgba::BLACK,
        )
        .unwrap();
        assert!(first.has_frame0);
        assert_eq!(first.state_bytes, b"count=1");

        host.session_mut().app_mut().count = 2;
        host.save_snapshot_if_configured();

        let second = replay_instant_boot_store_into_backend(
            &boot_dir,
            None,
            1,
            SurfaceId::ROOT,
            viewport,
            ColorRgba::BLACK,
        )
        .unwrap();
        assert_eq!(second.state_bytes, b"count=2");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
