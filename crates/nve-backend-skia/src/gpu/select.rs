//! Unified GPU backend selection & fallback ladder plus zero-dynamic-allocation enum dispatch
//! for [`GpuSwapchain`] and [`GpuSurface`].
//!
//! Eliminates duplication across offscreen swapchain, window-attached swapchain, and Skia
//! `GpuSurface` initialization by running one shared candidate-selection ladder.

use nve_abi::ColorRgba;

#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
use super::d3d12;
#[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
use super::metal;
#[cfg(feature = "skia-vulkan")]
use super::vulkan;
#[cfg(feature = "skia")]
use super::{CpuSkiaSurface, GpuSurfaceBackend, SkiaCanvasRenderer};
use super::{
    FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig,
    GpuSurfaceError, SoftwareFramePresenter, SwapchainExtent,
};
use crate::raster::PixelBuffer;
use crate::text::CosmicTextEngine;
use crate::DisplayList;

/// Hardware backend selected by the unified candidate ladder.
#[allow(dead_code)]
enum SelectedBackend {
    #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
    Metal(Box<metal::MetalSwapchain>),
    #[cfg(feature = "skia-vulkan")]
    Vulkan(Box<vulkan::VulkanSwapchain>),
    #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
    Direct3D12(Box<d3d12::D3D12Swapchain>),
    #[cfg(feature = "skia")]
    CpuSkia(Box<CpuSkiaSurface>),
    Software(Box<SoftwareFramePresenter>),
}

enum SelectionMode<'a, W: ?Sized> {
    OffscreenSwapchain,
    WindowSwapchain(&'a W),
    #[cfg(feature = "skia")]
    SkiaSurface,
}

fn format_fallback_reason(
    fallback_reasons: &[String],
    default_msg: Option<String>,
) -> Option<String> {
    if fallback_reasons.is_empty() {
        default_msg
    } else {
        Some(fallback_reasons.join("; "))
    }
}

/// Executes the single canonical GPU backend priority & fallback ladder for offscreen swapchains,
/// native window-attached swapchains, and Skia `GpuSurface` targets.
fn select_backend<W>(
    config: GpuSurfaceConfig,
    mode: SelectionMode<'_, W>,
) -> Result<SelectedBackend, GpuSurfaceError>
where
    W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + ?Sized,
{
    let extent = config.extent()?;
    let candidates =
        GpuBackendKind::selection_priority(config.preferred_backend, config.allow_cpu_fallback);
    let mut fallback_reasons: Vec<String> = Vec::new();
    let is_window = matches!(mode, SelectionMode::WindowSwapchain(_));

    for candidate in candidates {
        match candidate {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            GpuBackendKind::Metal => {
                let res = match mode {
                    SelectionMode::WindowSwapchain(window) => {
                        metal::MetalSwapchain::from_window(window, config)
                    }
                    _ => metal::MetalSwapchain::new_offscreen(config),
                };
                match res {
                    Ok(sw) => return Ok(SelectedBackend::Metal(Box::new(sw))),
                    Err(err) => {
                        if !config.allow_cpu_fallback {
                            return Err(err);
                        }
                        fallback_reasons.push(format!("metal: {err}"));
                    }
                }
            }
            #[cfg(feature = "skia-vulkan")]
            GpuBackendKind::Vulkan => {
                let res = match mode {
                    SelectionMode::WindowSwapchain(window) => {
                        vulkan::VulkanSwapchain::from_window(window, config)
                    }
                    _ => vulkan::VulkanSwapchain::new_offscreen(config),
                };
                match res {
                    Ok(sw) => return Ok(SelectedBackend::Vulkan(Box::new(sw))),
                    Err(err) => {
                        if !config.allow_cpu_fallback {
                            return Err(err);
                        }
                        fallback_reasons.push(format!("vulkan: {err}"));
                    }
                }
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            GpuBackendKind::Direct3D12 => {
                let res = match mode {
                    SelectionMode::WindowSwapchain(window) => {
                        d3d12::D3D12Swapchain::from_window(window, config)
                    }
                    _ => d3d12::D3D12Swapchain::new_offscreen(config),
                };
                match res {
                    Ok(sw) => return Ok(SelectedBackend::Direct3D12(Box::new(sw))),
                    Err(err) => {
                        if !config.allow_cpu_fallback {
                            return Err(err);
                        }
                        fallback_reasons.push(format!("d3d12: {err}"));
                    }
                }
            }
            #[cfg(feature = "skia")]
            GpuBackendKind::CpuRaster => {
                let default_msg = if is_window && config.preferred_backend.is_hardware() {
                    Some(format!(
                        "{} backend not compiled; using Skia CPU raster",
                        config.preferred_backend
                    ))
                } else {
                    None
                };
                let mut cpu = CpuSkiaSurface::new_with_scale(
                    extent.width,
                    extent.height,
                    extent.scale_factor,
                )?;
                cpu.set_fallback_reason(format_fallback_reason(&fallback_reasons, default_msg));
                return Ok(SelectedBackend::CpuSkia(Box::new(cpu)));
            }
            GpuBackendKind::Softbuffer => {
                #[cfg(feature = "skia")]
                if matches!(mode, SelectionMode::SkiaSurface) {
                    if config.allow_cpu_fallback {
                        let mut cpu = CpuSkiaSurface::new_with_scale(
                            extent.width,
                            extent.height,
                            extent.scale_factor,
                        )?;
                        cpu.set_fallback_reason(format_fallback_reason(&fallback_reasons, None));
                        return Ok(SelectedBackend::CpuSkia(Box::new(cpu)));
                    }
                    continue;
                }
                let default_msg = if config.preferred_backend != GpuBackendKind::Softbuffer {
                    let target_label = if is_window {
                        "software softbuffer"
                    } else {
                        "software rasterizer"
                    };
                    Some(format!(
                        "{} backend not compiled; using {target_label}",
                        config.preferred_backend
                    ))
                } else {
                    None
                };
                let reason = format_fallback_reason(&fallback_reasons, default_msg);
                return Ok(SelectedBackend::Software(Box::new(
                    SoftwareFramePresenter::new(
                        extent.width,
                        extent.height,
                        extent.scale_factor,
                        CosmicTextEngine::shared(),
                        reason,
                    )?,
                )));
            }
            other => {
                if !config.allow_cpu_fallback {
                    return Err(GpuSurfaceError::UnsupportedBackend(other));
                }
                fallback_reasons.push(format!("{other}: not compiled for this target"));
            }
        }
    }

    Err(GpuSurfaceError::UnsupportedBackend(
        config.preferred_backend,
    ))
}

/// Zero-sized placeholder window type used when calling [`select_backend`] in offscreen modes.
struct NoWindow;

impl raw_window_handle::HasWindowHandle for NoWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::Unavailable)
    }
}

impl raw_window_handle::HasDisplayHandle for NoWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::Unavailable)
    }
}

/// Zero-dynamic-allocation enum dispatch across all compiled hardware GPU swapchains,
/// CPU Skia raster surfaces, and the zero-C++ software presenter.
pub enum GpuSwapchain {
    #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
    Metal(Box<metal::MetalSwapchain>),
    #[cfg(feature = "skia-vulkan")]
    Vulkan(Box<vulkan::VulkanSwapchain>),
    #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
    Direct3D12(Box<d3d12::D3D12Swapchain>),
    #[cfg(feature = "skia")]
    CpuSkia(Box<CpuSkiaSurface>),
    Software(Box<SoftwareFramePresenter>),
}

impl From<SelectedBackend> for GpuSwapchain {
    fn from(selected: SelectedBackend) -> Self {
        match selected {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            SelectedBackend::Metal(sw) => Self::Metal(sw),
            #[cfg(feature = "skia-vulkan")]
            SelectedBackend::Vulkan(sw) => Self::Vulkan(sw),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            SelectedBackend::Direct3D12(sw) => Self::Direct3D12(sw),
            #[cfg(feature = "skia")]
            SelectedBackend::CpuSkia(cpu) => Self::CpuSkia(cpu),
            SelectedBackend::Software(sw) => Self::Software(sw),
        }
    }
}

impl GpuSwapchain {
    /// Creates an offscreen/headless swapchain according to `config`, honoring
    /// `config.preferred_backend` and `config.allow_cpu_fallback`.
    pub fn new_offscreen(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
        select_backend::<NoWindow>(config, SelectionMode::OffscreenSwapchain).map(Self::from)
    }

    /// Attaches a hardware GPU swapchain directly to a native `raw-window-handle` window,
    /// automatically falling back to `CpuSkia` / `Software` when hardware GPU initialization is
    /// unavailable and `config.allow_cpu_fallback` is `true`.
    pub fn attach_window<W>(
        window: &W,
        config: GpuSurfaceConfig,
    ) -> Result<Self, GpuSurfaceError>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        select_backend(config, SelectionMode::WindowSwapchain(window)).map(Self::from)
    }

    /// Returns `true` when this swapchain presents directly to a native GPU surface (no
    /// CPU `softbuffer` blit required).
    pub fn presents_directly(&self) -> bool {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => sw.is_window_attached(),
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => sw.is_window_attached(),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => sw.is_window_attached(),
            #[cfg(feature = "skia")]
            Self::CpuSkia(_) => false,
            Self::Software(_) => false,
        }
    }

    /// Reads back the last rendered frame into `target` (supported for CPU raster, software,
    /// and offscreen GPU surfaces).
    pub fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer) -> Result<(), GpuSurfaceError> {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => sw.copy_to_pixel_buffer(target),
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => sw.copy_to_pixel_buffer(target),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => sw.copy_to_pixel_buffer(target),
            #[cfg(feature = "skia")]
            Self::CpuSkia(cpu) => cpu.copy_to_pixel_buffer(target),
            Self::Software(sw) => {
                let src = sw.pixel_buffer();
                target.resize(src.width(), src.height());
                target.pixels_mut().copy_from_slice(src.pixels());
                Ok(())
            }
        }
    }
}

macro_rules! dispatch_swapchain_ref {
    ($self:expr, $var:ident => $body:expr) => {
        match $self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal($var) => {
                let $var: &metal::MetalSwapchain = $var;
                $body
            }
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan($var) => {
                let $var: &vulkan::VulkanSwapchain = $var;
                $body
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12($var) => {
                let $var: &d3d12::D3D12Swapchain = $var;
                $body
            }
            #[cfg(feature = "skia")]
            Self::CpuSkia($var) => {
                let $var: &CpuSkiaSurface = $var;
                $body
            }
            Self::Software($var) => {
                let $var: &SoftwareFramePresenter = $var;
                $body
            }
        }
    };
}

macro_rules! dispatch_swapchain_mut {
    ($self:expr, $var:ident => $body:expr) => {
        match $self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal($var) => {
                let $var: &mut metal::MetalSwapchain = $var;
                $body
            }
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan($var) => {
                let $var: &mut vulkan::VulkanSwapchain = $var;
                $body
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12($var) => {
                let $var: &mut d3d12::D3D12Swapchain = $var;
                $body
            }
            #[cfg(feature = "skia")]
            Self::CpuSkia($var) => {
                let $var: &mut CpuSkiaSurface = $var;
                $body
            }
            Self::Software($var) => {
                let $var: &mut SoftwareFramePresenter = $var;
                $body
            }
        }
    };
}

impl GpuFramePresenter for GpuSwapchain {
    fn kind(&self) -> GpuBackendKind {
        dispatch_swapchain_ref!(self, inner => GpuFramePresenter::kind(inner))
    }

    fn extent(&self) -> SwapchainExtent {
        dispatch_swapchain_ref!(self, inner => GpuFramePresenter::extent(inner))
    }

    fn lifecycle_state(&self) -> FrameLifecycleState {
        dispatch_swapchain_ref!(self, inner => GpuFramePresenter::lifecycle_state(inner))
    }

    fn swapchain_recreations(&self) -> u64 {
        dispatch_swapchain_ref!(self, inner => GpuFramePresenter::swapchain_recreations(inner))
    }

    fn last_telemetry(&self) -> Option<&FrameTelemetry> {
        dispatch_swapchain_ref!(self, inner => GpuFramePresenter::last_telemetry(inner))
    }

    fn fallback_reason(&self) -> Option<&str> {
        dispatch_swapchain_ref!(self, inner => GpuFramePresenter::fallback_reason(inner))
    }

    fn resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<(), GpuSurfaceError> {
        dispatch_swapchain_mut!(self, inner => GpuFramePresenter::resize(inner, width, height, scale_factor))
    }

    fn begin_frame(&mut self) -> Result<(), GpuSurfaceError> {
        dispatch_swapchain_mut!(self, inner => GpuFramePresenter::begin_frame(inner))
    }

    fn render_display_list(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        hud_banner: Option<&str>,
    ) -> Result<(), GpuSurfaceError> {
        dispatch_swapchain_mut!(self, inner => GpuFramePresenter::render_display_list(inner, display_list, scale_factor, clear_color, hud_banner))
    }

    fn submit_frame(&mut self) -> Result<(), GpuSurfaceError> {
        dispatch_swapchain_mut!(self, inner => GpuFramePresenter::submit_frame(inner))
    }

    fn present_frame(&mut self) -> Result<FrameTelemetry, GpuSurfaceError> {
        dispatch_swapchain_mut!(self, inner => GpuFramePresenter::present_frame(inner))
    }

    fn abort_frame(&mut self) {
        dispatch_swapchain_mut!(self, inner => GpuFramePresenter::abort_frame(inner))
    }
}

/// Concrete enum dispatch over Skia rendering surfaces (`--features skia`), backed by real
/// offscreen hardware GPU render targets when `skia-metal` / `skia-vulkan` / `skia-d3d` are
/// active, with automatic fallback to [`CpuSkiaSurface`].
#[cfg(feature = "skia")]
pub enum GpuSurface {
    #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
    Metal(Box<metal::MetalSwapchain>),
    #[cfg(feature = "skia-vulkan")]
    Vulkan(Box<vulkan::VulkanSwapchain>),
    #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
    Direct3D12(Box<d3d12::D3D12Swapchain>),
    CpuRaster(Box<CpuSkiaSurface>),
}

#[cfg(feature = "skia")]
impl GpuSurface {
    /// Creates a Skia surface according to `config`, initializing a real hardware GPU context
    /// when compiled and available, and falling back to [`CpuSkiaSurface`] when
    /// `config.allow_cpu_fallback` is enabled.
    pub fn new(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
        match select_backend::<NoWindow>(config, SelectionMode::SkiaSurface)? {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            SelectedBackend::Metal(sw) => Ok(Self::Metal(sw)),
            #[cfg(feature = "skia-vulkan")]
            SelectedBackend::Vulkan(sw) => Ok(Self::Vulkan(sw)),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            SelectedBackend::Direct3D12(sw) => Ok(Self::Direct3D12(sw)),
            SelectedBackend::CpuSkia(cpu) => Ok(Self::CpuRaster(cpu)),
            SelectedBackend::Software(_) => {
                Err(GpuSurfaceError::UnsupportedBackend(config.preferred_backend))
            }
        }
    }

    /// Creates a CPU-backed Skia raster surface directly (`skia_safe::surfaces::raster_n32_premul`).
    pub fn new_cpu(width: u32, height: u32) -> Result<Self, GpuSurfaceError> {
        Ok(Self::CpuRaster(Box::new(CpuSkiaSurface::new(
            width, height,
        )?)))
    }

    pub fn fallback_reason(&self) -> Option<&str> {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => sw.fallback_reason(),
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => sw.fallback_reason(),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => sw.fallback_reason(),
            Self::CpuRaster(cpu) => cpu.fallback_reason(),
        }
    }
}

#[cfg(feature = "skia")]
impl GpuSurfaceBackend for GpuSurface {
    fn kind(&self) -> GpuBackendKind {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(_) => GpuBackendKind::Metal,
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(_) => GpuBackendKind::Vulkan,
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(_) => GpuBackendKind::Direct3D12,
            Self::CpuRaster(_) => GpuBackendKind::CpuRaster,
        }
    }

    fn size(&self) -> (u32, u32) {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => {
                let e = sw.extent();
                (e.width, e.height)
            }
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => {
                let e = sw.extent();
                (e.width, e.height)
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => {
                let e = sw.extent();
                (e.width, e.height)
            }
            Self::CpuRaster(cpu) => cpu.size(),
        }
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError> {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => {
                let scale = sw.extent().scale_factor;
                sw.resize(width, height, scale)
            }
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => {
                let scale = sw.extent().scale_factor;
                sw.resize(width, height, scale)
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => {
                let scale = sw.extent().scale_factor;
                sw.resize(width, height, scale)
            }
            Self::CpuRaster(cpu) => GpuSurfaceBackend::resize(cpu.as_mut(), width, height),
        }
    }

    fn render_display_list(
        &mut self,
        renderer: &SkiaCanvasRenderer,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
    ) -> Result<(), GpuSurfaceError> {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => {
                sw.render_with_renderer(renderer, display_list, scale_factor, clear_color)
            }
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => {
                sw.render_with_renderer(renderer, display_list, scale_factor, clear_color)
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => {
                sw.render_with_renderer(renderer, display_list, scale_factor, clear_color)
            }
            Self::CpuRaster(cpu) => GpuSurfaceBackend::render_display_list(
                cpu.as_mut(),
                renderer,
                display_list,
                scale_factor,
                clear_color,
            ),
        }
    }

    fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer) -> Result<(), GpuSurfaceError> {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => sw.copy_to_pixel_buffer(target),
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => sw.copy_to_pixel_buffer(target),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => sw.copy_to_pixel_buffer(target),
            Self::CpuRaster(cpu) => cpu.copy_to_pixel_buffer(target),
        }
    }
}
