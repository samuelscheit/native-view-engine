//! `DioxusSurface` lifecycle, zero-copy `render_frame_slice`, signal hydration, and hot-patch queueing.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use dioxus_core::{Element, VirtualDom};
use nve_abi::{FrameId, SurfaceId};
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

    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
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
            self.initialized = true;
        } else {
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
