//! `GpuFramePresenter` frame-lifecycle implementation for [`super::VulkanSwapchain`].

use std::time::Instant;

use ash::vk as ash_vk;
use ash::vk::Handle as AshHandle;
use nve_abi::ColorRgba;
use skia_safe::gpu::{
    backend_render_targets, backend_semaphores, surfaces, vk, FlushInfo, Protected, SubmitInfo,
    SurfaceOrigin, SyncCpu,
};

use super::imp::{VulkanSwapchain, VulkanTarget};
use super::setup::{create_offscreen_surface, map_ash_to_skia_format};
use super::VulkanSemaphoreSyncHooks;
use crate::gpu::skia_renderer::to_skia_color;
use crate::gpu::{
    FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceError,
    SkiaPluginContext, SwapchainExtent,
};
use crate::DisplayList;

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
            .paint_display_list_opt_plugins(display_list, canvas, scale_factor, plugins);
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
