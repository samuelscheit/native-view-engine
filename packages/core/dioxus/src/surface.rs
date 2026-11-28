//! `DioxusSurface` lifecycle over a single-owner [`NodeStore`], direct zero-copy
//! `commit_to<B: StoreBackend>`, cross-boundary `NVE2` encoding (`render_nve2`), signal hydration,
//! and surgical Tier-1 hot-patching.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use dioxus_core::{ComponentFunction, Element, ScopeId, VirtualDom};
use nve_abi::{FrameId, SurfaceId};
use nve_tables::media::MediaQuery;
use nve_tables::tables::{Responsive, ResponsiveRules, ResponsiveRulesView};
use nve_engine::{BackendApplyStats, BackendError, HotPatchMsg, StoreBackend};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::hot_patch::PendingHotPatchQueue;
use crate::signal_archive::{HydrationReport, SignalArchiveError, SignalRegistry};
use crate::writer::NveMutationWriter;

#[cfg(not(target_arch = "wasm32"))]
struct FrameTimer(Instant);

#[cfg(not(target_arch = "wasm32"))]
impl FrameTimer {
    fn start() -> Self {
        Self(Instant::now())
    }

    fn elapsed_us(&self) -> u64 {
        self.0.elapsed().as_micros() as u64
    }
}

#[cfg(target_arch = "wasm32")]
struct FrameTimer;

#[cfg(target_arch = "wasm32")]
impl FrameTimer {
    fn start() -> Self {
        Self
    }

    fn elapsed_us(&self) -> u64 {
        0
    }
}

/// Non-generic Dioxus-backed surface owning a single [`NodeStore`] and committing directly into
/// any [`StoreBackend`] in-process (zero encode/decode, zero mirror tree) or encoding `NVE2`
/// deltas for cross-boundary FFI/wire hosts.
pub struct DioxusSurface {
    surface_id: SurfaceId,
    pub(crate) vdom: VirtualDom,
    pub(crate) writer: NveMutationWriter,
    pub(crate) event_slots: crate::events::EventSlots,
    initialized: bool,
    pending_hot_patches: PendingHotPatchQueue,
    signal_registry: SignalRegistry,
    wire_encoder: WireEncoder,
    wire_buf: Vec<u8>,
}

impl DioxusSurface {
    fn from_vdom(surface_id: SurfaceId, vdom: VirtualDom) -> Self {
        Self {
            surface_id,
            vdom,
            writer: NveMutationWriter::new(surface_id),
            event_slots: crate::events::EventSlots::new(),
            initialized: false,
            pending_hot_patches: PendingHotPatchQueue::default(),
            signal_registry: SignalRegistry::new(1),
            wire_encoder: WireEncoder::default(),
            wire_buf: Vec::new(),
        }
    }

    pub fn new(surface_id: SurfaceId, root: fn() -> Element) -> Self {
        Self::from_vdom(surface_id, VirtualDom::new(root))
    }

    /// Creates a surface with typed root props passed directly to `root`.
    pub fn new_with_props<P: Clone + 'static, M: 'static>(
        surface_id: SurfaceId,
        root: impl ComponentFunction<P, M>,
        props: P,
    ) -> Self {
        Self::from_vdom(surface_id, VirtualDom::new_with_props(root, props))
    }

    /// Creates a surface and injects a typed initial context into `ScopeId::ROOT` before mount.
    pub fn new_with_context<C: Clone + 'static>(
        surface_id: SurfaceId,
        root: fn() -> Element,
        context: C,
    ) -> Self {
        Self::new(surface_id, root).with_context(context)
    }

    /// Injects an additional typed context value into `ScopeId::ROOT` before initial mount.
    pub fn with_context<C: Clone + 'static>(self, context: C) -> Self {
        self.vdom.in_scope(ScopeId::ROOT, || {
            dioxus_core::provide_context(context);
        });
        self
    }

    /// Runs `f` inside `VirtualDom`'s `ScopeId::ROOT` runtime context.
    #[inline]
    pub fn in_root_scope<R>(&mut self, f: impl FnOnce() -> R) -> R {
        self.vdom.in_scope(ScopeId::ROOT, f)
    }

    /// Polls Dioxus's internal async task/effect scheduler (`VirtualDom::wait_for_work`) once
    /// without blocking, registering `waker` if no work is immediately ready.
    ///
    /// Returns `true` when one or more scopes became dirty and a subsequent `commit_to` /
    /// `render_nve2` should run. Hosts should re-call `poll_work` after flushing so newly
    /// spawned futures are polled and `waker` stays armed on the scheduler channel.
    pub fn poll_work(&mut self, waker: &core::task::Waker) -> bool {
        let mut cx = core::task::Context::from_waker(waker);
        let fut = core::pin::pin!(self.vdom.wait_for_work());
        matches!(
            core::future::Future::poll(fut, &mut cx),
            core::task::Poll::Ready(())
        )
    }

    /// Awaits the next dirty scope or completed async task on the underlying `VirtualDom`.
    ///
    /// Used by streaming SSR executors (`nve-ssr`) inside a cancellation-safe `select!` loop
    /// paired with [`Self::commit_to`] so mutations are streamed out as suspense boundaries
    /// resolve.
    pub async fn wait_for_work(&mut self) {
        self.vdom.wait_for_work().await;
    }

    #[inline]
    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    #[inline]
    pub fn store(&self) -> &NodeStore {
        self.writer.store()
    }

    #[inline]
    pub fn store_mut(&mut self) -> &mut NodeStore {
        self.writer.store_mut()
    }

    #[inline]
    pub fn last_changes(&self) -> &ChangeSet {
        self.writer.last_changes()
    }

    #[inline]
    pub fn node_show_on(&self, node_id: NodeId) -> Option<MediaQuery> {
        self.writer
            .store()
            .try_row::<Responsive>(node_id)
            .and_then(|r| r.show_on())
    }

    #[inline]
    pub fn node_responsive_rules(&self, node_id: NodeId) -> Option<ResponsiveRulesView<'_>> {
        let store = self.writer.store();
        let resp = store.try_row::<Responsive>(node_id)?;
        let res = resp.rules();
        if res.is_none() {
            return None;
        }
        store.resource::<ResponsiveRules>(res)
    }

    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
    }

    /// Encodes all signals registered on this surface into a binary `NVE_SIG1` archive.
    pub fn snapshot_signals(&self) -> Result<Vec<u8>, SignalArchiveError> {
        self.signal_registry.encode_full()
    }

    /// Hydrates registered signals on this surface from a binary `NVE_SIG1` archive.
    pub fn hydrate_signals(&mut self, bytes: &[u8]) -> Result<HydrationReport, SignalArchiveError> {
        self.signal_registry.hydrate_from_bytes(bytes)
    }

    /// Reconciles the Dioxus `VirtualDom` directly into the owned [`NodeStore`] and populates
    /// [`Self::last_changes`]. Returns reconcile time in microseconds.
    pub fn reconcile(&mut self, _frame_id: FrameId) -> u64 {
        let t_rec = FrameTimer::start();
        let vdom = &mut self.vdom;
        let initialized = &mut self.initialized;
        let patches = &mut self.pending_hot_patches;

        self.writer.with_transaction(|mtx| {
            if !*initialized {
                vdom.rebuild(mtx);
                *initialized = true;
            }
            // Exactly two synchronous passes: the second flushes scopes dirtied by the first (a
            // parent `ErrorBoundary` catching a render error, a `SuspenseBoundary` restoring its
            // fallback). Deeper cascades (a boundary's `on_error` re-rendering an ancestor that
            // throws into another boundary) settle in the drive loop, before anything is painted
            // or streamed: host pumps (`poll_work` -> commit, `MAX_PUMPS` per tick) and
            // `nve-ssr`'s `flush_ssr_until_quiescent`. More passes here would not be safe: every
            // pass also polls already-woken tasks, so a render-chained async burst (render wakes
            // a task, the task dirties the scope) would advance once per pass and defeat the
            // host's `MAX_PUMPS` paint yield; edit counts cannot tell the two apart (a re-render
            // that throws keeps its last good nodes and emits zero edits).
            vdom.render_immediate(mtx);
            vdom.render_immediate(mtx);
            patches.apply_all(mtx);
        });
        t_rec.elapsed_us()
    }

    /// Reconciles the `VirtualDom` into `NodeStore` and commits the resulting [`ChangeSet`]
    /// directly into `backend` with zero wire serialization.
    #[inline]
    pub fn commit_to<B: StoreBackend>(
        &mut self,
        frame_id: FrameId,
        backend: &mut B,
    ) -> Result<BackendApplyStats, BackendError> {
        let (_, stats) = self.commit_to_timed(frame_id, backend)?;
        Ok(stats)
    }

    /// Timed variant of [`Self::commit_to`] returning `(reconcile_us, stats)`.
    #[inline]
    pub fn commit_to_timed<B: StoreBackend>(
        &mut self,
        frame_id: FrameId,
        backend: &mut B,
    ) -> Result<(u64, BackendApplyStats), BackendError> {
        let reconcile_us = self.reconcile(frame_id);
        let stats = backend.commit(self.writer.store(), self.writer.last_changes())?;
        Ok((reconcile_us, stats))
    }

    /// Reconciles the `VirtualDom` and encodes the incremental `NVE2` delta frame into `out`.
    /// Returns `(reconcile_us, encode_us)`.
    pub fn render_nve2(&mut self, frame_id: FrameId, out: &mut Vec<u8>) -> (u64, u64) {
        let reconcile_us = self.reconcile(frame_id);
        let t_enc = FrameTimer::start();
        out.clear();
        self.wire_encoder
            .encode(self.writer.store(), self.writer.last_changes(), out);
        (reconcile_us, t_enc.elapsed_us())
    }

    /// Reconciles the `VirtualDom` and returns a borrowed `NVE2` delta slice valid until the next
    /// frame (used by cross-boundary FFI `dlopen` / `nve-wire` hosts).
    pub fn render_nve2_slice(&mut self, frame_id: FrameId) -> &[u8] {
        let _ = self.reconcile(frame_id);
        self.wire_buf.clear();
        self.wire_encoder.encode(
            self.writer.store(),
            self.writer.last_changes(),
            &mut self.wire_buf,
        );
        &self.wire_buf
    }

    /// Resets the `NVE2` encoder session so the next frame re-declares all tables.
    pub fn reset_wire_encoder(&mut self) {
        self.wire_encoder.reset();
    }

    pub fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        self.pending_hot_patches.push(patch)
    }

    /// Applies one Tier-1 hot patch immediately onto `NodeStore` (populating [`Self::last_changes`])
    /// without queuing or touching the `VirtualDom`. Zero steady-state heap allocations.
    pub fn apply_hot_patch_now(&mut self, patch: &HotPatchMsg) -> bool {
        self.writer.apply_hot_patch(patch)
    }
}
