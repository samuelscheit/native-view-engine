//! Window-independent controller driving rendering, resize, swapchain presentation, 2-tier hot
//! reload, and snapshot persistence for any [`DesktopApp`]. Interactive click/scroll/focus/
//! keyboard/IME dispatch methods live in [`super::input`].

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use nve_abi::{
    encode_events, ColorRgba, EventPayload, FrameId, NativeEvent, NativeEventKind, NodeId,
    SurfaceId, ROOT_NODE_ID,
};
use nve_backend_skia::gpu::{
    FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceError, GpuSwapchain,
};
use nve_backend_skia::raster::{draw_hud_banner, PixelBuffer};
#[cfg(feature = "skia")]
use nve_backend_skia::gpu::GpuSurfaceConfig;
#[cfg(feature = "skia")]
use nve_backend_skia::{GpuSurface, GpuSurfaceBackend, SkiaCanvasRenderer};
use nve_backend_skia::{CursorKind, SkiaBackend};
use nve_engine::{write_snapshot_atomic, HotPatchMsg, RenderBackend, SnapshotHeader, Viewport};

use super::clipboard::{default_clipboard, ClipboardProvider};
use super::hot_reload_client::resolve_dylib_path;
use super::{
    write_pixel_buffer_ppm, write_pixel_buffer_screenshot, DesktopApp, DesktopHostError,
};

#[inline]
pub(super) fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Window-independent controller driving rendering, hit-test -> event dispatch, 2-tier hot
/// reload, and snapshot persistence for any [`DesktopApp`].
pub struct DesktopSession<A: DesktopApp> {
    pub(super) app: A,
    pub(super) backend: SkiaBackend,
    pub(super) surface_id: SurfaceId,
    next_frame_id: FrameId,
    pub(super) viewport: Viewport,
    pixel_buffer: PixelBuffer,
    clear_color: ColorRgba,
    pub(super) cursor_logical: (f32, f32),
    pub(super) dragging_text_node: Option<NodeId>,
    pub(super) last_click_time: Option<Instant>,
    pub(super) last_click_pos: (f32, f32),
    pub(super) last_click_node: Option<NodeId>,
    pub(super) clipboard: Box<dyn ClipboardProvider>,
    status_banner: Option<String>,
    show_hud: bool,
    last_frame_bytes: Vec<u8>,
    frame_counter: u64,
    last_telemetry: Option<FrameTelemetry>,
    #[cfg(feature = "skia")]
    skia_renderer: SkiaCanvasRenderer,
    #[cfg(feature = "skia")]
    gpu_surface: Option<GpuSurface>,
}

impl<A: DesktopApp> DesktopSession<A> {
    pub fn new(
        app: A,
        surface_id: SurfaceId,
        viewport: Viewport,
        clear_color: ColorRgba,
        show_hud: bool,
    ) -> Self {
        let mut backend = SkiaBackend::new();
        backend.set_viewport(surface_id, viewport);
        let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
        let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
        Self::from_parts(
            app,
            backend,
            PixelBuffer::new(phys_w, phys_h),
            Vec::new(),
            surface_id,
            viewport,
            clear_color,
            show_hud,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        app: A,
        backend: SkiaBackend,
        pixel_buffer: PixelBuffer,
        last_frame_bytes: Vec<u8>,
        surface_id: SurfaceId,
        viewport: Viewport,
        clear_color: ColorRgba,
        show_hud: bool,
    ) -> Self {
        #[cfg(feature = "skia")]
        let gpu_surface = GpuSurface::new(GpuSurfaceConfig {
            width: pixel_buffer.width(),
            height: pixel_buffer.height(),
            scale_factor: viewport.scale_factor,
            ..Default::default()
        })
        .ok();
        Self {
            app,
            backend,
            surface_id,
            next_frame_id: 1,
            viewport,
            pixel_buffer,
            clear_color,
            cursor_logical: (0.0, 0.0),
            dragging_text_node: None,
            last_click_time: None,
            last_click_pos: (0.0, 0.0),
            last_click_node: None,
            clipboard: default_clipboard(),
            status_banner: None,
            show_hud,
            last_frame_bytes,
            frame_counter: 0,
            last_telemetry: None,
            #[cfg(feature = "skia")]
            skia_renderer: SkiaCanvasRenderer::new(),
            #[cfg(feature = "skia")]
            gpu_surface,
        }
    }

    pub fn app(&self) -> &A {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut A {
        &mut self.app
    }

    pub fn backend(&self) -> &SkiaBackend {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut SkiaBackend {
        &mut self.backend
    }

    pub fn active_cursor(&self) -> CursorKind {
        self.backend.active_cursor()
    }

    pub fn pixel_buffer(&self) -> &PixelBuffer {
        &self.pixel_buffer
    }

    pub fn last_frame_bytes(&self) -> &[u8] {
        &self.last_frame_bytes
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn status_banner(&self) -> Option<&str> {
        self.status_banner.as_deref()
    }

    pub fn set_status_banner(&mut self, banner: Option<String>) {
        self.status_banner = banner;
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.backend.focused_node()
    }

    pub fn with_clipboard(mut self, clipboard: Box<dyn ClipboardProvider>) -> Self {
        self.clipboard = clipboard;
        self
    }

    pub fn set_clipboard(&mut self, clipboard: Box<dyn ClipboardProvider>) {
        self.clipboard = clipboard;
    }

    pub fn clipboard_mut(&mut self) -> &mut dyn ClipboardProvider {
        self.clipboard.as_mut()
    }

    pub fn active_backend(&self) -> GpuBackendKind {
        #[cfg(feature = "skia")]
        if let Some(surface) = &self.gpu_surface {
            return surface.kind();
        }
        GpuBackendKind::Softbuffer
    }

    pub fn last_frame_telemetry(&self) -> Option<&FrameTelemetry> {
        self.last_telemetry.as_ref()
    }

    pub fn clear_color(&self) -> ColorRgba {
        self.clear_color
    }

    pub fn hud_banner_if_enabled(&self) -> Option<&str> {
        if self.show_hud {
            self.status_banner.as_deref()
        } else {
            None
        }
    }

    /// Replays a freeze-dried Frame-0 `CommandBuffer` directly into the backend and rasterizes it.
    pub fn replay_frame0_command_buffer(
        &mut self,
        frame0_bytes: &[u8],
    ) -> Result<&PixelBuffer, DesktopHostError> {
        if !frame0_bytes.is_empty() {
            self.backend.submit_frame(frame0_bytes)?;
            self.last_frame_bytes = frame0_bytes.to_vec();
        }
        self.rasterize_current();
        Ok(&self.pixel_buffer)
    }

    /// Atomically persists the last rendered Frame-0 `CommandBuffer` and live app state into an
    /// `NVE_SNAP` file on disk.
    pub fn save_snapshot(
        &mut self,
        path: &Path,
        schema_version: u32,
    ) -> Result<SnapshotHeader, DesktopHostError> {
        let state_bytes = self.app.snapshot_state();
        let header = write_snapshot_atomic(
            path,
            schema_version,
            0,
            &self.last_frame_bytes,
            state_bytes,
            &[],
        )?;
        Ok(header)
    }

    /// Dispatches a binary `NVH1` host-event batch into the mounted app.
    pub fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        self.app.dispatch_host_events(batch)
    }

    /// Writes the current rasterized [`PixelBuffer`] to a binary `P6` PPM file.
    pub fn write_screenshot_ppm(&self, path: &Path) -> Result<(), DesktopHostError> {
        write_pixel_buffer_ppm(&self.pixel_buffer, path)?;
        Ok(())
    }

    /// Writes the current rasterized [`PixelBuffer`] as PNG (for `.png` paths) or `P6` PPM.
    pub fn write_screenshot(&self, path: &Path) -> Result<(), DesktopHostError> {
        write_pixel_buffer_screenshot(&self.pixel_buffer, path)?;
        Ok(())
    }

    /// Dispatches the current logical viewport dimensions to the mounted app as a `Layout` event
    /// so responsive layouts (e.g. mobile portrait vs. desktop) react immediately.
    pub fn notify_viewport_resize(&mut self) -> bool {
        let event = NativeEvent {
            surface_id: self.surface_id,
            node_id: ROOT_NODE_ID,
            timestamp_us: now_us(),
            kind: NativeEventKind::Layout,
            payload: EventPayload::Layout {
                x: 0.0,
                y: 0.0,
                width: self.viewport.width,
                height: self.viewport.height,
            },
        };
        let encoded = encode_events(&[event]);
        self.app.dispatch_events(&encoded)
    }

    /// Presents the current retained `DisplayList` directly through a hardware or offscreen
    /// [`GpuSwapchain`] without touching the CPU software `PixelBuffer`.
    pub fn present_to_swapchain(
        &mut self,
        swapchain: &mut GpuSwapchain,
    ) -> Result<FrameTelemetry, GpuSurfaceError> {
        let hud = if self.show_hud {
            self.status_banner.as_deref()
        } else {
            None
        };
        let telemetry = swapchain.execute_frame(
            self.backend.display_list(),
            self.viewport.scale_factor,
            self.clear_color,
            hud,
        )?;
        self.last_telemetry = Some(telemetry.clone());
        Ok(telemetry)
    }

    pub fn set_cursor_logical(&mut self, x: f32, y: f32) {
        self.cursor_logical = (x, y);
    }

    pub fn cursor_logical(&self) -> (f32, f32) {
        self.cursor_logical
    }

    /// Scrolls the `ScrollView` under logical `(x, y)` by `(delta_x, delta_y)` (positive `delta_y`
    /// scrolls down), clamps to content bounds, dispatches `NativeEventKind::Scroll` into the app,
    /// and re-renders / re-rasterizes when either the app handled the event or the scroll offset
    /// changed.
    pub fn handle_scroll_logical(
        &mut self,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        let Some(scroll_node_id) = self.backend.hit_test_scroll_view(x, y) else {
            return Ok(false);
        };
        let prev = self.backend.scroll_offset(scroll_node_id);
        let (new_x, new_y) = self
            .backend
            .apply_scroll_delta(scroll_node_id, delta_x, delta_y);
        let offset_changed = (prev.0 - new_x).abs() > 1e-4 || (prev.1 - new_y).abs() > 1e-4;
        let (content_width, content_height) = self
            .backend
            .scroll_content_size(scroll_node_id)
            .unwrap_or((self.viewport.width, self.viewport.height));

        let event = NativeEvent {
            surface_id: self.surface_id,
            node_id: scroll_node_id,
            timestamp_us: now_us(),
            kind: NativeEventKind::Scroll,
            payload: EventPayload::Scroll {
                x: new_x,
                y: new_y,
                content_width,
                content_height,
            },
        };
        let encoded = encode_events(&[event]);
        let handled = self.app.dispatch_events(&encoded);
        if handled {
            self.render_and_rasterize()?;
        } else if offset_changed {
            self.rasterize_current();
        }
        Ok(handled || offset_changed)
    }

    /// Updates physical surface dimensions and HiDPI scale factor, then re-layouts and re-blits.
    pub fn resize(
        &mut self,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) -> Result<(), DesktopHostError> {
        let pw = physical_width.max(1);
        let ph = physical_height.max(1);
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        self.viewport = Viewport {
            width: pw as f32 / scale,
            height: ph as f32 / scale,
            scale_factor: scale,
        };
        self.backend.set_viewport(self.surface_id, self.viewport);
        self.pixel_buffer.resize(pw, ph);

        let app_handled_resize = self.notify_viewport_resize();

        if app_handled_resize || self.last_frame_bytes.is_empty() {
            self.render_and_rasterize()?;
        } else {
            self.backend.submit_frame(&self.last_frame_bytes)?;
            self.rasterize_current();
        }
        Ok(())
    }

    /// Renders a frame from the app into `SkiaBackend` and rasterizes into `pixel_buffer`.
    pub fn render_and_rasterize(&mut self) -> Result<&PixelBuffer, DesktopHostError> {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;
        let bytes = self.app.render(frame_id);
        if !bytes.is_empty() {
            self.backend.submit_frame(bytes)?;
            self.last_frame_bytes.clear();
            self.last_frame_bytes.extend_from_slice(bytes);
        }
        self.rasterize_current();
        Ok(&self.pixel_buffer)
    }

    pub(super) fn rasterize_current(&mut self) {
        let t0 = Instant::now();
        #[cfg(feature = "skia")]
        let (painted_via_skia, active_kind, fallback_reason) =
            if let Some(surface) = self.gpu_surface.as_mut() {
                let pw = self.pixel_buffer.width();
                let ph = self.pixel_buffer.height();
                let ok = surface.resize(pw, ph).is_ok()
                    && self
                        .backend
                        .paint_to_surface(&self.skia_renderer, surface, self.clear_color)
                        .is_ok()
                    && surface.copy_to_pixel_buffer(&mut self.pixel_buffer).is_ok();
                (
                    ok,
                    surface.kind(),
                    surface.fallback_reason().map(str::to_owned),
                )
            } else {
                (false, GpuBackendKind::Softbuffer, None)
            };
        #[cfg(not(feature = "skia"))]
        let (painted_via_skia, active_kind, fallback_reason): (bool, GpuBackendKind, Option<String>) =
            (false, GpuBackendKind::Softbuffer, None);

        if !painted_via_skia {
            self.backend
                .rasterize(&mut self.pixel_buffer, self.clear_color);
        }
        if self.show_hud
            && let Some(banner) = &self.status_banner
        {
            draw_hud_banner(&mut self.pixel_buffer, self.viewport.scale_factor, banner);
        }
        let elapsed_us = t0.elapsed().as_micros() as u64;
        self.frame_counter += 1;
        self.last_telemetry = Some(FrameTelemetry {
            backend: if painted_via_skia {
                active_kind
            } else {
                GpuBackendKind::Softbuffer
            },
            frame_index: self.frame_counter,
            width: self.pixel_buffer.width(),
            height: self.pixel_buffer.height(),
            scale_factor: self.viewport.scale_factor,
            acquire_us: 0,
            record_us: elapsed_us,
            submit_us: 0,
            present_us: 0,
            total_us: elapsed_us,
            swapchain_recreations: 0,
            fallback_reason,
        });
    }

    /// Processes a 2-tier hot-reload message from `nve dev` and returns `true` when the frame
    /// buffer was updated and should be presented.
    pub fn handle_hot_patch(&mut self, msg: HotPatchMsg) -> Result<bool, DesktopHostError> {
        match &msg {
            HotPatchMsg::TemplatePatch { generation, .. } => {
                let changed = self.app.apply_hot_patch(&msg.to_bytes());
                self.status_banner = Some(format!("Tier-1 patch gen #{generation}"));
                if changed {
                    self.render_and_rasterize()?;
                } else {
                    self.rasterize_current();
                }
                Ok(true)
            }
            HotPatchMsg::DylibReload {
                generation,
                dylib_path,
                build_ms,
            } => {
                let t0 = Instant::now();
                let resolved = resolve_dylib_path(dylib_path);
                match self.app.reload(&resolved, self.surface_id) {
                    Ok(new_gen) => {
                        self.backend = SkiaBackend::new();
                        self.backend.set_viewport(self.surface_id, self.viewport);
                        self.last_frame_bytes.clear();
                        let swap_ms = t0.elapsed().as_millis();
                        self.status_banner = Some(format!(
                            "Hot reload gen #{new_gen} (build {build_ms}ms, swap {swap_ms}ms)"
                        ));
                        self.render_and_rasterize()?;
                        Ok(true)
                    }
                    Err(err) => {
                        self.status_banner =
                            Some(format!("Reload gen #{generation} failed: {err}"));
                        self.rasterize_current();
                        Err(err)
                    }
                }
            }
            HotPatchMsg::DevStatus {
                message,
                latency_ms,
            } => {
                self.status_banner = if *latency_ms > 0 {
                    Some(format!("{message} ({latency_ms}ms)"))
                } else {
                    Some(message.clone())
                };
                self.rasterize_current();
                Ok(true)
            }
        }
    }
}
