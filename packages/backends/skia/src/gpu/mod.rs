//! Hardware GPU & CPU Skia swapchain/surface architecture.
//!
//! Organizes platform GPU swapchain backends cleanly behind opt-in Cargo features while keeping
//! the default build 100% zero-C++:
//!
//! - `skia-metal` (`macos` / `ios`): `MTLDevice` + `MTLCommandQueue` + `CAMetalLayer` +
//!   `skia_safe::gpu::direct_contexts::make_metal` + `backend_render_targets::make_mtl` /
//!   `wrap_mtk_view`.
//! - `skia-vulkan`: `ash` Vulkan instance/device/swapchain +
//!   `skia_safe::gpu::direct_contexts::make_vulkan` + `backend_render_targets::make_vk` +
//!   `BackendSemaphore` acquire/signal synchronization hooks.
//! - `skia-d3d` (`windows`): `IDXGIFactory4` + `ID3D12Device` + `IDXGISwapChain3` +
//!   `skia_safe::gpu::direct_contexts::make_d3d` + `backend_render_targets::make_d3d`.
//! - `skia`: `CpuSkiaSurface` (`skia_safe::surfaces::raster_n32_premul`) + `SkiaCanvasRenderer`.
//! - default: Zero-C++ `SoftwareFramePresenter` (backed by `PixelBuffer` +
//!   `CosmicTextEngine`, blitted to `softbuffer` by `hosts/skia-shell`).

#[cfg(feature = "skia")]
pub mod cpu_surface;
pub mod d3d12;
pub mod metal;
pub mod select;
#[cfg(feature = "skia")]
mod skia_graphics;
#[cfg(feature = "skia")]
pub mod skia_renderer;
pub mod vulkan;

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::ColorRgba;

use crate::raster::{
    draw_hud_banner, rasterize_display_list_with_text_engine,
    rasterize_display_list_with_text_engine_and_plugins, PixelBuffer,
};
use crate::table_system::SkiaTableRegistry;
use crate::text::CosmicTextEngine;
use crate::DisplayList;
use nve_store::{NodeStore, TableEventBus};

/// Borrowed plugin context (`NodeStore` + `SkiaTableRegistry` + `TableEventBus`) threaded into
/// Skia GPU/CPU/software frame presenters so `PaintOp::Plugin` (`Svg`, `Canvas`, 3P) renders.
pub type SkiaPluginContext<'a> = (&'a NodeStore, &'a SkiaTableRegistry, &'a mut TableEventBus);

pub use select::GpuSwapchain;
#[cfg(feature = "skia")]
pub use select::GpuSurface;
#[cfg(feature = "skia")]
pub use skia_renderer::{CpuSkiaSurface, GpuSurfaceBackend, SkiaCanvasRenderer};

/// Identifies the active rendering / presentation backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuBackendKind {
    /// Apple Metal GPU backend (macOS / iOS).
    Metal,
    /// Direct3D 12 GPU backend (Windows).
    Direct3D12,
    /// Vulkan GPU backend (Linux / Android / Windows / MoltenVK).
    Vulkan,
    /// CPU-backed Skia raster surface (`skia_safe::surfaces::raster_n32_premul`).
    CpuRaster,
    /// Pure-Rust zero-C++ software rasterizer / `softbuffer` presenter.
    Softbuffer,
}

impl GpuBackendKind {
    /// Returns the preferred hardware backend for the current compilation target.
    pub const fn platform_preferred() -> Self {
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            Self::Metal
        } else if cfg!(target_os = "windows") {
            Self::Direct3D12
        } else if cfg!(any(target_os = "linux", target_os = "android")) {
            Self::Vulkan
        } else {
            Self::CpuRaster
        }
    }

    /// Returns `true` if this backend executes draw calls on a hardware GPU API.
    pub const fn is_hardware(self) -> bool {
        matches!(self, Self::Metal | Self::Direct3D12 | Self::Vulkan)
    }

    /// Returns `true` if this backend's Cargo feature and target OS are compiled into this binary.
    pub const fn is_compiled(self) -> bool {
        match self {
            Self::Metal => cfg!(all(
                feature = "skia-metal",
                any(target_os = "macos", target_os = "ios")
            )),
            Self::Direct3D12 => cfg!(all(feature = "skia-d3d", target_os = "windows")),
            Self::Vulkan => cfg!(feature = "skia-vulkan"),
            Self::CpuRaster => cfg!(feature = "skia"),
            Self::Softbuffer => true,
        }
    }

    /// Computes the ordered backend selection priority for a requested backend and fallback policy.
    pub fn selection_priority(preferred: Self, allow_cpu_fallback: bool) -> Vec<Self> {
        let mut order = Vec::with_capacity(5);
        let push_unique = |list: &mut Vec<Self>, candidate: Self| {
            if !list.contains(&candidate) {
                list.push(candidate);
            }
        };

        if preferred.is_compiled() || !allow_cpu_fallback {
            push_unique(&mut order, preferred);
        }

        if allow_cpu_fallback {
            if preferred.is_hardware() {
                let plat = Self::platform_preferred();
                if plat.is_hardware() && plat.is_compiled() {
                    push_unique(&mut order, plat);
                }
                for hw in [Self::Metal, Self::Direct3D12, Self::Vulkan] {
                    if hw.is_compiled() {
                        push_unique(&mut order, hw);
                    }
                }
            }
            if preferred != Self::Softbuffer && Self::CpuRaster.is_compiled() {
                push_unique(&mut order, Self::CpuRaster);
            }
            push_unique(&mut order, Self::Softbuffer);
        }

        order
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metal => "metal",
            Self::Direct3D12 => "d3d12",
            Self::Vulkan => "vulkan",
            Self::CpuRaster => "cpu-raster",
            Self::Softbuffer => "softbuffer",
        }
    }
}

impl fmt::Display for GpuBackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validated physical pixel extent and HiDPI backing scale factor for a swapchain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwapchainExtent {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

impl SwapchainExtent {
    /// Validates physical dimensions (`width > 0 && height > 0`) and sanitizes `scale_factor`.
    pub fn new(width: u32, height: u32, scale_factor: f32) -> Result<Self, GpuSurfaceError> {
        if width == 0 || height == 0 {
            return Err(GpuSurfaceError::InvalidDimensions { width, height });
        }
        Ok(Self {
            width,
            height,
            scale_factor: sanitize_scale_factor(scale_factor),
        })
    }

    /// Logical size in device-independent points.
    pub fn logical_size(self) -> (f32, f32) {
        (
            self.width as f32 / self.scale_factor,
            self.height as f32 / self.scale_factor,
        )
    }
}

#[inline]
pub fn sanitize_scale_factor(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

/// Configuration for creating a Skia GPU swapchain or rendering surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuSurfaceConfig {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub preferred_backend: GpuBackendKind,
    /// When `true`, automatically falls back to `CpuRaster` / `Softbuffer` if a hardware GPU
    /// device or swapchain cannot be initialized on the current display/headless environment.
    pub allow_cpu_fallback: bool,
    /// Enables VSync / display-sync presentation when supported by the swapchain.
    pub vsync: bool,
    /// Requested swapchain backbuffer count (clamped to at least 2 by hardware swapchains).
    pub swapchain_image_count: u32,
}

impl Default for GpuSurfaceConfig {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            preferred_backend: GpuBackendKind::platform_preferred(),
            allow_cpu_fallback: true,
            vsync: true,
            swapchain_image_count: 2,
        }
    }
}

impl GpuSurfaceConfig {
    pub fn extent(&self) -> Result<SwapchainExtent, GpuSurfaceError> {
        SwapchainExtent::new(self.width, self.height, self.scale_factor)
    }
}

/// Per-frame timing and diagnostic telemetry emitted by [`GpuFramePresenter`] / [`GpuSwapchain`].
#[derive(Debug, Clone, PartialEq)]
pub struct FrameTelemetry {
    pub backend: GpuBackendKind,
    pub frame_index: u64,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub acquire_us: u64,
    pub record_us: u64,
    pub submit_us: u64,
    pub present_us: u64,
    pub total_us: u64,
    pub swapchain_recreations: u64,
    pub fallback_reason: Option<String>,
}

/// Explicit state machine enforcing the `begin_frame -> render -> submit -> present` frame contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameLifecycleState {
    Idle,
    Begun,
    Rendered,
    Submitted,
}

impl FrameLifecycleState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Begun => "begun",
            Self::Rendered => "rendered",
            Self::Submitted => "submitted",
        }
    }
}

/// Errors returned while creating, resizing, rendering, submitting, or presenting a surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuSurfaceError {
    InvalidDimensions { width: u32, height: u32 },
    UnsupportedBackend(GpuBackendKind),
    DeviceUnavailable(String),
    SurfaceCreationFailed(String),
    SwapchainOutOfDate(String),
    DrawableUnavailable,
    FlushFailed(String),
    PresentFailed(String),
    ReadbackFailed(String),
    WindowHandleError(String),
    InvalidFrameState {
        expected: &'static str,
        actual: &'static str,
    },
}

impl fmt::Display for GpuSurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions { width, height } => {
                write!(f, "invalid Skia surface dimensions: {width}x{height}")
            }
            Self::UnsupportedBackend(kind) => {
                write!(f, "Skia GPU backend `{kind}` is not available on this host")
            }
            Self::DeviceUnavailable(msg) => {
                write!(f, "hardware GPU device unavailable: {msg}")
            }
            Self::SurfaceCreationFailed(msg) => {
                write!(f, "failed to allocate Skia surface: {msg}")
            }
            Self::SwapchainOutOfDate(msg) => {
                write!(f, "GPU swapchain out of date: {msg}")
            }
            Self::DrawableUnavailable => {
                write!(f, "GPU swapchain returned no available drawable for frame")
            }
            Self::FlushFailed(msg) => {
                write!(f, "GPU context flush/submit failed: {msg}")
            }
            Self::PresentFailed(msg) => {
                write!(f, "GPU swapchain present failed: {msg}")
            }
            Self::ReadbackFailed(msg) => {
                write!(f, "failed to read back Skia surface pixels: {msg}")
            }
            Self::WindowHandleError(msg) => {
                write!(f, "failed to extract native window handle: {msg}")
            }
            Self::InvalidFrameState { expected, actual } => {
                write!(
                    f,
                    "invalid GPU frame lifecycle transition: expected `{expected}`, got `{actual}`"
                )
            }
        }
    }
}

impl std::error::Error for GpuSurfaceError {}

/// Uniform frame presentation trait implemented by every hardware GPU swapchain
/// (Metal, Vulkan, Direct3D 12), `CpuSkiaSurface`, and zero-C++ `SoftwareFramePresenter`.
pub trait GpuFramePresenter {
    fn kind(&self) -> GpuBackendKind;
    fn extent(&self) -> SwapchainExtent;
    fn lifecycle_state(&self) -> FrameLifecycleState;
    fn swapchain_recreations(&self) -> u64;
    fn last_telemetry(&self) -> Option<&FrameTelemetry>;
    fn fallback_reason(&self) -> Option<&str>;

    fn resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<(), GpuSurfaceError>;

    fn begin_frame(&mut self) -> Result<(), GpuSurfaceError>;

    fn render_display_list(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
    ) -> Result<(), GpuSurfaceError>;

    fn render_display_list_with_plugins(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
        _plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<(), GpuSurfaceError> {
        self.render_display_list(display_list, scale_factor, clear_color, hud_banner)
    }

    fn submit_frame(&mut self) -> Result<(), GpuSurfaceError>;

    fn present_frame(&mut self) -> Result<FrameTelemetry, GpuSurfaceError>;

    fn abort_frame(&mut self);

    fn execute_frame(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
    ) -> Result<FrameTelemetry, GpuSurfaceError> {
        self.execute_frame_with_plugins(display_list, scale_factor, clear_color, hud_banner, None)
    }

    fn execute_frame_with_plugins(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
        plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<FrameTelemetry, GpuSurfaceError> {
        self.begin_frame()?;
        if let Err(err) = self.render_display_list_with_plugins(
            display_list,
            scale_factor,
            clear_color,
            hud_banner,
            plugins,
        ) {
            self.abort_frame();
            return Err(err);
        }
        if let Err(err) = self.submit_frame() {
            self.abort_frame();
            return Err(err);
        }
        match self.present_frame() {
            Ok(telemetry) => Ok(telemetry),
            Err(err) => {
                self.abort_frame();
                Err(err)
            }
        }
    }
}

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
        self.render_display_list_with_plugins(display_list, scale_factor, clear_color, hud_banner, None)
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

#[cfg(test)]
mod tests;
