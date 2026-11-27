//! Framework-agnostic host-runtime application and per-frame tick hook contracts.
//!
//! Defined in `nve-engine` so frontend adapters (such as `nve-dioxus::DioxusRuntimeApp`) implement
//! [`RuntimeApp`] without depending on the native host runtime (`nve-runtime`, layout engines, or
//! network wire transports). Re-exported identically from `nve-runtime`.

use std::sync::Arc;

use nve_abi::{FrameId, NveDirectEvent, NveEventBatchRef, NvePropsRef};
use nve_store::{ChangeSet, NodeStore, Transaction};

use crate::HotPatchMsg;
use crate::wake::WakeBridge;

/// Output of one [`RuntimeApp::render_frame`] pass.
pub enum AppFrame<'a> {
    /// No new tree changes were produced this frame.
    Clean,
    /// In-process zero-serialization `ChangeSet` against [`RuntimeApp::direct_store`] (e.g. `DioxusRuntimeApp`).
    Direct { changes: &'a ChangeSet },
    /// Cross-boundary binary `NVE2` delta/keyframe slice (e.g. `NveApp`, `cdylib`, or remote wire).
    Wire(&'a [u8]),
}

/// The attached app does not implement an optional [`RuntimeApp`] capability (embedded props,
/// outbound events, state snapshots, or hot patching).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityUnsupported;

/// Framework-agnostic application contract hosted by `nve_runtime::SurfaceRuntime`.
///
/// Implementations may either own a `NodeStore` directly (zero wire encode/decode via
/// [`AppFrame::Direct`]) or emit binary `NVE2` frames ([`AppFrame::Wire`]).
pub trait RuntimeApp {
    /// Produces the next frame's changes (`Direct`, `Wire`, or `Clean`).
    fn render_frame(&mut self, frame_id: FrameId) -> AppFrame<'_>;

    /// Dispatches a direct typed event batch into the application. Returns `true` when the app
    /// dirtied state and needs a re-render.
    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool;

    /// Borrows the app's authoritative `NodeStore` when running in zero-copy direct mode.
    fn direct_store(&self) -> Option<&NodeStore> {
        None
    }

    /// Mutably borrows the app's authoritative `NodeStore` (for [`FrameTickHook`]s in direct mode).
    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        None
    }

    /// Polls any pending async work (e.g. Dioxus tasks or background streams) and arms the
    /// installed [`WakeBridge`] waker on newly spawned pending futures. Returns `true` when new
    /// work became ready and a render should follow.
    fn poll_async_work(&mut self) -> bool {
        false
    }

    /// Installs the runtime's single coalesced [`WakeBridge`] so async tasks wake the host loop
    /// directly without double-layered wake latches.
    ///
    /// Returns `true` if the app had a pending wake prior to installation; callers trigger the
    /// newly installed bridge *after* releasing any surface write lock.
    fn install_wake_bridge(&mut self, _wake: Arc<WakeBridge>) -> bool {
        false
    }

    /// Applies a Tier-1 binary hot-reload patch payload. `Ok(true)` when the surface dirtied.
    fn apply_hot_patch(&mut self, _patch: &HotPatchMsg) -> Result<bool, CapabilityUnsupported> {
        Err(CapabilityUnsupported)
    }

    /// Applies embedding-host props (RN / SwiftUI / Compose / Flutter widget props). `Ok(true)`
    /// when the app needs a re-render.
    fn set_props(&mut self, _props: NvePropsRef<'_>) -> Result<bool, CapabilityUnsupported> {
        Err(CapabilityUnsupported)
    }

    /// Drains component-emitted outbound events, borrowed until the next call on `self`.
    fn drain_emitted_events(&mut self) -> Result<&[NveDirectEvent], CapabilityUnsupported> {
        Err(CapabilityUnsupported)
    }

    /// Serializes app state (instant boot / Tier-2 hot swap), borrowed until the next call on `self`.
    fn snapshot_state(&mut self) -> Result<&[u8], CapabilityUnsupported> {
        Err(CapabilityUnsupported)
    }

    /// Restores state produced by [`Self::snapshot_state`]; the runtime re-renders afterwards.
    fn restore_state(&mut self, _state: &[u8]) -> Result<(), CapabilityUnsupported> {
        Err(CapabilityUnsupported)
    }
}

/// Generic per-frame mutation hook run inside `nve_runtime::SurfaceRuntime::tick`.
///
/// Optional subsystems (such as `packages/modules/motion` or physics/scroll springs) plug into
/// `nve-runtime` via this trait so core stays strictly decoupled from every module. All mutations
/// go through the runtime-owned `Transaction` so every table/layout/visual change is committed
/// into a reusable `ChangeSet`, fed into `LaidOutScene::commit`, and published with accurate
/// `dirty_indices`, `NveTableView::dirty_rows`, and `frame_id`.
pub trait FrameTickHook: Send {
    /// Advances time by `dt_s` seconds, reading `tx.store()` and writing any mutations into `tx`.
    /// Returns `true` if a scene refresh is requested even when no store words changed.
    fn tick(&mut self, tx: &mut Transaction<'_>, dt_s: f32) -> bool;
}
