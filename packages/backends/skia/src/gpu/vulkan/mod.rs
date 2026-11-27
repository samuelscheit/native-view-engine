//! Cross-platform Vulkan hardware GPU swapchain and offscreen render target
//! (`--features skia-vulkan`).
//!
//! Uses `ash` directly (zero `vulkano` bloat) + `skia_safe::gpu::vk::BackendContext` +
//! `direct_contexts::make_vulkan` + `backend_render_targets::make_vk` + explicit
//! `VkSemaphore` acquire/signal synchronization (`BackendSemaphore` +
//! `FlushInfo::set_signal_semaphores` + `MutableTextureState` layout transition to
//! `PRESENT_SRC_KHR`).

#[cfg(feature = "skia-vulkan")]
mod setup;

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

#[cfg(feature = "skia-vulkan")]
mod imp {
    use ash::vk as ash_vk;
    use ash::vk::Handle as AshHandle;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use skia_safe::{
        gpu::{
            backend_render_targets, backend_semaphores, surfaces, vk, DirectContext, FlushInfo,
            Protected, SubmitInfo, SurfaceOrigin, SyncCpu,
        },
        ColorType, Surface,
    };

    use super::setup::{
        create_device, create_instance, create_skia_vulkan_context, create_sync_objects,
        create_vk_surface, create_vk_swapchain, load_vulkan_entry, map_ash_to_skia_format,
        pick_physical_device, required_surface_extensions, FrameSyncObjects,
    };
    use super::VulkanSwapchainConfig;
    use crate::gpu::ganesh::{
        create_ganesh_offscreen_surface, GaneshPresenter, NativeSwapchain,
    };
    use crate::gpu::{GpuBackendKind, GpuSurfaceConfig, GpuSurfaceError, SwapchainExtent};

    struct VulkanWindowState {
        surface_loader: ash::khr::surface::Instance,
        swapchain_loader: ash::khr::swapchain::Device,
        surface_khr: ash_vk::SurfaceKHR,
        swapchain_khr: ash_vk::SwapchainKHR,
        images: Vec<ash_vk::Image>,
        format: ash_vk::Format,
        color_type: ColorType,
        sync_frames: Vec<FrameSyncObjects>,
        current_sync: usize,
        active_image_index: Option<u32>,
        active_surface: Option<Surface>,
        swapchain_dirty: bool,
        vsync: bool,
        min_image_count: u32,
    }

    enum VulkanTarget {
        Window(VulkanWindowState),
        Offscreen { surface: Surface },
    }

    /// Platform-specific Vulkan swapchain resources managed by [`GaneshPresenter`].
    pub struct VulkanNativeSwapchain {
        // Drop order: Skia surfaces & DirectContext must drop BEFORE Vulkan device/instance.
        target: VulkanTarget,
        direct_context: DirectContext,
        queue: ash_vk::Queue,
        queue_family_index: u32,
        physical_device: ash_vk::PhysicalDevice,
        device: ash::Device,
        instance: ash::Instance,
        _entry: ash::Entry,
    }

    /// Real Vulkan hardware GPU swapchain & offscreen render-target presenter.
    pub type VulkanSwapchain = GaneshPresenter<VulkanNativeSwapchain>;

    impl VulkanNativeSwapchain {
        fn recreate_window_swapchain(
            &mut self,
            next: SwapchainExtent,
        ) -> Result<(), GpuSurfaceError> {
            let VulkanTarget::Window(win) = &mut self.target else {
                return Ok(());
            };
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            // SAFETY: `self.device` is live; waiting for idle drains GPU work before replacing swapchain images.
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
            // SAFETY: Device is idle and all Skia surfaces referencing `old` swapchain images have been dropped.
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

    impl Drop for VulkanNativeSwapchain {
        fn drop(&mut self) {
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            // SAFETY: `self.device` is live; draining the device ensures no GPU command still references destroyed objects.
            unsafe {
                let _ = self.device.device_wait_idle();
            }
            if let VulkanTarget::Window(win) = &mut self.target {
                win.active_surface.take();
                // SAFETY: Device is idle, `active_surface` is dropped, and every handle belongs to `self.device`/`self.instance`.
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
            // SAFETY: Skia context is abandoned and all child Vulkan objects were destroyed above.
            unsafe {
                self.device.destroy_device(None);
                self.instance.destroy_instance(None);
            }
        }
    }

    impl NativeSwapchain for VulkanNativeSwapchain {
        const KIND: GpuBackendKind = GpuBackendKind::Vulkan;
        const WINDOW_READBACK_ERROR: &'static str =
            "window-attached Vulkan swapchain presents directly without CPU readback";

        fn is_window_attached(&self) -> bool {
            matches!(self.target, VulkanTarget::Window(_))
        }

        fn direct_context_mut(&mut self) -> &mut DirectContext {
            &mut self.direct_context
        }

        fn offscreen_surface_and_context_mut(
            &mut self,
        ) -> Option<(&mut Surface, &mut DirectContext)> {
            match &mut self.target {
                VulkanTarget::Offscreen { surface } => Some((surface, &mut self.direct_context)),
                VulkanTarget::Window(_) => None,
            }
        }

        fn resize_native(
            &mut self,
            prev: SwapchainExtent,
            next: SwapchainExtent,
        ) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                VulkanTarget::Window(_) => self.recreate_window_swapchain(next),
                VulkanTarget::Offscreen { surface } => {
                    if next.width != prev.width || next.height != prev.height {
                        let _ = self.direct_context.flush_submit_and_sync_cpu();
                        *surface = create_ganesh_offscreen_surface(
                            &mut self.direct_context,
                            next,
                            "Vulkan",
                        )?;
                    }
                    Ok(())
                }
            }
        }

        fn begin_native_frame(&mut self, extent: SwapchainExtent) -> Result<bool, GpuSurfaceError> {
            let needs_recreate = matches!(
                &self.target,
                VulkanTarget::Window(win) if win.swapchain_dirty
            );
            if needs_recreate {
                self.recreate_window_swapchain(extent)?;
            }

            if let VulkanTarget::Window(win) = &mut self.target {
                let sync = &win.sync_frames[win.current_sync];
                // SAFETY: `sync.in_flight` is a live fence owned by `self.device` and created signaled.
                unsafe {
                    self.device
                        .wait_for_fences(&[sync.in_flight], true, u64::MAX)
                        .map_err(|e| GpuSurfaceError::PresentFailed(format!("fence wait: {e}")))?;
                    self.device
                        .reset_fences(&[sync.in_flight])
                        .map_err(|e| GpuSurfaceError::PresentFailed(format!("fence reset: {e}")))?;
                }
                // SAFETY: `win.swapchain_khr` and `sync.image_available` are live handles on `self.device`.
                let acquired = unsafe {
                    win.swapchain_loader.acquire_next_image(
                        win.swapchain_khr,
                        u64::MAX,
                        sync.image_available,
                        ash_vk::Fence::null(),
                    )
                };
                let (image_index, suboptimal) = match acquired {
                    Ok(pair) => pair,
                    Err(ash_vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                        win.swapchain_dirty = true;
                        return Err(GpuSurfaceError::SwapchainOutOfDate(
                            "VK_ERROR_OUT_OF_DATE_KHR on acquire".to_string(),
                        ));
                    }
                    Err(e) => {
                        return Err(GpuSurfaceError::SurfaceCreationFailed(format!(
                            "vkAcquireNextImageKHR failed: {e}"
                        )));
                    }
                };
                if suboptimal {
                    win.swapchain_dirty = true;
                }

                let image = win.images[image_index as usize];
                let vk_format = map_ash_to_skia_format(win.format);
                // SAFETY: `image` is a live swapchain `VkImage` owned by `win.swapchain_khr` for this frame.
                let mut image_info = unsafe {
                    vk::ImageInfo::new(
                        image.as_raw() as _,
                        vk::Alloc::default(),
                        vk::ImageTiling::OPTIMAL,
                        vk::ImageLayout::UNDEFINED,
                        vk_format,
                        1,
                        self.queue_family_index,
                        None,
                        Protected::No,
                        vk::SharingMode::EXCLUSIVE,
                    )
                };
                image_info.image_usage_flags =
                    ash_vk::ImageUsageFlags::COLOR_ATTACHMENT.as_raw();

                let backend_rt = backend_render_targets::make_vk(
                    (extent.width as i32, extent.height as i32),
                    &image_info,
                );
                let mut surface = surfaces::wrap_backend_render_target(
                    &mut self.direct_context,
                    &backend_rt,
                    SurfaceOrigin::TopLeft,
                    win.color_type,
                    None,
                    None,
                )
                .ok_or_else(|| {
                    GpuSurfaceError::SurfaceCreationFailed(
                        "wrap_backend_render_target returned None for VkImage".to_string(),
                    )
                })?;

                // Hook the swapchain acquire semaphore into Skia's surface wait list so the GPU
                // stalls color attachment writes until presentation ownership transfers.
                // SAFETY: `sync.image_available` is a live `VkSemaphore` kept alive inside `VulkanWindowState`.
                let acquire_sem =
                    unsafe { backend_semaphores::make_vk(sync.image_available.as_raw() as _) };
                surface.wait(&[acquire_sem], false);

                win.active_image_index = Some(image_index);
                win.active_surface = Some(surface);
            }

            Ok(needs_recreate)
        }

        fn active_surface_mut(&mut self) -> Result<&mut Surface, GpuSurfaceError> {
            match &mut self.target {
                VulkanTarget::Window(win) => win
                    .active_surface
                    .as_mut()
                    .ok_or(GpuSurfaceError::DrawableUnavailable),
                VulkanTarget::Offscreen { surface } => Ok(surface),
            }
        }

        fn submit_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                VulkanTarget::Window(win) => {
                    let sync = &win.sync_frames[win.current_sync];
                    // SAFETY: `sync.render_finished` is a live `VkSemaphore` kept alive inside `VulkanWindowState`.
                    let mut signal_sem = [unsafe {
                        backend_semaphores::make_vk(sync.render_finished.as_raw() as _)
                    }];
                    let mut flush_info = FlushInfo::default();
                    // SAFETY: `signal_sem` stays live on the stack throughout `flush_surface_with_texture_state`.
                    unsafe {
                        flush_info.set_signal_semaphores(&mut signal_sem);
                    }
                    let present_state = vk::mutable_texture_states::new_vulkan(
                        vk::ImageLayout::PRESENT_SRC_KHR,
                        self.queue_family_index,
                    );
                    let mut surface = win
                        .active_surface
                        .take()
                        .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                    self.direct_context
                        .flush_surface_with_texture_state(
                            &mut surface,
                            &flush_info,
                            Some(&present_state),
                        )
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                    drop(surface);
                    if !self.direct_context.submit(SubmitInfo {
                        sync: SyncCpu::No,
                        ..SubmitInfo::default()
                    }) {
                        return Err(GpuSurfaceError::FlushFailed(
                            "DirectContext::submit returned false".to_string(),
                        ));
                    }
                }
                VulkanTarget::Offscreen { surface } => {
                    self.direct_context
                        .flush_and_submit_surface(surface, SyncCpu::Yes)
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                }
            }
            Ok(())
        }

        fn present_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
            if let VulkanTarget::Window(win) = &mut self.target {
                let image_index = win
                    .active_image_index
                    .take()
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                let sync = &win.sync_frames[win.current_sync];
                let wait_sems = [sync.render_finished];
                let swapchains = [win.swapchain_khr];
                let image_indices = [image_index];
                let present_info = ash_vk::PresentInfoKHR::default()
                    .wait_semaphores(&wait_sems)
                    .swapchains(&swapchains)
                    .image_indices(&image_indices);
                // SAFETY: `self.queue`, `win.swapchain_khr`, `sync.render_finished`, and `image_index` are valid handles
                // and `submit_native_frame` transitioned the image to `PRESENT_SRC_KHR`.
                match unsafe { win.swapchain_loader.queue_present(self.queue, &present_info) } {
                    Ok(suboptimal) => {
                        if suboptimal {
                            win.swapchain_dirty = true;
                        }
                    }
                    Err(ash_vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                        win.swapchain_dirty = true;
                    }
                    Err(e) => {
                        return Err(GpuSurfaceError::PresentFailed(format!(
                            "vkQueuePresentKHR failed: {e}"
                        )));
                    }
                }
                // Signal the frame fence once queued work drains so the next ring slot stays safe.
                // SAFETY: `self.queue` and `sync.in_flight` are live handles; the fence was reset in `begin_native_frame`.
                unsafe {
                    let _ = self.device.queue_submit(self.queue, &[], sync.in_flight);
                }
                win.current_sync = (win.current_sync + 1) % win.sync_frames.len();
            }
            Ok(())
        }

        fn abort_native_frame(&mut self) {
            if let VulkanTarget::Window(win) = &mut self.target {
                win.active_surface.take();
                win.active_image_index = None;
            }
        }
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
            let surface =
                create_ganesh_offscreen_surface(&mut direct_context, vk_cfg.extent, "Vulkan")?;
            Ok(Self::new(
                VulkanNativeSwapchain {
                    target: VulkanTarget::Offscreen { surface },
                    direct_context,
                    queue,
                    queue_family_index,
                    physical_device,
                    device,
                    instance,
                    _entry: entry,
                },
                vk_cfg.extent,
            ))
        }

        /// Creates a Vulkan `VkSurfaceKHR` + `VkSwapchainKHR` attached to a native window handle
        /// with explicit semaphore acquire/signal synchronization and `PRESENT_SRC_KHR` transitions.
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

            Ok(Self::new(
                VulkanNativeSwapchain {
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
                },
                vk_cfg.extent,
            ))
        }
    }
}

#[cfg(feature = "skia-vulkan")]
pub use imp::{VulkanNativeSwapchain, VulkanSwapchain};
