//! Windows Direct3D 12 + DXGI hardware GPU swapchain and offscreen render target
//! (`--features skia-d3d`).
//!
//! Uses `IDXGIFactory4` + `ID3D12Device` + `ID3D12CommandQueue` + `IDXGISwapChain3` +
//! `skia_safe::gpu::d3d::BackendContext` + `direct_contexts::make_d3d` +
//! `backend_render_targets::make_d3d` with `DXGI_SWAP_EFFECT_FLIP_DISCARD`.

use super::{GpuSurfaceConfig, GpuSurfaceError, SwapchainExtent};

/// Configuration for Direct3D 12 DXGI swapchain creation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct D3D12SwapchainConfig {
    pub extent: SwapchainExtent,
    pub vsync: bool,
    pub buffer_count: u32,
}

impl D3D12SwapchainConfig {
    pub fn from_surface_config(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
        let extent = config.extent()?;
        Ok(Self {
            extent,
            vsync: config.vsync,
            buffer_count: config.swapchain_image_count.clamp(2, 4),
        })
    }
}

#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
mod imp {
    use std::ffi::c_void;
    use std::time::Instant;

    use nve_abi::ColorRgba;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use skia_safe::{
        gpu::{
            self, backend_render_targets, d3d, direct_contexts, surfaces, BackendRenderTarget,
            DirectContext, Protected, SurfaceOrigin, SyncCpu,
        },
        ColorType, ImageInfo, Surface,
    };
    use windows::{
        core::Interface,
        Win32::{
            Foundation::HWND,
            Graphics::{
                Direct3D::D3D_FEATURE_LEVEL_11_0,
                Direct3D12::{
                    D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device,
                    D3D12_RESOURCE_STATE_COMMON,
                },
                Dxgi::{
                    Common::{
                        DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
                        DXGI_STANDARD_MULTISAMPLE_QUALITY_PATTERN,
                    },
                    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory4, IDXGISwapChain3,
                    DXGI_ADAPTER_FLAG, DXGI_ADAPTER_FLAG_NONE, DXGI_ADAPTER_FLAG_SOFTWARE,
                    DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD,
                    DXGI_USAGE_RENDER_TARGET_OUTPUT,
                },
            },
        },
    };

    use super::D3D12SwapchainConfig;
    use crate::gpu::skia_renderer::{
        read_surface_to_pixel_buffer, to_skia_color, SkiaCanvasRenderer,
    };
    use crate::gpu::{
        FrameLifecycleState, FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig,
        GpuSurfaceError, SwapchainExtent,
    };
    use crate::raster::PixelBuffer;
    use crate::DisplayList;

    struct D3D12WindowState {
        swap_chain: IDXGISwapChain3,
        buffers: Vec<(Surface, BackendRenderTarget)>,
        active_index: Option<usize>,
        vsync: bool,
        buffer_count: u32,
    }

    enum D3D12Target {
        Window(D3D12WindowState),
        Offscreen { surface: Surface },
    }

    /// Real Direct3D 12 hardware GPU swapchain & offscreen render-target presenter.
    pub struct D3D12Swapchain {
        // Drop order: surfaces & DirectContext must drop before device/queue/adapter.
        target: D3D12Target,
        direct_context: DirectContext,
        _backend_context: d3d::BackendContext,
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

    impl D3D12Swapchain {
        /// Creates an offscreen D3D12 hardware GPU context and VRAM-backed Skia render target.
        pub fn new_offscreen(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
            let d3d_cfg = D3D12SwapchainConfig::from_surface_config(config)?;
            let (_factory, backend_context, mut direct_context) = create_d3d12_context()?;
            let surface = create_offscreen_surface(&mut direct_context, d3d_cfg.extent)?;
            Ok(Self {
                target: D3D12Target::Offscreen { surface },
                direct_context,
                _backend_context: backend_context,
                renderer: SkiaCanvasRenderer::new(),
                extent: d3d_cfg.extent,
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

        /// Attaches a DXGI `IDXGISwapChain3` directly to a Win32 `HWND` window handle.
        pub fn from_window<W: HasWindowHandle>(
            window: &W,
            config: GpuSurfaceConfig,
        ) -> Result<Self, GpuSurfaceError> {
            let d3d_cfg = D3D12SwapchainConfig::from_surface_config(config)?;
            let raw = window
                .window_handle()
                .map_err(|e| GpuSurfaceError::WindowHandleError(e.to_string()))?
                .as_raw();
            let hwnd = match raw {
                RawWindowHandle::Win32(win32) => HWND(win32.hwnd.get() as *mut c_void),
                other => {
                    return Err(GpuSurfaceError::WindowHandleError(format!(
                        "unsupported RawWindowHandle for Direct3D 12: {other:?}"
                    )));
                }
            };

            let (factory, backend_context, mut direct_context) = create_d3d12_context()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: d3d_cfg.extent.width,
                Height: d3d_cfg.extent.height,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: d3d_cfg.buffer_count,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                ..Default::default()
            };
            let swap_chain: IDXGISwapChain3 = unsafe {
                factory.CreateSwapChainForHwnd(&backend_context.queue, hwnd, &desc, None, None)
            }
            .map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!(
                    "CreateSwapChainForHwnd failed: {e}"
                ))
            })?
            .cast()
            .map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!("cast IDXGISwapChain3 failed: {e}"))
            })?;

            let buffers = wrap_dxgi_backbuffers(
                &swap_chain,
                &mut direct_context,
                d3d_cfg.extent,
                d3d_cfg.buffer_count,
            )?;

            Ok(Self {
                target: D3D12Target::Window(D3D12WindowState {
                    swap_chain,
                    buffers,
                    active_index: None,
                    vsync: d3d_cfg.vsync,
                    buffer_count: d3d_cfg.buffer_count,
                }),
                direct_context,
                _backend_context: backend_context,
                renderer: SkiaCanvasRenderer::new(),
                extent: d3d_cfg.extent,
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
            matches!(self.target, D3D12Target::Window(_))
        }

        pub fn render_with_renderer(
            &mut self,
            renderer: &SkiaCanvasRenderer,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
        ) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                D3D12Target::Offscreen { surface } => {
                    let canvas = surface.canvas();
                    canvas.clear(to_skia_color(clear_color));
                    renderer.paint_display_list(display_list, canvas, scale_factor);
                    self.direct_context
                        .flush_and_submit_surface(surface, SyncCpu::Yes)
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                    Ok(())
                }
                D3D12Target::Window(_) => {
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
                D3D12Target::Offscreen { surface } => {
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
                D3D12Target::Window(_) => Err(GpuSurfaceError::ReadbackFailed(
                    "window-attached DXGI swapchain presents directly without CPU readback"
                        .to_string(),
                )),
            }
        }
    }

    impl GpuFramePresenter for D3D12Swapchain {
        fn kind(&self) -> GpuBackendKind {
            GpuBackendKind::Direct3D12
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
                D3D12Target::Window(win) => {
                    if next.width != self.extent.width || next.height != self.extent.height {
                        let _ = self.direct_context.flush_submit_and_sync_cpu();
                        // Release all Skia surfaces & COM buffer references before ResizeBuffers.
                        win.buffers.clear();
                        win.active_index = None;
                        unsafe {
                            win.swap_chain.ResizeBuffers(
                                win.buffer_count,
                                next.width,
                                next.height,
                                DXGI_FORMAT_R8G8B8A8_UNORM,
                                Default::default(),
                            )
                        }
                        .map_err(|e| {
                            GpuSurfaceError::SurfaceCreationFailed(format!(
                                "IDXGISwapChain::ResizeBuffers failed: {e}"
                            ))
                        })?;
                        win.buffers = wrap_dxgi_backbuffers(
                            &win.swap_chain,
                            &mut self.direct_context,
                            next,
                            win.buffer_count,
                        )?;
                    }
                }
                D3D12Target::Offscreen { surface } => {
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

            if let D3D12Target::Window(win) = &mut self.target {
                let idx = unsafe { win.swap_chain.GetCurrentBackBufferIndex() } as usize;
                if idx >= win.buffers.len() {
                    return Err(GpuSurfaceError::DrawableUnavailable);
                }
                win.active_index = Some(idx);
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
                D3D12Target::Window(win) => {
                    let idx = win.active_index.ok_or(GpuSurfaceError::DrawableUnavailable)?;
                    &mut win.buffers[idx].0
                }
                D3D12Target::Offscreen { surface } => surface,
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
            let surface = match &mut self.target {
                D3D12Target::Window(win) => {
                    let idx = win.active_index.ok_or(GpuSurfaceError::DrawableUnavailable)?;
                    &mut win.buffers[idx].0
                }
                D3D12Target::Offscreen { surface } => surface,
            };
            let sync = match self.target {
                D3D12Target::Window(_) => SyncCpu::No,
                D3D12Target::Offscreen { .. } => SyncCpu::Yes,
            };
            self.direct_context
                .flush_and_submit_surface(surface, sync)
                .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
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
            if let D3D12Target::Window(win) = &mut self.target {
                let sync_interval = u32::from(win.vsync);
                unsafe { win.swap_chain.Present(sync_interval, DXGI_PRESENT::default()) }
                    .ok()
                    .map_err(|e| {
                        GpuSurfaceError::PresentFailed(format!("IDXGISwapChain::Present: {e}"))
                    })?;
                win.active_index = None;
            }

            self.frame_index += 1;
            let present_us = t_present.elapsed().as_micros() as u64;
            let total_us = self.frame_start.take().map_or(
                self.acquire_us + self.record_us + self.submit_us + present_us,
                |t| t.elapsed().as_micros() as u64,
            );
            let telemetry = FrameTelemetry {
                backend: GpuBackendKind::Direct3D12,
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
            if let D3D12Target::Window(win) = &mut self.target {
                win.active_index = None;
            }
            self.frame_start = None;
            self.state = FrameLifecycleState::Idle;
        }
    }

    fn create_d3d12_context(
    ) -> Result<(IDXGIFactory4, d3d::BackendContext, DirectContext), GpuSurfaceError> {
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.map_err(|e| {
            GpuSurfaceError::DeviceUnavailable(format!("CreateDXGIFactory1 failed: {e}"))
        })?;
        let (adapter, device) = pick_hardware_adapter(&factory)?;
        let queue: ID3D12CommandQueue = unsafe { device.CreateCommandQueue(&Default::default()) }
            .map_err(|e| {
            GpuSurfaceError::DeviceUnavailable(format!("CreateCommandQueue failed: {e}"))
        })?;
        let backend_context = d3d::BackendContext {
            adapter,
            device,
            queue,
            memory_allocator: None,
            protected_context: Protected::No,
        };
        let direct_context =
            unsafe { direct_contexts::make_d3d(&backend_context, None) }.ok_or_else(|| {
                GpuSurfaceError::SurfaceCreationFailed(
                    "skia_safe::gpu::direct_contexts::make_d3d returned None".to_string(),
                )
            })?;
        Ok((factory, backend_context, direct_context))
    }

    fn pick_hardware_adapter(
        factory: &IDXGIFactory4,
    ) -> Result<(IDXGIAdapter1, ID3D12Device), GpuSurfaceError> {
        for idx in 0.. {
            let adapter = match unsafe { factory.EnumAdapters1(idx) } {
                Ok(a) => a,
                Err(_) => break,
            };
            let desc = match unsafe { adapter.GetDesc1() } {
                Ok(d) => d,
                Err(_) => continue,
            };
            if (DXGI_ADAPTER_FLAG(desc.Flags as _) & DXGI_ADAPTER_FLAG_SOFTWARE)
                != DXGI_ADAPTER_FLAG_NONE
            {
                continue;
            }
            let mut device: Option<ID3D12Device> = None;
            if unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) }.is_ok() {
                if let Some(dev) = device {
                    return Ok((adapter, dev));
                }
            }
        }
        Err(GpuSurfaceError::DeviceUnavailable(
            "no hardware Direct3D 12 adapter available".to_string(),
        ))
    }

    fn wrap_dxgi_backbuffers(
        swap_chain: &IDXGISwapChain3,
        direct_context: &mut DirectContext,
        extent: SwapchainExtent,
        buffer_count: u32,
    ) -> Result<Vec<(Surface, BackendRenderTarget)>, GpuSurfaceError> {
        let mut out = Vec::with_capacity(buffer_count as usize);
        for i in 0..buffer_count {
            let resource = unsafe { swap_chain.GetBuffer(i) }.map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!("GetBuffer({i}) failed: {e}"))
            })?;
            let backend_rt = backend_render_targets::make_d3d(
                (extent.width as i32, extent.height as i32),
                &d3d::TextureResourceInfo {
                    resource,
                    alloc: None,
                    resource_state: D3D12_RESOURCE_STATE_COMMON,
                    format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    sample_count: 1,
                    level_count: 0,
                    sample_quality_pattern: DXGI_STANDARD_MULTISAMPLE_QUALITY_PATTERN,
                    protected: Protected::No,
                },
            );
            let surface = surfaces::wrap_backend_render_target(
                direct_context,
                &backend_rt,
                SurfaceOrigin::TopLeft,
                ColorType::RGBA8888,
                None,
                None,
            )
            .ok_or_else(|| {
                GpuSurfaceError::SurfaceCreationFailed(format!(
                    "wrap_backend_render_target returned None for DXGI buffer {i}"
                ))
            })?;
            out.push((surface, backend_rt));
        }
        Ok(out)
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
                "D3D12 GPU render_target returned None for {}x{}",
                extent.width, extent.height
            ))
        })
    }
}

#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
pub use imp::D3D12Swapchain;
