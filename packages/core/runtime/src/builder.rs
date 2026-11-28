//! [`RuntimeBuilder`], [`RuntimeError`], and platform-safe monotonic [`Timer`] for
//! [`SurfaceRuntime`].

use std::cell::UnsafeCell;
use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nve_abi::{
    FrameId, NveHostEventBatchBuilder, NveRuntimeMetrics, NveRuntimeStatus, NveSceneView,
    SurfaceId, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_tables::tables::{core_realization_order, register_core_tables};
use nve_engine::Viewport;
use nve_layout::{DynLayoutEngine, LaidOutScene, TaffyStoreLayout, TextMeasurer};
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeStore, RealizationOrder};

use crate::app::{FrameTickHook, RuntimeApp};
use crate::delta::SceneDelta;
use crate::hooks::IntrinsicSizeNotifier;
use crate::host_router::{unbind_and_drop_app, OwnerBindingHandle, RuntimeHostRouter};
use crate::lock::SceneLock;
use crate::state::{PublishedResources, RuntimeInner, SurfaceState};
use crate::surface::SurfaceRuntime;
use crate::wake::WakeBridge;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Timer(std::time::Instant);

#[cfg(not(target_arch = "wasm32"))]
impl Timer {
    #[inline]
    pub(crate) fn start() -> Self {
        Self(std::time::Instant::now())
    }

    #[inline]
    pub(crate) fn elapsed_us(&self) -> u64 {
        self.0.elapsed().as_micros() as u64
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct Timer;

#[cfg(target_arch = "wasm32")]
impl Timer {
    #[inline]
    pub(crate) fn start() -> Self {
        Self
    }

    #[inline]
    pub(crate) fn elapsed_us(&self) -> u64 {
        0
    }
}

/// Error returned by [`SurfaceRuntime`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// A host still holds an active `scene_acquire` read lock or concurrent hit-test lock.
    SceneBusy,
    /// Invalid binary `NVE2` wire frame payload.
    WireDecode(String),
    /// Transport or runtime I/O failure (e.g. pairing client spawn/connect failure).
    Io(String),
    /// Invalid argument or owner-thread contract violation.
    InvalidArg(&'static str),
    /// Feature or capability not compiled into this build (e.g. wire pairing disabled).
    Unsupported(&'static str),
    /// Target node was missing, stale, or not scrollable.
    NotFound(&'static str),
    /// A previous mutating operation panicked and invalidated the surface state.
    Poisoned,
}

impl RuntimeError {
    pub const fn status(&self) -> NveRuntimeStatus {
        match self {
            Self::SceneBusy => NveRuntimeStatus::Busy,
            Self::WireDecode(_) | Self::Io(_) => NveRuntimeStatus::Io,
            Self::InvalidArg(_) => NveRuntimeStatus::InvalidArg,
            Self::Unsupported(_) => NveRuntimeStatus::Unsupported,
            Self::NotFound(_) => NveRuntimeStatus::NotFound,
            Self::Poisoned => NveRuntimeStatus::Panic,
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SceneBusy => {
                f.write_str("runtime scene is busy (held by an active reader or writer)")
            }
            Self::WireDecode(msg) => write!(f, "NVE2 wire decode failed: {msg}"),
            Self::Io(msg) => write!(f, "runtime I/O error: {msg}"),
            Self::InvalidArg(msg) => write!(f, "invalid runtime argument: {msg}"),
            Self::Unsupported(msg) => write!(f, "unsupported runtime operation: {msg}"),
            Self::NotFound(msg) => write!(f, "runtime target not found: {msg}"),
            Self::Poisoned => f.write_str("runtime surface was poisoned by a prior panic"),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// Builder for configuring a [`SurfaceRuntime`] with a custom layout engine, text measurer,
/// realization order, frame-tick hooks, and initial app.
pub struct RuntimeBuilder {
    surface_id: SurfaceId,
    viewport: Viewport,
    layout: Option<DynLayoutEngine>,
    text_measurer: Option<Arc<dyn TextMeasurer>>,
    realization: Option<RealizationOrder>,
    app: Option<Box<dyn RuntimeApp>>,
    tick_hooks: Vec<Box<dyn FrameTickHook>>,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            surface_id: SurfaceId::ROOT,
            viewport: Viewport::default(),
            layout: None,
            text_measurer: None,
            realization: None,
            app: None,
            tick_hooks: Vec::new(),
        }
    }

    pub fn surface_id(mut self, id: SurfaceId) -> Self {
        self.surface_id = id;
        self
    }

    pub fn viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = viewport;
        self
    }

    pub fn layout(mut self, engine: DynLayoutEngine) -> Self {
        self.layout = Some(engine);
        self
    }

    pub fn text_measurer(mut self, measurer: Arc<dyn TextMeasurer>) -> Self {
        self.text_measurer = Some(measurer);
        self
    }

    pub fn realization_order(mut self, order: RealizationOrder) -> Self {
        self.realization = Some(order);
        self
    }

    pub fn app(mut self, app: Box<dyn RuntimeApp>) -> Self {
        self.app = Some(app);
        self
    }

    pub fn tick_hook(mut self, hook: Box<dyn FrameTickHook>) -> Self {
        self.tick_hooks.push(hook);
        self
    }

    pub fn build(self) -> SurfaceRuntime {
        register_core_tables();
        let layout: DynLayoutEngine = self
            .layout
            .unwrap_or_else(|| Box::new(TaffyStoreLayout::new()));
        let mut scene = match self.text_measurer {
            Some(m) => LaidOutScene::with_text_measurer(layout, m),
            None => LaidOutScene::new(layout),
        };
        scene.set_viewport(self.viewport);

        let realization_order = self.realization.unwrap_or_else(core_realization_order);
        let mut replica_store = NodeStore::replica();
        replica_store.set_realization_order(realization_order.clone());

        let wake = Arc::new(WakeBridge::new());
        let host_router = RuntimeHostRouter::new(Arc::clone(&wake));
        // Unwind guard ensures `app` is unbound and dropped BEFORE `host_binding` if either
        // `bind_host` or `install_wake_bridge` panics during `build()`.
        struct BuildAppGuard<'a> {
            host_binding: Option<OwnerBindingHandle>,
            app: Option<Box<dyn RuntimeApp>>,
            router: &'a RuntimeHostRouter,
        }
        impl Drop for BuildAppGuard<'_> {
            fn drop(&mut self) {
                if self.app.is_some() || self.host_binding.is_some() {
                    unbind_and_drop_app(self.app.take(), self.host_binding.take(), self.router);
                }
            }
        }
        let mut init_guard = BuildAppGuard {
            host_binding: None,
            app: self.app,
            router: &host_router,
        };
        let mut active_host_owner_tag = 0u64;
        if let Some(ref mut a) = init_guard.app {
            let binding = host_router.new_owner_binding();
            active_host_owner_tag = binding.owner_tag();
            let callbacks = binding.callbacks();
            init_guard.host_binding = Some(binding);
            a.bind_host(Some(callbacks));
            if a.install_wake_bridge(Arc::clone(&wake)) {
                wake.trigger();
            }
        }
        let app = init_guard.app.take();
        let host_binding = init_guard.host_binding.take();
        drop(init_guard);
        let layout_engine_name = scene.layout_engine_name();

        SurfaceRuntime {
            surface_id: self.surface_id,
            lock: SceneLock::new(),
            wake,
            intrinsic_size: IntrinsicSizeNotifier::new(),
            host_router,
            pair_connected: Arc::new(AtomicBool::new(false)),
            #[cfg(not(target_arch = "wasm32"))]
            owner_thread: std::thread::current().id(),
            poisoned: AtomicBool::new(false),
            inner: UnsafeCell::new(RuntimeInner {
                app,
                host_binding,
                pair: None,
                replica_store,
                state: SurfaceState {
                    frame_id: FrameId::ZERO,
                    scene_epoch: 0,
                    layout_engine_name,
                    realization_order,
                    wire_decoder: WireDecoder::default(),
                    has_replica_state: false,
                    wire_changes: ChangeSet::default(),
                    hook_changes: ChangeSet::default(),
                    merged_changes: ChangeSet::default(),
                    empty_changes: ChangeSet::default(),
                    scene,
                    tick_hooks: self.tick_hooks,
                    delta: SceneDelta::default(),
                    published_tables: Vec::new(),
                    published_dirty_buf: Vec::new(),
                    published_scroll: Vec::new(),
                    dirty_resources: Vec::new(),
                    published_resources: PublishedResources::None,
                    committed_store: 0,
                    published_view: NveSceneView {
                        schema_hash: NVE_RUNTIME_SCHEMA_HASH,
                        ..NveSceneView::default()
                    },
                    metrics: NveRuntimeMetrics::default(),
                    host_batch_builder: NveHostEventBatchBuilder::new(),
                    active_host_owner_tag,
                    intrinsic_size_dirty: false,
                    needs_render: true,
                    needs_publish: true,
                },
            }),
        }
    }
}
