//! Word-table `NodeStore` backend trait (`StoreBackend`) and cross-boundary `NVE2` replica
//! adapter (`StoreWireReplica`).
//!
//! In-process backends consume `(&NodeStore, &ChangeSet)` directly with zero encoding, zero
//! copying, zero `ComponentKind`, and zero `set_raw_prop`. Cross-boundary callers (`dlopen`,
//! `nve-wire`, remote hosts) decode `NVE2` frames into a replica [`NodeStore`] via
//! [`StoreWireReplica`] and forward `(&store, &changes)` into the exact same [`StoreBackend`].

use nve_abi::{EventBatchBuf, NodeId, SurfaceId};
use nve_store::wire::{WireDecoder, WireError};
use nve_store::{ChangeSet, NodeStore, RealizationOrder};

use crate::{BackendApplyStats, BackendError, LayoutRect, Viewport};

/// Structured error returned by [`StoreWireReplica::submit_frame0_nve2`] so `Frame0File`
/// distinguishes corrupt wire/footer payloads from transient allocation or backend commit errors.
pub(crate) enum Frame0SubmitError {
    Wire,
    Alloc,
    NodeCountMismatch,
    Backend(BackendError),
}

/// Universal backend trait implemented by every NVE renderer:
/// - `nve-backend-headless` (pure-Rust `TaffyStoreLayout` + `NveSceneView` publisher)
/// - `nve-backend-skia` (macOS, Windows, Linux, iOS, Android GPU Skia + `SkiaTableRegistry`)
/// - `nve-backend-web` (WASM DOM + SSR + hydration + `WebTableRegistry`)
pub trait StoreBackend {
    /// Human-readable backend identifier (e.g. `"headless"`, `"skia"`, `"web"`).
    fn name(&self) -> &'static str;

    /// Updates the active viewport for `surface_id`.
    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport);

    /// Applies one committed `(&NodeStore, &ChangeSet)` frame in place with zero copies.
    fn commit(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError>;

    /// Drains any pending native input/layout events since the last frame into `out`.
    fn drain_events(&mut self, _out: &mut EventBatchBuf) {}

    /// Returns computed layout rectangle for a node if available.
    fn layout_of(&self, _node_id: NodeId) -> Option<LayoutRect> {
        None
    }

    /// Resets backend-retained state (e.g. surface teardown or full keyframe resync).
    fn reset(&mut self);
}

/// Cross-boundary `NVE2` wire replica pairing a [`NodeStore::replica`], [`WireDecoder`], and
/// reusable [`ChangeSet`] with any [`StoreBackend`].
pub struct StoreWireReplica<B: StoreBackend> {
    store: NodeStore,
    realization_order: RealizationOrder,
    decoder: WireDecoder,
    changes: ChangeSet,
    needs_keyframe: bool,
    backend: B,
}

impl<B: StoreBackend> StoreWireReplica<B> {
    /// Creates a new replica wrapping `backend` with the given `realization_order`.
    pub fn new(backend: B, realization_order: RealizationOrder) -> Self {
        let mut store = NodeStore::replica();
        store.set_realization_order(realization_order.clone());
        Self {
            store,
            realization_order,
            decoder: WireDecoder::default(),
            changes: ChangeSet::default(),
            needs_keyframe: false,
            backend,
        }
    }

    /// Returns `true` after a mid-frame decode failure reset the replica; subsequent deltas are
    /// rejected with [`BackendError::NeedsKeyframe`] until a `frame == 1` keyframe succeeds.
    #[inline]
    pub fn needs_keyframe(&self) -> bool {
        self.needs_keyframe
    }

    #[inline]
    pub fn store(&self) -> &NodeStore {
        &self.store
    }

    #[inline]
    pub fn last_changes(&self) -> &ChangeSet {
        &self.changes
    }

    #[inline]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    #[inline]
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    #[inline]
    pub fn store_and_backend_mut(&mut self) -> (&NodeStore, &mut B) {
        (&self.store, &mut self.backend)
    }

    /// Consumes the replica, returning its decoded `NodeStore` and committed backend.
    #[inline]
    pub fn into_parts(self) -> (NodeStore, B) {
        (self.store, self.backend)
    }

    /// Decodes one `NVE2` wire frame into the replica `NodeStore` and commits it to `backend`.
    ///
    /// Validates the `NVE2` header before resetting on `frame == 1` (preventing corrupt packets
    /// from wiping live state), and resets the replica cleanly if mid-frame decoding fails so
    /// partial uncommitted mutations can never leak into live state.
    pub fn submit_nve2_frame(
        &mut self,
        wire_bytes: &[u8],
    ) -> Result<BackendApplyStats, BackendError> {
        self.submit_nve2_frame_with_hint(wire_bytes, 0)
    }

    /// Decodes a full `SceneKeyframe` `NVE2` payload, unconditionally resetting prior replica state
    /// (even when the `NVE2` header carries `frame > 1`) once the header validates.
    pub fn submit_nve2_keyframe(
        &mut self,
        wire_bytes: &[u8],
    ) -> Result<BackendApplyStats, BackendError> {
        self.decode_nve2_into_replica(wire_bytes, 0, true)?;
        self.commit_decoded_or_reset()
    }

    /// Like [`Self::submit_nve2_frame`], additionally pre-sizing `NodeStore` and `ChangeSet` for
    /// `node_count_hint` nodes before decoding a Frame-0 keyframe (`frame == 1`).
    pub fn submit_nve2_frame_with_hint(
        &mut self,
        wire_bytes: &[u8],
        node_count_hint: usize,
    ) -> Result<BackendApplyStats, BackendError> {
        self.decode_nve2_into_replica(wire_bytes, node_count_hint, false)?;
        self.commit_decoded_or_reset()
    }

    /// Frame-0 boot path: validates `live_nodes == expected_live_nodes` **before** invoking
    /// `backend.commit`, and resets the replica cleanly on any decode, mismatch, allocation, or
    /// backend commit failure so the store and backend never diverge.
    pub(crate) fn submit_frame0_nve2(
        &mut self,
        wire_bytes: &[u8],
        expected_live_nodes: usize,
    ) -> Result<BackendApplyStats, Frame0SubmitError> {
        if let Err(err) = self.decode_nve2_into_replica(wire_bytes, expected_live_nodes, false) {
            return Err(match err {
                BackendError::Wire(WireError::Alloc) => Frame0SubmitError::Alloc,
                BackendError::Wire(_) | BackendError::NeedsKeyframe(Some(_)) => {
                    Frame0SubmitError::Wire
                }
                other => Frame0SubmitError::Backend(other),
            });
        }
        if self.store.stats().live_nodes != expected_live_nodes {
            self.reset();
            self.needs_keyframe = true;
            return Err(Frame0SubmitError::NodeCountMismatch);
        }
        self.commit_decoded_or_reset()
            .map_err(Frame0SubmitError::Backend)
    }

    /// `is_keyframe` resets prior state whatever the header's `frame`; otherwise only `frame == 1`
    /// does, and deltas are refused while [`Self::needs_keyframe`].
    fn decode_nve2_into_replica(
        &mut self,
        wire_bytes: &[u8],
        node_count_hint: usize,
        is_keyframe: bool,
    ) -> Result<(), BackendError> {
        let (frame, _ops) = WireDecoder::validate_header(wire_bytes)?;
        if !is_keyframe && self.needs_keyframe && frame != 1 {
            return Err(BackendError::NeedsKeyframe(None));
        }
        if (is_keyframe || frame == 1)
            && (self.needs_keyframe || self.store.frame() > 0 || self.store.capacity() > 1)
        {
            self.reset();
        }
        if frame == 1 && node_count_hint > 0 {
            let bounded_hint =
                node_count_hint.min(WireDecoder::max_frame0_node_hint(wire_bytes.len()));
            // Pre-size only `ChangeSet` here; `WireDecoder::apply` sizes `NodeStore` inside
            // `Op::CreateNodes` under `SessionBudget` so hostile hints can never bypass budgets.
            if self
                .changes
                .reserve_for_frame0(bounded_hint, wire_bytes.len())
                .is_err()
            {
                self.reset();
                self.needs_keyframe = true;
                return Err(BackendError::Wire(WireError::Alloc));
            }
        }
        let apply_res = {
            let mut tx = self.store.tx();
            let res = self.decoder.apply(&mut tx, wire_bytes);
            if res.is_ok() {
                tx.commit_into(&mut self.changes);
            }
            res
        };
        if let Err(err) = apply_res {
            self.reset();
            self.needs_keyframe = true;
            return Err(match err {
                WireError::Alloc => BackendError::Wire(WireError::Alloc),
                other => BackendError::NeedsKeyframe(Some(other)),
            });
        }
        Ok(())
    }

    fn commit_decoded_or_reset(&mut self) -> Result<BackendApplyStats, BackendError> {
        match self.backend.commit(&self.store, &self.changes) {
            Ok(stats) => {
                self.needs_keyframe = false;
                Ok(stats)
            }
            Err(err) => {
                self.reset();
                self.needs_keyframe = true;
                Err(err)
            }
        }
    }

    /// Commits an explicit [`ChangeSet`] against the replica `NodeStore` into `backend`.
    pub fn commit_changes(
        &mut self,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError> {
        self.backend.commit(&self.store, changes)
    }

    /// Transforms the wrapped backend in place while preserving the decoded replica `NodeStore`.
    pub fn map_backend<B2: StoreBackend>(self, f: impl FnOnce(B) -> B2) -> StoreWireReplica<B2> {
        StoreWireReplica {
            store: self.store,
            realization_order: self.realization_order,
            decoder: self.decoder,
            changes: self.changes,
            needs_keyframe: self.needs_keyframe,
            backend: f(self.backend),
        }
    }

    /// Resets both the replica `NodeStore` and underlying backend.
    pub fn reset(&mut self) {
        self.store = NodeStore::replica();
        self.store
            .set_realization_order(self.realization_order.clone());
        self.decoder = WireDecoder::default();
        self.changes = ChangeSet::default();
        self.needs_keyframe = false;
        self.backend.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::wire::{WireEncoder, MAGIC};
    use nve_store::{NveTable};

    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "engine.test.tag", storage = "sparse")]
    struct TestTag {
        #[effect(paint)]
        val: u32,
    }

    struct CountingBackend {
        commits: usize,
        resets: usize,
    }

    impl StoreBackend for CountingBackend {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn set_viewport(&mut self, _: SurfaceId, _: Viewport) {}
        fn commit(
            &mut self,
            _: &NodeStore,
            _: &ChangeSet,
        ) -> Result<BackendApplyStats, BackendError> {
            self.commits += 1;
            Ok(BackendApplyStats::default())
        }
        fn reset(&mut self) {
            self.resets += 1;
        }
    }

    #[test]
    fn corrupt_header_does_not_wipe_live_replica_and_mid_frame_error_resets_cleanly() {
        let mut src = NodeStore::new();
        let mut changes = ChangeSet::default();
        let mut enc = WireEncoder::default();
        let mut frame1 = Vec::new();
        let child = {
            let mut tx = src.tx();
            let c = tx.create();
            tx.set_children(NodeId::ROOT, &[c]);
            tx.edit::<TestTag>(c).val(42);
            tx.commit_into(&mut changes);
            c
        };
        enc.encode(&src, &changes, &mut frame1);

        let mut replica = StoreWireReplica::new(
            CountingBackend {
                commits: 0,
                resets: 0,
            },
            RealizationOrder::default(),
        );
        replica.submit_nve2_frame(&frame1).expect("frame 1");
        assert!(replica.store().is_live(child));
        assert_eq!(replica.backend().commits, 1);

        // Corrupt magic claiming frame 1 must fail header validation BEFORE resetting live replica.
        let mut bad_hdr = frame1.clone();
        bad_hdr[0] ^= 0xFF;
        assert!(replica.submit_nve2_frame(&bad_hdr).is_err());
        assert!(
            replica.store().is_live(child),
            "corrupt header must not wipe live replica"
        );
        assert_eq!(replica.backend().resets, 0);

        // Valid header (`frame == 2`, total_words == 7) with truncated op body fails mid-apply
        // and resets cleanly so partial uncommitted state never leaks.
        let mut truncated_f2 = Vec::new();
        truncated_f2.extend_from_slice(&MAGIC.to_le_bytes());
        truncated_f2.extend_from_slice(&nve_store::wire::VERSION.to_le_bytes());
        truncated_f2.extend_from_slice(&0u16.to_le_bytes());
        truncated_f2.extend_from_slice(&2u64.to_le_bytes());
        truncated_f2.extend_from_slice(&1u32.to_le_bytes());
        truncated_f2.extend_from_slice(&7u32.to_le_bytes());
        // Op header claiming 16 bytes, but 0 bytes follow.
        truncated_f2.extend_from_slice(&((1u32) | (16u32 << 16)).to_le_bytes());
        assert!(matches!(
            replica.submit_nve2_frame(&truncated_f2),
            Err(BackendError::NeedsKeyframe(Some(_)))
        ));
        assert!(replica.needs_keyframe());
        assert_eq!(replica.backend().resets, 1);
        assert!(!replica.store().is_live(child));

        // Subsequent delta frame (`frame == 3`) must be rejected until a fresh keyframe arrives.
        let mut delta_f3 = Vec::new();
        {
            let mut tx = src.tx();
            tx.edit::<TestTag>(child).val(99);
            tx.commit_into(&mut changes);
        }
        enc.encode(&src, &changes, &mut delta_f3);
        // Force header frame to 3 so we specifically test delta rejection after reset.
        delta_f3[8..16].copy_from_slice(&3u64.to_le_bytes());
        assert!(matches!(
            replica.submit_nve2_frame(&delta_f3),
            Err(BackendError::NeedsKeyframe(None))
        ));
        assert!(replica.needs_keyframe());
        assert_eq!(replica.backend().commits, 1);

        // Fresh keyframe (`frame == 1`) clears `needs_keyframe` and restores live state.
        replica.submit_nve2_frame(&frame1).expect("keyframe resync");
        assert!(!replica.needs_keyframe());
        assert!(replica.store().is_live(child));
        assert_eq!(replica.backend().commits, 2);
    }
}
