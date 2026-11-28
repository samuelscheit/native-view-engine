//! Direct zero-serialization [`nve_engine::RuntimeApp`] adapter over [`DioxusSurface`] with
//! single-bridge coalesced async task wakeup via [`nve_engine::WakeBridge`], embedding-host props
//! ([`HostProps`]), outbound event emission ([`HostEventEmitter`]), `NVE_SIG1` state snapshots,
//! and `NVE_IFACE_HOST = 5` host services ([`FfiHostServices`]).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::task::Waker;

use dioxus_core::ScopeId;
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_abi::{
    encode_bin_value_to_vec, BinValueView, EventBatchBuf, FrameId, NodeId,
    NveHostCallbacks, NvePropsRef, SurfaceId, TimestampUs, ValidatedEvents,
};
use nve_engine::{AppFrame, CapabilityUnsupported, HotPatchMsg, RuntimeApp, WakeBridge, WakeToken};
use nve_host_services::FfiHostServices;
use nve_store::NodeStore;

use crate::signal_archive::SignalArchiveError;
use crate::DioxusSurface;

/// Embedding-host props snapshot provided in `ScopeId::ROOT` context by [`DioxusRuntimeApp`].
///
/// Each prop value is stored as compact binary `NveBinValue` bytes decoded zero-copy via
/// [`BinValueView`].
#[derive(Clone, Default)]
pub struct HostProps {
    entries: Arc<RwLock<BTreeMap<String, Vec<u8>>>>,
    revision: Arc<AtomicU64>,
    rev_signal: Option<Signal<u64>>,
}

impl HostProps {
    fn new_reactive(owner_scope: ScopeId) -> Self {
        Self {
            entries: Arc::new(RwLock::new(BTreeMap::new())),
            revision: Arc::new(AtomicU64::new(0)),
            rev_signal: Some(Signal::new_in_scope(0, owner_scope)),
        }
    }

    #[inline]
    fn track_reactive(&self) {
        if let Some(sig) = self.rev_signal
            && dioxus_core::Runtime::try_current().is_some()
        {
            let _ = *sig.read();
        }
    }

    pub fn revision(&self) -> u64 {
        self.track_reactive();
        self.revision.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.track_reactive();
        self.entries.read().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    pub fn len(&self) -> usize {
        self.track_reactive();
        self.entries.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn get_with<R>(&self, key: &str, f: impl FnOnce(BinValueView<'_>) -> R) -> Option<R> {
        self.track_reactive();
        let guard = self.entries.read().unwrap_or_else(|e| e.into_inner());
        let bytes = guard.get(key)?;
        let view = BinValueView::parse(bytes).ok()?;
        Some(f(view))
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.get_with(key, |v| v.as_str().map(str::to_owned)).flatten()
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get_with(key, |v| v.as_i64()).flatten()
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get_with(key, |v| v.as_f64()).flatten()
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_with(key, |v| v.as_bool()).flatten()
    }

    fn replace_from_props(&self, props: NvePropsRef<'_>) -> bool {
        let mut next = BTreeMap::new();
        for (key, value) in props.iter() {
            if let Ok(encoded) = encode_bin_value_to_vec(value) {
                next.insert(key.to_owned(), encoded);
            }
        }
        let mut guard = self.entries.write().unwrap_or_else(|e| e.into_inner());
        if *guard == next {
            return false;
        }
        *guard = next;
        let rev = self.revision.fetch_add(1, Ordering::Release) + 1;
        drop(guard);
        if let Some(mut sig) = self.rev_signal {
            sig.set(rev);
        }
        true
    }
}

/// Outbound component event queue provided in `ScopeId::ROOT` context by [`DioxusRuntimeApp`],
/// drained by embedding hosts via `NveComponentApi::drain_emitted_events` / `RuntimeApp::drain_emitted_events`.
#[derive(Clone, Default)]
pub struct HostEventEmitter {
    surface_id: SurfaceId,
    buf: Arc<Mutex<EventBatchBuf>>,
}

impl HostEventEmitter {
    pub fn new(surface_id: SurfaceId) -> Self {
        Self {
            surface_id,
            buf: Arc::new(Mutex::new(EventBatchBuf::new())),
        }
    }

    /// Appends a custom named outbound event for the embedding host.
    pub fn emit_custom(&self, node: NodeId, name: &str, data: &[u8]) {
        let mut guard = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        guard.push_custom(self.surface_id, node, TimestampUs(0), name, data);
    }

    /// Runs `f` directly against the underlying outbound [`EventBatchBuf`].
    pub fn with_batch(&self, f: impl FnOnce(&mut EventBatchBuf)) {
        let mut guard = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
    }

    fn drain_into(&self, dst: &mut EventBatchBuf) {
        dst.clear();
        let mut guard = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::swap(&mut *guard, dst);
    }
}

/// Hosts an in-process [`DioxusSurface`] directly inside any `SurfaceRuntime` with zero `NVE2`
/// wire serialization, single-bridge coalesced cross-thread async task wakeups, host props,
/// outbound event emission, `NVE_SIG1` signal snapshots, and `NVE_IFACE_HOST = 5` host services.
pub struct DioxusRuntimeApp {
    // Declared FIRST so dropping `DioxusRuntimeApp` disables zombie wakers BEFORE `surface` drops.
    _wake_token: WakeToken,
    waker: Waker,
    wake: Arc<WakeBridge>,
    surface: DioxusSurface,
    mounted: bool,
    host_props: HostProps,
    event_emitter: HostEventEmitter,
    emitted_drain: EventBatchBuf,
    state_scratch: Vec<u8>,
    last_state_error: Option<SignalArchiveError>,
    host_services: Arc<FfiHostServices>,
}

impl DioxusRuntimeApp {
    pub fn new(mut surface: DioxusSurface) -> Self {
        let wake = Arc::new(WakeBridge::new());
        let (waker, wake_token) = wake.detachable_waker();
        let event_emitter = HostEventEmitter::new(surface.surface_id());
        let host_services = Arc::new(FfiHostServices::new());
        let host_props = surface.in_root_scope(|| {
            let props = HostProps::new_reactive(ScopeId::ROOT);
            dioxus_core::provide_context(props.clone());
            dioxus_core::provide_context(event_emitter.clone());
            dioxus_core::provide_context(Arc::clone(&host_services));
            props
        });
        Self {
            _wake_token: wake_token,
            waker,
            wake,
            surface,
            mounted: false,
            host_props,
            event_emitter,
            emitted_drain: EventBatchBuf::new(),
            state_scratch: Vec::new(),
            last_state_error: None,
            host_services,
        }
    }

    /// Last signal snapshot/hydration error observed by [`RuntimeApp::snapshot_state`] or
    /// [`RuntimeApp::restore_state`] (`None` after a clean call).
    #[inline]
    pub fn last_state_error(&self) -> Option<&SignalArchiveError> {
        self.last_state_error.as_ref()
    }

    #[inline]
    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
    }

    #[inline]
    pub fn surface_mut(&mut self) -> &mut DioxusSurface {
        &mut self.surface
    }

    #[inline]
    pub fn wake_bridge(&self) -> &Arc<WakeBridge> {
        &self.wake
    }

    #[inline]
    pub fn host_props(&self) -> &HostProps {
        &self.host_props
    }

    #[inline]
    pub fn event_emitter(&self) -> &HostEventEmitter {
        &self.event_emitter
    }

    #[inline]
    pub fn host_services(&self) -> &Arc<FfiHostServices> {
        &self.host_services
    }
}

impl Drop for DioxusRuntimeApp {
    fn drop(&mut self) {
        // Cancel any pending host-service callbacks while `self.surface` is still alive inside
        // `ScopeId::ROOT` so Signal-touching cancellation closures never hit `NoCurrentRuntime`.
        let host_services = Arc::clone(&self.host_services);
        self.surface.in_root_scope(|| {
            host_services.bind(None);
        });
    }
}

impl RuntimeApp for DioxusRuntimeApp {
    fn render_frame(&mut self, frame_id: FrameId) -> AppFrame<'_> {
        let _ = self.surface.reconcile(frame_id);
        let first_mount = !self.mounted;
        self.mounted = true;
        if !first_mount && self.surface.last_changes().is_empty() {
            return AppFrame::Clean;
        }
        AppFrame::Direct {
            changes: self.surface.last_changes(),
        }
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        if events.is_empty() {
            return false;
        }
        self.surface.dispatch_events(events)
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(self.surface.store())
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(self.surface.store_mut())
    }

    fn poll_async_work(&mut self) -> bool {
        self.surface.poll_work(&self.waker)
    }

    fn install_wake_bridge(&mut self, wake: Arc<WakeBridge>) -> bool {
        let had_pending = self.wake.take_pending();
        let (waker, wake_token) = wake.detachable_waker();
        // Dropping the previous `_wake_token` immediately disables zombie wakers from prior apps.
        self._wake_token = wake_token;
        self.waker = waker;
        self.wake = wake;
        had_pending
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, CapabilityUnsupported> {
        Ok(self.surface.apply_hot_patch(patch))
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> Result<bool, CapabilityUnsupported> {
        let host_props = &self.host_props;
        let changed = self
            .surface
            .in_root_scope(|| host_props.replace_from_props(props));
        if changed && self.mounted {
            self.surface.vdom.mark_dirty(ScopeId::APP);
        }
        Ok(changed)
    }

    fn drain_emitted_events(&mut self) -> Result<ValidatedEvents<'_>, CapabilityUnsupported> {
        self.event_emitter.drain_into(&mut self.emitted_drain);
        Ok(self.emitted_drain.as_validated())
    }

    fn supports_state(&self) -> bool {
        true
    }

    fn snapshot_state(&mut self) -> Result<&[u8], CapabilityUnsupported> {
        self.state_scratch.clear();
        if self.surface.signal_registry().is_empty() {
            self.last_state_error = None;
            return Ok(&[]);
        }
        match self.surface.snapshot_signals() {
            Ok(bytes) => {
                self.state_scratch = bytes;
                self.last_state_error = None;
            }
            Err(err) => {
                self.last_state_error = Some(err);
            }
        }
        Ok(&self.state_scratch)
    }

    fn restore_state(&mut self, state: &[u8]) -> Result<(), CapabilityUnsupported> {
        if state.is_empty() {
            self.last_state_error = None;
            return Ok(());
        }
        match self.surface.hydrate_signals(state) {
            Ok(report) => {
                self.last_state_error = None;
                if self.mounted && report.hydrated_slots > 0 {
                    self.surface.vdom.mark_dirty(ScopeId::APP);
                }
            }
            Err(err) => {
                self.last_state_error = Some(err);
            }
        }
        Ok(())
    }

    fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
        let host_services = Arc::clone(&self.host_services);
        // `FfiHostServices::bind` synchronously invokes `Cancelled` callbacks on unbind/rebind,
        // which may mutate live `dioxus_signals::Signal<T>` handles.
        self.surface.in_root_scope(|| host_services.bind(callbacks));
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        if batch.is_empty() {
            return false;
        }
        let host_services = Arc::clone(&self.host_services);
        // Completion and `TypedEventBus` callbacks may read/write live `Signal<T>` handles.
        match self.surface.in_root_scope(|| host_services.ingest_batch(batch)) {
            Ok(stats) if stats.total() > 0 => {
                if self.mounted {
                    self.surface.vdom.mark_dirty(ScopeId::APP);
                }
                true
            }
            _ => false,
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Waker};

    use dioxus_core::Element;
    use nve_abi::{SurfaceId, NVE_RUNTIME_SCHEMA_HASH};
    use nve_engine::Viewport;
    use nve_runtime::{RuntimeBuilder, RuntimeError};

    use super::DioxusRuntimeApp;
    use crate::DioxusSurface;
    use crate::prelude::*;

    fn sample_app() -> Element {
        rsx! {
            view {
                nve_style: TypedStyle::new().width_px(240.0).height_px(96.0).flex_col(),
                text { "Zero-copy DioxusRuntimeApp" }
            }
        }
    }

    #[derive(Clone, Default)]
    struct OneShotState {
        inner: Arc<Mutex<(Option<String>, Option<Waker>)>>,
    }

    impl OneShotState {
        fn send(&self, value: &str) {
            let waker = {
                let mut g = self.inner.lock().expect("oneshot lock");
                g.0 = Some(value.to_owned());
                g.1.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        }
    }

    struct OneShotFut(OneShotState);

    impl Future for OneShotFut {
        type Output = String;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let mut g = self.0.inner.lock().expect("oneshot lock");
            if let Some(val) = g.0.take() {
                Poll::Ready(val)
            } else {
                g.1 = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    fn async_app() -> Element {
        let ch = consume_context::<OneShotState>();
        let mut label = use_signal(|| "loading".to_owned());
        dioxus_core::use_hook(move || {
            dioxus_core::spawn(async move {
                let val = OneShotFut(ch).await;
                label.set(val);
            });
        });
        rsx! {
            view {
                nve_style: TypedStyle::new().width_px(240.0).height_px(96.0),
                text { "{label}" }
            }
        }
    }

    fn read_text_slab(guard: &nve_runtime::SceneReadGuard<'_>) -> String {
        let view = guard.view();
        assert!(!view.text_slab.is_null());
        // SAFETY: `guard` holds the reader lock for `text_slab_len` bytes.
        let slab = unsafe {
            std::slice::from_raw_parts(view.text_slab, view.text_slab_len as usize)
        };
        std::str::from_utf8(slab).expect("UTF-8 text slab").to_owned()
    }

    #[test]
    fn dioxus_runtime_app_drives_surface_runtime_zero_copy() {
        let surface = DioxusSurface::new(SurfaceId::ROOT, sample_app);
        let mut rt = RuntimeBuilder::new()
            .surface_id(SurfaceId::ROOT)
            .viewport(Viewport {
                width: 400.0,
                height: 300.0,
                scale_factor: 1.0,
            })
            .app(Box::new(DioxusRuntimeApp::new(surface)))
            .build();

        assert!(rt.tick(1.0 / 60.0).expect("initial direct tick"));
        // Post-mount clean tick returns `false` (no redundant re-publish).
        assert!(!rt.tick(1.0 / 60.0).expect("clean second tick"));
        let guard = rt.acquire_scene().expect("acquire scene");
        let view = guard.view();
        assert_eq!(view.schema_hash, NVE_RUNTIME_SCHEMA_HASH);
        assert_eq!(view.frame_id, 1);
        assert!(view.node_capacity >= 3);
        assert!(!view.rows.is_null());
        assert!(!view.rects.is_null());
        assert!(view.text_slab_len > 0);
        assert!(read_text_slab(&guard).contains("Zero-copy DioxusRuntimeApp"));
    }

    #[test]
    fn dioxus_runtime_app_multi_tick_async_rerender_and_scene_busy_preserves_wakes() {
        let ch = OneShotState::default();
        let surface = DioxusSurface::new_with_context(SurfaceId::ROOT, async_app, ch.clone());
        let mut rt = RuntimeBuilder::new()
            .surface_id(SurfaceId::ROOT)
            .viewport(Viewport {
                width: 400.0,
                height: 300.0,
                scale_factor: 1.0,
            })
            .app(Box::new(DioxusRuntimeApp::new(surface)))
            .build();

        let wakes = Arc::new(AtomicUsize::new(0));
        let w = Arc::clone(&wakes);
        rt.set_wake_callback(Arc::new(move || {
            w.fetch_add(1, Ordering::Release);
        }));

        // Tick 1: mounts initial "loading" text AND post-render polls/arms the spawned future.
        assert!(rt.tick(1.0 / 60.0).expect("tick 1"));
        {
            let guard = rt.acquire_scene().expect("acquire 1");
            assert!(read_text_slab(&guard).contains("loading"));
            assert_eq!(guard.view().frame_id, 1);

            // Resolve the future on a background thread while the scene reader lock is held.
            let sender = ch.clone();
            std::thread::spawn(move || sender.send("async-resolved-42"))
                .join()
                .expect("worker thread");
            assert_eq!(
                wakes.load(Ordering::Acquire),
                1,
                "post-render waker must fire host callback from worker thread"
            );
            assert!(rt.wake_bridge().is_pending());

            // Ticking while `guard` is held returns `SceneBusy` and MUST NOT clear `wake` pending!
            assert_eq!(rt.tick_busy_probe(), Err(RuntimeError::SceneBusy));
            assert!(
                rt.wake_bridge().is_pending(),
                "SceneBusy tick must preserve pending wakeup for retry"
            );
        }

        // Tick 2 after releasing reader guard: consumes pending wake, updates signal, renders new text.
        assert!(rt.tick(1.0 / 60.0).expect("tick 2"));
        assert!(!rt.wake_bridge().is_pending());
        {
            let guard = rt.acquire_scene().expect("acquire 2");
            assert_eq!(guard.view().frame_id, 2);
            assert!(read_text_slab(&guard).contains("async-resolved-42"));
        }

        // Tick 3: steady-state clean tick returns false and keeps frame_id == 2.
        assert!(!rt.tick(1.0 / 60.0).expect("tick 3 clean"));
        assert_eq!(rt.acquire_scene().expect("acquire 3").view().frame_id, 2);
    }

    trait BusyProbeExt {
        fn tick_busy_probe(&self) -> Result<bool, RuntimeError>;
    }

    impl BusyProbeExt for nve_runtime::SurfaceRuntime {
        fn tick_busy_probe(&self) -> Result<bool, RuntimeError> {
            // Exercises the C ABI path (shared `&self`) while a `SceneReadGuard` is alive on stack.
            let api_ptr = nve_runtime::nve_runtime_get_interface(nve_abi::NVE_RUNTIME_ABI_VERSION);
            // SAFETY: Static vtable returned for matching ABI version.
            let api = unsafe { *api_ptr };
            let mut changed = 0u32;
            let handle = (self as *const nve_runtime::SurfaceRuntime)
                .cast_mut()
                .cast::<std::ffi::c_void>();
            // SAFETY: `self` is a live `SurfaceRuntime` on the owner thread.
            let rc = unsafe { (api.tick)(handle, 1.0 / 60.0, &mut changed) };
            match nve_abi::NveRuntimeStatus::from_i32(rc) {
                nve_abi::NveRuntimeStatus::Ok => Ok(changed != 0),
                nve_abi::NveRuntimeStatus::Busy => Err(RuntimeError::SceneBusy),
                _ => Err(RuntimeError::InvalidArg("unexpected status")),
            }
        }
    }
}
