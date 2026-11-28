//! Zero-C++ pure-Rust software frame presenter (`PixelBuffer` + `CosmicTextEngine`).

use std::sync::Arc;
use std::time::Instant;

use nve_abi::ColorRgba;

use super::{
    sanitize_scale_factor, FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter,
    GpuSurfaceError, SkiaPluginContext, SwapchainExtent,
};
use crate::raster::{
    draw_hud_banner, rasterize_display_list_with_text_engine,
    rasterize_display_list_with_text_engine_and_plugins, PixelBuffer,
};
use crate::retained::CompositorScene;
use crate::text::CosmicTextEngine;
use crate::DisplayList;

/// Zero-C++ pure-Rust software frame presenter (`PixelBuffer` + `CosmicTextEngine`).
pub struct SoftwareFramePresenter {
    extent: SwapchainExtent,
    pixel_buffer: PixelBuffer,
    text_engine: Arc<CosmicTextEngine>,
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

impl SoftwareFramePresenter {
    pub fn new(
        width: u32,
        height: u32,
        scale_factor: f32,
        text_engine: Arc<CosmicTextEngine>,
        fallback_reason: Option<String>,
    ) -> Result<Self, GpuSurfaceError> {
        let extent = SwapchainExtent::new(width, height, scale_factor)?;
        Ok(Self {
            extent,
            pixel_buffer: PixelBuffer::new(extent.width, extent.height),
            text_engine,
            state: FrameLifecycleState::Idle,
            frame_index: 0,
            swapchain_recreations: 0,
            frame_start: None,
            acquire_us: 0,
            record_us: 0,
            submit_us: 0,
            last_telemetry: None,
            fallback_reason,
        })
    }

    pub fn pixel_buffer(&self) -> &PixelBuffer {
        &self.pixel_buffer
    }

    pub fn pixel_buffer_mut(&mut self) -> &mut PixelBuffer {
        &mut self.pixel_buffer
    }
}

impl GpuFramePresenter for SoftwareFramePresenter {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::Softbuffer
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
        if next.width != self.extent.width
            || next.height != self.extent.height
            || (next.scale_factor - self.extent.scale_factor).abs() > f32::EPSILON
        {
            self.pixel_buffer.resize(next.width, next.height);
            self.extent = next;
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
        let scale = sanitize_scale_factor(scale_factor);
        let t0 = Instant::now();
        if let Some(p) = plugins {
            rasterize_display_list_with_text_engine_and_plugins(
                display_list,
                &mut self.pixel_buffer,
                scale,
                clear_color,
                self.text_engine.as_ref(),
                Some(p),
            );
        } else {
            rasterize_display_list_with_text_engine(
                display_list,
                &mut self.pixel_buffer,
                scale,
                clear_color,
                self.text_engine.as_ref(),
            );
        }
        if let Some(banner) = hud_banner {
            draw_hud_banner(&mut self.pixel_buffer, scale, banner);
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
        let scale = sanitize_scale_factor(scale_factor);
        let t0 = Instant::now();
        if let Some((store, registry, events)) = plugins {
            scene.rasterize_with_plugins(
                &mut self.pixel_buffer,
                scale,
                clear_color,
                self.text_engine.as_ref(),
                store,
                registry,
                events,
            );
        } else {
            scene.rasterize_with_text_engine(
                &mut self.pixel_buffer,
                scale,
                clear_color,
                self.text_engine.as_ref(),
            );
        }
        if let Some(banner) = hud_banner {
            draw_hud_banner(&mut self.pixel_buffer, scale, banner);
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
            backend: GpuBackendKind::Softbuffer,
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
