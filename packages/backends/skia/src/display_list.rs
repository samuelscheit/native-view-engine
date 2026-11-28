//! Shared geometry, clipping, scrollbar thumb, typography inheritance, and `display: none`
//! helpers for [`crate::CompositorScene`] (plus the `#[cfg(test)]` ground-truth [`emit_node_into`]
//! reference oracle).

use nve_abi::{ColorRgba, NodeId};
use nve_backend_headless::HeadlessBackend;
use nve_engine::LayoutRect;
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeStore, ResolvedRow, ResourceId, Rgba};
use nve_tables::snapshots::VisualSnapshot;
#[cfg(test)]
use nve_tables::tables::{
    BackgroundFillRes, BoxShadows, Image, ImageSrc, ImageSrcView, TextContent, TextInput,
};
use nve_tables::tables::{
    is_hidden_text_input, Display, Layout, Overflow, Scroll, TextAlign, Typography, Visual,
};

use crate::frame_hook::SkiaFrameHooks;
#[cfg(test)]
use crate::image_cache::ImageCache;
use crate::retained::ResolvedChunkText;
#[cfg(test)]
use crate::scroll::compute_scroll_content_size;
#[cfg(test)]
use crate::table_system::SkiaTableRegistry;
#[cfg(test)]
use crate::text::CosmicTextEngine;
use crate::text_cache::normalize_text_align;
#[cfg(test)]
use crate::text_input::emit_text_input_ops;
#[cfg(test)]
use crate::{PaintOp, TextFocusState};
use crate::{MAX_HIT_TEST_DEPTH, SCROLLBAR_THUMB_COLOR};

const SCROLLBAR_THICKNESS: f32 = 6.0;
const SCROLLBAR_MARGIN: f32 = 3.0;
const SCROLLBAR_MIN_THUMB: f32 = 20.0;

#[cfg(test)]
#[inline]
pub(crate) fn offset_rect(rect: LayoutRect, offset: (f32, f32)) -> LayoutRect {
    LayoutRect {
        x: rect.x + offset.0,
        y: rect.y + offset.1,
        width: rect.width,
        height: rect.height,
    }
}

#[inline]
pub(crate) fn intersect_rects(a: LayoutRect, b: LayoutRect) -> Option<LayoutRect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    if x1 > x0 && y1 > y0 {
        Some(LayoutRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    } else {
        None
    }
}

#[inline]
pub(crate) fn intersects_clip(clip: LayoutRect, r: LayoutRect, pad: f32) -> bool {
    let x0 = r.x - pad;
    let y0 = r.y - pad;
    let x1 = r.x + r.width + pad;
    let y1 = r.y + r.height + pad;
    x1 > clip.x && x0 < clip.x + clip.width && y1 > clip.y && y0 < clip.y + clip.height
}

#[inline]
pub(crate) fn should_clip_children(store: &NodeStore, id: NodeId) -> bool {
    if store.has::<Scroll>(id) {
        return true;
    }
    store.try_row::<Visual>(id).is_some_and(|v| {
        matches!(
            v.overflow_x(),
            Overflow::Hidden | Overflow::Scroll | Overflow::Auto
        ) || matches!(
            v.overflow_y(),
            Overflow::Hidden | Overflow::Scroll | Overflow::Auto
        )
    })
}

/// Computes overlay vertical & horizontal scrollbar thumb geometry for a `Scroll` container and
/// yields each visible `(thumb_rect, corner_radius, color)` to `emit`.
#[inline]
pub(crate) fn for_each_scrollbar_thumb(
    rect: LayoutRect,
    sx: f32,
    sy: f32,
    cw: f32,
    ch: f32,
    mut emit: impl FnMut(LayoutRect, f32, ColorRgba),
) {
    let has_v = ch > rect.height + 0.5;
    let has_h = cw > rect.width + 0.5;
    if !has_v && !has_h {
        return;
    }
    let corner_gap = if has_v && has_h {
        SCROLLBAR_THICKNESS + SCROLLBAR_MARGIN
    } else {
        0.0
    };
    if has_v
        && rect.height > SCROLLBAR_MARGIN * 2.0 + corner_gap + 4.0
        && rect.width > SCROLLBAR_THICKNESS + SCROLLBAR_MARGIN * 2.0
    {
        let track_h = (rect.height - SCROLLBAR_MARGIN * 2.0 - corner_gap).max(1.0);
        let thumb_h =
            ((rect.height / ch) * track_h).clamp(SCROLLBAR_MIN_THUMB.min(track_h), track_h);
        let max_sy = (ch - rect.height).max(1.0);
        let ratio_y = (sy / max_sy).clamp(0.0, 1.0);
        let thumb_y = rect.y + SCROLLBAR_MARGIN + ratio_y * (track_h - thumb_h);
        let thumb_x = rect.x + rect.width - SCROLLBAR_THICKNESS - SCROLLBAR_MARGIN;
        emit(
            LayoutRect {
                x: thumb_x,
                y: thumb_y,
                width: SCROLLBAR_THICKNESS,
                height: thumb_h,
            },
            SCROLLBAR_THICKNESS * 0.5,
            SCROLLBAR_THUMB_COLOR,
        );
    }
    if has_h
        && rect.width > SCROLLBAR_MARGIN * 2.0 + corner_gap + 4.0
        && rect.height > SCROLLBAR_THICKNESS + SCROLLBAR_MARGIN * 2.0
    {
        let track_w = (rect.width - SCROLLBAR_MARGIN * 2.0 - corner_gap).max(1.0);
        let thumb_w =
            ((rect.width / cw) * track_w).clamp(SCROLLBAR_MIN_THUMB.min(track_w), track_w);
        let max_sx = (cw - rect.width).max(1.0);
        let ratio_x = (sx / max_sx).clamp(0.0, 1.0);
        let thumb_x = rect.x + SCROLLBAR_MARGIN + ratio_x * (track_w - thumb_w);
        let thumb_y = rect.y + rect.height - SCROLLBAR_THICKNESS - SCROLLBAR_MARGIN;
        emit(
            LayoutRect {
                x: thumb_x,
                y: thumb_y,
                width: thumb_w,
                height: SCROLLBAR_THICKNESS,
            },
            SCROLLBAR_THICKNESS * 0.5,
            SCROLLBAR_THUMB_COLOR,
        );
    }
}

/// Returns `true` if `id` itself is hidden via dead handle, `InputKind::Hidden`, `StateRules`
/// `display: none`, or `Layout::display == Display::None`.
#[inline]
pub(crate) fn is_node_display_none_effective<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    id: NodeId,
) -> bool {
    if !store.is_live(id) || is_hidden_text_input(store, id) {
        return true;
    }
    let mut patched_display: Option<Display> = None;
    layout_tree.visit_effective_state_patches(store, id, |pv| {
        if let Some(d) = pv.get_prop(Layout::DISPLAY) {
            patched_display = Some(d);
        }
    });
    patched_display == Some(Display::None)
        || (patched_display.is_none()
            && store
                .try_row::<Layout>(id)
                .is_some_and(|l| l.display() == Display::None))
}

/// Returns `true` if `id` itself is effectively `display: contents` (including `StateRules`
/// overrides) and is neither root nor `InputKind::Hidden`.
#[inline]
pub(crate) fn is_node_display_contents_effective<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    id: NodeId,
) -> bool {
    if id == NodeId::ROOT || !store.is_live(id) || is_hidden_text_input(store, id) {
        return false;
    }
    let mut patched_display: Option<Display> = None;
    layout_tree.visit_effective_state_patches(store, id, |pv| {
        if let Some(d) = pv.get_prop(Layout::DISPLAY) {
            patched_display = Some(d);
        }
    });
    patched_display == Some(Display::Contents)
        || (patched_display.is_none()
            && store
                .try_row::<Layout>(id)
                .is_some_and(|l| l.display() == Display::Contents))
}

/// Returns `true` if `id` or any ancestor up to root is effectively `display: none` (including
/// `StateRules` pseudo-state overrides and `InputKind::Hidden`).
#[inline]
pub(crate) fn is_subtree_display_none_effective<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    id: NodeId,
) -> bool {
    let mut cur = Some(id);
    let mut depth = 0usize;
    while let Some(nid) = cur {
        if depth > MAX_HIT_TEST_DEPTH {
            return true;
        }
        depth += 1;
        if is_node_display_none_effective(store, layout_tree, nid) {
            return true;
        }
        cur = store.parent(nid);
    }
    false
}

/// Resolves the effective `BackgroundFillRes` resource id for `id`, respecting `StateRules`
/// overrides (including solid `bg(...)` overrides clearing a base gradient).
#[inline]
pub(crate) fn effective_background_fill_res<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    id: NodeId,
) -> Option<ResourceId> {
    let mut patched_bg_fill: Option<ResourceId> = None;
    layout_tree.visit_effective_state_patches(store, id, |pv| {
        if let Some(rid) = pv.get_res_id(Visual::BACKGROUND_FILL) {
            patched_bg_fill = Some(rid);
        } else if pv.get_prop(Visual::BACKGROUND).is_some() {
            patched_bg_fill = Some(ResourceId::NONE);
        }
    });
    match patched_bg_fill {
        Some(rid) => (!rid.is_none()).then_some(rid),
        None => store
            .try_row::<Visual>(id)
            .map(|v| v.background_fill())
            .filter(|rid| !rid.is_none()),
    }
}

/// Resolves effective typography for `id` using branchless `ResolvedRow<Typography>` inheritance
/// over `parent_text`, layered with `StateRules` patches and active `SkiaFrameHooks` overrides.
#[inline]
pub(crate) fn resolve_node_text_style<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    frame_hooks: &SkiaFrameHooks,
    id: NodeId,
    style: &VisualSnapshot,
    parent_text: Option<ResolvedChunkText>,
) -> ResolvedChunkText {
    let mut parent_row = ResolvedRow::<Typography>::root(store);
    if let Some(pt) = parent_text {
        parent_row.override_prop(Typography::COLOR, Rgba(pt.text_color.to_u32()));
        parent_row.override_prop(Typography::FONT_SIZE, pt.font_size);
        parent_row.override_prop(Typography::FONT_WEIGHT, pt.font_weight);
        parent_row.override_prop(Typography::TEXT_ALIGN, pt.text_align);
    }
    let resolved_typo = ResolvedRow::<Typography>::resolve(store, id, &parent_row);
    let typo = resolved_typo.get();

    let mut color_override = frame_hooks.overrides_text_color(id);
    let mut font_size_override = false;
    let mut weight_override: Option<u16> = None;
    let mut align_override: Option<TextAlign> = None;
    layout_tree.visit_effective_state_patches(store, id, |pv| {
        if pv.get_prop(Typography::COLOR).is_some() {
            color_override = true;
        }
        if pv.get_prop(Typography::FONT_SIZE).is_some() {
            font_size_override = true;
        }
        if let Some(w) = pv.get_prop(Typography::FONT_WEIGHT) {
            weight_override = Some(w);
        }
        if let Some(a) = pv.get_prop(Typography::TEXT_ALIGN) {
            align_override = Some(normalize_text_align(a));
        }
    });

    ResolvedChunkText {
        font_weight: weight_override.unwrap_or_else(|| typo.font_weight()),
        text_align: align_override.unwrap_or_else(|| normalize_text_align(typo.text_align())),
        text_color: if color_override {
            style.text_color
        } else {
            ColorRgba::from_u32(typo.color().0)
        },
        font_size: if font_size_override {
            style.font_size
        } else {
            typo.font_size()
        },
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_node_into<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    focus: &TextFocusState,
    frame_hooks: &SkiaFrameHooks,
    table_registry: &SkiaTableRegistry,
    plugin_epochs: &[u64],
    base_plugin_epoch: u64,
    scale_factor: f32,
    text_engine: &CosmicTextEngine,
    clip_bounds: LayoutRect,
    offset: (f32, f32),
    in_transform: bool,
    inherited_hidden: bool,
    parent_text: Option<ResolvedChunkText>,
    sort_scratch: &mut Vec<NodeId>,
    ops: &mut Vec<PaintOp>,
    id: NodeId,
) {
    if is_node_display_none_effective(store, layout_tree, id) {
        return;
    }
    let effective_hidden = store
        .try_row::<Visual>(id)
        .map_or(inherited_hidden, |v| v.visibility().resolve(inherited_hidden));

    let Some(base_rect) = layout_tree.layout_of(id) else {
        return;
    };
    let (stx, sty) = layout_tree.compute_sticky_delta(store, id);
    let node_offset = (offset.0 + stx, offset.1 + sty);
    let rect = offset_rect(base_rect, node_offset);

    let mut style = layout_tree.effective_visual(store, id);
    frame_hooks.apply_visual(id, &mut style);
    if style.opacity <= 0.001 {
        return;
    }

    let effective_tf =
        frame_hooks.resolve_transform(id, layout_tree.effective_transform(store, id));
    let has_tf = effective_tf.is_some_and(|t| !t.is_identity());
    let any_tf = in_transform || has_tf;

    let opacity_op_idx = ops.len();
    let pushed_opacity = style.opacity < 0.999;
    if pushed_opacity {
        ops.push(PaintOp::SaveOpacity(style.opacity));
    }

    let tf_op_idx = ops.len();
    if let Some(tf) = effective_tf
        && has_tf
    {
        ops.push(PaintOp::PushTransform {
            matrix: tf.to_surface_matrix_2d(rect.x, rect.y, rect.width, rect.height),
        });
    }

    let radius = style.border_radius.max(0.0);
    let in_clip = any_tf || intersects_clip(clip_bounds, rect, 1.0);

    let shadow_view = if !effective_hidden
        && let Some(v) = store.try_row::<Visual>(id)
        && !v.box_shadows().is_none()
    {
        store.resource::<BoxShadows>(v.box_shadows())
    } else {
        None
    };

    // 1. Outer box shadows (behind background)
    if let Some(shadows) = shadow_view {
        for sh in shadows.iter().filter(|sh| !sh.inset) {
            let shadow_bounds = LayoutRect {
                x: rect.x + sh.offset_x - sh.spread_radius,
                y: rect.y + sh.offset_y - sh.spread_radius,
                width: rect.width + sh.spread_radius * 2.0,
                height: rect.height + sh.spread_radius * 2.0,
            };
            if any_tf || intersects_clip(clip_bounds, shadow_bounds, sh.blur_radius * 1.5 + 1.0) {
                ops.push(PaintOp::DrawShadow {
                    rect,
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
    }

    let resolved_text =
        resolve_node_text_style(store, layout_tree, frame_hooks, id, &style, parent_text);

    // 2-5. Background, Image, Inset box shadows, Border, Text/TextInput, and Plugins
    if !effective_hidden && in_clip {
        let mut painted_gradient = false;
        if let Some(fill_res) = effective_background_fill_res(store, layout_tree, id)
            && let Some(fill) = store.resource::<BackgroundFillRes>(fill_res)
        {
            ops.push(PaintOp::FillRoundedRectGradient {
                rect,
                radius,
                gradient: fill.to_owned(),
            });
            painted_gradient = true;
        }
        if !painted_gradient && style.background.a > 0 {
            ops.push(PaintOp::FillRoundedRect {
                rect,
                radius,
                color: style.background,
            });
        }

        if let Some(img) = store.try_row::<Image>(id) {
            let src_res = img.src();
            if !src_res.is_none()
                && let Some(ImageSrcView::Url(url)) = store.resource::<ImageSrc>(src_res)
                && !url.is_empty()
            {
                ops.push(PaintOp::DrawImage {
                    rect,
                    radius,
                    src: url.to_owned(),
                    fit: img.fit(),
                    resolved: ImageCache::shared().resolve_cached_handle(url),
                });
            }
        }

        // Inset box shadows (above background/gradient/image, below border/text/children)
        if let Some(shadows) = shadow_view {
            for sh in shadows.iter().filter(|sh| sh.inset) {
                ops.push(PaintOp::DrawShadow {
                    rect,
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
            ops.push(PaintOp::StrokeRoundedRect {
                rect,
                radius,
                width: style.border_width,
                color: style.border_color,
            });
        }

        if store.has::<TextInput>(id) {
            emit_text_input_ops(
                store,
                id,
                rect,
                radius,
                resolved_text.text_color,
                resolved_text.font_size,
                resolved_text.font_weight,
                scale_factor,
                focus,
                text_engine,
                ops,
            );
        } else if let Some(tc) = store.try_row::<TextContent>(id) {
            let text = tc.text();
            if !text.is_empty() && resolved_text.font_size > 0.0 && resolved_text.text_color.a > 0 {
                ops.push(PaintOp::DrawText {
                    rect,
                    text: text.to_owned(),
                    font_size: resolved_text.font_size,
                    font_weight: resolved_text.font_weight,
                    text_align: resolved_text.text_align,
                    color: resolved_text.text_color,
                });
            }
        }

        if table_registry
            .iter()
            .iter()
            .any(|sys| store.has_slot(id, sys.slot()))
        {
            let plugin_epoch = plugin_epochs
                .get(id.index())
                .copied()
                .unwrap_or(0)
                .max(base_plugin_epoch);
            ops.push(PaintOp::Plugin {
                node_id: id,
                rect,
                inherited_text_color: Rgba(resolved_text.text_color.to_u32()),
                plugin_epoch,
            });
        }
    }

    // 6. Children (with clip, scroll shift, and z-index sorting)
    let children = store.children(id);
    if !children.is_empty() {
        let clips = should_clip_children(store, id);
        let next_clip = if clips && !any_tf {
            intersect_rects(clip_bounds, rect)
        } else {
            Some(clip_bounds)
        };
        if let Some(child_clip) = next_clip {
            let clip_op_idx = ops.len();
            if clips {
                ops.push(PaintOp::PushClip { rect, radius });
            }

            let (sx, sy) = layout_tree.scene().visual_scroll_shift(store, id);
            let child_offset = (node_offset.0 - sx, node_offset.1 - sy);

            let has_z_order = children.len() > 1
                && children
                    .iter()
                    .any(|&cid| store.try_row::<Visual>(cid).is_some_and(|v| v.z_index() != 0));
            if has_z_order {
                let start = sort_scratch.len();
                sort_scratch.extend_from_slice(children);
                sort_scratch[start..].sort_by_key(|&cid| {
                    store.try_row::<Visual>(cid).map_or(0, |v| v.z_index())
                });
                let end = sort_scratch.len();
                for i in start..end {
                    let cid = sort_scratch[i];
                    emit_node_into(
                        store,
                        layout_tree,
                        focus,
                        frame_hooks,
                        table_registry,
                        plugin_epochs,
                        base_plugin_epoch,
                        scale_factor,
                        text_engine,
                        child_clip,
                        child_offset,
                        any_tf,
                        effective_hidden,
                        Some(resolved_text),
                        sort_scratch,
                        ops,
                        cid,
                    );
                }
                sort_scratch.truncate(start);
            } else {
                for &cid in children {
                    emit_node_into(
                        store,
                        layout_tree,
                        focus,
                        frame_hooks,
                        table_registry,
                        plugin_epochs,
                        base_plugin_epoch,
                        scale_factor,
                        text_engine,
                        child_clip,
                        child_offset,
                        any_tf,
                        effective_hidden,
                        Some(resolved_text),
                        sort_scratch,
                        ops,
                        cid,
                    );
                }
            }

            if clips {
                if ops.len() == clip_op_idx + 1 {
                    ops.pop();
                } else {
                    ops.push(PaintOp::PopClip);
                }
            }
        }
    }

    // 7. Scrollbars for `Scroll` containers
    if !effective_hidden && in_clip && store.has::<Scroll>(id)
        && let Some((cw, ch)) = compute_scroll_content_size(store, layout_tree, id)
    {
        let (sx, sy) = layout_tree.scene().scroll_offset(id);
        for_each_scrollbar_thumb(rect, sx, sy, cw, ch, |thumb_rect, thumb_radius, color| {
            ops.push(PaintOp::FillRoundedRect {
                rect: thumb_rect,
                radius: thumb_radius,
                color,
            });
        });
    }

    if has_tf {
        if ops.len() == tf_op_idx + 1 {
            ops.pop();
        } else {
            ops.push(PaintOp::PopTransform);
        }
    }
    if pushed_opacity {
        if ops.len() == opacity_op_idx + 1 {
            ops.pop();
        } else {
            ops.push(PaintOp::Restore);
        }
    }
}
