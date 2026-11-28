//! Window-independent controller driving rendering, resize, swapchain presentation, 2-tier hot
//! reload, and snapshot persistence for any [`DesktopApp`]. Interactive click/scroll/focus/
//! keyboard/IME dispatch methods live in [`super::input`].

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use nve_abi::{ColorRgba, EventBatchBuf, FrameId, NodeId, SurfaceId, TimestampUs, ROOT_NODE_ID};
use nve_backend_skia::gpu::{
    FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceError, GpuSwapchain,
};
use nve_backend_skia::raster::{draw_hud_banner, PixelBuffer};
#[cfg(feature = "skia")]
use nve_backend_skia::gpu::GpuSurfaceConfig;
#[cfg(feature = "skia")]
use nve_backend_skia::{GpuSurface, GpuSurfaceBackend, SkiaCanvasRenderer};
use nve_backend_skia::{CursorKind, SkiaBackend};
use nve_canvas_skia::CanvasSkiaSystem;
use nve_tables::tables::{core_realization_order, register_core_tables};
use nve_engine::{
    Frame0File, Frame0Footer, Frame0Meta, HotPatchMsg, LayoutRect, StoreBackend, StoreWireReplica,
    Viewport,
};
use nve_layout::DynLayoutEngine;
use nve_store::ChangeSet;
#[cfg(test)]
use nve_store::NodeStore;
use nve_motion_skia::MotionDriver;
use nve_svg_skia::SvgSkiaSystem;

use super::clipboard::{default_clipboard, ClipboardProvider};
use super::hot_reload_client::resolve_dylib_path;
#[cfg(test)]
use super::write_pixel_buffer_ppm;
use super::{write_pixel_buffer_screenshot, DesktopApp, DesktopHostError, LayoutEngineChoice};
use crate::host_services::SkiaHostEvent;

/// Shell Skia backend parameterized over a pluggable [`DynLayoutEngine`] (Taffy default or Yoga opt-in).
pub type ShellSkiaBackend = SkiaBackend<DynLayoutEngine>;

#[inline]
pub(super) fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Creates a [`StoreWireReplica<ShellSkiaBackend>`] configured with shell table systems (`Canvas` +
/// `Svg`), the `nve-motion` frame hook, and an explicit pluggable [`DynLayoutEngine`].
pub fn new_shell_skia_replica_with_layout(
    surface_id: SurfaceId,
    viewport: Viewport,
    layout: DynLayoutEngine,
) -> StoreWireReplica<ShellSkiaBackend> {
    register_core_tables();
    let mut backend = SkiaBackend::with_layout(layout);
    backend.register_table_system(CanvasSkiaSystem);
    backend.register_table_system(SvgSkiaSystem);
    backend.register_frame_hook(MotionDriver::new());
    backend.set_viewport(surface_id, viewport);
    StoreWireReplica::new(backend, core_realization_order())
}

/// Pre-warms a cold [`StoreWireReplica<ShellSkiaBackend>`] and [`PixelBuffer`] from a Frame-0
/// `frame0.nve2` file on disk, running live `layout_choice` layout against `viewport`.
pub(crate) fn prewarm_frame0_into_replica(
    path: &Path,
    expected: Frame0Meta,
    surface_id: SurfaceId,
    viewport: Viewport,
    clear_color: ColorRgba,
    layout_choice: LayoutEngineChoice,
) -> Result<(StoreWireReplica<ShellSkiaBackend>, PixelBuffer, Frame0Footer), DesktopHostError> {
    let layout = layout_choice.build_layout_engine()?;
    let mut replica = new_shell_skia_replica_with_layout(surface_id, viewport, layout);
    let (footer, _) = Frame0File::boot_into_replica(path, expected, &mut replica)?;
    let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
    let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
    let mut pixel_buffer = PixelBuffer::new(phys_w, phys_h);
    let (store, backend) = replica.store_and_backend_mut();
    backend.rasterize_with_store(store, &mut pixel_buffer, clear_color);
    Ok((replica, pixel_buffer, footer))
}

/// Window-independent controller driving rendering, hit-test -> event dispatch, 2-tier hot
/// reload, and Frame-0 `frame0.nve2` persistence for any [`DesktopApp`].
pub struct DesktopSession<A: DesktopApp> {
    pub(super) app: A,
    pub(super) replica: StoreWireReplica<ShellSkiaBackend>,
    layout_choice: LayoutEngineChoice,
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
    pub(super) last_click_count: u8,
    pub(super) clipboard: Box<dyn ClipboardProvider>,
    status_banner: Option<String>,
    show_hud: bool,
    last_frame_bytes: Vec<u8>,
    frame_counter: FrameId,
    store_generation: u32,
    pub(super) a11y_synced_wire_frame: u64,
    last_telemetry: Option<FrameTelemetry>,
    /// Wall-clock instant of the previous animation tick (`None` while idle).
    last_animation_tick: Option<Instant>,
    #[cfg(feature = "skia")]
    skia_renderer: SkiaCanvasRenderer,
    #[cfg(feature = "skia")]
    gpu_surface: Option<GpuSurface>,
}

impl<A: DesktopApp> DesktopSession<A> {
    #[cfg(test)]
    pub fn new(
        app: A,
        surface_id: SurfaceId,
        viewport: Viewport,
        clear_color: ColorRgba,
        show_hud: bool,
    ) -> Self {
        Self::new_with_layout(
            app,
            surface_id,
            viewport,
            clear_color,
            show_hud,
            LayoutEngineChoice::Taffy,
        )
        .expect("default Taffy layout engine")
    }

    pub fn new_with_layout(
        app: A,
        surface_id: SurfaceId,
        viewport: Viewport,
        clear_color: ColorRgba,
        show_hud: bool,
        layout_choice: LayoutEngineChoice,
    ) -> Result<Self, DesktopHostError> {
        let layout = layout_choice.build_layout_engine()?;
        let replica = new_shell_skia_replica_with_layout(surface_id, viewport, layout);
        let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
        let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
        Ok(Self::from_parts(
            app,
            replica,
            PixelBuffer::new(phys_w, phys_h),
            Vec::new(),
            surface_id,
            viewport,
            clear_color,
            show_hud,
        )
        .with_layout_choice(layout_choice))
    }

    pub fn with_layout_choice(mut self, layout_choice: LayoutEngineChoice) -> Self {
        self.layout_choice = layout_choice;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        app: A,
        replica: StoreWireReplica<ShellSkiaBackend>,
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
            replica,
            layout_choice: LayoutEngineChoice::Taffy,
            surface_id,
            next_frame_id: FrameId(1),
            viewport,
            pixel_buffer,
            clear_color,
            cursor_logical: (0.0, 0.0),
            dragging_text_node: None,
            last_click_time: None,
            last_click_pos: (0.0, 0.0),
            last_click_node: None,
            last_click_count: 0,
            clipboard: default_clipboard(),
            status_banner: None,
            show_hud,
            last_frame_bytes,
            frame_counter: FrameId::ZERO,
            store_generation: 1,
            a11y_synced_wire_frame: 0,
            last_telemetry: None,
            last_animation_tick: None,
            #[cfg(feature = "skia")]
            skia_renderer: SkiaCanvasRenderer::new(),
            #[cfg(feature = "skia")]
            gpu_surface,
        }
    }

    #[inline]
    pub fn store_generation(&self) -> u32 {
        self.store_generation
    }

    pub fn app(&self) -> &A {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut A {
        &mut self.app
    }

    #[cfg(test)]
    pub fn backend(&self) -> &ShellSkiaBackend {
        self.replica.backend()
    }

    #[cfg(test)]
    pub fn store(&self) -> &NodeStore {
        self.replica.store()
    }

    pub fn active_cursor(&self) -> CursorKind {
        self.replica.backend().current_cursor(self.replica.store())
    }

    pub fn pixel_buffer(&self) -> &PixelBuffer {
        &self.pixel_buffer
    }

    #[cfg(test)]
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

    #[cfg(test)]
    pub fn focused_node(&self) -> Option<NodeId> {
        self.replica.backend().focused_node()
    }

    #[cfg(test)]
    pub fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.replica.backend().layout_tree().layout_of(node_id)
    }

    #[cfg(test)]
    pub fn selected_text(&self) -> Option<String> {
        let id = self.replica.backend().focused_node()?;
        let (s, e) = self.replica.backend().focus_state().selection_range()?;
        let val = self
            .replica
            .backend()
            .text_input_value(self.replica.store(), id)?;
        val.get(s..e).map(str::to_owned)
    }

    /// Logical viewport rectangle of the live caret or active IME preedit span.
    pub fn ime_cursor_rect_logical(&self) -> Option<LayoutRect> {
        self.replica.backend().ime_cursor_rect(self.replica.store())
    }

    /// Physical-pixel viewport rectangle of the live caret or active IME preedit span.
    pub fn ime_cursor_rect_physical(&self) -> Option<LayoutRect> {
        let r = self.ime_cursor_rect_logical()?;
        let s = self.viewport.scale_factor;
        Some(LayoutRect {
            x: r.x * s,
            y: r.y * s,
            width: r.width * s,
            height: r.height * s,
        })
    }

    #[cfg(test)]
    pub fn with_clipboard(mut self, clipboard: Box<dyn ClipboardProvider>) -> Self {
        self.clipboard = clipboard;
        self
    }

    #[cfg(test)]
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

    /// Atomically persists the current `NodeStore` as a Frame-0 `frame0.nve2` keyframe on disk.
    pub fn save_frame0(
        &self,
        path: &Path,
        meta: Frame0Meta,
    ) -> Result<Frame0Footer, DesktopHostError> {
        Ok(Frame0File::save(path, self.replica.store(), meta)?)
    }

    /// Dispatches a direct typed [`SkiaHostEvent`] into the mounted app with zero serialization.
    pub fn dispatch_host_event(&mut self, event: &SkiaHostEvent) -> bool {
        if let SkiaHostEvent::ThemeChanged(theme) = event {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.set_reduced_motion(store, theme.reduced_motion);
        }
        self.app.dispatch_host_event(event)
    }

    /// Writes the current rasterized [`PixelBuffer`] to a binary `P6` PPM file.
    #[cfg(test)]
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
        let mut batch = EventBatchBuf::with_capacity(1, 0);
        batch.push_layout(
            self.surface_id,
            ROOT_NODE_ID,
            TimestampUs(now_us()),
            0.0,
            0.0,
            self.viewport.width,
            self.viewport.height,
        );
        self.app.dispatch_events(batch.as_validated())
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
        let (store, backend) = self.replica.store_and_backend_mut();
        let (dl, registry, events) = backend.paint_parts_mut();
        let telemetry = swapchain.execute_frame_with_plugins(
            dl,
            self.viewport.scale_factor,
            self.clear_color,
            hud,
            Some((store, registry, events)),
        )?;
        self.last_telemetry = Some(telemetry.clone());
        Ok(telemetry)
    }

    pub fn cursor_logical(&self) -> (f32, f32) {
        self.cursor_logical
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
        self.replica
            .backend_mut()
            .set_viewport(self.surface_id, self.viewport);
        self.pixel_buffer.resize(pw, ph);

        let app_handled_resize = self.notify_viewport_resize();

        if app_handled_resize || self.last_frame_bytes.is_empty() {
            self.render_and_rasterize()?;
        } else {
            self.replica.commit_changes(&ChangeSet::default())?;
            self.rasterize_current();
        }
        Ok(())
    }

    /// Advances frame-hook animations by the wall-clock time since the previous tick (clamped to
    /// 100 ms) and re-rasterizes; returns whether another frame must be scheduled.
    pub fn tick_animations(&mut self) -> bool {
        if !self.replica.backend().needs_next_frame() {
            self.last_animation_tick = None;
            return false;
        }
        let now = Instant::now();
        let dt_s = self
            .last_animation_tick
            .replace(now)
            .map_or(0.0, |prev| now.duration_since(prev).as_secs_f32().min(0.1));
        let (store, backend) = self.replica.store_and_backend_mut();
        let animating = backend.advance_animations(store, dt_s);
        self.rasterize_current();
        if !animating {
            self.last_animation_tick = None;
        }
        animating
    }

    /// Renders a frame (or drains ordered queued remote frames) into `SkiaBackend` and rasterizes.
    pub fn render_and_rasterize(&mut self) -> Result<&PixelBuffer, DesktopHostError> {
        let mut saw_queued = false;
        while let Some((is_keyframe, bytes)) = self.app.take_queued_wire_frame() {
            saw_queued = true;
            if bytes.is_empty() {
                continue;
            }
            let prev_wire_frame = self.replica.last_changes().frame;
            let res = if is_keyframe {
                self.replica.submit_nve2_keyframe(&bytes)
            } else {
                self.replica.submit_nve2_frame(&bytes)
            };
            match res {
                Ok(_) => {
                    let next_wire_frame = self.replica.last_changes().frame;
                    if is_keyframe || (prev_wire_frame > 0 && next_wire_frame <= prev_wire_frame) {
                        self.store_generation = self.store_generation.wrapping_add(1).max(1);
                        self.a11y_synced_wire_frame = 0;
                    }
                    self.app.on_queued_wire_frame_applied(is_keyframe);
                    self.last_frame_bytes = bytes;
                }
                Err(_) => {
                    self.app.on_queued_wire_frame_error();
                    break;
                }
            }
        }
        if !saw_queued && !self.app.uses_queued_wire_frames() {
            let frame_id = self.next_frame_id;
            self.next_frame_id = self.next_frame_id.next();
            let bytes = self.app.render(frame_id);
            if !bytes.is_empty() {
                let prev_wire_frame = self.replica.last_changes().frame;
                self.replica.submit_nve2_frame(bytes)?;
                let next_wire_frame = self.replica.last_changes().frame;
                if prev_wire_frame > 0 && next_wire_frame <= prev_wire_frame {
                    self.store_generation = self.store_generation.wrapping_add(1).max(1);
                    self.a11y_synced_wire_frame = 0;
                }
                self.last_frame_bytes.clear();
                self.last_frame_bytes.extend_from_slice(bytes);
            }
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
                let (store, backend) = self.replica.store_and_backend_mut();
                let (dl, registry, events) = backend.paint_parts_mut();
                let ok = surface.resize(pw, ph).is_ok()
                    && surface
                        .render_display_list_with_plugins(
                            &self.skia_renderer,
                            dl,
                            self.viewport.scale_factor,
                            self.clear_color,
                            Some((store, registry, events)),
                        )
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
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.rasterize_with_store(store, &mut self.pixel_buffer, self.clear_color);
        }
        if self.show_hud
            && let Some(banner) = &self.status_banner
        {
            draw_hud_banner(&mut self.pixel_buffer, self.viewport.scale_factor, banner);
        }
        let elapsed_us = t0.elapsed().as_micros() as u64;
        self.frame_counter = self.frame_counter.next();
        self.last_telemetry = Some(FrameTelemetry {
            backend: if painted_via_skia {
                active_kind
            } else {
                GpuBackendKind::Softbuffer
            },
            frame_index: self.frame_counter.raw(),
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
                let bytes = msg
                    .to_bytes()
                    .map_err(|e| DesktopHostError::Reload(e.to_string()))?;
                let changed = self.app.apply_hot_patch(&bytes);
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
                        let layout = self.layout_choice.build_layout_engine()?;
                        self.replica =
                            new_shell_skia_replica_with_layout(self.surface_id, self.viewport, layout);
                        self.store_generation = self.store_generation.wrapping_add(1).max(1);
                        self.a11y_synced_wire_frame = 0;
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
