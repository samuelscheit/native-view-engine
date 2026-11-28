//! CPU-backed Skia raster surface (`CpuSkiaSurface`), [`GpuSurfaceBackend`] trait, and
//! zero-copy-readback pixel extraction (`--features skia`).

use std::time::Instant;

use nve_abi::ColorRgba;
use skia_safe::{AlphaType, Canvas, ColorType, ImageInfo, Surface};

use super::skia_renderer::{to_skia_color, RendererCacheStats, SkiaCanvasRenderer};
use super::{
    FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceError,
    SkiaPluginContext, SwapchainExtent,
};
use crate::raster::PixelBuffer;
use crate::retained::CompositorScene;
use crate::DisplayList;

/// Reads back a Skia [`Surface`] into a [`PixelBuffer`] reusing `scratch_rgba` to avoid
/// per-frame heap allocations.
pub(crate) fn read_surface_to_pixel_buffer(
    surface: &mut Surface,
    width: u32,
    height: u32,
    scratch_rgba: &mut Vec<u8>,
    target: &mut PixelBuffer,
) -> Result<(), GpuSurfaceError> {
    let needed = (width as usize) * (height as usize) * 4;
    if scratch_rgba.len() != needed {
        scratch_rgba.resize(needed, 0);
    }
    target.resize(width, height);
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let row_bytes = (width as usize) * 4;
    let ok = surface.read_pixels(&info, scratch_rgba, row_bytes, (0, 0));
    if !ok {
        return Err(GpuSurfaceError::ReadbackFailed(
            "skia_safe::Surface::read_pixels returned false".to_string(),
        ));
    }
    let dst = target.pixels_mut();
    let (rgba_chunks, _) = scratch_rgba.as_chunks::<4>();
    for (px, rgba) in dst.iter_mut().zip(rgba_chunks) {
        *px = PixelBuffer::pack_rgb(rgba[0], rgba[1], rgba[2]);
    }
    Ok(())
}

/// Unified Skia surface abstraction implemented by CPU raster surfaces and native
/// GPU surface adapters (Metal on macOS, Direct3D 12 on Windows, Vulkan on Linux).
pub trait GpuSurfaceBackend {
    fn kind(&self) -> GpuBackendKind;
    fn size(&self) -> (u32, u32);
    fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError>;
    fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer) -> Result<(), GpuSurfaceError>;
}

/// CPU-backed Skia raster surface (`skia_safe::surfaces::raster_n32_premul`).
pub struct CpuSkiaSurface {
    extent: SwapchainExtent,
    renderer: SkiaCanvasRenderer,
    surface: Surface,
    readback_rgba: Vec<u8>,
    state: FrameLifecycleState,
    frame_index: u64,
    swapchain_recreations: u64,
    frame_start: Option<Instant>,
    acquire_us: u64,
    record_us: u64,
    submit_us: u64,
    last_telemetry: Option<FrameTelemetry>,
    fallback_reason: Option<String>,
}

impl CpuSkiaSurface {
    pub fn new(width: u32, height: u32) -> Result<Self, GpuSurfaceError> {
        Self::new_with_scale(width, height, 1.0)
    }

    pub fn new_with_scale(
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<Self, GpuSurfaceError> {
        Self::with_text_engine(
            width,
            height,
            scale_factor,
            crate::text::CosmicTextEngine::shared(),
        )
    }

    pub fn with_text_engine(
        width: u32,
        height: u32,
        scale_factor: f32,
        text_engine: std::sync::Arc<crate::text::CosmicTextEngine>,
    ) -> Result<Self, GpuSurfaceError> {
        let extent = SwapchainExtent::new(width, height, scale_factor)?;
        let surface =
            skia_safe::surfaces::raster_n32_premul((extent.width as i32, extent.height as i32))
                .ok_or_else(|| {
                    GpuSurfaceError::SurfaceCreationFailed(format!(
                        "raster_n32_premul returned None for {}x{}",
                        extent.width, extent.height
                    ))
                })?;
        Ok(Self {
            extent,
            renderer: SkiaCanvasRenderer::with_text_engine(text_engine),
            surface,
            readback_rgba: vec![0u8; (extent.width as usize) * (extent.height as usize) * 4],
            state: FrameLifecycleState::Idle,
            frame_index: 0,
            swapchain_recreations: 0,
            frame_start: None,
            acquire_us: 0,
            record_us: 0,
            submit_us: 0,
            last_telemetry: None,
            fallback_reason: None,
        })
    }

    pub fn renderer(&self) -> &SkiaCanvasRenderer {
        &self.renderer
    }

    pub fn cache_stats(&self) -> RendererCacheStats {
        self.renderer.cache_stats()
    }

    pub fn reset_cache_stats(&self) {
        self.renderer.reset_cache_stats();
    }

    pub fn evict_gpu_caches(&self) {
        self.renderer.evict_gpu_caches();
    }

    pub fn render_cached_with_plugins(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<(), GpuSurfaceError> {
        let canvas = self.surface.canvas();
        canvas.clear(to_skia_color(clear_color));
        self.renderer
            .paint_display_list_opt_plugins(display_list, canvas, scale_factor, plugins);
        Ok(())
    }

    pub fn render_scene_cached_with_plugins(
        &mut self,
        scene: &CompositorScene,
        scale_factor: f32,
        clear_color: ColorRgba,
        plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<(), GpuSurfaceError> {
        let canvas = self.surface.canvas();
        canvas.clear(to_skia_color(clear_color));
        self.renderer
            .paint_scene_opt_plugins(scene, canvas, scale_factor, plugins);
        Ok(())
    }

    pub fn set_fallback_reason(&mut self, reason: Option<String>) {
        self.fallback_reason = reason;
    }

    pub fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    pub fn canvas(&mut self) -> &Canvas {
        self.surface.canvas()
    }
}

impl GpuSurfaceBackend for CpuSkiaSurface {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::CpuRaster
    }

    fn size(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError> {
        let scale = self.extent.scale_factor;
        <Self as GpuFramePresenter>::resize(self, width, height, scale)
    }

    fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer) -> Result<(), GpuSurfaceError> {
        read_surface_to_pixel_buffer(
            &mut self.surface,
            self.extent.width,
            self.extent.height,
            &mut self.readback_rgba,
            target,
        )
    }
}

impl GpuFramePresenter for CpuSkiaSurface {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::CpuRaster
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
        self.fallback_reason.as_deref()
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
        if next.width != self.extent.width || next.height != self.extent.height {
            let next_surface =
                skia_safe::surfaces::raster_n32_premul((next.width as i32, next.height as i32))
                    .ok_or_else(|| {
                        GpuSurfaceError::SurfaceCreationFailed(format!(
                            "raster_n32_premul returned None for {}x{}",
                            next.width, next.height
                        ))
                    })?;
            self.surface = next_surface;
            self.readback_rgba
                .resize((next.width as usize) * (next.height as usize) * 4, 0);
        }
        self.extent = next;
        self.swapchain_recreations += 1;
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
        self.acquire_us = t0.elapsed().as_micros() as u64;
        self.record_us = 0;
        self.submit_us = 0;
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
        GpuFramePresenter::render_display_list_with_plugins(
            self,
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
        let canvas = self.surface.canvas();
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

    fn render_scene_with_plugins(
        &mut self,
        scene: &CompositorScene,
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
        let canvas = self.surface.canvas();
        canvas.clear(to_skia_color(clear_color));
        self.renderer
            .paint_scene_opt_plugins(scene, canvas, scale_factor, plugins);
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
        self.frame_index += 1;
        let present_us = t_present.elapsed().as_micros() as u64;
        let total_us = self
            .frame_start
            .take()
            .map_or(self.acquire_us + self.record_us + self.submit_us + present_us, |t| {
                t.elapsed().as_micros() as u64
            });
        let telemetry = FrameTelemetry {
            backend: GpuBackendKind::CpuRaster,
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
            fallback_reason: self.fallback_reason.clone(),
        };
        self.last_telemetry = Some(telemetry.clone());
        self.state = FrameLifecycleState::Idle;
        Ok(telemetry)
    }

    fn abort_frame(&mut self) {
        self.frame_start = None;
        self.state = FrameLifecycleState::Idle;
    }
}
