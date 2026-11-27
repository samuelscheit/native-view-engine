//! Zero-copy publication of live resources (`NveRuntimeSceneView::resources` /
//! `dirty_resources`).
//!
//! Every live resource is published by raw `ResourceId` as its kind and wire bytes. Replica
//! stores (all `NVE2` paths: C ABI apps, `submit_nve2`, pairing) keep those bytes, so views
//! borrow them straight from the store. Typed values without stored bytes (in-process
//! `direct_store` apps' ephemeral resources) are encoded once per (re)definition into a per-slot
//! scratch that only grows.

use nve_abi::{NveResourceView, NVE_RESOURCE_FLAG_EPHEMERAL};
use nve_store::{ChangeSet, NodeStore, ResourceId};

#[derive(Default)]
pub(crate) struct ResourcePublisher {
    views: Vec<NveResourceView>,
    dirty: Vec<u32>,
    /// Encoded bytes of typed values the store keeps no bytes for, by slot.
    scratch: Vec<Vec<u8>>,
}

impl ResourcePublisher {
    /// Drops every borrowed pointer (store reset, app swap, unwind).
    pub(crate) fn clear(&mut self) {
        self.views.clear();
        self.dirty.clear();
        for s in &mut self.scratch {
            s.clear();
        }
    }

    /// Rebuilds every view from `store` (O(resource slots), allocation-free once warm) and records
    /// the unique ids (re)defined in `changes`.
    pub(crate) fn publish(&mut self, store: &NodeStore, changes: &ChangeSet) {
        let res = store.resources();
        let slots = res.slots();
        if self.scratch.len() < slots {
            self.scratch.resize_with(slots, Vec::new);
        }
        self.dirty.clear();
        for &id in &changes.resources_defined {
            if (id.0 as usize) < slots && res.is_live(id) {
                self.dirty.push(id.0);
            }
        }
        self.dirty.sort_unstable();
        self.dirty.dedup();
        for &raw in &self.dirty {
            let id = ResourceId(raw);
            let scratch = &mut self.scratch[raw as usize];
            scratch.clear();
            if res.bytes(id).is_none() {
                let _ = res.encode(id, scratch);
            }
        }
        self.views.clear();
        self.views.resize(slots, NveResourceView::EMPTY);
        for (i, view) in self.views.iter_mut().enumerate().skip(1) {
            let id = ResourceId(i as u32);
            if !res.is_live(id) {
                self.scratch[i].clear();
                continue;
            }
            let kind = res.kind(id).unwrap_or_default();
            let bytes = res.bytes(id).unwrap_or(&self.scratch[i]);
            *view = NveResourceView {
                kind_lo: kind.lo(),
                kind_hi: kind.hi(),
                len: bytes.len() as u32,
                flags: if res.is_ephemeral(id) { NVE_RESOURCE_FLAG_EPHEMERAL } else { 0 },
                bytes: if bytes.is_empty() { core::ptr::null() } else { bytes.as_ptr() },
            };
        }
    }

    /// `(resources, resource_capacity, dirty_resources, dirty_resource_count)` for the scene view.
    pub(crate) fn view_parts(&self) -> (*const NveResourceView, u32, *const u32, u32) {
        (slab_ptr(&self.views), self.views.len() as u32, slab_ptr(&self.dirty), self.dirty.len() as u32)
    }

    /// Whether every published view still borrows exactly what `store` holds now.
    pub(crate) fn matches_store(&self, store: &NodeStore) -> bool {
        let res = store.resources();
        if self.views.is_empty() {
            return true;
        }
        if self.views.len() != res.slots() {
            return false;
        }
        self.views.iter().enumerate().skip(1).all(|(i, v)| {
            let id = ResourceId(i as u32);
            if v.len == 0 {
                return v.bytes.is_null();
            }
            res.is_live(id)
                && res
                    .bytes(id)
                    .map_or(v.bytes == self.scratch[i].as_ptr(), |b| v.bytes == b.as_ptr() && v.len as usize == b.len())
        })
    }
}

/// Slice pointer, or null for an empty slice (C hosts check `count == 0` / null pairs).
fn slab_ptr<T>(s: &[T]) -> *const T {
    if s.is_empty() { core::ptr::null() } else { s.as_ptr() }
}
