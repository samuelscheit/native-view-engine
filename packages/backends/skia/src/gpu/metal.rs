//! Apple Metal hardware GPU swapchain and offscreen render target (`--features skia-metal`).
//!
//! Uses `MTLCreateSystemDefaultDevice` + `MTLCommandQueue` + `CAMetalLayer` +
//! `skia_safe::gpu::direct_contexts::make_metal` + `backend_render_targets::make_mtl` (with
//! `wrap_mtk_view` helper for native `MTKView` embedding) and HiDPI `contentsScale` /
//! `drawableSize` synchronization.

use super::{GpuSurfaceConfig, SwapchainExtent};

/// Configuration summary for the Apple Metal swapchain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetalSwapchainConfig {
    pub extent: SwapchainExtent,
    pub vsync: bool,
    pub maximum_drawable_count: u32,
    pub presents_with_transaction: bool,
}

impl MetalSwapchainConfig {
    pub fn from_surface_config(
        config: GpuSurfaceConfig,
    ) -> Result<Self, super::GpuSurfaceError> {
        let extent = config.extent()?;
        Ok(Self {
            extent,
            vsync: config.vsync,
            maximum_drawable_count: config.swapchain_image_count.clamp(2, 3),
            presents_with_transaction: false,
        })
    }
}

#[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
mod imp {
    use std::time::Instant;

    use nve_abi::ColorRgba;
    use objc2::{
        rc::{autoreleasepool, Retained},
        runtime::ProtocolObject,
    };
    use objc2_core_foundation::CGSize;
    use objc2_metal::{
        MTLCommandBuffer, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable,
        MTLPixelFormat,
    };
    use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use skia_safe::{
        gpu::{self, backend_render_targets, direct_contexts, mtl, surfaces, DirectContext, SurfaceOrigin, SyncCpu},
        ColorType, ImageInfo, Surface,
    };

    #[cfg(target_os = "macos")]
    use objc2_app_kit::NSView;
    #[cfg(target_os = "ios")]
    use objc2_ui_kit::UIView;

    use super::MetalSwapchainConfig;
    use crate::gpu::skia_renderer::{read_surface_to_pixel_buffer, to_skia_color, SkiaCanvasRenderer};
    use crate::gpu::{
        FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig,
        GpuSurfaceError, SwapchainExtent,
    };
    use crate::raster::PixelBuffer;
    use crate::DisplayList;

    enum MetalTarget {
        /// Attached to a native window's `CAMetalLayer` (zero-copy hardware presentation).
        WindowLayer {
            layer: Retained<CAMetalLayer>,
            active_drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
            active_surface: Option<Surface>,
        },
        /// GPU-resident offscreen `SkSurface` render target backed by a Metal texture in VRAM.
        Offscreen { surface: Surface },
    }

    /// Real Apple Metal hardware GPU swapchain & offscreen render-target presenter.
    pub struct MetalSwapchain {
        // Drop order matters: Skia surfaces & DirectContext must drop before device/queue/layer.
        target: MetalTarget,
        direct_context: DirectContext,
        command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
        _device: Retained<ProtocolObject<dyn MTLDevice>>,
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
        readback_rgba: Vec<u8>,
    }

    impl MetalSwapchain {
        /// Creates an offscreen hardware Metal GPU context and GPU-backed Skia render target.
        pub fn new_offscreen(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
            let mtl_cfg = MetalSwapchainConfig::from_surface_config(config)?;
            let (device, command_queue, mut direct_context) = create_metal_context()?;
            let surface = create_offscreen_surface(&mut direct_context, mtl_cfg.extent)?;
            Ok(Self {
                target: MetalTarget::Offscreen { surface },
                direct_context,
                command_queue,
                _device: device,
                renderer: SkiaCanvasRenderer::new(),
                extent: mtl_cfg.extent,
                state: FrameLifecycleState::Idle,
                frame_index: 0,
                swapchain_recreations: 0,
                frame_start: None,
                acquire_us: 0,
                record_us: 0,
                submit_us: 0,
                last_telemetry: None,
                readback_rgba: Vec::new(),
            })
        }

        /// Attaches a `CAMetalLayer` swapchain directly to a `raw-window-handle` window (`NSView`
        /// on macOS, `UIView` on iOS) with HiDPI backing scale factor and VSync configuration.
        pub fn from_window<W: HasWindowHandle>(
            window: &W,
            config: GpuSurfaceConfig,
        ) -> Result<Self, GpuSurfaceError> {
            let mtl_cfg = MetalSwapchainConfig::from_surface_config(config)?;
            let handle = window
                .window_handle()
                .map_err(|e| GpuSurfaceError::WindowHandleError(e.to_string()))?;
            let (device, command_queue, direct_context) = create_metal_context()?;
            let layer = create_and_attach_metal_layer(&device, handle.as_raw(), mtl_cfg)?;
            Ok(Self {
                target: MetalTarget::WindowLayer {
                    layer,
                    active_drawable: None,
                    active_surface: None,
                },
                direct_context,
                command_queue,
                _device: device,
                renderer: SkiaCanvasRenderer::new(),
                extent: mtl_cfg.extent,
                state: FrameLifecycleState::Idle,
                frame_index: 0,
                swapchain_recreations: 0,
                frame_start: None,
                acquire_us: 0,
                record_us: 0,
                submit_us: 0,
                last_telemetry: None,
                readback_rgba: Vec::new(),
            })
        }

        pub fn is_window_attached(&self) -> bool {
            matches!(self.target, MetalTarget::WindowLayer { .. })
        }

        pub fn direct_context_mut(&mut self) -> &mut DirectContext {
            &mut self.direct_context
        }

        /// Renders a [`DisplayList`] directly into a host-managed `MTKView*` using
        /// `skia_safe::gpu::surfaces::wrap_mtk_view`.
        ///
        /// # Safety
        /// `mtk_view` must be a valid, live `MTKView*` pointer on the main thread whose
        /// `currentDrawable` is ready for rendering.
        pub unsafe fn render_to_mtk_view(
            &mut self,
            mtk_view: mtl::Handle,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
            hud_banner: Option<&str>,
        ) -> Result<(), GpuSurfaceError> {
            if mtk_view.is_null() {
                return Err(GpuSurfaceError::SurfaceCreationFailed(
                    "MTKView pointer is null".to_string(),
                ));
            }
            let mut surface = unsafe {
                surfaces::wrap_mtk_view(
                    &mut self.direct_context,
                    mtk_view,
                    SurfaceOrigin::TopLeft,
                    None,
                    ColorType::BGRA8888,
                    None,
                    None,
                )
            }
            .ok_or_else(|| {
                GpuSurfaceError::SurfaceCreationFailed("wrap_mtk_view returned None".to_string())
            })?;
            let canvas = surface.canvas();
            canvas.clear(to_skia_color(clear_color));
            self.renderer
                .paint_display_list(display_list, canvas, scale_factor);
            if let Some(banner) = hud_banner {
                self.renderer.paint_hud_banner(canvas, scale_factor, banner);
            }
            self.direct_context
                .flush_and_submit_surface(&mut surface, SyncCpu::No)
                .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
            Ok(())
        }

        /// Direct helper for [`crate::gpu::GpuSurface`] (`GpuSurfaceBackend::render_display_list`)
        /// on an offscreen Metal GPU render target.
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
            plugins: Option<super::super::SkiaPluginContext<'_>>,
        ) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                MetalTarget::Offscreen { surface } => {
                    let canvas = surface.canvas();
                    canvas.clear(to_skia_color(clear_color));
                    renderer.paint_display_list_opt_plugins(display_list, canvas, scale_factor, plugins);
                    self.direct_context
                        .flush_and_submit_surface(surface, SyncCpu::Yes)
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                    Ok(())
                }
                MetalTarget::WindowLayer { .. } => self
                    .execute_frame_with_plugins(display_list, scale_factor, clear_color, None, plugins)
                    .map(|_| ()),
            }
        }

        /// Reads back pixels from an offscreen Metal GPU render target into `target`.
        pub fn copy_to_pixel_buffer(
            &mut self,
            target: &mut PixelBuffer,
        ) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                MetalTarget::Offscreen { surface } => {
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
                MetalTarget::WindowLayer { .. } => Err(GpuSurfaceError::ReadbackFailed(
                    "window-attached CAMetalLayer presents directly without CPU readback"
                        .to_string(),
                )),
            }
        }
    }

    impl GpuFramePresenter for MetalSwapchain {
        fn kind(&self) -> GpuBackendKind {
            GpuBackendKind::Metal
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
                MetalTarget::WindowLayer { layer, .. } => {
                    layer.setContentsScale(next.scale_factor as f64);
                    layer.setDrawableSize(CGSize::new(next.width as f64, next.height as f64));
                }
                MetalTarget::Offscreen { surface } => {
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

            match &mut self.target {
                MetalTarget::WindowLayer {
                    layer,
                    active_drawable,
                    active_surface,
                } => {
                    let (drawable, surface) = autoreleasepool(|_| {
                        let drawable = layer
                            .nextDrawable()
                            .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                        let drawable_size = layer.drawableSize();
                        let w = (drawable_size.width.round() as i32).max(1);
                        let h = (drawable_size.height.round() as i32).max(1);
                        let texture = drawable.texture();
                        let texture_info =
                            unsafe { mtl::TextureInfo::new(Retained::as_ptr(&texture) as mtl::Handle) };
                        let backend_rt = backend_render_targets::make_mtl((w, h), &texture_info);
                        let surface = surfaces::wrap_backend_render_target(
                            &mut self.direct_context,
                            &backend_rt,
                            SurfaceOrigin::TopLeft,
                            ColorType::BGRA8888,
                            None,
                            None,
                        )
                        .ok_or_else(|| {
                            GpuSurfaceError::SurfaceCreationFailed(
                                "wrap_backend_render_target returned None for CAMetalDrawable"
                                    .to_string(),
                            )
                        })?;
                        Ok::<_, GpuSurfaceError>((drawable, surface))
                    })?;
                    *active_drawable = Some(drawable);
                    *active_surface = Some(surface);
                }
                MetalTarget::Offscreen { .. } => {}
            }

            self.acquire_us = t0.elapsed().as_micros() as u64;
            self.state = FrameLifecycleState::Begun;
            Ok(())
        }

        fn render_display_list(
            &mut self,
            dl: &DisplayList,
            scale: f32,
            clear: ColorRgba,
            hud: Option<&str>,
        ) -> Result<(), GpuSurfaceError> {
            self.render_display_list_with_plugins(dl, scale, clear, hud, None)
        }

        fn render_display_list_with_plugins(
            &mut self,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
            hud_banner: Option<&str>,
            plugins: Option<super::super::SkiaPluginContext<'_>>,
        ) -> Result<(), GpuSurfaceError> {
            if self.state != FrameLifecycleState::Begun {
                return Err(GpuSurfaceError::InvalidFrameState {
                    expected: FrameLifecycleState::Begun.as_str(),
                    actual: self.state.as_str(),
                });
            }
            let t0 = Instant::now();
            let surface = match &mut self.target {
                MetalTarget::WindowLayer { active_surface, .. } => active_surface
                    .as_mut()
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?,
                MetalTarget::Offscreen { surface } => surface,
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
            let flush_res = match &mut self.target {
                MetalTarget::WindowLayer { active_surface, .. } => {
                    let res = self.direct_context.flush_and_submit();
                    // Drop the wrapped Skia Surface before presenting the drawable so Skia
                    // releases its reference on the Metal texture.
                    active_surface.take();
                    res
                }
                MetalTarget::Offscreen { surface } => self
                    .direct_context
                    .flush_and_submit_surface(surface, SyncCpu::Yes),
            };
            flush_res.map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
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
            if let MetalTarget::WindowLayer {
                active_drawable, ..
            } = &mut self.target
            {
                let drawable = active_drawable
                    .take()
                    .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                autoreleasepool(|_| {
                    let cmd_buf = self.command_queue.commandBuffer().ok_or_else(|| {
                        GpuSurfaceError::PresentFailed(
                            "MTLCommandQueue::commandBuffer returned None".to_string(),
                        )
                    })?;
                    let mtl_drawable: Retained<ProtocolObject<dyn MTLDrawable>> =
                        (&drawable).into();
                    cmd_buf.presentDrawable(&mtl_drawable);
                    cmd_buf.commit();
                    Ok::<(), GpuSurfaceError>(())
                })?;
            }

            self.frame_index += 1;
            let present_us = t_present.elapsed().as_micros() as u64;
            let total_us = self.frame_start.take().map_or(
                self.acquire_us + self.record_us + self.submit_us + present_us,
                |t| t.elapsed().as_micros() as u64,
            );
            let telemetry = FrameTelemetry {
                backend: GpuBackendKind::Metal,
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
            if let MetalTarget::WindowLayer {
                active_drawable,
                active_surface,
                ..
            } = &mut self.target
            {
                active_surface.take();
                active_drawable.take();
            }
            self.frame_start = None;
            self.state = FrameLifecycleState::Idle;
        }
    }

    type MetalContextInit = (
        Retained<ProtocolObject<dyn MTLDevice>>,
        Retained<ProtocolObject<dyn MTLCommandQueue>>,
        DirectContext,
    );

    fn create_metal_context() -> Result<MetalContextInit, GpuSurfaceError> {
        let device = MTLCreateSystemDefaultDevice().ok_or_else(|| {
            GpuSurfaceError::DeviceUnavailable(
                "MTLCreateSystemDefaultDevice returned None".to_string(),
            )
        })?;
        let command_queue = device.newCommandQueue().ok_or_else(|| {
            GpuSurfaceError::DeviceUnavailable("MTLDevice::newCommandQueue returned None".to_string())
        })?;
        let backend = unsafe {
            mtl::BackendContext::new(
                Retained::as_ptr(&device) as mtl::Handle,
                Retained::as_ptr(&command_queue) as mtl::Handle,
            )
        };
        let direct_context = direct_contexts::make_metal(&backend, None).ok_or_else(|| {
            GpuSurfaceError::SurfaceCreationFailed(
                "skia_safe::gpu::direct_contexts::make_metal returned None".to_string(),
            )
        })?;
        Ok((device, command_queue, direct_context))
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
                "Metal GPU render_target returned None for {}x{}",
                extent.width, extent.height
            ))
        })
    }

    fn create_and_attach_metal_layer(
        device: &ProtocolObject<dyn MTLDevice>,
        raw_handle: RawWindowHandle,
        cfg: MetalSwapchainConfig,
    ) -> Result<Retained<CAMetalLayer>, GpuSurfaceError> {
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setPresentsWithTransaction(cfg.presents_with_transaction);
        // Required so Skia blend modes & readbacks can sample from the drawable texture.
        layer.setFramebufferOnly(false);
        layer.setContentsScale(cfg.extent.scale_factor as f64);
        layer.setDrawableSize(CGSize::new(
            cfg.extent.width as f64,
            cfg.extent.height as f64,
        ));
        #[cfg(target_os = "macos")]
        {
            layer.setDisplaySyncEnabled(cfg.vsync);
            layer.setMaximumDrawableCount(cfg.maximum_drawable_count as usize);
        }

        match raw_handle {
            #[cfg(target_os = "macos")]
            RawWindowHandle::AppKit(appkit) => {
                let view_ptr = appkit.ns_view.as_ptr() as *mut NSView;
                let view = unsafe { view_ptr.as_ref() }.ok_or_else(|| {
                    GpuSurfaceError::WindowHandleError("AppKit ns_view pointer is null".to_string())
                })?;
                view.setWantsLayer(true);
                view.setLayer(Some(&layer.clone().into_super()));
                Ok(layer)
            }
            #[cfg(target_os = "ios")]
            RawWindowHandle::UiKit(uikit) => {
                let view_ptr = uikit.ui_view.as_ptr() as *mut UIView;
                let view = unsafe { view_ptr.as_ref() }.ok_or_else(|| {
                    GpuSurfaceError::WindowHandleError("UiKit ui_view pointer is null".to_string())
                })?;
                layer.setFrame(view.layer().frame());
                view.layer().addSublayer(&layer);
                Ok(layer)
            }
            other => Err(GpuSurfaceError::WindowHandleError(format!(
                "unsupported RawWindowHandle for Metal: {other:?}"
            ))),
        }
    }
}

#[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
pub use imp::MetalSwapchain;
