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
mod imp {
    use std::ffi::{CStr, CString};
    use std::ptr;
    use std::time::Instant;

    use ash::vk as ash_vk;
    use ash::vk::Handle as AshHandle;
    use nve_abi::ColorRgba;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    use skia_safe::{
        gpu::{
            self, backend_render_targets, backend_semaphores, direct_contexts, surfaces, vk,
            DirectContext, FlushInfo, Protected, SubmitInfo, SurfaceOrigin, SyncCpu,
        },
        ColorType, ImageInfo, Surface,
    };

    use super::{VulkanSemaphoreSyncHooks, VulkanSwapchainConfig};
    use crate::gpu::skia_renderer::{
        read_surface_to_pixel_buffer, to_skia_color, SkiaCanvasRenderer,
    };
    use crate::gpu::{
        FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig,
        GpuSurfaceError, SwapchainExtent,
    };
    use crate::raster::PixelBuffer;
    use crate::DisplayList;

    struct FrameSyncObjects {
        image_available: ash_vk::Semaphore,
        render_finished: ash_vk::Semaphore,
        in_flight: ash_vk::Fence,
    }

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

    /// Real Vulkan hardware GPU swapchain & offscreen render-target presenter.
    pub struct VulkanSwapchain {
        // Drop order: Skia surfaces & DirectContext must drop BEFORE Vulkan device/instance.
        target: VulkanTarget,
        direct_context: DirectContext,
        queue: ash_vk::Queue,
        queue_family_index: u32,
        physical_device: ash_vk::PhysicalDevice,
        device: ash::Device,
        instance: ash::Instance,
        _entry: ash::Entry,
        renderer: SkiaCanvasRenderer,
        extent: SwapchainExtent,
        state: FrameLifecycleState,
        frame_index: u64,
        swapchain_recreations: u64,
        frame_start: Option<Instant>,
        acquire_us: u64,
        record_us: u64,
        submit_us: u64,
        last_telemetry: Option<FrameTelemetry>,
        last_sync_hooks: VulkanSemaphoreSyncHooks,
        readback_rgba: Vec<u8>,
    }

    fn load_vulkan_entry() -> Result<ash::Entry, GpuSurfaceError> {
        // SAFETY: `ash::Entry::load` opens the platform Vulkan loader library via `dlopen`
        // (`libvulkan.so.1` / `libMoltenVK.dylib` / `vulkan-1.dll`) with standard OS semantics.
        unsafe { ash::Entry::load() }.map_err(|e| {
            GpuSurfaceError::DeviceUnavailable(format!("failed to load Vulkan loader: {e}"))
        })
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
            match &mut self.target {
                VulkanTarget::Offscreen { surface } => {
                    let canvas = surface.canvas();
                    canvas.clear(to_skia_color(clear_color));
                    renderer.paint_display_list(display_list, canvas, scale_factor);
                    self.direct_context
                        .flush_and_submit_surface(surface, SyncCpu::Yes)
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                    Ok(())
                }
                VulkanTarget::Window(_) => {
                    self.execute_frame(display_list, scale_factor, clear_color, None)
                        .map(|_| ())
                }
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

        fn recreate_window_swapchain(&mut self, next: SwapchainExtent) -> Result<(), GpuSurfaceError> {
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

    impl GpuFramePresenter for VulkanSwapchain {
        fn kind(&self) -> GpuBackendKind {
            GpuBackendKind::Vulkan
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

            match &mut self.target {
                VulkanTarget::Window(_) => {
                    self.recreate_window_swapchain(next)?;
                }
                VulkanTarget::Offscreen { surface } => {
                    if next.width != self.extent.width || next.height != self.extent.height {
                        let _ = self.direct_context.flush_submit_and_sync_cpu();
                        *surface = create_offscreen_surface(&mut self.direct_context, next)?;
                    }
                }
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
            self.record_us = 0;
            self.submit_us = 0;

            let needs_recreate = matches!(
                &self.target,
                VulkanTarget::Window(win) if win.swapchain_dirty
            );
            if needs_recreate {
                self.recreate_window_swapchain(self.extent)?;
                self.swapchain_recreations += 1;
            }

            if let VulkanTarget::Window(win) = &mut self.target {
                let sync = &win.sync_frames[win.current_sync];
                unsafe {
                    self.device
                        .wait_for_fences(&[sync.in_flight], true, u64::MAX)
                        .map_err(|e| GpuSurfaceError::PresentFailed(format!("fence wait: {e}")))?;
                    self.device
                        .reset_fences(&[sync.in_flight])
                        .map_err(|e| GpuSurfaceError::PresentFailed(format!("fence reset: {e}")))?;
                }
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
                        )))
                    }
                };
                if suboptimal {
                    win.swapchain_dirty = true;
                }

                let image = win.images[image_index as usize];
                let vk_format = map_ash_to_skia_format(win.format);
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
                    (self.extent.width as i32, self.extent.height as i32),
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
                let acquire_sem =
                    unsafe { backend_semaphores::make_vk(sync.image_available.as_raw() as _) };
                surface.wait(&[acquire_sem], false);

                self.last_sync_hooks = VulkanSemaphoreSyncHooks {
                    image_available_semaphore: sync.image_available.as_raw(),
                    render_finished_semaphore: sync.render_finished.as_raw(),
                    in_flight_fence: sync.in_flight.as_raw(),
                    queue_family_index: self.queue_family_index,
                };
                win.active_image_index = Some(image_index);
                win.active_surface = Some(surface);
            }

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
            if self.state != FrameLifecycleState::Begun {
                return Err(GpuSurfaceError::InvalidFrameState {
                    expected: FrameLifecycleState::Begun.as_str(),
                    actual: self.state.as_str(),
                });
            }
            let t0 = Instant::now();
            let surface = match &mut self.target {
                VulkanTarget::Window(win) => win
                    .active_surface
                    .as_mut()
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?,
                VulkanTarget::Offscreen { surface } => surface,
            };
            let canvas = surface.canvas();
            canvas.clear(to_skia_color(clear_color));
            self.renderer
                .paint_display_list(display_list, canvas, scale_factor);
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
            match &mut self.target {
                VulkanTarget::Window(win) => {
                    let sync = &win.sync_frames[win.current_sync];
                    let mut signal_sem = [unsafe {
                        backend_semaphores::make_vk(sync.render_finished.as_raw() as _)
                    }];
                    let mut flush_info = FlushInfo::default();
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
                        )))
                    }
                }
                // Signal the frame fence once queued work drains so the next ring slot stays safe.
                unsafe {
                    let _ = self.device.queue_submit(self.queue, &[], sync.in_flight);
                }
                win.current_sync = (win.current_sync + 1) % win.sync_frames.len();
            }

            self.frame_index += 1;
            let present_us = t_present.elapsed().as_micros() as u64;
            let total_us = self.frame_start.take().map_or(
                self.acquire_us + self.record_us + self.submit_us + present_us,
                |t| t.elapsed().as_micros() as u64,
            );
            let telemetry = FrameTelemetry {
                backend: GpuBackendKind::Vulkan,
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
            if let VulkanTarget::Window(win) = &mut self.target {
                win.active_surface.take();
                win.active_image_index = None;
            }
            self.frame_start = None;
            self.state = FrameLifecycleState::Idle;
        }
    }

    fn required_surface_extensions(
        display: RawDisplayHandle,
    ) -> Result<Vec<&'static CStr>, GpuSurfaceError> {
        let mut exts = vec![ash::khr::surface::NAME];
        match display {
            #[cfg(target_os = "macos")]
            RawDisplayHandle::AppKit(_) => exts.push(ash::ext::metal_surface::NAME),
            #[cfg(target_os = "ios")]
            RawDisplayHandle::UiKit(_) => exts.push(ash::ext::metal_surface::NAME),
            #[cfg(target_os = "windows")]
            RawDisplayHandle::Windows(_) => exts.push(ash::khr::win32_surface::NAME),
            #[cfg(target_os = "android")]
            RawDisplayHandle::Android(_) => exts.push(ash::khr::android_surface::NAME),
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            RawDisplayHandle::Xlib(_) => exts.push(ash::khr::xlib_surface::NAME),
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            RawDisplayHandle::Xcb(_) => exts.push(ash::khr::xcb_surface::NAME),
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            RawDisplayHandle::Wayland(_) => exts.push(ash::khr::wayland_surface::NAME),
            other => {
                return Err(GpuSurfaceError::WindowHandleError(format!(
                    "unsupported RawDisplayHandle for Vulkan surface: {other:?}"
                )));
            }
        }
        Ok(exts)
    }

    fn create_instance(
        entry: &ash::Entry,
        extra_exts: &[&'static CStr],
    ) -> Result<(ash::Instance, Vec<String>), GpuSurfaceError> {
        let app_name = CString::new("nve-backend-skia").unwrap();
        let app_info = ash_vk::ApplicationInfo::default()
            .application_name(&app_name)
            .engine_name(&app_name)
            .api_version(ash_vk::API_VERSION_1_1);

        let available = unsafe { entry.enumerate_instance_extension_properties(None) }
            .unwrap_or_default();
        let has_portability = available.iter().any(|p| unsafe {
            CStr::from_ptr(p.extension_name.as_ptr()) == ash::khr::portability_enumeration::NAME
        });

        let mut ext_ptrs: Vec<*const i8> = extra_exts.iter().map(|e| e.as_ptr()).collect();
        let mut ext_names: Vec<String> = extra_exts
            .iter()
            .map(|e| e.to_string_lossy().into_owned())
            .collect();
        let mut flags = ash_vk::InstanceCreateFlags::empty();
        if has_portability {
            ext_ptrs.push(ash::khr::portability_enumeration::NAME.as_ptr());
            ext_names.push(
                ash::khr::portability_enumeration::NAME
                    .to_string_lossy()
                    .into_owned(),
            );
            flags |= ash_vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
        }

        let create_info = ash_vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&ext_ptrs)
            .flags(flags);
        let instance = unsafe { entry.create_instance(&create_info, None) }.map_err(|e| {
            GpuSurfaceError::DeviceUnavailable(format!("vkCreateInstance failed: {e}"))
        })?;
        Ok((instance, ext_names))
    }

    fn pick_physical_device(
        instance: &ash::Instance,
        surface: Option<(&ash::khr::surface::Instance, ash_vk::SurfaceKHR)>,
        required_device_exts: &[&'static CStr],
    ) -> Result<(ash_vk::PhysicalDevice, u32), GpuSurfaceError> {
        let devices = unsafe { instance.enumerate_physical_devices() }.map_err(|e| {
            GpuSurfaceError::DeviceUnavailable(format!("vkEnumeratePhysicalDevices failed: {e}"))
        })?;

        let mut best: Option<(u32, ash_vk::PhysicalDevice, u32)> = None;
        for pdev in devices {
            let exts = unsafe { instance.enumerate_device_extension_properties(pdev) }
                .unwrap_or_default();
            let supports_all = required_device_exts.iter().all(|req| {
                exts.iter().any(|avail| unsafe {
                    CStr::from_ptr(avail.extension_name.as_ptr()) == *req
                })
            });
            if !supports_all {
                continue;
            }
            let queues = unsafe { instance.get_physical_device_queue_family_properties(pdev) };
            let found_queue = queues.iter().enumerate().find_map(|(idx, props)| {
                if !props.queue_flags.contains(ash_vk::QueueFlags::GRAPHICS) {
                    return None;
                }
                if let Some((surf_loader, surf_khr)) = surface {
                    let ok = unsafe {
                        surf_loader.get_physical_device_surface_support(pdev, idx as u32, surf_khr)
                    }
                    .unwrap_or(false);
                    if !ok {
                        return None;
                    }
                }
                Some(idx as u32)
            });
            let Some(q_idx) = found_queue else {
                continue;
            };
            let props = unsafe { instance.get_physical_device_properties(pdev) };
            let rank = match props.device_type {
                ash_vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                ash_vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                ash_vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
                ash_vk::PhysicalDeviceType::CPU => 3,
                _ => 4,
            };
            if best.is_none_or(|(best_rank, _, _)| rank < best_rank) {
                best = Some((rank, pdev, q_idx));
            }
        }

        best.map(|(_, pdev, q_idx)| (pdev, q_idx)).ok_or_else(|| {
            GpuSurfaceError::DeviceUnavailable("no suitable Vulkan physical device found".to_string())
        })
    }

    fn create_device(
        instance: &ash::Instance,
        physical_device: ash_vk::PhysicalDevice,
        queue_family_index: u32,
        required_exts: &[&'static CStr],
    ) -> Result<(ash::Device, ash_vk::Queue, Vec<String>), GpuSurfaceError> {
        let priorities = [1.0f32];
        let queue_infos = [ash_vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)];
        let ext_ptrs: Vec<*const i8> = required_exts.iter().map(|e| e.as_ptr()).collect();
        let ext_names: Vec<String> = required_exts
            .iter()
            .map(|e| e.to_string_lossy().into_owned())
            .collect();
        let create_info = ash_vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&ext_ptrs);
        let device = unsafe { instance.create_device(physical_device, &create_info, None) }
            .map_err(|e| {
                GpuSurfaceError::DeviceUnavailable(format!("vkCreateDevice failed: {e}"))
            })?;
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        Ok((device, queue, ext_names))
    }

    #[allow(clippy::too_many_arguments)]
    fn create_skia_vulkan_context(
        entry: &ash::Entry,
        instance: &ash::Instance,
        physical_device: ash_vk::PhysicalDevice,
        device: &ash::Device,
        queue: ash_vk::Queue,
        queue_family_index: u32,
        instance_exts: &[String],
        device_exts: &[String],
    ) -> Result<DirectContext, GpuSurfaceError> {
        let get_proc = |gpo: vk::GetProcOf| unsafe {
            let f = match gpo {
                vk::GetProcOf::Instance(inst, name) => {
                    let vk_inst = ash_vk::Instance::from_raw(inst as _);
                    entry.get_instance_proc_addr(vk_inst, name)
                }
                vk::GetProcOf::Device(dev, name) => {
                    let vk_dev = ash_vk::Device::from_raw(dev as _);
                    (instance.fp_v1_0().get_device_proc_addr)(vk_dev, name)
                }
            };
            f.map(|fp| fp as *const std::ffi::c_void)
                .unwrap_or(ptr::null())
        };

        let inst_ext_refs: Vec<&str> = instance_exts.iter().map(String::as_str).collect();
        let dev_ext_refs: Vec<&str> = device_exts.iter().map(String::as_str).collect();
        let backend_context = unsafe {
            vk::BackendContext::new_builder(
                instance.handle().as_raw() as _,
                physical_device.as_raw() as _,
                device.handle().as_raw() as _,
                (queue.as_raw() as _, queue_family_index as usize),
                &get_proc,
                Some((1, 1, 0).into()),
            )
            .with_extensions(&inst_ext_refs, &dev_ext_refs)
            .build()
        };

        direct_contexts::make_vulkan(&backend_context, None).ok_or_else(|| {
            GpuSurfaceError::SurfaceCreationFailed(
                "skia_safe::gpu::direct_contexts::make_vulkan returned None".to_string(),
            )
        })
    }

    fn create_offscreen_surface(
        direct_context: &mut DirectContext,
        extent: SwapchainExtent,
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
                "Vulkan GPU render_target returned None for {}x{}",
                extent.width, extent.height
            ))
        })
    }

    fn create_vk_surface(
        entry: &ash::Entry,
        instance: &ash::Instance,
        display: RawDisplayHandle,
        window: RawWindowHandle,
    ) -> Result<ash_vk::SurfaceKHR, GpuSurfaceError> {
        match (display, window) {
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            (RawDisplayHandle::Xlib(d), RawWindowHandle::Xlib(w)) => {
                let dpy = d.display.ok_or_else(|| {
                    GpuSurfaceError::WindowHandleError("Xlib display pointer is null".to_string())
                })?;
                let info = ash_vk::XlibSurfaceCreateInfoKHR::default()
                    .dpy(dpy.as_ptr() as *mut _)
                    .window(w.window);
                let loader = ash::khr::xlib_surface::Instance::new(entry, instance);
                unsafe { loader.create_xlib_surface(&info, None) }
                    .map_err(|e| GpuSurfaceError::SurfaceCreationFailed(format!("xlib surface: {e}")))
            }
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            (RawDisplayHandle::Xcb(d), RawWindowHandle::Xcb(w)) => {
                let conn = d.connection.ok_or_else(|| {
                    GpuSurfaceError::WindowHandleError("Xcb connection pointer is null".to_string())
                })?;
                let info = ash_vk::XcbSurfaceCreateInfoKHR::default()
                    .connection(conn.as_ptr())
                    .window(w.window.get());
                let loader = ash::khr::xcb_surface::Instance::new(entry, instance);
                unsafe { loader.create_xcb_surface(&info, None) }
                    .map_err(|e| GpuSurfaceError::SurfaceCreationFailed(format!("xcb surface: {e}")))
            }
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) => {
                let info = ash_vk::WaylandSurfaceCreateInfoKHR::default()
                    .display(d.display.as_ptr())
                    .surface(w.surface.as_ptr());
                let loader = ash::khr::wayland_surface::Instance::new(entry, instance);
                unsafe { loader.create_wayland_surface(&info, None) }.map_err(|e| {
                    GpuSurfaceError::SurfaceCreationFailed(format!("wayland surface: {e}"))
                })
            }
            #[cfg(target_os = "windows")]
            (RawDisplayHandle::Windows(_), RawWindowHandle::Win32(w)) => {
                let hinstance = w
                    .hinstance
                    .map(|h| h.get() as isize)
                    .unwrap_or(0);
                let info = ash_vk::Win32SurfaceCreateInfoKHR::default()
                    .hinstance(hinstance)
                    .hwnd(w.hwnd.get());
                let loader = ash::khr::win32_surface::Instance::new(entry, instance);
                unsafe { loader.create_win32_surface(&info, None) }.map_err(|e| {
                    GpuSurfaceError::SurfaceCreationFailed(format!("win32 surface: {e}"))
                })
            }
            #[cfg(target_os = "android")]
            (RawDisplayHandle::Android(_), RawWindowHandle::AndroidNdk(w)) => {
                let info = ash_vk::AndroidSurfaceCreateInfoKHR::default()
                    .window(w.a_native_window.as_ptr());
                let loader = ash::khr::android_surface::Instance::new(entry, instance);
                unsafe { loader.create_android_surface(&info, None) }.map_err(|e| {
                    GpuSurfaceError::SurfaceCreationFailed(format!("android surface: {e}"))
                })
            }
            #[cfg(target_os = "macos")]
            (RawDisplayHandle::AppKit(_), RawWindowHandle::AppKit(w)) => {
                let info = ash_vk::MetalSurfaceCreateInfoEXT::default()
                    .layer(w.ns_view.as_ptr() as *const _);
                let loader = ash::ext::metal_surface::Instance::new(entry, instance);
                unsafe { loader.create_metal_surface(&info, None) }.map_err(|e| {
                    GpuSurfaceError::SurfaceCreationFailed(format!("metal vk surface: {e}"))
                })
            }
            (d, w) => {
                let _ = (entry, instance);
                Err(GpuSurfaceError::WindowHandleError(format!(
                    "unsupported display/window handle pair for Vulkan: ({d:?}, {w:?})"
                )))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn create_vk_swapchain(
        surface_loader: &ash::khr::surface::Instance,
        swapchain_loader: &ash::khr::swapchain::Device,
        physical_device: ash_vk::PhysicalDevice,
        surface: ash_vk::SurfaceKHR,
        extent: SwapchainExtent,
        vsync: bool,
        requested_images: u32,
        old_swapchain: ash_vk::SwapchainKHR,
    ) -> Result<(ash_vk::SwapchainKHR, Vec<ash_vk::Image>, ash_vk::Format, ColorType), GpuSurfaceError>
    {
        let caps = unsafe {
            surface_loader.get_physical_device_surface_capabilities(physical_device, surface)
        }
        .map_err(|e| GpuSurfaceError::SurfaceCreationFailed(format!("surface caps: {e}")))?;
        let formats =
            unsafe { surface_loader.get_physical_device_surface_formats(physical_device, surface) }
                .map_err(|e| {
                    GpuSurfaceError::SurfaceCreationFailed(format!("surface formats: {e}"))
                })?;

        let surface_fmt = formats
            .iter()
            .copied()
            .find(|f| {
                f.format == ash_vk::Format::B8G8R8A8_UNORM
                    || f.format == ash_vk::Format::R8G8B8A8_UNORM
            })
            .or_else(|| formats.first().copied())
            .ok_or_else(|| {
                GpuSurfaceError::SurfaceCreationFailed(
                    "Vulkan surface reported zero formats".to_string(),
                )
            })?;

        let color_type = match surface_fmt.format {
            ash_vk::Format::R8G8B8A8_UNORM | ash_vk::Format::R8G8B8A8_SRGB => ColorType::RGBA8888,
            _ => ColorType::BGRA8888,
        };

        let mut image_count = requested_images.max(caps.min_image_count).max(2);
        if caps.max_image_count > 0 && image_count > caps.max_image_count {
            image_count = caps.max_image_count;
        }

        let present_mode = if vsync {
            ash_vk::PresentModeKHR::FIFO
        } else {
            let modes = unsafe {
                surface_loader.get_physical_device_surface_present_modes(physical_device, surface)
            }
            .unwrap_or_default();
            if modes.contains(&ash_vk::PresentModeKHR::MAILBOX) {
                ash_vk::PresentModeKHR::MAILBOX
            } else if modes.contains(&ash_vk::PresentModeKHR::IMMEDIATE) {
                ash_vk::PresentModeKHR::IMMEDIATE
            } else {
                ash_vk::PresentModeKHR::FIFO
            }
        };

        let vk_extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            ash_vk::Extent2D {
                width: extent
                    .width
                    .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: extent
                    .height
                    .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };

        let info = ash_vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(surface_fmt.format)
            .image_color_space(surface_fmt.color_space)
            .image_extent(vk_extent)
            .image_array_layers(1)
            .image_usage(ash_vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(ash_vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(ash_vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain);

        let swapchain = unsafe { swapchain_loader.create_swapchain(&info, None) }.map_err(|e| {
            GpuSurfaceError::SurfaceCreationFailed(format!("vkCreateSwapchainKHR failed: {e}"))
        })?;
        let images = unsafe { swapchain_loader.get_swapchain_images(swapchain) }.map_err(|e| {
            GpuSurfaceError::SurfaceCreationFailed(format!("vkGetSwapchainImagesKHR failed: {e}"))
        })?;
        Ok((swapchain, images, surface_fmt.format, color_type))
    }

    fn create_sync_objects(
        device: &ash::Device,
        count: usize,
    ) -> Result<Vec<FrameSyncObjects>, GpuSurfaceError> {
        let mut out = Vec::with_capacity(count);
        let sem_info = ash_vk::SemaphoreCreateInfo::default();
        let fence_info = ash_vk::FenceCreateInfo::default().flags(ash_vk::FenceCreateFlags::SIGNALED);
        for _ in 0..count {
            let image_available = unsafe { device.create_semaphore(&sem_info, None) }.map_err(
                |e| GpuSurfaceError::SurfaceCreationFailed(format!("create_semaphore: {e}")),
            )?;
            let render_finished = unsafe { device.create_semaphore(&sem_info, None) }.map_err(
                |e| GpuSurfaceError::SurfaceCreationFailed(format!("create_semaphore: {e}")),
            )?;
            let in_flight = unsafe { device.create_fence(&fence_info, None) }.map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!("create_fence: {e}"))
            })?;
            out.push(FrameSyncObjects {
                image_available,
                render_finished,
                in_flight,
            });
        }
        Ok(out)
    }

    fn map_ash_to_skia_format(fmt: ash_vk::Format) -> vk::Format {
        match fmt {
            ash_vk::Format::R8G8B8A8_UNORM => vk::Format::R8G8B8A8_UNORM,
            ash_vk::Format::R8G8B8A8_SRGB => vk::Format::R8G8B8A8_SRGB,
            ash_vk::Format::B8G8R8A8_SRGB => vk::Format::B8G8R8A8_SRGB,
            _ => vk::Format::B8G8R8A8_UNORM,
        }
    }
}

#[cfg(feature = "skia-vulkan")]
pub use imp::VulkanSwapchain;
