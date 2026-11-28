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

/// Lazy, RAII-isolated native `skia_safe::Canvas` surface passed to
/// [`SkiaTableSystem::paint_skia_canvas`].
///
/// Delays `canvas.save()` / `clip_rect` / `save_layer` / `translate` until a plugin commits to
/// painting via [`Self::begin`] (zero Skia FFI overhead when returning `false`), isolates the
/// plugin surface so `BlendMode::Clear` / `BlendMode::Src` inside `clear()` replace only the
/// plugin's own pixels while compositing cleanly over parent backdrops and node backgrounds, and
/// guarantees `restore_to_count` on [`Drop`] even if a direct paint closure panics.
#[cfg(feature = "skia")]
pub struct NativeSkiaSurface<'a> {
    canvas: &'a skia_safe::Canvas,
    rect: LayoutRect,
    scale: f32,
    saved: Option<usize>,
}

#[cfg(feature = "skia")]
impl<'a> NativeSkiaSurface<'a> {
    #[inline]
    pub(crate) fn new(canvas: &'a skia_safe::Canvas, rect: LayoutRect, scale: f32) -> Self {
        Self {
            canvas,
            rect,
            scale,
            saved: None,
        }
    }

    #[inline]
    pub fn width(&self) -> f32 {
        self.rect.width
    }

    #[inline]
    pub fn height(&self) -> f32 {
        self.rect.height
    }

    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Lazily establishes the clipped, translated, isolated offscreen layer on `canvas` and
    /// returns it ready for logical-coordinate `(0..width, 0..height)` drawing.
    #[inline]
    pub fn begin(&mut self) -> &'a skia_safe::Canvas {
        if self.saved.is_none() {
            let saved = self.canvas.save();
            self.saved = Some(saved);
            let dst = skia_safe::Rect::from_xywh(
                self.rect.x,
                self.rect.y,
                self.rect.width,
                self.rect.height,
            );
            self.canvas
                .clip_rect(dst, Some(skia_safe::ClipOp::Intersect), Some(true));
            self.canvas
                .save_layer(&skia_safe::canvas::SaveLayerRec::default().bounds(&dst));
            self.canvas.translate((self.rect.x, self.rect.y));
        }
        self.canvas
    }
}

#[cfg(feature = "skia")]
impl Drop for NativeSkiaSurface<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(saved) = self.saved {
            self.canvas.restore_to_count(saved);
        }
    }
}

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

    /// Cleans up per-node plugin state when `node` detaches this table or is removed.
    fn remove_node(&self, _node: NodeId) {}

    /// Clears all per-node plugin state when the owning backend is reset.
    fn reset(&self) {}

    /// Returns `true` when this plugin prefers painting `node` directly onto a native
    /// `skia_safe::Canvas` (bypassing CPU `PixelBuffer` rasterization and GPU texture upload).
    #[cfg(feature = "skia")]
    fn prefers_skia_canvas(&self, _store: &NodeStore, _node: NodeId) -> bool {
        false
    }

    /// Optional native `skia_safe::Canvas` paint hook (`--features skia`).
    ///
    /// Implementations should confirm they will paint *before* calling `surface.begin()`, keeping
    /// non-native nodes at zero Skia FFI overhead.
    #[cfg(feature = "skia")]
    fn paint_skia_canvas(
        &self,
        _store: &NodeStore,
        _node: NodeId,
        _surface: &mut NativeSkiaSurface<'_>,
    ) -> bool {
        false
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

    /// Notifies every registered plugin that `node` has been removed from the store.
    pub fn remove_node(&self, node: NodeId) {
        for sys in &self.systems {
            sys.remove_node(node);
        }
    }

    /// Notifies every registered plugin that the owning backend has been reset.
    pub fn reset(&self) {
        for sys in &self.systems {
            sys.reset();
        }
    }

    /// Single-pass native `skia_safe::Canvas` dispatch with lazy RAII layer isolation.
    ///
    /// - Common single-plugin nodes scan `self.systems` once and delegate directly to
    ///   [`SkiaTableSystem::paint_skia_canvas`] (at most one plugin lock; zero Skia FFI calls if
    ///   the plugin declines).
    /// - Multi-plugin nodes verify [`SkiaTableSystem::prefers_skia_canvas`] across all attached
    ///   plugins before beginning the layer, preventing mixed-plugin double-painting.
    #[cfg(feature = "skia")]
    pub fn try_paint_skia_canvas(
        &self,
        store: &NodeStore,
        node: NodeId,
        canvas: &skia_safe::Canvas,
        rect: LayoutRect,
        scale: f32,
    ) -> bool {
        let mut attached = [0usize; 8];
        let mut count = 0usize;
        for (i, sys) in self.systems.iter().enumerate() {
            if store.has_slot(node, sys.slot()) {
                if count < attached.len() {
                    attached[count] = i;
                }
                count += 1;
            }
        }
        if count == 0 {
            return false;
        }
        if count == 1 {
            let mut surface = NativeSkiaSurface::new(canvas, rect, scale);
            return self.systems[attached[0]].paint_skia_canvas(store, node, &mut surface);
        }
        if count <= attached.len() {
            for &idx in &attached[..count] {
                if !self.systems[idx].prefers_skia_canvas(store, node) {
                    return false;
                }
            }
            let mut surface = NativeSkiaSurface::new(canvas, rect, scale);
            let mut painted = false;
            for &idx in &attached[..count] {
                if self.systems[idx].paint_skia_canvas(store, node, &mut surface) {
                    painted = true;
                }
            }
            return painted;
        }
        for sys in &self.systems {
            if store.has_slot(node, sys.slot()) && !sys.prefers_skia_canvas(store, node) {
                return false;
            }
        }
        let mut surface = NativeSkiaSurface::new(canvas, rect, scale);
        let mut painted = false;
        for sys in &self.systems {
            if store.has_slot(node, sys.slot())
                && sys.paint_skia_canvas(store, node, &mut surface)
            {
                painted = true;
            }
        }
        painted
    }
}
