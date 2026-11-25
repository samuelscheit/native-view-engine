//! Core framework-agnostic runtime: `RenderBackend` trait, `Surface` coordinator,
//! 6-stage `FrameProfiler` with Perfetto/Chrome trace export, and 2-tier hot-reload spine.

pub mod app;
pub mod hotreload;
pub mod layout;
#[cfg(not(target_arch = "wasm32"))]
pub mod loader;
pub mod profiler;
pub mod snapshot;

pub use nve_abi as abi;
use nve_abi::{DecodeError, FrameId, NativeEvent, NodeId, SurfaceId};

pub use app::NveApp;
pub use hotreload::HotPatchMsg;
pub use layout::{
    BuiltinLayout, DynLayoutEngine, HeuristicTextMeasurer, LayoutEngine, TextMeasurer,
};
pub use profiler::{FrameProfiler, FrameTiming};
pub use snapshot::{
    encode_snapshot, fnv1a_64, write_snapshot_atomic, MmapSnapshot, RelocatableStateArena,
    RelocatableTableView, RelocatableValue, SnapshotError, SnapshotHeader, NVE_SNAP_FORMAT_VERSION,
    NVE_SNAP_MAGIC, SNAPSHOT_HEADER_SIZE,
};

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
    InvalidNode(NodeId),
    Internal(String),
}

impl From<DecodeError> for BackendError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for BackendError {}

/// Universal backend trait implemented by:
/// - `nve-backend-headless` (pure-Rust Taffy layout + mutation recorder)
/// - `cxx/nve-fabric-bridge` (React Native Fabric iOS `UIView` & Android `View`)
/// - `nve-backend-skia` (macOS, Windows, Linux, iOS, Android GPU Skia)
/// - `nve-backend-web` (WASM -> browser DOM)
pub trait RenderBackend {
    fn name(&self) -> &'static str;

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport);

    /// Applies one batched binary `CommandBuffer` frame and returns backend timing breakdown.
    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError>;

    /// Drains any pending native input/layout events since the last frame.
    fn drain_events(&mut self) -> Vec<NativeEvent>;

    /// Returns computed layout rectangle for a node if available.
    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect>;
}

/// Manages frame sequencing and stage profiling for one rendered surface.
pub struct SurfaceRunner<B: RenderBackend> {
    surface_id: SurfaceId,
    next_frame_id: FrameId,
    backend: B,
    profiler: FrameProfiler,
}

impl<B: RenderBackend> SurfaceRunner<B> {
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

    pub fn submit_recorded_frame(
        &mut self,
        frame_id: FrameId,
        reconcile_us: u64,
        encode_us: u64,
        command_bytes: &[u8],
    ) -> Result<FrameTiming, BackendError> {
        let stats = self.backend.submit_frame(command_bytes)?;
        let timing = FrameTiming {
            frame_id,
            op_count: stats.op_count,
            mutation_count: stats.mutation_count,
            reconcile_us,
            encode_us,
            decode_us: stats.decode_us,
            layout_us: stats.layout_us,
            diff_us: stats.diff_us,
            mount_us: stats.mount_us,
        };
        self.profiler.record(timing);
        Ok(timing)
    }
}
