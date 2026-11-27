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

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use skia_safe::{
        gpu::{
            backend_render_targets, d3d, direct_contexts, surfaces, BackendRenderTarget,
            DirectContext, FlushInfo, Protected, SurfaceOrigin, SyncCpu,
        },
        surfaces::BackendSurfaceAccess,
        ColorType, Surface,
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
    use crate::gpu::ganesh::{
        create_ganesh_offscreen_surface, GaneshPresenter, NativeSwapchain,
    };
    use crate::gpu::{GpuBackendKind, GpuSurfaceConfig, GpuSurfaceError, SwapchainExtent};

    struct D3D12WindowState {
        // Drop order: Skia surfaces & BackendRenderTargets must drop BEFORE IDXGISwapChain3.
        buffers: Vec<(Surface, BackendRenderTarget)>,
        swap_chain: IDXGISwapChain3,
        buffer_extent: SwapchainExtent,
        active_index: Option<usize>,
        vsync: bool,
        buffer_count: u32,
    }

    enum D3D12Target {
        Window(D3D12WindowState),
        Offscreen { surface: Option<Surface> },
    }

    /// Platform-specific Direct3D 12 swapchain resources managed by [`GaneshPresenter`].
    pub struct D3D12NativeSwapchain {
        // Drop order: surfaces & DirectContext must drop before device/queue/adapter.
        target: D3D12Target,
        direct_context: DirectContext,
        _backend_context: d3d::BackendContext,
    }

    impl Drop for D3D12NativeSwapchain {
        fn drop(&mut self) {
            let _ = self.direct_context.flush_submit_and_sync_cpu();
            match &mut self.target {
                D3D12Target::Window(win) => {
                    win.buffers.clear();
                    win.active_index = None;
                }
                D3D12Target::Offscreen { surface } => {
                    surface.take();
                }
            }
            self.direct_context.release_resources_and_abandon();
        }
    }

    /// Real Direct3D 12 hardware GPU swapchain & offscreen render-target presenter.
    pub type D3D12Swapchain = GaneshPresenter<D3D12NativeSwapchain>;

    impl NativeSwapchain for D3D12NativeSwapchain {
        const KIND: GpuBackendKind = GpuBackendKind::Direct3D12;
        const WINDOW_READBACK_ERROR: &'static str =
            "window-attached DXGI swapchain presents directly without CPU readback";

        fn is_window_attached(&self) -> bool {
            matches!(self.target, D3D12Target::Window(_))
        }

        fn direct_context_mut(&mut self) -> &mut DirectContext {
            &mut self.direct_context
        }

        fn offscreen_surface_and_context_mut(
            &mut self,
        ) -> Option<(&mut Surface, &mut DirectContext)> {
            match &mut self.target {
                D3D12Target::Offscreen {
                    surface: Some(surface),
                } => Some((surface, &mut self.direct_context)),
                _ => None,
            }
        }

        fn resize_native(
            &mut self,
            prev: SwapchainExtent,
            next: SwapchainExtent,
        ) -> Result<bool, GpuSurfaceError> {
            match &mut self.target {
                D3D12Target::Window(win) => {
                    if next.width == win.buffer_extent.width
                        && next.height == win.buffer_extent.height
                    {
                        win.buffer_extent.scale_factor = next.scale_factor;
                        return Ok(false);
                    }
                    let _ = self.direct_context.flush_submit_and_sync_cpu();
                    // DXGI requires releasing all COM backbuffer references before ResizeBuffers.
                    win.buffers.clear();
                    win.active_index = None;
                    // SAFETY: All Skia surfaces and COM backbuffer references were flushed and dropped above.
                    let resize_res = unsafe {
                        win.swap_chain.ResizeBuffers(
                            win.buffer_count,
                            next.width,
                            next.height,
                            DXGI_FORMAT_R8G8B8A8_UNORM,
                            Default::default(),
                        )
                    };
                    if let Err(err) = resize_res {
                        // Restore wrappers for the unchanged previous backbuffers so `win.buffers`
                        // is not left empty if `ResizeBuffers` fails transiently.
                        if let Ok(restored) = wrap_dxgi_backbuffers(
                            &win.swap_chain,
                            &mut self.direct_context,
                            win.buffer_extent,
                            win.buffer_count,
                        ) {
                            win.buffers = restored;
                        }
                        return Err(GpuSurfaceError::SurfaceCreationFailed(format!(
                            "IDXGISwapChain::ResizeBuffers failed: {err}"
                        )));
                    }
                    // DXGI swapchain dimensions are now `next`. Keep `win.buffer_extent` and
                    // `GaneshPresenter::extent` synchronized even if eager wrapping fails; lazy
                    // re-wrap in `begin_native_frame` will retry and surface any wrap error.
                    win.buffer_extent = next;
                    if let Ok(wrapped) = wrap_dxgi_backbuffers(
                        &win.swap_chain,
                        &mut self.direct_context,
                        win.buffer_extent,
                        win.buffer_count,
                    ) {
                        win.buffers = wrapped;
                    }
                    Ok(true)
                }
                D3D12Target::Offscreen { surface } => {
                    if next.width == prev.width && next.height == prev.height {
                        return Ok(false);
                    }
                    let _ = self.direct_context.flush_submit_and_sync_cpu();
                    *surface = Some(create_ganesh_offscreen_surface(
                        &mut self.direct_context,
                        next,
                        "D3D12",
                    )?);
                    Ok(true)
                }
            }
        }

        fn begin_native_frame(
            &mut self,
            extent: SwapchainExtent,
            _recreations: &mut u64,
        ) -> Result<(), GpuSurfaceError> {
            if let D3D12Target::Window(win) = &mut self.target {
                debug_assert_eq!(win.buffer_extent, extent);
                if win.buffers.is_empty() {
                    win.buffers = wrap_dxgi_backbuffers(
                        &win.swap_chain,
                        &mut self.direct_context,
                        win.buffer_extent,
                        win.buffer_count,
                    )?;
                }
                // SAFETY: `win.swap_chain` is a valid live `IDXGISwapChain3`.
                let idx = unsafe { win.swap_chain.GetCurrentBackBufferIndex() } as usize;
                if idx >= win.buffers.len() {
                    return Err(GpuSurfaceError::DrawableUnavailable);
                }
                win.active_index = Some(idx);
            }
            Ok(())
        }

        fn active_surface_mut(&mut self) -> Result<&mut Surface, GpuSurfaceError> {
            match &mut self.target {
                D3D12Target::Window(win) => {
                    let idx = win.active_index.ok_or(GpuSurfaceError::DrawableUnavailable)?;
                    win.buffers
                        .get_mut(idx)
                        .map(|(surface, _)| surface)
                        .ok_or(GpuSurfaceError::DrawableUnavailable)
                }
                D3D12Target::Offscreen { surface } => surface
                    .as_mut()
                    .ok_or(GpuSurfaceError::DrawableUnavailable),
            }
        }

        fn submit_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
            match &mut self.target {
                D3D12Target::Window(win) => {
                    let idx = win.active_index.ok_or(GpuSurfaceError::DrawableUnavailable)?;
                    let (surface, _) = win
                        .buffers
                        .get_mut(idx)
                        .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                    self.direct_context
                        .flush_surface_with_access(
                            surface,
                            BackendSurfaceAccess::Present,
                            &FlushInfo::default(),
                        )
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))?;
                    if !self.direct_context.submit(SyncCpu::No) {
                        return Err(GpuSurfaceError::FlushFailed(
                            "DirectContext::submit returned false".to_string(),
                        ));
                    }
                    Ok(())
                }
                D3D12Target::Offscreen { surface } => {
                    let surface = surface
                        .as_mut()
                        .ok_or(GpuSurfaceError::DrawableUnavailable)?;
                    self.direct_context
                        .flush_and_submit_surface(surface, SyncCpu::Yes)
                        .map(|_| ())
                        .map_err(|e| GpuSurfaceError::FlushFailed(e.to_string()))
                }
            }
        }

        fn present_native_frame(&mut self) -> Result<(), GpuSurfaceError> {
            if let D3D12Target::Window(win) = &mut self.target {
                let sync_interval = u32::from(win.vsync);
                // SAFETY: `win.swap_chain` is live and the active backbuffer surface was flushed in `submit_native_frame`.
                unsafe { win.swap_chain.Present(sync_interval, DXGI_PRESENT::default()) }
                    .ok()
                    .map_err(|e| {
                        GpuSurfaceError::PresentFailed(format!("IDXGISwapChain::Present: {e}"))
                    })?;
                win.active_index = None;
            }
            Ok(())
        }

        fn abort_native_frame(&mut self) {
            if let D3D12Target::Window(win) = &mut self.target {
                win.active_index = None;
            }
        }
    }

    impl D3D12Swapchain {
        /// Creates an offscreen D3D12 hardware GPU context and VRAM-backed Skia render target.
        pub fn new_offscreen(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
            let d3d_cfg = D3D12SwapchainConfig::from_surface_config(config)?;
            let (_factory, backend_context, mut direct_context) = create_d3d12_context()?;
            let surface = match create_ganesh_offscreen_surface(
                &mut direct_context,
                d3d_cfg.extent,
                "D3D12",
            ) {
                Ok(s) => s,
                Err(err) => {
                    direct_context.release_resources_and_abandon();
                    return Err(err);
                }
            };
            Ok(Self::new(
                D3D12NativeSwapchain {
                    target: D3D12Target::Offscreen {
                        surface: Some(surface),
                    },
                    direct_context,
                    _backend_context: backend_context,
                },
                d3d_cfg.extent,
            ))
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
            // SAFETY: `factory`, `backend_context.queue`, and the caller-supplied `hwnd` are valid live handles.
            let swap_chain: IDXGISwapChain3 = match unsafe {
                factory.CreateSwapChainForHwnd(&backend_context.queue, hwnd, &desc, None, None)
            }
            .map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!(
                    "CreateSwapChainForHwnd failed: {e}"
                ))
            })
            .and_then(|sc| {
                sc.cast().map_err(|e| {
                    GpuSurfaceError::SurfaceCreationFailed(format!(
                        "cast IDXGISwapChain3 failed: {e}"
                    ))
                })
            }) {
                Ok(sc) => sc,
                Err(err) => {
                    direct_context.release_resources_and_abandon();
                    return Err(err);
                }
            };

            let buffers = match wrap_dxgi_backbuffers(
                &swap_chain,
                &mut direct_context,
                d3d_cfg.extent,
                d3d_cfg.buffer_count,
            ) {
                Ok(bufs) => bufs,
                Err(err) => {
                    // Abandon and drop DirectContext BEFORE `swap_chain` (and `_backend_context`).
                    let _ = direct_context.flush_submit_and_sync_cpu();
                    direct_context.release_resources_and_abandon();
                    drop(direct_context);
                    drop(swap_chain);
                    return Err(err);
                }
            };

            Ok(Self::new(
                D3D12NativeSwapchain {
                    target: D3D12Target::Window(D3D12WindowState {
                        swap_chain,
                        buffers,
                        buffer_extent: d3d_cfg.extent,
                        active_index: None,
                        vsync: d3d_cfg.vsync,
                        buffer_count: d3d_cfg.buffer_count,
                    }),
                    direct_context,
                    _backend_context: backend_context,
                },
                d3d_cfg.extent,
            ))
        }
    }

    fn create_d3d12_context(
    ) -> Result<(IDXGIFactory4, d3d::BackendContext, DirectContext), GpuSurfaceError> {
        // SAFETY: `CreateDXGIFactory1` has no memory preconditions and returns an owned COM pointer.
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.map_err(|e| {
            GpuSurfaceError::DeviceUnavailable(format!("CreateDXGIFactory1 failed: {e}"))
        })?;
        let (adapter, device) = pick_hardware_adapter(&factory)?;
        // SAFETY: `device` is a live `ID3D12Device` and `Default::default()` initializes a valid direct queue desc.
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
        // SAFETY: `backend_context` holds live D3D12 COM interfaces kept alive inside `D3D12NativeSwapchain` longer than `DirectContext`.
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
            // SAFETY: `factory` is a valid live `IDXGIFactory4`; DXGI signals end of list via `Err`.
            let adapter = match unsafe { factory.EnumAdapters1(idx) } {
                Ok(a) => a,
                Err(_) => break,
            };
            // SAFETY: `adapter` is a valid live `IDXGIAdapter1`.
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
            // SAFETY: `adapter` is a valid live `IDXGIAdapter1` and `&mut device` is a valid COM out-parameter.
            if unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) }.is_ok()
                && let Some(dev) = device
            {
                return Ok((adapter, dev));
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
            // SAFETY: `swap_chain` is live and `i < buffer_count` indexes an allocated swapchain backbuffer.
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
}

#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
pub use imp::{D3D12NativeSwapchain, D3D12Swapchain};
