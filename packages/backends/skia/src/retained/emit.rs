//! Node-local paint-chunk emission and property-node population for [`super::CompositorScene`].

use nve_abi::{xxh64, ColorRgba, NodeId};
use nve_backend_headless::HeadlessBackend;
use nve_engine::LayoutRect;
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeStore, Rgba};
use nve_tables::snapshots::VisualSnapshot;
use nve_tables::tables::{
    BackgroundFillRes, BoxShadows, Image, ImageSrc, ImageSrcView, Layout, Position, Scroll,
    TextContent, TextInput, Visual,
};
use nve_tables::transform::Transform2D;

use super::chunks::{ChunkArenas, LocalPaintOp, NodeChunkMeta, ResolvedChunkText};
use super::property_tree::{
    ClipNode, EffectNode, ScrollPropIdx, ScrollPropertyNode, TransformNode, TF_HAS_TRANSFORM,
    TF_IS_STICKY,
};
use crate::display_list::{effective_background_fill_res, should_clip_children};
use crate::frame_hook::SkiaFrameHooks;
use crate::image_cache::ImageCache;
use crate::scroll::compute_scroll_content_size;
use crate::table_system::SkiaTableRegistry;
use crate::text::CosmicTextEngine;
use crate::text_cache::{canonical_f32, normalize_text_align};
use crate::TextFocusState;

/// Deterministic `xxh64` digest of inherited text style + visibility propagated to children,
/// used to stop-prune subtree repaints whenever a descendant overrides all changed fields.
#[inline]
pub(crate) fn compute_inherit_hash(text: ResolvedChunkText, effective_hidden: bool) -> u64 {
    let mut bytes = [0u8; 16];
    bytes[0..2].copy_from_slice(&text.font_weight.to_le_bytes());
    bytes[2] = normalize_text_align(text.text_align) as u8;
    bytes[3] = u8::from(effective_hidden);
    bytes[4..8].copy_from_slice(&text.text_color.to_u32().to_le_bytes());
    let font_size_bits = canonical_f32(text.font_size).to_bits();
    bytes[8..12].copy_from_slice(&font_size_bits.to_le_bytes());
    xxh64(&bytes, 0)
}

/// Shared read-only inputs threaded into chunk emission and sync passes.
pub(crate) struct SceneSyncEnv<'a, L: StoreLayoutEngine> {
    pub store: &'a NodeStore,
    pub layout_tree: &'a HeadlessBackend<L>,
    pub focus: &'a TextFocusState,
    pub frame_hooks: &'a SkiaFrameHooks,
    pub table_registry: &'a SkiaTableRegistry,
    pub plugin_epochs: &'a [u64],
    pub base_plugin_epoch: u64,
    pub text_engine: &'a CosmicTextEngine,
}

impl<L: StoreLayoutEngine> SceneSyncEnv<'_, L> {
    #[inline]
    pub(crate) fn plugin_epoch_of(&self, id: NodeId) -> u64 {
        self.plugin_epochs
            .get(id.index())
            .copied()
            .unwrap_or(0)
            .max(self.base_plugin_epoch)
    }

    /// Populates `TransformNode`, `ClipNode`, `EffectNode`, and (sparsely) `scrolls` for `id`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill_property_nodes(
        &self,
        id: NodeId,
        base_rect: LayoutRect,
        inherited_hidden: bool,
        style: &VisualSnapshot,
        tf_node: &mut TransformNode,
        clip_node: &mut ClipNode,
        effect_node: &mut EffectNode,
        scrolls: &mut Vec<ScrollPropertyNode>,
    ) {
        let (stx, sty) = self.layout_tree.compute_sticky_delta(self.store, id);
        let effective_tf = self
            .frame_hooks
            .resolve_transform(id, self.layout_tree.effective_transform(self.store, id));
        let is_sticky = self
            .store
            .try_row::<Layout>(id)
            .is_some_and(|l| l.position() == Position::Sticky);

        let scroll_idx = if self.store.has::<Scroll>(id) {
            let (scroll_shift_x, scroll_shift_y) =
                self.layout_tree.scene().visual_scroll_shift(self.store, id);
            let (scroll_raw_x, scroll_raw_y) = self.layout_tree.scene().scroll_offset(id);
            let (content_w, content_h) = compute_scroll_content_size(self.store, self.layout_tree, id)
                .unwrap_or((base_rect.width, base_rect.height));
            let idx = ScrollPropIdx(scrolls.len() as u32);
            scrolls.push(ScrollPropertyNode {
                scroll_shift_x,
                scroll_shift_y,
                scroll_raw_x,
                scroll_raw_y,
                content_w,
                content_h,
            });
            idx
        } else {
            ScrollPropIdx::NONE
        };

        tf_node.base_x = base_rect.x;
        tf_node.base_y = base_rect.y;
        tf_node.width = base_rect.width;
        tf_node.height = base_rect.height;
        tf_node.sticky_x = stx;
        tf_node.sticky_y = sty;
        tf_node.scroll_idx = scroll_idx;
        tf_node.flags = 0;
        if let Some(t) = effective_tf
            && !t.is_identity()
        {
            tf_node.local_transform = t;
            tf_node.flags |= TF_HAS_TRANSFORM;
        } else {
            tf_node.local_transform = Transform2D::IDENTITY;
        }
        if is_sticky {
            tf_node.flags |= TF_IS_STICKY;
        }

        clip_node.clips_children = should_clip_children(self.store, id);
        clip_node.radius = style.border_radius.max(0.0);

        let effective_hidden = self
            .store
            .try_row::<Visual>(id)
            .map_or(inherited_hidden, |v| v.visibility().resolve(inherited_hidden));
        effect_node.opacity = style.opacity;
        effect_node.effective_hidden = effective_hidden;
    }

    /// Re-emits all node-local shadow and body paint ops for `meta` into `arenas`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_chunk_local_ops(
        &self,
        id: NodeId,
        width: f32,
        height: f32,
        style: &VisualSnapshot,
        resolved_text: ResolvedChunkText,
        effective_hidden: bool,
        arenas: &mut ChunkArenas,
        meta: &mut NodeChunkMeta,
    ) {
        arenas.retire_chunk_ops(meta);

        let local_rect = LayoutRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        let radius = style.border_radius.max(0.0);
        let plugin_epoch = self.plugin_epoch_of(id);
        meta.plugin_epoch = plugin_epoch;

        if effective_hidden {
            return;
        }

        let shadow_view = self
            .store
            .try_row::<Visual>(id)
            .map(|v| v.box_shadows())
            .filter(|rid| !rid.is_none())
            .and_then(|rid| self.store.resource::<BoxShadows>(rid));

        // 1. Outer box shadows (separate span so replay culls shadows with blur/spread padding).
        let shadow_start = arenas.ops.len() as u32;
        if let Some(shadows) = shadow_view {
            for sh in shadows.iter().filter(|sh| !sh.inset) {
                arenas.ops.push(LocalPaintOp::DrawShadow {
                    radius,
                    offset_x: sh.offset_x,
                    offset_y: sh.offset_y,
                    blur_radius: sh.blur_radius,
                    spread_radius: sh.spread_radius,
                    color: ColorRgba::from_u32(sh.color.0),
                    border_width: 0.0,
                    inset: false,
                });
            }
        }
        let shadow_len = (arenas.ops.len() as u32) - shadow_start;

        // 2. Body ops in CSS paint order: background gradient/fill -> image -> inset shadows ->
        //    border -> text/text_input -> plugins.
        let op_start = arenas.ops.len() as u32;

        let mut painted_gradient = false;
        if let Some(fill_res) = effective_background_fill_res(self.store, self.layout_tree, id)
            && let Some(fill) = self.store.resource::<BackgroundFillRes>(fill_res)
        {
            let gradient_idx = arenas.push_gradient(fill.to_owned());
            arenas.ops.push(LocalPaintOp::FillRoundedRectGradient {
                local_rect,
                radius,
                gradient_idx,
            });
            painted_gradient = true;
        }
        if !painted_gradient && style.background.a > 0 {
            arenas.ops.push(LocalPaintOp::FillRoundedRect {
                local_rect,
                radius,
                color: style.background,
            });
        }

        if let Some(img) = self.store.try_row::<Image>(id) {
            let src_res = img.src();
            if !src_res.is_none()
                && let Some(ImageSrcView::Url(url)) = self.store.resource::<ImageSrc>(src_res)
                && !url.is_empty()
            {
                let resolved = ImageCache::shared().resolve_cached_handle(url);
                let handle_idx = arenas.push_image_handle(resolved);
                let (str_off, str_len) = arenas.push_str(url);
                arenas.ops.push(LocalPaintOp::DrawImage {
                    local_rect,
                    radius,
                    str_off,
                    str_len,
                    fit: img.fit(),
                    handle_idx,
                });
            }
        }

        if let Some(shadows) = shadow_view {
            for sh in shadows.iter().filter(|sh| sh.inset) {
                arenas.ops.push(LocalPaintOp::DrawShadow {
                    radius,
                    offset_x: sh.offset_x,
                    offset_y: sh.offset_y,
                    blur_radius: sh.blur_radius,
                    spread_radius: sh.spread_radius,
                    color: ColorRgba::from_u32(sh.color.0),
                    border_width: style.border_width.max(0.0),
                    inset: true,
                });
            }
        }

        if style.border_width > 0.0 && style.border_color.a > 0 {
            arenas.ops.push(LocalPaintOp::StrokeRoundedRect {
                local_rect,
                radius,
                width: style.border_width,
                color: style.border_color,
            });
        }

        if self.store.has::<TextInput>(id) {
            self.emit_text_input_local_ops(
                id,
                local_rect,
                radius,
                resolved_text.text_color,
                resolved_text.font_size,
                resolved_text.font_weight,
                arenas,
            );
        } else if let Some(tc) = self.store.try_row::<TextContent>(id) {
            let text = tc.text();
            if !text.is_empty() && resolved_text.font_size > 0.0 && resolved_text.text_color.a > 0 {
                let (str_off, str_len) = arenas.push_str(text);
                arenas.ops.push(LocalPaintOp::DrawText {
                    local_rect,
                    str_off,
                    str_len,
                    font_size: resolved_text.font_size,
                    font_weight: resolved_text.font_weight,
                    text_align: resolved_text.text_align,
                    color: resolved_text.text_color,
                });
            }
        }

        if self
            .table_registry
            .iter()
            .iter()
            .any(|sys| self.store.has_slot(id, sys.slot()))
        {
            arenas.ops.push(LocalPaintOp::Plugin {
                node_id: id,
                local_rect,
                inherited_text_color: Rgba(resolved_text.text_color.to_u32()),
                plugin_epoch,
            });
        }

        let op_len = (arenas.ops.len() as u32) - op_start;
        meta.shadow_start = shadow_start;
        meta.shadow_len = shadow_len;
        meta.op_start = op_start;
        meta.op_len = op_len;
        arenas.live_op_count += (shadow_len as usize) + (op_len as usize);
    }
}
