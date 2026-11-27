//! Generic Skia Ganesh hardware GPU frame presenter (`GaneshPresenter<S>`) shared across
//! Apple Metal, Vulkan, and Direct3D 12 swapchains.
//!
//! Centralizes the frame lifecycle state machine (`Idle -> Begun -> Rendered -> Submitted -> Idle`),
//! per-stage timing telemetry, `SkiaCanvasRenderer` dispatch, offscreen readback, and HiDPI resize
//! guards so each platform backend implements only its thin [`NativeSwapchain`] surface hooks.

use std::time::Instant;

use nve_abi::ColorRgba;
use skia_safe::{
    gpu::{self, surfaces, DirectContext, SurfaceOrigin, SyncCpu},
    ImageInfo, Surface,
};

use super::skia_renderer::{read_surface_to_pixel_buffer, to_skia_color, SkiaCanvasRenderer};
use super::{
    FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceError,
    SkiaPluginContext, SwapchainExtent,
};
use crate::raster::PixelBuffer;
use crate::DisplayList;

/// Platform-specific Skia Ganesh swapchain & offscreen render-target hooks driven by
/// [`GaneshPresenter`].
pub trait NativeSwapchain {
    /// Hardware backend kind reported in telemetry and [`GpuFramePresenter::kind`].
    const KIND: GpuBackendKind;

    /// Diagnostic message returned when CPU pixel readback is requested on a window-attached
    /// swapchain that presents directly to the compositor.
    const WINDOW_READBACK_ERROR: &'static str;

    /// Returns `true` when presenting directly to a native window surface.
    fn is_window_attached(&self) -> bool;

    /// Mutable access to the underlying Skia Ganesh [`DirectContext`].
    fn direct_context_mut(&mut self) -> &mut DirectContext;

    /// Returns the offscreen GPU [`Surface`] and [`DirectContext`] when offscreen, or `None` when
    /// attached to a native window.
    fn offscreen_surface_and_context_mut(&mut self) -> Option<(&mut Surface, &mut DirectContext)>;

    /// Resizes the native swapchain or offscreen GPU render target to `next`.
    ///
    /// Returns `Ok(true)` if underlying GPU swapchain/render-target resources were recreated
    /// (incrementing `swapchain_recreations`), or `Ok(false)` for either a scale-factor-only
    /// metadata update or a deferred recreation handled by [`Self::begin_native_frame`].
    fn resize_native(
        &mut self,
        prev: SwapchainExtent,
        next: SwapchainExtent,
    ) -> Result<bool, GpuSurfaceError>;

    /// Acquires the next drawable / swapchain image for rendering.
    ///
    /// Increments `recreations` immediately if an internal dirty swapchain is recreated so the
    /// counter is preserved even if a subsequent acquire step fails.
    fn begin_native_frame(
        &mut self,
        extent: SwapchainExtent,
        recreations: &mut u64,
    ) -> Result<(), GpuSurfaceError>;

    /// Returns the active [`Surface`] to record Skia draw commands into.
    fn active_surface_mut(&mut self) -> Result<&mut Surface, GpuSurfaceError>;

    /// Flushes and submits recorded GPU commands for the current frame.
    fn submit_native_frame(&mut self) -> Result<(), GpuSurfaceError>;

    /// Presents the submitted drawable/image on window targets (no-op for offscreen targets).
    fn present_native_frame(&mut self) -> Result<(), GpuSurfaceError>;

    /// Releases any per-frame acquired drawable/surface after an error or explicit abort.
    fn abort_native_frame(&mut self);
}

/// Allocates a budgeted GPU-resident offscreen Skia `Surface` for any Ganesh backend.
pub(crate) fn create_ganesh_offscreen_surface(
    direct_context: &mut DirectContext,
    extent: SwapchainExtent,
    backend_label: &'static str,
) -> Result<Surface, GpuSurfaceError> {
    let info = ImageInfo::new_n32_premul((extent.width as i32, extent.height as i32), None);
    surfaces::render_target(
        direct_context,
        gpu::Budgeted::Yes,
        &info,
        None,
        SurfaceOrigin::TopLeft,
        None,
        None,
        None,
    )
    .ok_or_else(|| {
        GpuSurfaceError::SurfaceCreationFailed(format!(
            "{backend_label} GPU render_target returned None for {}x{}",
            extent.width, extent.height
        ))
    })
}

/// Shared Skia Ganesh frame presenter parameterized over a platform [`NativeSwapchain`].
pub struct GaneshPresenter<S: NativeSwapchain> {
    pub(super) inner: S,
    pub(super) renderer: SkiaCanvasRenderer,
    extent: SwapchainExtent,
    state: FrameLifecycleState,
    frame_index: u64,
    swapchain_recreations: u64,
    frame_start: Option<Instant>,
    acquire_us: u64,
    record_us: u64,
    submit_us: u64,
    last_telemetry: Option<FrameTelemetry>,
    readback_rgba: Vec<u8>,
}

impl<S: NativeSwapchain> GaneshPresenter<S> {
    /// Wraps a constructed platform [`NativeSwapchain`] with lifecycle state & telemetry tracking.
    pub fn new(inner: S, extent: SwapchainExtent) -> Self {
        Self {
            inner,
            renderer: SkiaCanvasRenderer::new(),
            extent,
            state: FrameLifecycleState::Idle,
            frame_index: 0,
            swapchain_recreations: 0,
            frame_start: None,
            acquire_us: 0,
            record_us: 0,
            submit_us: 0,
            last_telemetry: None,
            readback_rgba: Vec::new(),
        }
    }

    pub fn is_window_attached(&self) -> bool {
        self.inner.is_window_attached()
    }

    pub fn direct_context_mut(&mut self) -> &mut DirectContext {
        self.inner.direct_context_mut()
    }

    /// Direct helper for [`crate::gpu::GpuSurface`] (`GpuSurfaceBackend::render_display_list`) on
    /// an offscreen GPU render target, falling back to a full frame on window-attached swapchains.
    pub fn render_with_renderer(
        &mut self,
        renderer: &SkiaCanvasRenderer,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
    ) -> Result<(), GpuSurfaceError> {
        self.render_with_renderer_plugins(renderer, display_list, scale_factor, clear_color, None)
    }

    /// Plugin-aware variant of [`Self::render_with_renderer`].
    pub fn render_with_renderer_plugins(
        &mut self,
        renderer: &SkiaCanvasRenderer,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<(), GpuSurfaceError> {
        if let Some((surface, direct_context)) = self.inner.offscreen_surface_and_context_mut() {
            let canvas = surface.canvas();
            canvas.clear(to_skia_color(clear_color));
            renderer.paint_display_list_opt_plugins(display_list, canvas, scale_factor, plugins);
            direct_context
                .flush_and_submit_surface(surface, SyncCpu::Yes)
                .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
            Ok(())
        } else {
            self.execute_frame_with_plugins(display_list, scale_factor, clear_color, None, plugins)
                .map(|_| ())
        }
    }

    /// Reads back pixels from an offscreen GPU render target into `target`.
    pub fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer) -> Result<(), GpuSurfaceError> {
        if let Some((surface, direct_context)) = self.inner.offscreen_surface_and_context_mut() {
            direct_context
                .flush_and_submit_surface(surface, SyncCpu::Yes)
                .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
            read_surface_to_pixel_buffer(
                surface,
                self.extent.width,
                self.extent.height,
                &mut self.readback_rgba,
                target,
            )
        } else {
            Err(GpuSurfaceError::ReadbackFailed(
                S::WINDOW_READBACK_ERROR.to_string(),
            ))
        }
    }
}

impl<S: NativeSwapchain> GpuFramePresenter for GaneshPresenter<S> {
    fn kind(&self) -> GpuBackendKind {
        S::KIND
    }

    fn extent(&self) -> SwapchainExtent {
        self.extent
    }

    fn lifecycle_state(&self) -> FrameLifecycleState {
        self.state
    }

    fn swapchain_recreations(&self) -> u64 {
        self.swapchain_recreations
    }

    fn last_telemetry(&self) -> Option<&FrameTelemetry> {
        self.last_telemetry.as_ref()
    }

    fn fallback_reason(&self) -> Option<&str> {
        None
    }

    fn resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Idle {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Idle.as_str(),
                actual: self.state.as_str(),
            });
        }
        let next = SwapchainExtent::new(width, height, scale_factor)?;
        if next.width == self.extent.width
            && next.height == self.extent.height
            && (next.scale_factor - self.extent.scale_factor).abs() <= f32::EPSILON
        {
            return Ok(());
        }

        let recreated = self.inner.resize_native(self.extent, next)?;
        self.extent = next;
        if recreated {
            self.swapchain_recreations += 1;
        }
        Ok(())
    }

    fn begin_frame(&mut self) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Idle {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Idle.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t0 = Instant::now();
        self.frame_start = Some(t0);
        self.record_us = 0;
        self.submit_us = 0;

        self.inner
            .begin_native_frame(self.extent, &mut self.swapchain_recreations)?;

        self.acquire_us = t0.elapsed().as_micros() as u64;
        self.state = FrameLifecycleState::Begun;
        Ok(())
    }

    fn render_display_list(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
    ) -> Result<(), GpuSurfaceError> {
        self.render_display_list_with_plugins(
            display_list,
            scale_factor,
            clear_color,
            hud_banner,
            None,
        )
    }

    fn render_display_list_with_plugins(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
        plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Begun {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Begun.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t0 = Instant::now();
        let surface = self.inner.active_surface_mut()?;
        let canvas = surface.canvas();
        canvas.clear(to_skia_color(clear_color));
        self.renderer
            .paint_display_list_opt_plugins(display_list, canvas, scale_factor, plugins);
        if let Some(banner) = hud_banner {
            self.renderer.paint_hud_banner(canvas, scale_factor, banner);
        }
        self.record_us = t0.elapsed().as_micros() as u64;
        self.state = FrameLifecycleState::Rendered;
        Ok(())
    }

    fn submit_frame(&mut self) -> Result<(), GpuSurfaceError> {
        if self.state != FrameLifecycleState::Rendered {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Rendered.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t0 = Instant::now();
        self.inner.submit_native_frame()?;
        self.submit_us = t0.elapsed().as_micros() as u64;
        self.state = FrameLifecycleState::Submitted;
        Ok(())
    }

    fn present_frame(&mut self) -> Result<FrameTelemetry, GpuSurfaceError> {
        if self.state != FrameLifecycleState::Submitted {
            return Err(GpuSurfaceError::InvalidFrameState {
                expected: FrameLifecycleState::Submitted.as_str(),
                actual: self.state.as_str(),
            });
        }
        let t_present = Instant::now();
        self.inner.present_native_frame()?;

        self.frame_index += 1;
        let present_us = t_present.elapsed().as_micros() as u64;
        let total_us = self.frame_start.take().map_or(
            self.acquire_us + self.record_us + self.submit_us + present_us,
            |t| t.elapsed().as_micros() as u64,
        );
        let telemetry = FrameTelemetry {
            backend: S::KIND,
            frame_index: self.frame_index,
            width: self.extent.width,
            height: self.extent.height,
            scale_factor: self.extent.scale_factor,
            acquire_us: self.acquire_us,
            record_us: self.record_us,
            submit_us: self.submit_us,
            present_us,
            total_us,
            swapchain_recreations: self.swapchain_recreations,
            fallback_reason: None,
        };
        self.last_telemetry = Some(telemetry.clone());
        self.state = FrameLifecycleState::Idle;
        Ok(telemetry)
    }

    fn abort_frame(&mut self) {
        self.inner.abort_native_frame();
        self.frame_start = None;
        self.state = FrameLifecycleState::Idle;
    }
}
