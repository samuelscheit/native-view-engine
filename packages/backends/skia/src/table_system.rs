//! Uniform plugin system (`SkiaTableSystem` + `SkiaTableRegistry`) for `nve-backend-skia`.
//!
//! `nve-backend-skia` has **zero dependencies** on any capability module (`svg`, `canvas`, or 3P
//! plugins). Thin leaf crates (`nve-svg-skia`, `nve-canvas-skia`, 3P crates) implement
//! [`SkiaTableSystem`] and register onto a [`SkiaTableRegistry`].

use std::sync::Arc;

use nve_abi::EventBatchBuf;
use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore, Rgba, TableSlot};

use crate::raster::PixelBuffer;

/// Paint context passed to [`SkiaTableSystem::paint_node`].
pub struct SkiaSystemPaintCtx<'a> {
    pub buffer: &'a mut PixelBuffer,
    pub rect: LayoutRect,
    /// Optional active clip rectangle in physical pixel coordinates matching `rect`.
    pub clip_rect: Option<LayoutRect>,
    /// Physical-to-logical backing scale factor (`>= 1.0`).
    pub scale_factor: f32,
    pub opacity: f32,
    pub inherited_text_color: Rgba,
    pub events: &'a mut EventBatchBuf,
}

/// Uniform capability plugin trait for Skia / software rasterization.
pub trait SkiaTableSystem: Send + Sync + 'static {
    /// Table slot handled by this plugin (e.g. `Svg::slot()`, `Canvas::slot()`).
    fn slot(&self) -> TableSlot;

    /// Paints `node` into `ctx.buffer` within `ctx.rect`.
    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId);

    /// Handles a typed imperative command addressed to this table on `node`.
    fn handle_command(
        &self,
        _store: &NodeStore,
        _node: NodeId,
        _op: u32,
        _payload: &[u8],
        _events: &mut EventBatchBuf,
    ) {
    }
}

/// Registry of [`SkiaTableSystem`] plugins installed on a Skia renderer.
pub struct SkiaTableRegistry {
    systems: Vec<Arc<dyn SkiaTableSystem>>,
    generation: u64,
}

impl Default for SkiaTableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaTableRegistry {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
            generation: crate::next_plugin_epoch(),
        }
    }

    pub fn register<S: SkiaTableSystem>(&mut self, system: S) -> &mut Self {
        let slot = system.slot();
        self.systems.retain(|s| s.slot() != slot);
        self.systems.push(Arc::new(system));
        self.generation = crate::next_plugin_epoch();
        self
    }

    /// Process-wide monotonic generation bumped on registry construction and every [`Self::register`]
    /// call so GPU/CPU plugin caches invalidate even when callers mutate the registry directly.
    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[inline]
    pub fn get(&self, slot: TableSlot) -> Option<&Arc<dyn SkiaTableSystem>> {
        self.systems.iter().find(|s| s.slot() == slot)
    }

    #[inline]
    pub fn iter(&self) -> &[Arc<dyn SkiaTableSystem>] {
        &self.systems
    }

    /// Invokes every registered plugin whose table is attached to `node`.
    /// Returns the number of plugins that painted.
    pub fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) -> usize {
        let mut painted = 0;
        for sys in &self.systems {
            if store.has_slot(node, sys.slot()) {
                sys.paint_node(ctx, store, node);
                painted += 1;
            }
        }
        painted
    }

    /// Dispatches a typed command to the registered plugin for `slot`.
    pub fn dispatch_command(
        &self,
        store: &NodeStore,
        node: NodeId,
        slot: TableSlot,
        op: u32,
        payload: &[u8],
        events: &mut EventBatchBuf,
    ) -> bool {
        if let Some(sys) = self.get(slot) {
            sys.handle_command(store, node, op, payload, events);
            true
        } else {
            false
        }
    }
}
