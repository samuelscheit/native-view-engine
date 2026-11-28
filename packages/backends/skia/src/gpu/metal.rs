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
        gpu::{
            backend_render_targets, direct_contexts, mtl, surfaces, DirectContext, SurfaceOrigin,
            SyncCpu,
        },
        ColorType, Surface,
    };

    #[cfg(target_os = "macos")]
    use objc2_app_kit::NSView;
    #[cfg(target_os = "ios")]
    use objc2_ui_kit::UIView;

    use super::MetalSwapchainConfig;
    use crate::gpu::ganesh::{
        create_ganesh_offscreen_surface, GaneshPresenter, NativeSwapchain,
    };
    use crate::gpu::skia_renderer::to_skia_color;
    use crate::gpu::{GpuBackendKind, GpuSurfaceConfig, GpuSurfaceError, SwapchainExtent};
    use crate::DisplayList;

    enum MetalTarget {
        /// Attached to a native window's `CAMetalLayer` (zero-copy hardware presentation).
        /// Drop order: Skia `active_surface` drops BEFORE `active_drawable` which drops BEFORE `layer`.
        WindowLayer {
            active_surface: Option<Surface>,
            active_drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
            layer: Retained<CAMetalLayer>,
        },
        /// GPU-resident offscreen `SkSurface` render target backed by a Metal texture in VRAM.
        Offscreen { surface: Surface },
    }

    /// Platform-specific Apple Metal swapchain resources managed by [`GaneshPresenter`].
    pub struct MetalNativeSwapchain {
        // Drop order matters: Skia surfaces & DirectContext must drop BEFORE queue & device.
        target: MetalTarget,
        direct_context: DirectContext,
        command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
        _device: Retained<ProtocolObject<dyn MTLDevice>>,
    }

    impl Drop for MetalNativeSwapchain {
        fn drop(&mut self) {
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            if let MetalTarget::WindowLayer {
                active_surface,
                active_drawable,
                ..
            } = &mut self.target
            {
                // Window surfaces wrap a borrowed `CAMetalDrawable` texture and must drop BEFORE
                // the drawable and `release_resources_and_abandon()`.
                active_surface.take();
                active_drawable.take();
            }
            // Note: `MetalTarget::Offscreen { surface }` owns a Skia-managed Metal texture;
            // Rust drops `target` right after `Drop::drop` (before `direct_context`, `command_queue`,
            // and `_device`), and Skia safely releases offscreen surfaces on an abandoned context.
            self.direct_context.release_resources_and_abandon();
        }
    }

    /// Real Apple Metal hardware GPU swapchain & offscreen render-target presenter.
    pub type MetalSwapchain = GaneshPresenter<MetalNativeSwapchain>;

    impl NativeSwapchain for MetalNativeSwapchain {
        const KIND: GpuBackendKind = GpuBackendKind::Metal;
        const WINDOW_READBACK_ERROR: &'static str =
            "window-attached CAMetalLayer presents directly without CPU readback";

        fn is_window_attached(&self) -> bool {
            matches!(self.target, MetalTarget::WindowLayer { .. })
        }

        fn offscreen_surface_and_context_mut(
            &mut self,
        ) -> Option<(&mut Surface, &mut DirectContext)> {
            match &mut self.target {
                MetalTarget::Offscreen { surface } => Some((surface, &mut self.direct_context)),
                MetalTarget::WindowLayer { .. } => None,
            }
        }

        fn resize_native(
            &mut self,
            prev: SwapchainExtent,
            next: SwapchainExtent,
        ) -> Result<bool, GpuSurfaceError> {
            let dims_changed = next.width != prev.width || next.height != prev.height;
            match &mut self.target {
                MetalTarget::WindowLayer { layer, .. } => {
                    layer.setContentsScale(next.scale_factor as f64);
                    if dims_changed {
                        layer.setDrawableSize(CGSize::new(next.width as f64, next.height as f64));
                    }
                    Ok(dims_changed)
                }
                MetalTarget::Offscreen { surface } => {
                    if dims_changed {
                        let _ = self.direct_context.flush_submit_and_sync_cpu();
                        *surface = create_ganesh_offscreen_surface(
                            &mut self.direct_context,
                            next,
                            "Metal",
                        )?;
                    }
                    Ok(dims_changed)
                }
            }
        }

        fn begin_native_frame(
            &mut self,
            _extent: SwapchainExtent,
            _recreations: &mut u64,
        ) -> Result<(), GpuSurfaceError> {
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
                        // SAFETY: `drawable` retains `texture` for the lifetime of the wrapped surface.
                        let texture_info = unsafe {
                            mtl::TextureInfo::new(Retained::as_ptr(&texture) as mtl::Handle)
                        };
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
            Ok(())
        }

        fn active_surface_and_context_mut(
            &mut self,
        ) -> Result<(&mut Surface, &mut DirectContext), GpuSurfaceError> {
            match &mut self.target {
                MetalTarget::WindowLayer { active_surface, .. } => active_surface
                    .as_mut()
                    .map(|s| (s, &mut self.direct_context))
                    .ok_or(GpuSurfaceError::DrawableUnavailable),
                MetalTarget::Offscreen { surface } => Ok((surface, &mut self.direct_context)),
            }
        }

        fn submit_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
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
            flush_res
                .map(|_| ())
                .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))
        }

        fn present_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
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
            Ok(())
        }

        fn abort_native_frame(&mut self) {
            if let MetalTarget::WindowLayer {
                active_drawable,
                active_surface,
                ..
            } = &mut self.target
            {
                active_surface.take();
                active_drawable.take();
            }
        }
    }

    impl MetalSwapchain {
        /// Creates an offscreen hardware Metal GPU context and GPU-backed Skia render target.
        pub fn new_offscreen(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
            let mtl_cfg = MetalSwapchainConfig::from_surface_config(config)?;
            let (device, command_queue, mut direct_context) = create_metal_context()?;
            let surface =
                create_ganesh_offscreen_surface(&mut direct_context, mtl_cfg.extent, "Metal")?;
            Ok(Self::new(
                MetalNativeSwapchain {
                    target: MetalTarget::Offscreen { surface },
                    direct_context,
                    command_queue,
                    _device: device,
                },
                mtl_cfg.extent,
            ))
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
            Ok(Self::new(
                MetalNativeSwapchain {
                    target: MetalTarget::WindowLayer {
                        layer,
                        active_drawable: None,
                        active_surface: None,
                    },
                    direct_context,
                    command_queue,
                    _device: device,
                },
                mtl_cfg.extent,
            ))
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
            // SAFETY: Non-null `mtk_view` checked above; caller guarantees a valid live `MTKView*` on the main thread.
            let mut surface = unsafe {
                surfaces::wrap_mtk_view(
                    &mut self.inner.direct_context,
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
            self.renderer.paint_display_list_with_gpu_context(
                display_list,
                canvas,
                scale_factor,
                None,
                Some(&mut self.inner.direct_context),
            );
            if let Some(banner) = hud_banner {
                self.renderer.paint_hud_banner(canvas, scale_factor, banner);
            }
            self.inner
                .direct_context
                .flush_and_submit_surface(&mut surface, SyncCpu::No)
                .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
            Ok(())
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
        // SAFETY: `device` and `command_queue` are valid retained Metal objects kept alive in `MetalNativeSwapchain`.
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
                // SAFETY: Caller supplies a valid main-thread AppKit `NSView*` raw window handle.
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
                // SAFETY: Caller supplies a valid main-thread UiKit `UIView*` raw window handle.
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
pub use imp::{MetalNativeSwapchain, MetalSwapchain};
