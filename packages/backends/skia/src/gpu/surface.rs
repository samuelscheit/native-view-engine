//! Concrete enum dispatch over Skia rendering surfaces (`--features skia`), backed by real
//! offscreen hardware GPU render targets when `skia-metal` / `skia-vulkan` / `skia-d3d` are
//! active, with automatic fallback to [`CpuSkiaSurface`].

use nve_abi::ColorRgba;

#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
use super::d3d12;
#[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
use super::metal;
use super::select::{select_backend, NoWindow, SelectedBackend, SelectionMode};
#[cfg(feature = "skia-vulkan")]
use super::vulkan;
#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
use super::GpuFramePresenter;
use super::{
    CpuSkiaSurface, GpuBackendKind, GpuSurfaceBackend, GpuSurfaceConfig, GpuSurfaceError,
    RendererCacheStats, SkiaPluginContext,
};
use crate::raster::PixelBuffer;
use crate::retained::CompositorScene;
use crate::DisplayList;

/// Concrete enum dispatch over Skia rendering surfaces (`--features skia`), backed by real
/// offscreen hardware GPU render targets when `skia-metal` / `skia-vulkan` / `skia-d3d` are
/// active, with automatic fallback to [`CpuSkiaSurface`].
pub enum GpuSurface {
    #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
    Metal(Box<metal::MetalSwapchain>),
    #[cfg(feature = "skia-vulkan")]
    Vulkan(Box<vulkan::VulkanSwapchain>),
    #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
    Direct3D12(Box<d3d12::D3D12Swapchain>),
    CpuRaster(Box<CpuSkiaSurface>),
}

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

    pub fn cache_stats(&self) -> RendererCacheStats {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => sw.cache_stats(),
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => sw.cache_stats(),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => sw.cache_stats(),
            Self::CpuRaster(cpu) => cpu.cache_stats(),
        }
    }

    pub fn reset_cache_stats(&self) {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => sw.reset_cache_stats(),
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => sw.reset_cache_stats(),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => sw.reset_cache_stats(),
            Self::CpuRaster(cpu) => cpu.reset_cache_stats(),
        }
    }

    pub fn evict_gpu_caches(&self) {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => sw.evict_gpu_caches(),
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => sw.evict_gpu_caches(),
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => sw.evict_gpu_caches(),
            Self::CpuRaster(cpu) => cpu.evict_gpu_caches(),
        }
    }

    /// Renders `display_list` using the surface's own GPU-cached [`SkiaCanvasRenderer`] and
    /// `DirectContext` (preserving strict drop-order soundness).
    pub fn render_cached_with_plugins(
        &mut self,
        display_list: &DisplayList,
        scale_factor: f32,
        clear_color: ColorRgba,
        plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<(), GpuSurfaceError> {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => {
                sw.render_cached_with_plugins(display_list, scale_factor, clear_color, plugins)
            }
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => {
                sw.render_cached_with_plugins(display_list, scale_factor, clear_color, plugins)
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => {
                sw.render_cached_with_plugins(display_list, scale_factor, clear_color, plugins)
            }
            Self::CpuRaster(cpu) => {
                cpu.render_cached_with_plugins(display_list, scale_factor, clear_color, plugins)
            }
        }
    }

    /// Renders retained [`CompositorScene`] directly using the surface's own GPU-cached
    /// [`SkiaCanvasRenderer`] and `DirectContext` with zero intermediate `DisplayList` allocation.
    pub fn render_scene_cached_with_plugins(
        &mut self,
        scene: &CompositorScene,
        scale_factor: f32,
        clear_color: ColorRgba,
        plugins: Option<SkiaPluginContext<'_>>,
    ) -> Result<(), GpuSurfaceError> {
        match self {
            #[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
            Self::Metal(sw) => {
                sw.render_scene_cached_with_plugins(scene, scale_factor, clear_color, plugins)
            }
            #[cfg(feature = "skia-vulkan")]
            Self::Vulkan(sw) => {
                sw.render_scene_cached_with_plugins(scene, scale_factor, clear_color, plugins)
            }
            #[cfg(all(feature = "skia-d3d", target_os = "windows"))]
            Self::Direct3D12(sw) => {
                sw.render_scene_cached_with_plugins(scene, scale_factor, clear_color, plugins)
            }
            Self::CpuRaster(cpu) => {
                cpu.render_scene_cached_with_plugins(scene, scale_factor, clear_color, plugins)
            }
        }
    }
}

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
