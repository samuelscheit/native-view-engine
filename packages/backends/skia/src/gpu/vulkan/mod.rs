//! Cross-platform Vulkan hardware GPU swapchain and offscreen render target
//! (`--features skia-vulkan`).
//!
//! Uses `ash` directly (zero `vulkano` bloat) + `skia_safe::gpu::vk::BackendContext` +
//! `direct_contexts::make_vulkan` + `backend_render_targets::make_vk` + explicit
//! `VkSemaphore` acquire/signal synchronization hooks (`BackendSemaphore` +
//! `FlushInfo::set_signal_semaphores` + `MutableTextureState` layout transition to
//! `PRESENT_SRC_KHR`).

use super::{GpuSurfaceConfig, GpuSurfaceError, SwapchainExtent};

/// Configuration for Vulkan swapchain creation and synchronization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VulkanSwapchainConfig {
    pub extent: SwapchainExtent,
    pub vsync: bool,
    pub min_image_count: u32,
    pub frames_in_flight: usize,
}

impl VulkanSwapchainConfig {
    pub fn from_surface_config(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
        let extent = config.extent()?;
        let min_image_count = config.swapchain_image_count.clamp(2, 4);
        Ok(Self {
            extent,
            vsync: config.vsync,
            min_image_count,
            frames_in_flight: min_image_count as usize,
        })
    }
}

/// Semaphore synchronization hook record for a Vulkan frame submission.
///
/// Tracks the raw `VkSemaphore` handles (as `u64` / opaque handles) used to synchronize
/// `vkAcquireNextImageKHR` -> Skia Ganesh `Surface::wait` -> Skia `FlushInfo::set_signal_semaphores`
/// -> `vkQueuePresentKHR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VulkanSemaphoreSyncHooks {
    pub image_available_semaphore: u64,
    pub render_finished_semaphore: u64,
    pub in_flight_fence: u64,
    pub queue_family_index: u32,
}

#[cfg(feature = "skia-vulkan")]
mod presenter;
#[cfg(feature = "skia-vulkan")]
mod setup;

#[cfg(feature = "skia-vulkan")]
mod imp {
    use std::time::Instant;

    use ash::vk as ash_vk;
    use nve_abi::ColorRgba;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use skia_safe::{
        gpu::{DirectContext, SyncCpu},
        ColorType, Surface,
    };

    use super::setup::{
        create_device, create_instance, create_offscreen_surface, create_skia_vulkan_context,
        create_sync_objects, create_vk_surface, create_vk_swapchain, load_vulkan_entry,
        pick_physical_device, required_surface_extensions,
    };
    use super::{VulkanSemaphoreSyncHooks, VulkanSwapchainConfig};
    use crate::gpu::skia_renderer::{
        read_surface_to_pixel_buffer, to_skia_color, SkiaCanvasRenderer,
    };
    use crate::gpu::{
        FrameLifecycleState, FrameTelemetry, GpuFramePresenter, GpuSurfaceConfig, GpuSurfaceError,
        SkiaPluginContext, SwapchainExtent,
    };
    use crate::raster::PixelBuffer;
    use crate::DisplayList;

    pub(super) struct FrameSyncObjects {
        pub(super) image_available: ash_vk::Semaphore,
        pub(super) render_finished: ash_vk::Semaphore,
        pub(super) in_flight: ash_vk::Fence,
    }

    pub(super) struct VulkanWindowState {
        pub(super) surface_loader: ash::khr::surface::Instance,
        pub(super) swapchain_loader: ash::khr::swapchain::Device,
        pub(super) surface_khr: ash_vk::SurfaceKHR,
        pub(super) swapchain_khr: ash_vk::SwapchainKHR,
        pub(super) images: Vec<ash_vk::Image>,
        pub(super) format: ash_vk::Format,
        pub(super) color_type: ColorType,
        pub(super) sync_frames: Vec<FrameSyncObjects>,
        pub(super) current_sync: usize,
        pub(super) active_image_index: Option<u32>,
        pub(super) active_surface: Option<Surface>,
        pub(super) swapchain_dirty: bool,
        pub(super) vsync: bool,
        pub(super) min_image_count: u32,
    }

    pub(super) enum VulkanTarget {
        Window(VulkanWindowState),
        Offscreen { surface: Surface },
    }

    /// Real Vulkan hardware GPU swapchain & offscreen render-target presenter.
    pub struct VulkanSwapchain {
        // Drop order: Skia surfaces & DirectContext must drop BEFORE Vulkan device/instance.
        pub(super) target: VulkanTarget,
        pub(super) direct_context: DirectContext,
        pub(super) queue: ash_vk::Queue,
        pub(super) queue_family_index: u32,
        pub(super) physical_device: ash_vk::PhysicalDevice,
        pub(super) device: ash::Device,
        pub(super) instance: ash::Instance,
        pub(super) _entry: ash::Entry,
        pub(super) renderer: SkiaCanvasRenderer,
        pub(super) extent: SwapchainExtent,
        pub(super) state: FrameLifecycleState,
        pub(super) frame_index: u64,
        pub(super) swapchain_recreations: u64,
        pub(super) frame_start: Option<Instant>,
        pub(super) acquire_us: u64,
        pub(super) record_us: u64,
        pub(super) submit_us: u64,
        pub(super) last_telemetry: Option<FrameTelemetry>,
        pub(super) last_sync_hooks: VulkanSemaphoreSyncHooks,
        pub(super) readback_rgba: Vec<u8>,
    }

    impl VulkanSwapchain {
        /// Creates an offscreen Vulkan GPU context and VRAM-backed Skia render target.
        pub fn new_offscreen(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
            let vk_cfg = VulkanSwapchainConfig::from_surface_config(config)?;
            let entry = load_vulkan_entry()?;
            let (instance, instance_exts) = create_instance(&entry, &[])?;
            let (physical_device, queue_family_index) =
                pick_physical_device(&instance, None, &[])?;
            let (device, queue, device_exts) =
                create_device(&instance, physical_device, queue_family_index, &[])?;
            let mut direct_context = create_skia_vulkan_context(
                &entry,
                &instance,
                physical_device,
                &device,
                queue,
                queue_family_index,
                &instance_exts,
                &device_exts,
            )?;
            let surface = create_offscreen_surface(&mut direct_context, vk_cfg.extent)?;
            Ok(Self {
                target: VulkanTarget::Offscreen { surface },
                direct_context,
                queue,
                queue_family_index,
                physical_device,
                device,
                instance,
                _entry: entry,
                renderer: SkiaCanvasRenderer::new(),
                extent: vk_cfg.extent,
                state: FrameLifecycleState::Idle,
                frame_index: 0,
                swapchain_recreations: 0,
                frame_start: None,
                acquire_us: 0,
                record_us: 0,
                submit_us: 0,
                last_telemetry: None,
                last_sync_hooks: VulkanSemaphoreSyncHooks {
                    queue_family_index,
                    ..Default::default()
                },
                readback_rgba: Vec::new(),
            })
        }

        /// Creates a Vulkan `VkSurfaceKHR` + `VkSwapchainKHR` attached to a native window handle
        /// with explicit semaphore acquire/signal hooks and `PRESENT_SRC_KHR` transitions.
        pub fn from_window<W: HasWindowHandle + HasDisplayHandle>(
            window: &W,
            config: GpuSurfaceConfig,
        ) -> Result<Self, GpuSurfaceError> {
            let vk_cfg = VulkanSwapchainConfig::from_surface_config(config)?;
            let win_handle = window
                .window_handle()
                .map_err(|e| GpuSurfaceError::WindowHandleError(e.to_string()))?;
            let disp_handle = window
                .display_handle()
                .map_err(|e| GpuSurfaceError::WindowHandleError(e.to_string()))?;

            let entry = load_vulkan_entry()?;
            let surface_exts = required_surface_extensions(disp_handle.as_raw())?;
            let (instance, instance_exts) = create_instance(&entry, &surface_exts)?;
            let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
            let surface_khr = create_vk_surface(
                &entry,
                &instance,
                disp_handle.as_raw(),
                win_handle.as_raw(),
            )?;

            let swapchain_ext = ash::khr::swapchain::NAME;
            let (physical_device, queue_family_index) = pick_physical_device(
                &instance,
                Some((&surface_loader, surface_khr)),
                &[swapchain_ext],
            )?;
            let (device, queue, device_exts) =
                create_device(&instance, physical_device, queue_family_index, &[swapchain_ext])?;

            let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);
            let (swapchain_khr, images, format, color_type) = create_vk_swapchain(
                &surface_loader,
                &swapchain_loader,
                physical_device,
                surface_khr,
                vk_cfg.extent,
                vk_cfg.vsync,
                vk_cfg.min_image_count,
                ash_vk::SwapchainKHR::null(),
            )?;

            let sync_frames = create_sync_objects(&device, vk_cfg.frames_in_flight)?;
            let direct_context = create_skia_vulkan_context(
                &entry,
                &instance,
                physical_device,
                &device,
                queue,
                queue_family_index,
                &instance_exts,
                &device_exts,
            )?;

            Ok(Self {
                target: VulkanTarget::Window(VulkanWindowState {
                    surface_loader,
                    swapchain_loader,
                    surface_khr,
                    swapchain_khr,
                    images,
                    format,
                    color_type,
                    sync_frames,
                    current_sync: 0,
                    active_image_index: None,
                    active_surface: None,
                    swapchain_dirty: false,
                    vsync: vk_cfg.vsync,
                    min_image_count: vk_cfg.min_image_count,
                }),
                direct_context,
                queue,
                queue_family_index,
                physical_device,
                device,
                instance,
                _entry: entry,
                renderer: SkiaCanvasRenderer::new(),
                extent: vk_cfg.extent,
                state: FrameLifecycleState::Idle,
                frame_index: 0,
                swapchain_recreations: 0,
                frame_start: None,
                acquire_us: 0,
                record_us: 0,
                submit_us: 0,
                last_telemetry: None,
                last_sync_hooks: VulkanSemaphoreSyncHooks {
                    queue_family_index,
                    ..Default::default()
                },
                readback_rgba: Vec::new(),
            })
        }

        pub fn is_window_attached(&self) -> bool {
            matches!(self.target, VulkanTarget::Window(_))
        }

        pub fn last_sync_hooks(&self) -> VulkanSemaphoreSyncHooks {
            self.last_sync_hooks
        }

        pub fn render_with_renderer(
            &mut self,
            renderer: &SkiaCanvasRenderer,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
        ) -> Result<(), GpuSurfaceError> {
            self.render_with_renderer_plugins(renderer, display_list, scale_factor, clear_color, None)
        }

        pub fn render_with_renderer_plugins(
            &mut self,
            renderer: &SkiaCanvasRenderer,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
            plugins: Option<SkiaPluginContext<'_>>,
        ) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                VulkanTarget::Offscreen { surface } => {
                    let canvas = surface.canvas();
                    canvas.clear(to_skia_color(clear_color));
                    renderer.paint_display_list_opt_plugins(display_list, canvas, scale_factor, plugins);
                    self.direct_context
                        .flush_and_submit_surface(surface, SyncCpu::Yes)
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                    Ok(())
                }
                VulkanTarget::Window(_) => self
                    .execute_frame_with_plugins(display_list, scale_factor, clear_color, None, plugins)
                    .map(|_| ()),
            }
        }

        pub fn copy_to_pixel_buffer(
            &mut self,
            target: &mut PixelBuffer,
        ) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                VulkanTarget::Offscreen { surface } => {
                    self.direct_context
                        .flush_and_submit_surface(surface, SyncCpu::Yes)
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                    read_surface_to_pixel_buffer(
                        surface,
                        self.extent.width,
                        self.extent.height,
                        &mut self.readback_rgba,
                        target,
                    )
                }
                VulkanTarget::Window(_) => Err(GpuSurfaceError::ReadbackFailed(
                    "window-attached Vulkan swapchain presents directly without CPU readback"
                        .to_string(),
                )),
            }
        }

        pub(super) fn recreate_window_swapchain(
            &mut self,
            next: SwapchainExtent,
        ) -> Result<(), GpuSurfaceError> {
            let VulkanTarget::Window(win) = &mut self.target else {
                return Ok(());
            };
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            unsafe {
                let _ = self.device.device_wait_idle();
            }
            win.active_surface.take();
            win.active_image_index = None;
            let old = win.swapchain_khr;
            let (new_sc, new_images, new_format, new_ct) = create_vk_swapchain(
                &win.surface_loader,
                &win.swapchain_loader,
                self.physical_device,
                win.surface_khr,
                next,
                win.vsync,
                win.min_image_count,
                old,
            )?;
            unsafe {
                win.swapchain_loader.destroy_swapchain(old, None);
            }
            win.swapchain_khr = new_sc;
            win.images = new_images;
            win.format = new_format;
            win.color_type = new_ct;
            win.swapchain_dirty = false;
            Ok(())
        }
    }

    impl Drop for VulkanSwapchain {
        fn drop(&mut self) {
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            unsafe {
                let _ = self.device.device_wait_idle();
            }
            if let VulkanTarget::Window(win) = &mut self.target {
                win.active_surface.take();
                unsafe {
                    for sync in &win.sync_frames {
                        self.device.destroy_semaphore(sync.image_available, None);
                        self.device.destroy_semaphore(sync.render_finished, None);
                        self.device.destroy_fence(sync.in_flight, None);
                    }
                    win.swapchain_loader
                        .destroy_swapchain(win.swapchain_khr, None);
                    win.surface_loader.destroy_surface(win.surface_khr, None);
                }
            }
            self.direct_context.release_resources_and_abandon();
            unsafe {
                self.device.destroy_device(None);
                self.instance.destroy_instance(None);
            }
        }
    }
}

#[cfg(feature = "skia-vulkan")]
pub use imp::VulkanSwapchain;
