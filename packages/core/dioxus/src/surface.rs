//! `DioxusSurface` lifecycle, zero-copy `render_frame_slice`, signal hydration, and hot-patch queueing.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use dioxus_core::{Element, VirtualDom};
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
use nve_engine::HotPatchMsg;

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

/// Complete Dioxus-backed surface producing batched `nve-abi` `CommandBuffer` frames.
pub struct DioxusSurface {
    surface_id: SurfaceId,
    pub(crate) vdom: VirtualDom,
    pub(crate) writer: NveMutationWriter,
    initialized: bool,
    pending_hot_patches: Vec<HotPatchMsg>,
    signal_registry: SignalRegistry,
}

impl DioxusSurface {
    pub fn new(surface_id: SurfaceId, root: fn() -> Element) -> Self {
        Self {
            surface_id,
            vdom: VirtualDom::new(root),
            writer: NveMutationWriter::new(surface_id),
            initialized: false,
            pending_hot_patches: Vec::new(),
            signal_registry: SignalRegistry::new(1),
        }
    }

    /// Runs `f` inside `VirtualDom`'s `ScopeId::ROOT` runtime context (e.g. for host callbacks
    /// updating a [`crate::ReactiveTraitStore`] or root signals).
    #[inline]
    pub fn in_root_scope<R>(&mut self, f: impl FnOnce() -> R) -> R {
        self.vdom.in_scope(dioxus_core::ScopeId::ROOT, f)
    }

    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
    }

    /// Direct typed [`MediaQuery`] visibility gate stored on `node_id` (zero serialization).
    #[inline]
    pub fn node_show_on(&self, node_id: NodeId) -> Option<MediaQuery> {
        self.writer.node_show_on(node_id)
    }

    /// Direct typed [`ResponsiveRule`] slice stored on `node_id` (zero serialization).
    #[inline]
    pub fn node_responsive_rules(&self, node_id: NodeId) -> &[ResponsiveRule] {
        self.writer.node_responsive_rules(node_id)
    }

    /// Iterates over all active nodes with a declarative [`MediaQuery`] visibility gate.
    #[inline]
    pub fn iter_show_on(&self) -> impl Iterator<Item = (NodeId, MediaQuery)> + '_ {
        self.writer.iter_show_on()
    }

    /// Iterates over all active nodes with one or more [`ResponsiveRule`] style overrides.
    #[inline]
    pub fn iter_responsive_rules(&self) -> impl Iterator<Item = (NodeId, &[ResponsiveRule])> + '_ {
        self.writer.iter_responsive_rules()
    }

    /// Direct typed [`CursorKind`] stored on `node_id` (zero serialization).
    #[inline]
    pub fn node_cursor(&self, node_id: NodeId) -> Option<CursorKind> {
        self.writer.node_cursor(node_id)
    }

    /// Direct typed [`StateStyleRule`] slice stored on `node_id` (zero serialization).
    #[inline]
    pub fn node_state_rules(&self, node_id: NodeId) -> &[StateStyleRule] {
        self.writer.node_state_rules(node_id)
    }

    /// Iterates over all active nodes with an explicit [`CursorKind`].
    #[inline]
    pub fn iter_cursors(&self) -> impl Iterator<Item = (NodeId, CursorKind)> + '_ {
        self.writer.iter_cursors()
    }

    /// Iterates over all active nodes with one or more interactive [`StateStyleRule`] overrides.
    #[inline]
    pub fn iter_state_rules(&self) -> impl Iterator<Item = (NodeId, &[StateStyleRule])> + '_ {
        self.writer.iter_state_rules()
    }

    /// Direct typed [`TransitionSpec`] slice stored on `node_id` (zero serialization).
    #[inline]
    pub fn node_transitions(&self, node_id: NodeId) -> &[TransitionSpec] {
        self.writer.node_transitions(node_id)
    }

    /// Iterates over all active nodes with one or more declarative [`TransitionSpec`]s.
    #[inline]
    pub fn iter_transitions(&self) -> impl Iterator<Item = (NodeId, &[TransitionSpec])> + '_ {
        self.writer.iter_transitions()
    }

    /// Direct typed [`LayoutExt`] stored on `node_id` (zero serialization).
    #[inline]
    pub fn node_layout_ext(&self, node_id: NodeId) -> Option<&LayoutExt> {
        self.writer.node_layout_ext(node_id)
    }

    /// Iterates over all active nodes with non-default [`LayoutExt`] metadata.
    #[inline]
    pub fn iter_layout_ext(&self) -> impl Iterator<Item = (NodeId, &LayoutExt)> + '_ {
        self.writer.iter_layout_ext()
    }

    /// Direct typed [`Transform2D`] stored on `node_id` (zero serialization).
    #[inline]
    pub fn node_transform(&self, node_id: NodeId) -> Option<Transform2D> {
        self.writer.node_transform(node_id)
    }

    /// Iterates over all active nodes with an explicit [`Transform2D`].
    #[inline]
    pub fn iter_transforms(&self) -> impl Iterator<Item = (NodeId, Transform2D)> + '_ {
        self.writer.iter_transforms()
    }

    /// Enables or disables pure in-process mode (zero `NVE_DRAW1` and zero `NVNP` wire encoding
    /// in `CommandBuffer`; backends receive direct typed [`DrawList`] & [`TypedNativeViewProps`]).
    #[inline]
    pub fn set_in_process_mode(&mut self, enabled: bool) {
        self.writer.set_in_process_mode(enabled);
    }

    /// Builder-style variant of [`Self::set_in_process_mode`].
    #[inline]
    pub fn with_in_process_mode(mut self, enabled: bool) -> Self {
        self.writer.set_in_process_mode(enabled);
        self
    }

    #[inline]
    pub fn in_process_mode(&self) -> bool {
        self.writer.in_process_mode()
    }

    /// Direct zero-serialization [`DrawList`] (`Arc<[CanvasOp]>`) stored on `node_id`.
    #[inline]
    pub fn node_draw_list(&self, node_id: NodeId) -> Option<&DrawList> {
        self.writer.node_draw_list(node_id)
    }

    /// Iterates over all active canvas nodes with a direct typed [`DrawList`].
    #[inline]
    pub fn iter_draw_lists(&self) -> impl Iterator<Item = (NodeId, &DrawList)> + '_ {
        self.writer.iter_draw_lists()
    }

    /// Direct zero-serialization [`TypedNativeViewProps`] stored on `node_id`.
    #[inline]
    pub fn node_native_view_props(&self, node_id: NodeId) -> Option<&TypedNativeViewProps> {
        self.writer.node_native_view_props(node_id)
    }

    /// Iterates over all active `<NativeView>` nodes with direct typed [`TypedNativeViewProps`].
    #[inline]
    pub fn iter_native_views(&self) -> impl Iterator<Item = (NodeId, &TypedNativeViewProps)> + '_ {
        self.writer.iter_native_views()
    }

    /// Direct zero-serialization [`BackgroundFill`] stored on `node_id`.
    #[inline]
    pub fn node_background_fill(&self, node_id: NodeId) -> Option<&BackgroundFill> {
        self.writer.node_background_fill(node_id)
    }

    /// Iterates over all active nodes with a direct typed [`BackgroundFill`].
    #[inline]
    pub fn iter_background_fills(&self) -> impl Iterator<Item = (NodeId, &BackgroundFill)> + '_ {
        self.writer.iter_background_fills()
    }

    /// Direct zero-serialization [`SvgPicture`] stored on `node_id`.
    #[inline]
    pub fn node_svg_picture(&self, node_id: NodeId) -> Option<&SvgPicture> {
        self.writer.node_svg_picture(node_id)
    }

    /// Iterates over all active nodes with a direct typed [`SvgPicture`].
    #[inline]
    pub fn iter_svg_pictures(&self) -> impl Iterator<Item = (NodeId, &SvgPicture)> + '_ {
        self.writer.iter_svg_pictures()
    }

    /// Encodes all signals registered on this surface into a zero-JSON `NVE_SIG1` binary archive.
    pub fn snapshot_signals(&self) -> Result<Vec<u8>, SignalArchiveError> {
        self.signal_registry.encode_full()
    }

    /// Hydrates registered signals on this surface from a binary `NVE_SIG1` archive with
    /// schema-evolution safety.
    pub fn hydrate_signals(&mut self, bytes: &[u8]) -> Result<HydrationReport, SignalArchiveError> {
        self.signal_registry.hydrate_from_bytes(bytes)
    }

    fn reconcile_frame(&mut self, frame_id: FrameId) {
        self.writer.begin_frame(self.surface_id, frame_id);
        if !self.initialized {
            self.vdom.rebuild(&mut self.writer);
            // Flush any scopes dirtied during initial mount (e.g. an ErrorBoundary catching a
            // child's initial render error/panic) so frame 1 immediately emits the fallback UI.
            self.vdom.render_immediate(&mut self.writer);
            self.initialized = true;
        } else {
            self.vdom.render_immediate(&mut self.writer);
            // Flush any follow-up scopes queued during render (e.g. an ErrorBoundary catching a
            // child's `Result::Err` or panic during an incremental update, which queues its
            // subscriber onto the runtime event channel without draining mid-pass).
            self.vdom.render_immediate(&mut self.writer);
        }
        for patch in self.pending_hot_patches.drain(..) {
            self.writer.apply_hot_patch(&patch);
        }
    }

    /// Reconciles the `VirtualDom` and returns a zero-copy slice of the encoded binary
    /// `CommandBuffer` valid until the next frame call.
    pub fn render_frame_slice(&mut self, frame_id: FrameId) -> &[u8] {
        self.reconcile_frame(frame_id);
        self.writer.finish_frame(true)
    }

    /// Reconciles the `VirtualDom` and returns `(reconcile_us, encode_us, command_buffer_bytes)`.
    pub fn render_frame(&mut self, frame_id: FrameId) -> (u64, u64, Vec<u8>) {
        let t_rec = FrameTimer::start();
        self.reconcile_frame(frame_id);
        let reconcile_us = t_rec.elapsed_us();

        let t_enc = FrameTimer::start();
        let bytes = self.writer.finish_frame(true).to_vec();
        let encode_us = t_enc.elapsed_us();
        (reconcile_us, encode_us, bytes)
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
