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
    use std::mem::ManuallyDrop;

    use ash::vk as ash_vk;
    use ash::vk::Handle as AshHandle;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use skia_safe::{
        gpu::{
            backend_semaphores, vk, DirectContext, FlushInfo, SemaphoresSubmitted, SubmitInfo,
            SyncCpu,
        },
        ConditionallySend, Surface,
    };

    use super::guards::{
        DeviceGuard, InstanceGuard, SurfaceGuard, SwapchainGuard, SyncObjectsGuard,
        VulkanDeviceOwner, VulkanWindowState,
    };
    use super::setup::{
        create_device, create_instance, create_skia_vulkan_context, create_sync_objects,
        create_vk_surface, create_vk_swapchain, destroy_sync_objects, load_vulkan_entry,
        pick_physical_device, required_surface_extensions, wrap_vk_swapchain_image,
        SkiaVulkanContextParams, VkSwapchainParams,
    };
    use super::VulkanSwapchainConfig;
    use crate::gpu::ganesh::{
        create_ganesh_offscreen_surface, GaneshPresenter, NativeSwapchain,
    };
    use crate::gpu::{GpuBackendKind, GpuSurfaceConfig, GpuSurfaceError, SwapchainExtent};

    /// Platform-specific Vulkan swapchain resources managed by [`GaneshPresenter`].
    pub struct VulkanNativeSwapchain {
        // Strict Rust field drop order (top to bottom):
        //   1. `surface` (Skia `Surface`)
        //   2. `direct_context` (Skia `GrDirectContext`)
        //   3. `window` (`sync_frames` -> `VkSwapchainKHR` -> `VkSurfaceKHR`)
        //   4. `owner` (`VkDevice` -> `VkInstance` -> `ash::Entry`)
        surface: Option<Surface>,
        direct_context: DirectContext,
        window: Option<Box<VulkanWindowState>>,
        queue: ash_vk::Queue,
        queue_family_index: u32,
        physical_device: ash_vk::PhysicalDevice,
        owner: VulkanDeviceOwner,
    }

    /// Real Vulkan hardware GPU swapchain & offscreen render-target presenter.
    pub type VulkanSwapchain = GaneshPresenter<VulkanNativeSwapchain>;

    impl VulkanNativeSwapchain {
        fn recreate_window_swapchain(
            &mut self,
            next: SwapchainExtent,
        ) -> Result<(), GpuSurfaceError> {
            let Some(win) = self.window.as_mut() else {
                return Ok(());
            };
            let frame_count = win.sync_frames.len().max(win.min_image_count as usize);
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            // SAFETY: `self.owner.device` is live; waiting for idle drains GPU work before replacing swapchain & sync objects.
            unsafe {
                let _ = self.owner.device.device_wait_idle();
            }
            self.surface.take();
            win.active_image_index = None;
            win.swapchain_dirty = true;
            // Destroy and recreate binary semaphores & fences across swapchain recreation so no
            // signaled/unconsumed semaphore from an aborted or failed frame leaks (VUID 01286).
            destroy_sync_objects(&self.owner.device, &win.sync_frames);
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
            let swapchain_guard = SwapchainGuard::new(&win.swapchain_loader, created.swapchain);
            let new_sync = create_sync_objects(&self.owner.device, frame_count)?;
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
            // SAFETY: `self.owner.device` is still live because field drops run AFTER `Drop::drop`.
            unsafe {
                let _ = self.owner.device.device_wait_idle();
            }
            // Drop any active Skia Surface and abandon DirectContext; Rust then drops fields in
            // declaration order: `surface` -> `direct_context` -> `window` -> `owner` (VkDevice/VkInstance).
            self.surface.take();
            self.direct_context.release_resources_and_abandon();
            // If an external Skia handle (e.g. a GPU texture `Image`) still retains a reference
            // to `GrDirectContext`, destroying `VkDevice` / unloading `libvulkan` before that
            // external handle drops would use-after-free. `ConditionallySend::can_send` checks
            // `SkRefCntBase::unique()` safely via `skia-safe`; leak the device owner if not unique
            // (`VulkanDeviceOwner::drop` emits the single diagnostic warning).
            if !self.direct_context.can_send() {
                self.owner.skip_destroy = true;
            }
        }
    }

    #[cfg(test)]
    impl VulkanNativeSwapchain {
        pub(crate) fn direct_context_mut(&mut self) -> &mut DirectContext {
            &mut self.direct_context
        }

        pub(crate) fn leaked_owner_flag_for_test(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
            std::sync::Arc::clone(&self.owner.leaked_flag)
        }
    }

    impl NativeSwapchain for VulkanNativeSwapchain {
        const KIND: GpuBackendKind = GpuBackendKind::Vulkan;
        const WINDOW_READBACK_ERROR: &'static str =
            "window-attached Vulkan swapchain presents directly without CPU readback";

        fn is_window_attached(&self) -> bool {
            self.window.is_some()
        }

        fn offscreen_surface_and_context_mut(
            &mut self,
        ) -> Option<(&mut Surface, &mut DirectContext)> {
            if self.window.is_some() {
                return None;
            }
            self.surface
                .as_mut()
                .map(|surface| (surface, &mut self.direct_context))
        }

        fn resize_native(
            &mut self,
            prev: SwapchainExtent,
            next: SwapchainExtent,
        ) -> Result<bool, GpuSurfaceError> {
            if next.width == prev.width && next.height == prev.height {
                return Ok(false);
            }
            if self.window.is_some() {
                // Advance `GaneshPresenter::extent` to `next` even if recreation fails (e.g.
                // 0x0 minimized or OOM); `swapchain_dirty` stays `true` for `begin_native_frame`.
                Ok(self.recreate_window_swapchain(next).is_ok())
            } else {
                let _ = self.direct_context.flush_submit_and_sync_cpu();
                self.surface = Some(create_ganesh_offscreen_surface(
                    &mut self.direct_context,
                    next,
                    "Vulkan",
                )?);
                Ok(true)
            }
        }

        fn begin_native_frame(
            &mut self,
            extent: SwapchainExtent,
            recreations: &mut u64,
        ) -> Result<(), GpuSurfaceError> {
            let needs_recreate = self.window.as_ref().is_some_and(|win| {
                win.swapchain_dirty || win.swapchain_khr.is_none() || win.sync_frames.is_empty()
            });
            if needs_recreate {
                self.recreate_window_swapchain(extent)?;
                *recreations += 1;
            }

            if let Some(win) = self.window.as_mut() {
                let swapchain_khr = win
                    .swapchain_khr
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                let sync = &mut win.sync_frames[win.current_sync];
                if sync.in_flight_pending {
                    // SAFETY: `sync.in_flight` is a live fence owned by `self.owner.device`.
                    unsafe {
                        self.owner
                            .device
                            .wait_for_fences(&[sync.in_flight], true, u64::MAX)
                            .map_err(|e| {
                                GpuSurfaceError::PresentFailed(format!("fence wait: {e}"))
                            })?;
                    }
                    sync.in_flight_pending = false;
                }
                // SAFETY: `swapchain_khr` and `sync.image_available` are live handles on `self.owner.device`.
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
                    win.swapchain_dirty = true;
                    return Err(GpuSurfaceError::SurfaceCreationFailed(
                        "wrap_backend_render_target returned None for VkImage".to_string(),
                    ));
                };

                win.active_image_index = Some(image_index);
                self.surface = Some(surface);
            }

            Ok(())
        }

        fn active_surface_and_context_mut(
            &mut self,
        ) -> Result<(&mut Surface, &mut DirectContext), GpuSurfaceError> {
            let s = self
                .surface
                .as_mut()
                .ok_or(GpuSurfaceError::DrawableUnavailable)?;
            Ok((s, &mut self.direct_context))
        }

        fn submit_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
            if let Some(win) = self.window.as_mut() {
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
                let mut surface = self
                    .surface
                    .take()
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                match self.direct_context.flush_surface_with_texture_state(
                    &mut surface,
                    &flush_info,
                    Some(&present_state),
                ) {
                    Ok(SemaphoresSubmitted::Yes) => {}
                    Ok(SemaphoresSubmitted::No) => {
                        win.swapchain_dirty = true;
                        return Err(GpuSurfaceError::FlushFailed(
                            "DirectContext did not submit render_finished semaphore".to_string(),
                        ));
                    }
                    Err(e) => {
                        win.swapchain_dirty = true;
                        return Err(GpuSurfaceError::FlushFailed(e.to_string()));
                    }
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
            } else {
                let surface = self
                    .surface
                    .as_mut()
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                self.direct_context
                    .flush_and_submit_surface(surface, SyncCpu::Yes)
                    .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
            }
            Ok(())
        }

        fn present_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
            if let Some(win) = self.window.as_mut() {
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
                // SAFETY: `self.queue`, `swapchain_khr`, `sync.render_finished`, and `image_index` are valid handles.
                let present_err =
                    match unsafe { win.swapchain_loader.queue_present(self.queue, &present_info) } {
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
                    // SAFETY: `self.owner.device` is live.
                    unsafe {
                        let _ = self.owner.device.device_wait_idle();
                    }
                    sync.in_flight_pending = false;
                    return Err(err);
                }
                // SAFETY: `self.owner.device`, `self.queue`, and `sync.in_flight` are valid live handles.
                let fence_res = unsafe {
                    self.owner
                        .device
                        .reset_fences(&[sync.in_flight])
                        .map_err(|e| GpuSurfaceError::PresentFailed(format!("fence reset: {e}")))
                        .and_then(|()| {
                            self.owner
                                .device
                                .queue_submit(self.queue, &[], sync.in_flight)
                                .map_err(|e| {
                                    GpuSurfaceError::PresentFailed(format!("fence submit: {e}"))
                                })
                        })
                };
                if let Err(err) = fence_res {
                    win.swapchain_dirty = true;
                    // SAFETY: `self.owner.device` is live.
                    unsafe {
                        let _ = self.owner.device.device_wait_idle();
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
            if let Some(win) = self.window.as_mut() {
                if win.active_image_index.is_some() {
                    win.swapchain_dirty = true;
                }
                self.surface.take();
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
            let instance_guard = InstanceGuard::new(instance);
            let (physical_device, queue_family_index) =
                pick_physical_device(instance_guard.get(), None, &[])?;
            let (device, queue, device_exts) =
                create_device(instance_guard.get(), physical_device, queue_family_index, &[])?;
            let device_guard = DeviceGuard::new(device);
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
                    drop(direct_context);
                    return Err(err);
                }
            };
            Ok(Self::new(
                VulkanNativeSwapchain {
                    surface: Some(surface),
                    direct_context,
                    window: None,
                    queue,
                    queue_family_index,
                    physical_device,
                    owner: VulkanDeviceOwner {
                        device: device_guard.disarm(),
                        instance: instance_guard.disarm(),
                        entry: ManuallyDrop::new(entry),
                        skip_destroy: false,
                        #[cfg(test)]
                        leaked_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    },
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
            let instance_guard = InstanceGuard::new(instance);
            let surface_loader = ash::khr::surface::Instance::new(&entry, instance_guard.get());
            let surface_khr = create_vk_surface(
                &entry,
                instance_guard.get(),
                disp_handle.as_raw(),
                win_handle.as_raw(),
            )?;
            let surface_guard = SurfaceGuard::new(&surface_loader, surface_khr);

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
            let device_guard = DeviceGuard::new(device);

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
            let swapchain_guard = SwapchainGuard::new(&swapchain_loader, created.swapchain);

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
                    surface: None,
                    direct_context,
                    window: Some(Box::new(VulkanWindowState {
                        device: device.clone(),
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
                        swapchain_dirty: false,
                        vsync: vk_cfg.vsync,
                        min_image_count: vk_cfg.min_image_count,
                    })),
                    queue,
                    queue_family_index,
                    physical_device,
                    owner: VulkanDeviceOwner {
                        device,
                        instance,
                        entry: ManuallyDrop::new(entry),
                        skip_destroy: false,
                        #[cfg(test)]
                        leaked_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    },
                },
                vk_cfg.extent,
            ))
        }
    }
}

#[cfg(feature = "skia-vulkan")]
pub use imp::{VulkanNativeSwapchain, VulkanSwapchain};
