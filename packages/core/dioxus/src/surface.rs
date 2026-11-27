//! `DioxusSurface` lifecycle, monomorphized zero-overhead `flush_into<S: RenderSink>`,
//! cross-boundary `BatchSink` (`render_frame_slice` / `render_frame`), signal hydration, and
//! hot-patch queueing.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use dioxus_core::{ComponentFunction, Element, ScopeId, VirtualDom};
use nve_abi::{FrameId, NodeId, SurfaceId};
use nve_components::canvas_ir::DrawList;
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::native_view::TypedNativeViewProps;
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
use nve_components::transform::Transform2D;
use nve_engine::{BackendApplyStats, BackendError, BatchSink, HotPatchMsg, RenderSink};

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

/// Non-generic Dioxus-backed surface flushing directly into any monomorphized [`RenderSink`]
/// in-process (zero encode/decode, zero vtable) and lazily using [`BatchSink`] only when
/// cross-boundary binary `CommandBuffer` output is requested.
pub struct DioxusSurface {
    surface_id: SurfaceId,
    pub(crate) vdom: VirtualDom,
    pub(crate) writer: NveMutationWriter,
    initialized: bool,
    pending_hot_patches: Vec<HotPatchMsg>,
    signal_registry: SignalRegistry,
    batch_sink: Option<BatchSink>,
}

impl DioxusSurface {
    fn from_vdom(surface_id: SurfaceId, vdom: VirtualDom) -> Self {
        Self {
            surface_id,
            vdom,
            writer: NveMutationWriter::new(surface_id),
            initialized: false,
            pending_hot_patches: Vec::new(),
            signal_registry: SignalRegistry::new(1),
            batch_sink: None,
        }
    }

    pub fn new(surface_id: SurfaceId, root: fn() -> Element) -> Self {
        Self::from_vdom(surface_id, VirtualDom::new(root))
    }

    /// Creates a surface with typed root props passed directly to `root` without thread-local
    /// bootstrap queues.
    pub fn new_with_props<P: Clone + 'static, M: 'static>(
        surface_id: SurfaceId,
        root: impl ComponentFunction<P, M>,
        props: P,
    ) -> Self {
        Self::from_vdom(surface_id, VirtualDom::new_with_props(root, props))
    }

    /// Creates a surface and injects a typed initial context into `ScopeId::ROOT` before the
    /// first render so root and descendant components can read it via `consume_context::<C>()`.
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

    /// Runs `f` inside `VirtualDom`'s `ScopeId::ROOT` runtime context (e.g. for host callbacks
    /// updating a [`crate::ReactiveTraitStore`] or root signals).
    #[inline]
    pub fn in_root_scope<R>(&mut self, f: impl FnOnce() -> R) -> R {
        self.vdom.in_scope(ScopeId::ROOT, f)
    }

    /// Polls Dioxus's internal async task/effect scheduler (`VirtualDom::wait_for_work`) once
    /// without blocking, registering `waker` if no work is immediately ready.
    ///
    /// Returns `true` when one or more scopes became dirty and a subsequent `flush_into` /
    /// `render_frame` should run. Hosts should re-call `poll_work` after flushing so newly
    /// spawned futures are polled and `waker` stays armed on the scheduler channel.
    pub fn poll_work(&mut self, waker: &core::task::Waker) -> bool {
        let mut cx = core::task::Context::from_waker(waker);
        let fut = core::pin::pin!(self.vdom.wait_for_work());
        matches!(
            core::future::Future::poll(fut, &mut cx),
            core::task::Poll::Ready(())
        )
    }

    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
    }

    #[inline]
    pub fn node_show_on(&self, node_id: NodeId) -> Option<MediaQuery> {
        self.writer.node_show_on(node_id)
    }

    #[inline]
    pub fn node_responsive_rules(&self, node_id: NodeId) -> &[ResponsiveRule] {
        self.writer.node_responsive_rules(node_id)
    }

    #[inline]
    pub fn node_cursor(&self, node_id: NodeId) -> Option<CursorKind> {
        self.writer.node_cursor(node_id)
    }

    #[inline]
    pub fn node_state_rules(&self, node_id: NodeId) -> &[StateStyleRule] {
        self.writer.node_state_rules(node_id)
    }

    #[inline]
    pub fn node_transitions(&self, node_id: NodeId) -> &[TransitionSpec] {
        self.writer.node_transitions(node_id)
    }

    #[inline]
    pub fn node_layout_ext(&self, node_id: NodeId) -> Option<&LayoutExt> {
        self.writer.node_layout_ext(node_id)
    }

    #[inline]
    pub fn node_transform(&self, node_id: NodeId) -> Option<Transform2D> {
        self.writer.node_transform(node_id)
    }

    #[inline]
    pub fn node_draw_list(&self, node_id: NodeId) -> Option<&DrawList> {
        self.writer.node_draw_list(node_id)
    }

    #[inline]
    pub fn node_native_view_props(&self, node_id: NodeId) -> Option<&TypedNativeViewProps> {
        self.writer.node_native_view_props(node_id)
    }

    #[inline]
    pub fn node_background_fill(&self, node_id: NodeId) -> Option<&BackgroundFill> {
        self.writer.node_background_fill(node_id)
    }

    #[inline]
    pub fn node_svg_picture(&self, node_id: NodeId) -> Option<&SvgPicture> {
        self.writer.node_svg_picture(node_id)
    }

    /// Encodes all signals registered on this surface into a binary `NVE_SIG1` archive.
    pub fn snapshot_signals(&self) -> Result<Vec<u8>, SignalArchiveError> {
        self.signal_registry.encode_full()
    }

    /// Hydrates registered signals on this surface from a binary `NVE_SIG1` archive with
    /// schema-evolution safety.
    pub fn hydrate_signals(&mut self, bytes: &[u8]) -> Result<HydrationReport, SignalArchiveError> {
        self.signal_registry.hydrate_from_bytes(bytes)
    }

    fn reconcile_into_parts<S: RenderSink>(
        surface_id: SurfaceId,
        vdom: &mut VirtualDom,
        writer: &mut NveMutationWriter,
        initialized: &mut bool,
        pending_hot_patches: &mut Vec<HotPatchMsg>,
        frame_id: FrameId,
        sink: &mut S,
    ) -> Result<(u64, BackendApplyStats), BackendError> {
        let t_rec = FrameTimer::start();
        let mut tx = writer.begin_tx(sink, surface_id, frame_id);
        if !*initialized {
            vdom.rebuild(&mut tx);
            // Flush any scopes dirtied during initial mount (e.g. an ErrorBoundary catching a
            // child's initial render error/panic) so frame 1 immediately emits the fallback UI.
            vdom.render_immediate(&mut tx);
            *initialized = true;
        } else {
            vdom.render_immediate(&mut tx);
            // Flush any follow-up scopes queued during render (e.g. an ErrorBoundary catching a
            // child's `Result::Err` or panic during an incremental update).
            vdom.render_immediate(&mut tx);
        }
        for patch in pending_hot_patches.drain(..) {
            tx.writer.apply_hot_patch(tx.sink, &patch);
        }
        let reconcile_us = t_rec.elapsed_us();
        let stats = tx.finish_frame(true)?;
        Ok((reconcile_us, stats))
    }

    /// Reconciles the `VirtualDom` and streams all core & extended typed mutations directly into
    /// `sink` with zero binary encoding, zero decoding, and zero vtable indirection.
    #[inline]
    pub fn flush_into<S: RenderSink>(
        &mut self,
        frame_id: FrameId,
        sink: &mut S,
    ) -> Result<BackendApplyStats, BackendError> {
        let (_, stats) = Self::reconcile_into_parts(
            self.surface_id,
            &mut self.vdom,
            &mut self.writer,
            &mut self.initialized,
            &mut self.pending_hot_patches,
            frame_id,
            sink,
        )?;
        Ok(stats)
    }

    /// Timed variant of [`Self::flush_into`] returning `(reconcile_us, stats)` for `SurfaceRunner`
    /// profiling.
    #[inline]
    pub fn flush_into_timed<S: RenderSink>(
        &mut self,
        frame_id: FrameId,
        sink: &mut S,
    ) -> Result<(u64, BackendApplyStats), BackendError> {
        Self::reconcile_into_parts(
            self.surface_id,
            &mut self.vdom,
            &mut self.writer,
            &mut self.initialized,
            &mut self.pending_hot_patches,
            frame_id,
            sink,
        )
    }

    /// Reconciles the `VirtualDom` into the lazy cross-boundary [`BatchSink`] and returns a
    /// zero-copy `CommandBuffer` slice valid until the next frame call.
    pub fn render_frame_slice(&mut self, frame_id: FrameId) -> &[u8] {
        let sink = self
            .batch_sink
            .get_or_insert_with(|| BatchSink::new(self.surface_id));
        let _ = Self::reconcile_into_parts(
            self.surface_id,
            &mut self.vdom,
            &mut self.writer,
            &mut self.initialized,
            &mut self.pending_hot_patches,
            frame_id,
            sink,
        );
        sink.bytes()
    }

    /// Reconciles the `VirtualDom` into the lazy cross-boundary [`BatchSink`] and returns
    /// `(reconcile_us, encode_us, command_buffer_bytes)`.
    pub fn render_frame(&mut self, frame_id: FrameId) -> (u64, u64, Vec<u8>) {
        let sink = self
            .batch_sink
            .get_or_insert_with(|| BatchSink::new(self.surface_id));
        let t_enc = FrameTimer::start();
        let reconcile_us = Self::reconcile_into_parts(
            self.surface_id,
            &mut self.vdom,
            &mut self.writer,
            &mut self.initialized,
            &mut self.pending_hot_patches,
            frame_id,
            sink,
        )
        .map_or(0, |(r, _)| r);
        let total_us = t_enc.elapsed_us();
        let encode_us = total_us.saturating_sub(reconcile_us);
        (reconcile_us, encode_us, sink.to_vec())
    }

    pub fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        if matches!(patch, HotPatchMsg::TemplatePatch { .. }) {
            self.pending_hot_patches.push(patch.clone());
            true
        } else {
            false
        }
    }
}
