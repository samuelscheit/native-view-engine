//! Optional in-process [`DirectCanvasRegistry`] and unified [`write_canvas`] helper.
//!
//! Keeps [`crate::Canvas`] (`nve.canvas`) strictly 2 words (`draw_list: Res<DrawList>`,
//! `generation: u32`) with zero cross-language ABI drift while letting in-process Skia and Web
//! hosts execute direct [`CanvasPainter`] callbacks with zero `DrawList` allocations/encode/decode.

use core::fmt;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use nve_store::{NodeId, TxCore};

use crate::{Canvas, CanvasPaintSize, CanvasPainter, DrawList};

/// Type-erased in-process direct canvas paint closure invoked by backends during painting.
pub type DirectPaintFn = dyn Fn(&mut dyn CanvasPainter, CanvasPaintSize) + Send + Sync + 'static;

struct DirectSlot {
    node: NodeId,
    painter: Arc<DirectPaintFn>,
}

// `Arc` NonNull niche keeps `Option<DirectSlot>` compact (`24` bytes on 64-bit, `12` on wasm32).
const _: () = assert!(core::mem::size_of::<Option<DirectSlot>>() <= 24);

struct DirectRegistryInner {
    slots: RwLock<Vec<Option<DirectSlot>>>,
    live_count: AtomicUsize,
    next_gen: AtomicU32,
}

impl Default for DirectRegistryInner {
    fn default() -> Self {
        Self {
            slots: RwLock::new(Vec::new()),
            live_count: AtomicUsize::new(0),
            next_gen: AtomicU32::new(1),
        }
    }
}

static RECORDED_GEN: AtomicU32 = AtomicU32::new(1);

#[inline]
fn next_nonzero_u32(counter: &AtomicU32) -> u32 {
    let mut cur = counter.load(Ordering::Relaxed);
    loop {
        let val = if cur == 0 { 1 } else { cur };
        let next = val.wrapping_add(1).max(1);
        match counter.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return val,
            Err(actual) => cur = actual,
        }
    }
}

/// `NodeId`-indexed generational registry mapping canvas nodes to direct in-process paint closures.
///
/// - O(1) dense array lookup by `node.index()` verified against full generational `NodeId`
///   equality (immune to recycled `NodeId` slots).
/// - Zero-lock fast skip when empty (`live_count == 0`) so recorded-only hosts pay 0 lock cost.
/// - Drops read/write locks before invoking or dropping user closures (zero deadlock risk).
#[derive(Clone, Default)]
pub struct DirectCanvasRegistry {
    inner: Arc<DirectRegistryInner>,
}

impl fmt::Debug for DirectCanvasRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectCanvasRegistry")
            .field("live_count", &self.len())
            .finish()
    }
}

impl DirectCanvasRegistry {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds a direct paint closure to `node`, clearing any stale recorded `DrawList` and
    /// advancing `Canvas::generation` atomically so backends mark the node dirty.
    pub fn bind<F>(&self, tx: &mut TxCore<'_>, node: NodeId, painter: F) -> u32
    where
        F: Fn(&mut dyn CanvasPainter, CanvasPaintSize) + Send + Sync + 'static,
    {
        self.bind_arc(tx, node, Arc::new(painter))
    }

    /// Binds a pre-shared `Arc<DirectPaintFn>` to `node` without re-boxing the closure.
    pub fn bind_arc(&self, tx: &mut TxCore<'_>, node: NodeId, painter: Arc<DirectPaintFn>) -> u32 {
        if node.is_null() {
            return 0;
        }
        let idx = node.index();
        let next_gen = next_nonzero_u32(&self.inner.next_gen);
        let replaced = {
            let mut slots = self
                .inner
                .slots
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if idx >= slots.len() {
                slots.resize_with(idx + 1, || None);
            }
            let old = slots[idx].replace(DirectSlot { node, painter });
            if old.is_none() {
                self.inner.live_count.fetch_add(1, Ordering::Release);
            }
            old
        };
        // Drop replaced closure outside the write lock to avoid re-entrant deadlocks.
        drop(replaced);

        tx.edit::<Canvas>(node)
            .draw_list(None)
            .generation(next_gen);
        next_gen
    }

    /// Bumps `Canvas::generation` for an already-bound direct canvas without reallocating its
    /// closure (0 heap allocations for 120 Hz animating canvases capturing shared state).
    ///
    /// Returns `Some(next_gen)` if `node` is currently bound with matching generation, or `None`
    /// without mutating `tx` otherwise.
    pub fn invalidate(&self, tx: &mut TxCore<'_>, node: NodeId) -> Option<u32> {
        if node.is_null() || self.inner.live_count.load(Ordering::Acquire) == 0 {
            return None;
        }
        let bound = {
            let slots = self.inner.slots.read().unwrap_or_else(|e| e.into_inner());
            slots
                .get(node.index())
                .and_then(Option::as_ref)
                .is_some_and(|slot| slot.node == node)
        };
        if !bound {
            return None;
        }
        let next_gen = next_nonzero_u32(&self.inner.next_gen);
        tx.edit::<Canvas>(node).generation(next_gen);
        Some(next_gen)
    }

    /// Invokes the bound direct painter for `node` into `painter` with `size`.
    ///
    /// Returns `true` if a matching live direct painter was invoked, or `false` if the caller
    /// should fall back to replaying `Canvas::draw_list`.
    #[inline]
    pub fn paint(
        &self,
        node: NodeId,
        painter: &mut dyn CanvasPainter,
        size: CanvasPaintSize,
    ) -> bool {
        if node.is_null() || self.inner.live_count.load(Ordering::Acquire) == 0 {
            return false;
        }
        let cb = {
            let slots = self.inner.slots.read().unwrap_or_else(|e| e.into_inner());
            slots
                .get(node.index())
                .and_then(Option::as_ref)
                .filter(|slot| slot.node == node)
                .map(|slot| Arc::clone(&slot.painter))
        };
        if let Some(cb) = cb {
            cb(painter, size);
            true
        } else {
            false
        }
    }

    /// Removes the direct painter for `node` (no-op if unbound or generation differs).
    /// Drops any removed closure outside the write lock.
    pub fn remove(&self, node: NodeId) -> bool {
        if node.is_null() || self.inner.live_count.load(Ordering::Acquire) == 0 {
            return false;
        }
        let removed = {
            let mut slots = self
                .inner
                .slots
                .write()
                .unwrap_or_else(|e| e.into_inner());
            match slots.get_mut(node.index()) {
                Some(slot) if slot.as_ref().is_some_and(|s| s.node == node) => {
                    self.inner.live_count.fetch_sub(1, Ordering::Release);
                    slot.take()
                }
                _ => None,
            }
        };
        let did_remove = removed.is_some();
        drop(removed);
        did_remove
    }

    /// Clears all bound direct painters, dropping captured closures outside the write lock.
    pub fn clear(&self) {
        if self.inner.live_count.load(Ordering::Acquire) == 0 {
            return;
        }
        let old = {
            let mut slots = self
                .inner
                .slots
                .write()
                .unwrap_or_else(|e| e.into_inner());
            self.inner.live_count.store(0, Ordering::Release);
            std::mem::take(&mut *slots)
        };
        drop(old);
    }

    /// Number of currently bound live direct painters.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.live_count.load(Ordering::Acquire)
    }

    /// Returns `true` when zero direct painters are currently bound.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` when `node` (exact generational `NodeId`) has a bound direct painter.
    #[inline]
    pub fn contains(&self, node: NodeId) -> bool {
        if node.is_null() || self.inner.live_count.load(Ordering::Acquire) == 0 {
            return false;
        }
        let slots = self.inner.slots.read().unwrap_or_else(|e| e.into_inner());
        slots
            .get(node.index())
            .and_then(Option::as_ref)
            .is_some_and(|slot| slot.node == node)
    }
}

/// Unified consumer helper: binds `draw` directly into `direct` when running in-process, or
/// records a flat [`DrawList`] at `fallback_size` when `direct` is `None` (SSR, C ABI, `NVE2`,
/// `Frame0File`, or out-of-process hosts).
pub fn write_canvas<F>(
    tx: &mut TxCore<'_>,
    node: NodeId,
    direct: Option<&DirectCanvasRegistry>,
    fallback_size: CanvasPaintSize,
    draw: F,
) -> u32
where
    F: Fn(&mut dyn CanvasPainter, CanvasPaintSize) + Send + Sync + 'static,
{
    if let Some(reg) = direct {
        reg.bind(tx, node, draw)
    } else {
        let next_gen = next_nonzero_u32(&RECORDED_GEN);
        tx.edit::<Canvas>(node)
            .draw_list_ephemeral_with(|out| {
                DrawList::record_into_words(out, |rec| draw(rec, fallback_size));
                true
            })
            .generation(next_gen);
        next_gen
    }
}
