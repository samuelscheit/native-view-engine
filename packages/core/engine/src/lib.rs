//! Core framework-agnostic runtime: `StoreBackend` trait, `SurfaceRunner`
//! coordinator, `StoreWireReplica` for cross-boundary `NVE2` frames, 6-stage `FrameProfiler` with
//! Perfetto/Chrome trace export, and 2-tier hot-reload spine.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod app;
pub mod hotreload;
pub mod instant_boot;
#[cfg(not(target_arch = "wasm32"))]
pub mod loader;
pub mod mmap_arena;
pub mod profiler;
pub mod reloc;
pub mod store_backend;

pub use nve_abi as abi;
use nve_abi::{DecodeError, FrameId, NodeId, SurfaceId};
use nve_store::{ChangeSet, NodeStore};

pub use app::{clear_last_ffi_panic, last_ffi_panic, take_last_ffi_panic, NveApp};
pub use store_backend::{StoreBackend, StoreWireReplica};

pub use hotreload::HotPatchMsg;
pub use instant_boot::{
    default_instant_boot_dir, is_snapshot_disabled, is_snapshot_reset_requested,
    reset_instant_boot_dir, resolve_sidecar_frame0_path, InstantBootError, InstantFrame0,
    NativeInstantBootStore, NativeStateStore, StateBlobRoot, StateRecoveryStats, StateSaveOutcome,
    StateStoreConfig, DEFAULT_STATE_ARENA_CAPACITY, DEFAULT_STATE_ARENA_FILE,
    DEFAULT_STATE_COMPACT_HEADROOM, DEFAULT_USER_FRAME0_FILE,
};
pub use mmap_arena::{
    ArenaError, ArenaHeaderSlot, ArenaPod, MmapOffsetArena, OffsetPtr, OffsetSlice, OffsetStr,
    TrustedUtf8Str,
};
pub use profiler::{FrameProfiler, FrameTiming};
pub use reloc::{
    compute_reloc_checksum, encode_reloc_bundle, encode_scene_reloc, encode_timeline_archive,
    write_reloc_atomic, MmapReloc, NveNodePatch, NveNodePatchField, NveNodePatchValue,
    NveSceneArchive, NveSceneArchiveView, NveTimelineArchive, NveTimelineArchiveView,
    NveTimelineEntry, NveTimelineEntryInput, NveTimelinePatchInput, NveTimelineRoot, RelocBuf,
    RelocBundleRoot, RelocBundleView, RelocError, RelocHeader, RelocLayoutProps, RelocPod,
    RelocPtr, RelocSceneNode, RelocSceneRoot, RelocSlice, RelocStr, RelocView, RelocWriter,
    SceneRelocInput, SceneRelocNodeInput, SceneRelocView, VerifyMode,
    NVE_NODE_PATCH_FLAG_STRING_SPAN, RELOC_BUF_ALIGN, RELOC_HEADER_SIZE, RELOC_KIND_BUNDLE,
    RELOC_KIND_GENERIC, RELOC_KIND_SCENE, RELOC_KIND_TIMELINE, RELOC_KIND_WORKSPACE, RELOC_MAGIC,
    RELOC_VERSION, SCENE_NODE_CLIP_CHILDREN, SCENE_NODE_HAS_EXPLICIT_STYLE, SCENE_NODE_HIDDEN,
    SCENE_NODE_INTERACTIVE,
};

/// Self-documenting alias for the memory-mapped zero-copy state arena (`MmapOffsetArena`).
pub type NveStateArena = MmapOffsetArena;
/// Self-documenting alias for an encoded signal/state snapshot archive (`RelocBuf`).
pub type NveSignalSnapshot = RelocBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub scale_factor: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 393.0,
            height: 852.0,
            scale_factor: 3.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BackendApplyStats {
    pub op_count: u32,
    pub mutation_count: u32,
    pub decode_us: u64,
    pub layout_us: u64,
    pub diff_us: u64,
    pub mount_us: u64,
}

#[derive(Debug)]
pub enum BackendError {
    Decode(DecodeError),
    Wire(nve_store::wire::WireError),
    /// Replica hit a mid-frame wire failure and reset; deltas are rejected until a `frame == 1` keyframe arrives.
    NeedsKeyframe(Option<nve_store::wire::WireError>),
    InvalidNode(NodeId),
    Internal(String),
}

impl From<DecodeError> for BackendError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl From<nve_store::wire::WireError> for BackendError {
    fn from(e: nve_store::wire::WireError) -> Self {
        Self::Wire(e)
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for BackendError {}

/// Manages frame sequencing and stage profiling for one rendered surface backed by a [`StoreBackend`].
pub struct SurfaceRunner<B: StoreBackend> {
    surface_id: SurfaceId,
    next_frame_id: FrameId,
    backend: B,
    profiler: FrameProfiler,
}

impl<B: StoreBackend> SurfaceRunner<B> {
    pub fn new(surface_id: SurfaceId, viewport: Viewport, mut backend: B) -> Self {
        backend.set_viewport(surface_id, viewport);
        Self {
            surface_id,
            next_frame_id: 1,
            backend,
            profiler: FrameProfiler::new(512),
        }
    }

    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn profiler(&self) -> &FrameProfiler {
        &self.profiler
    }

    pub fn next_frame_id(&mut self) -> FrameId {
        let id = self.next_frame_id;
        self.next_frame_id += 1;
        id
    }

    /// Commits one already-committed `(&NodeStore, &ChangeSet)` directly into the backend with
    /// zero wire serialization or copying, recording exact `FrameTiming` in `profiler`.
    pub fn commit_store(
        &mut self,
        frame_id: FrameId,
        reconcile_us: u64,
        store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<FrameTiming, BackendError> {
        let stats = self.backend.commit(store, changes)?;
        let timing = FrameTiming {
            frame_id,
            op_count: stats.op_count,
            mutation_count: stats.mutation_count,
            reconcile_us,
            encode_us: 0,
            decode_us: stats.decode_us,
            layout_us: stats.layout_us,
            diff_us: stats.diff_us,
            mount_us: stats.mount_us,
        };
        self.profiler.record(timing);
        Ok(timing)
    }

    /// Runs a timed producer/commit closure against the backend and records exact `FrameTiming`.
    pub fn commit_with<F>(&mut self, flush: F) -> Result<FrameTiming, BackendError>
    where
        F: FnOnce(FrameId, &mut B) -> Result<(u64, BackendApplyStats), BackendError>,
    {
        let frame_id = self.next_frame_id();
        let (reconcile_us, stats) = flush(frame_id, &mut self.backend)?;
        let timing = FrameTiming {
            frame_id,
            op_count: stats.op_count,
            mutation_count: stats.mutation_count,
            reconcile_us,
            encode_us: 0,
            decode_us: stats.decode_us,
            layout_us: stats.layout_us,
            diff_us: stats.diff_us,
            mount_us: stats.mount_us,
        };
        self.profiler.record(timing);
        Ok(timing)
    }
}
