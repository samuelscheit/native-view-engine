//! Cross-platform Vulkan hardware GPU swapchain and offscreen render target
//! (`--features skia-vulkan`).
//!
//! Uses `ash` directly (zero `vulkano` bloat) + `skia_safe::gpu::vk::BackendContext` +
//! `direct_contexts::make_vulkan` + `backend_render_targets::make_vk` + explicit
//! `VkSemaphore` acquire/signal synchronization (`BackendSemaphore` +
//! `FlushInfo::set_signal_semaphores` + `MutableTextureState` layout transition to
//! `PRESENT_SRC_KHR`).

#[cfg(feature = "skia-vulkan")]
mod guards;
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
        gpu::{backend_semaphores, vk, DirectContext, FlushInfo, SubmitInfo, SyncCpu},
        ColorType, Surface,
    };

    use super::guards::{
        DeviceGuard, InstanceGuard, SurfaceGuard, SwapchainGuard, SyncObjectsGuard,
    };
    use super::setup::{
        create_device, create_instance, create_skia_vulkan_context, create_sync_objects,
        create_vk_surface, create_vk_swapchain, destroy_sync_objects, load_vulkan_entry,
        pick_physical_device, required_surface_extensions, wrap_vk_swapchain_image,
        FrameSyncObjects, SkiaVulkanContextParams, VkSwapchainParams,
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
        swapchain_khr: Option<ash_vk::SwapchainKHR>,
        images: Vec<ash_vk::Image>,
        vk_extent: ash_vk::Extent2D,
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
        Window(Box<VulkanWindowState>),
        Offscreen { surface: Option<Surface> },
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
            let frame_count = win.sync_frames.len().max(win.min_image_count as usize);
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            // SAFETY: `self.device` is live; waiting for idle drains GPU work before replacing swapchain & sync objects.
            unsafe {
                let _ = self.device.device_wait_idle();
            }
            win.active_surface.take();
            win.active_image_index = None;
            win.swapchain_dirty = true;
            // Destroy and recreate binary semaphores & fences across swapchain recreation so no
            // signaled/unconsumed semaphore from an aborted or failed frame leaks (VUID 01286).
            destroy_sync_objects(&self.device, &win.sync_frames);
            win.sync_frames.clear();
            win.current_sync = 0;

            let old = win.swapchain_khr.take();
            let created_res = create_vk_swapchain(&VkSwapchainParams {
                surface_loader: &win.surface_loader,
                swapchain_loader: &win.swapchain_loader,
                physical_device: self.physical_device,
                surface: win.surface_khr,
                extent: next,
                vsync: win.vsync,
                requested_images: win.min_image_count,
                old_swapchain: old.unwrap_or_else(ash_vk::SwapchainKHR::null),
            });
            // Destroy `old` immediately whether creation succeeded or failed: once passed as
            // `oldSwapchain` it is retired and must never be passed again (VUID 01933).
            if let Some(old_sc) = old {
                // SAFETY: Device is idle and all Skia surfaces referencing `old_sc` were dropped above.
                unsafe {
                    win.swapchain_loader.destroy_swapchain(old_sc, None);
                }
            }
            let created = created_res?;
            let swapchain_guard = SwapchainGuard(&win.swapchain_loader, Some(created.swapchain));
            let new_sync = create_sync_objects(&self.device, frame_count)?;
            win.swapchain_khr = Some(swapchain_guard.disarm());
            win.images = created.images;
            win.vk_extent = created.extent;
            win.format = created.format;
            win.color_type = created.color_type;
            win.sync_frames = new_sync;
            win.current_sync = 0;
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
            match &mut self.target {
                VulkanTarget::Window(win) => {
                    win.active_surface.take();
                    destroy_sync_objects(&self.device, &win.sync_frames);
                    win.sync_frames.clear();
                    // SAFETY: Device is idle, `active_surface` is dropped, and every handle belongs to `self.device`/`self.instance`.
                    unsafe {
                        if let Some(sc) = win.swapchain_khr.take() {
                            win.swapchain_loader.destroy_swapchain(sc, None);
                        }
                        win.surface_loader.destroy_surface(win.surface_khr, None);
                    }
                }
                VulkanTarget::Offscreen { surface } => {
                    // Drop the Skia Surface BEFORE abandoning DirectContext and destroying VkDevice/VkInstance.
                    surface.take();
                }
            }
            self.direct_context.release_resources_and_abandon();
            // SAFETY: All Skia surfaces are dropped, Skia context is abandoned, and all child Vulkan objects were destroyed above.
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
                VulkanTarget::Offscreen { surface: Some(surface) } => {
                    Some((surface, &mut self.direct_context))
                }
                _ => None,
            }
        }

        fn resize_native(
            &mut self,
            prev: SwapchainExtent,
            next: SwapchainExtent,
        ) -> Result<bool, GpuSurfaceError> {
            if next.width == prev.width && next.height == prev.height {
                return Ok(false);
            }
            match &mut self.target {
                VulkanTarget::Window(_) => {
                    self.recreate_window_swapchain(next)?;
                    Ok(true)
                }
                VulkanTarget::Offscreen { surface } => {
                    let _ = self.direct_context.flush_submit_and_sync_cpu();
                    *surface = Some(create_ganesh_offscreen_surface(
                        &mut self.direct_context,
                        next,
                        "Vulkan",
                    )?);
                    Ok(true)
                }
            }
        }

        fn begin_native_frame(
            &mut self,
            extent: SwapchainExtent,
            recreations: &mut u64,
        ) -> Result<(), GpuSurfaceError> {
            let needs_recreate = matches!(
                &self.target,
                VulkanTarget::Window(win)
                    if win.swapchain_dirty || win.swapchain_khr.is_none() || win.sync_frames.is_empty()
            );
            if needs_recreate {
                self.recreate_window_swapchain(extent)?;
                *recreations += 1;
            }

            if let VulkanTarget::Window(win) = &mut self.target {
                let swapchain_khr = win
                    .swapchain_khr
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                let sync = &mut win.sync_frames[win.current_sync];
                if sync.in_flight_pending {
                    // SAFETY: `sync.in_flight` is a live fence owned by `self.device` with a submitted signal in flight.
                    unsafe {
                        self.device
                            .wait_for_fences(&[sync.in_flight], true, u64::MAX)
                            .map_err(|e| {
                                GpuSurfaceError::PresentFailed(format!("fence wait: {e}"))
                            })?;
                    }
                    sync.in_flight_pending = false;
                }
                // SAFETY: `swapchain_khr` and `sync.image_available` are live handles on `self.device`.
                // `sync.in_flight` is reset immediately before `queue_submit` in `present_native_frame`.
                let acquired = unsafe {
                    win.swapchain_loader.acquire_next_image(
                        swapchain_khr,
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
                        win.swapchain_dirty = true;
                        return Err(GpuSurfaceError::SurfaceCreationFailed(format!(
                            "vkAcquireNextImageKHR failed: {e}"
                        )));
                    }
                };
                if suboptimal {
                    win.swapchain_dirty = true;
                }

                let Some(&image) = win.images.get(image_index as usize) else {
                    win.swapchain_dirty = true;
                    return Err(GpuSurfaceError::DrawableUnavailable);
                };
                let Some(surface) = wrap_vk_swapchain_image(
                    &mut self.direct_context,
                    image,
                    win.vk_extent,
                    win.format,
                    win.color_type,
                    self.queue_family_index,
                    sync.image_available,
                ) else {
                    // `acquire_next_image` already signaled `sync.image_available`; force swapchain + sync rebuild.
                    win.swapchain_dirty = true;
                    return Err(GpuSurfaceError::SurfaceCreationFailed(
                        "wrap_backend_render_target returned None for VkImage".to_string(),
                    ));
                };

                win.active_image_index = Some(image_index);
                win.active_surface = Some(surface);
            }

            Ok(())
        }

        fn active_surface_mut(&mut self) -> Result<&mut Surface, GpuSurfaceError> {
            match &mut self.target {
                VulkanTarget::Window(win) => win
                    .active_surface
                    .as_mut()
                    .ok_or(GpuSurfaceError::DrawableUnavailable),
                VulkanTarget::Offscreen { surface } => surface
                    .as_mut()
                    .ok_or(GpuSurfaceError::DrawableUnavailable),
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
                    if let Err(e) = self.direct_context.flush_surface_with_texture_state(
                        &mut surface,
                        &flush_info,
                        Some(&present_state),
                    ) {
                        win.swapchain_dirty = true;
                        return Err(GpuSurfaceError::FlushFailed(e.to_string()));
                    }
                    drop(surface);
                    if !self.direct_context.submit(SubmitInfo {
                        sync: SyncCpu::No,
                        ..SubmitInfo::default()
                    }) {
                        win.swapchain_dirty = true;
                        return Err(GpuSurfaceError::FlushFailed(
                            "DirectContext::submit returned false".to_string(),
                        ));
                    }
                }
                VulkanTarget::Offscreen { surface } => {
                    let surface = surface
                        .as_mut()
                        .ok_or(GpuSurfaceError::DrawableUnavailable)?;
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
                let swapchain_khr = win
                    .swapchain_khr
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                let sync = &mut win.sync_frames[win.current_sync];
                let wait_sems = [sync.render_finished];
                let swapchains = [swapchain_khr];
                let image_indices = [image_index];
                let present_info = ash_vk::PresentInfoKHR::default()
                    .wait_semaphores(&wait_sems)
                    .swapchains(&swapchains)
                    .image_indices(&image_indices);
                // SAFETY: `self.queue`, `swapchain_khr`, `sync.render_finished`, and `image_index` are valid handles
                // and `submit_native_frame` transitioned the image to `PRESENT_SRC_KHR`.
                let present_err = match unsafe { win.swapchain_loader.queue_present(self.queue, &present_info) } {
                    Ok(suboptimal) => {
                        if suboptimal {
                            win.swapchain_dirty = true;
                        }
                        None
                    }
                    Err(ash_vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                        win.swapchain_dirty = true;
                        None
                    }
                    Err(e) => {
                        win.swapchain_dirty = true;
                        Some(GpuSurfaceError::PresentFailed(format!(
                            "vkQueuePresentKHR failed: {e}"
                        )))
                    }
                };
                if let Some(err) = present_err {
                    // Drain submitted GPU work referencing this slot's semaphores before returning.
                    // SAFETY: `self.device` is live.
                    unsafe {
                        let _ = self.device.device_wait_idle();
                    }
                    sync.in_flight_pending = false;
                    return Err(err);
                }
                // Reset immediately before `queue_submit`; on failure drain device and rebuild swapchain.
                // SAFETY: `self.device`, `self.queue`, and `sync.in_flight` are valid live handles.
                let fence_res = unsafe {
                    self.device
                        .reset_fences(&[sync.in_flight])
                        .map_err(|e| GpuSurfaceError::PresentFailed(format!("fence reset: {e}")))
                        .and_then(|()| {
                            self.device
                                .queue_submit(self.queue, &[], sync.in_flight)
                                .map_err(|e| GpuSurfaceError::PresentFailed(format!("fence submit: {e}")))
                        })
                };
                if let Err(err) = fence_res {
                    win.swapchain_dirty = true;
                    // SAFETY: `self.device` is live.
                    unsafe {
                        let _ = self.device.device_wait_idle();
                    }
                    sync.in_flight_pending = false;
                    return Err(err);
                }
                sync.in_flight_pending = true;
                win.current_sync = (win.current_sync + 1) % win.sync_frames.len();
            }
            Ok(())
        }

        fn abort_native_frame(&mut self) {
            if let VulkanTarget::Window(win) = &mut self.target {
                if win.active_image_index.is_some() {
                    win.swapchain_dirty = true;
                }
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
            let instance_guard = InstanceGuard(Some(instance));
            let (physical_device, queue_family_index) =
                pick_physical_device(instance_guard.get(), None, &[])?;
            let (device, queue, device_exts) =
                create_device(instance_guard.get(), physical_device, queue_family_index, &[])?;
            let device_guard = DeviceGuard(Some(device));
            let mut direct_context = create_skia_vulkan_context(&SkiaVulkanContextParams {
                entry: &entry,
                instance: instance_guard.get(),
                physical_device,
                device: device_guard.get(),
                queue,
                queue_family_index,
                instance_exts: &instance_exts,
                device_exts: &device_exts,
            })?;
            let surface = match create_ganesh_offscreen_surface(
                &mut direct_context,
                vk_cfg.extent,
                "Vulkan",
            ) {
                Ok(s) => s,
                Err(err) => {
                    direct_context.release_resources_and_abandon();
                    return Err(err);
                }
            };
            Ok(Self::new(
                VulkanNativeSwapchain {
                    target: VulkanTarget::Offscreen {
                        surface: Some(surface),
                    },
                    direct_context,
                    queue,
                    queue_family_index,
                    physical_device,
                    device: device_guard.disarm(),
                    instance: instance_guard.disarm(),
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
            let instance_guard = InstanceGuard(Some(instance));
            let surface_loader = ash::khr::surface::Instance::new(&entry, instance_guard.get());
            let surface_khr = create_vk_surface(
                &entry,
                instance_guard.get(),
                disp_handle.as_raw(),
                win_handle.as_raw(),
            )?;
            let surface_guard = SurfaceGuard(&surface_loader, Some(surface_khr));

            let swapchain_ext = ash::khr::swapchain::NAME;
            let (physical_device, queue_family_index) = pick_physical_device(
                instance_guard.get(),
                Some((&surface_loader, surface_guard.get())),
                &[swapchain_ext],
            )?;
            let (device, queue, device_exts) = create_device(
                instance_guard.get(),
                physical_device,
                queue_family_index,
                &[swapchain_ext],
            )?;
            let device_guard = DeviceGuard(Some(device));

            let swapchain_loader =
                ash::khr::swapchain::Device::new(instance_guard.get(), device_guard.get());
            let created = create_vk_swapchain(&VkSwapchainParams {
                surface_loader: &surface_loader,
                swapchain_loader: &swapchain_loader,
                physical_device,
                surface: surface_guard.get(),
                extent: vk_cfg.extent,
                vsync: vk_cfg.vsync,
                requested_images: vk_cfg.min_image_count,
                old_swapchain: ash_vk::SwapchainKHR::null(),
            })?;
            let swapchain_guard = SwapchainGuard(&swapchain_loader, Some(created.swapchain));

            let sync_frames = create_sync_objects(device_guard.get(), vk_cfg.frames_in_flight)?;
            let sync_guard = SyncObjectsGuard(device_guard.get(), sync_frames);
            let direct_context = create_skia_vulkan_context(&SkiaVulkanContextParams {
                entry: &entry,
                instance: instance_guard.get(),
                physical_device,
                device: device_guard.get(),
                queue,
                queue_family_index,
                instance_exts: &instance_exts,
                device_exts: &device_exts,
            })?;

            let sync_frames = sync_guard.disarm();
            let swapchain_khr = swapchain_guard.disarm();
            let device = device_guard.disarm();
            let surface_khr = surface_guard.disarm();
            let instance = instance_guard.disarm();

            Ok(Self::new(
                VulkanNativeSwapchain {
                    target: VulkanTarget::Window(Box::new(VulkanWindowState {
                        surface_loader,
                        swapchain_loader,
                        surface_khr,
                        swapchain_khr: Some(swapchain_khr),
                        images: created.images,
                        vk_extent: created.extent,
                        format: created.format,
                        color_type: created.color_type,
                        sync_frames,
                        current_sync: 0,
                        active_image_index: None,
                        active_surface: None,
                        swapchain_dirty: false,
                        vsync: vk_cfg.vsync,
                        min_image_count: vk_cfg.min_image_count,
                    })),
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
