//! Retained `DisplayList` compilation directly from `NodeStore` word tables:
//! culling, `box-shadow` & gradient emission, `Scroll` overlay scrollbars, `Image`, `TextInput`,
//! inherited typography propagation, zero-allocation z-index sorting, and uniform
//! `SkiaTableRegistry` plugin dispatch (`PaintOp::Plugin`).

use nve_abi::{ColorRgba, NodeId};
use nve_backend_headless::HeadlessBackend;
use nve_engine::LayoutRect;
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeStore, ResolvedRow, ResourceId, Rgba};
use nve_tables::tables::{
    BackgroundFillRes, BoxShadows, Display, Image, ImageSrc, ImageSrcView, Layout, Overflow,
    Scroll, TextAlign as NveTextAlign, TextContent, TextInput, Typography, Visual,
};

use crate::frame_hook::SkiaFrameHooks;
use crate::scroll::compute_scroll_content_size;
use crate::table_system::SkiaTableRegistry;
use crate::text::{CosmicTextEngine, TextAlign};
use crate::text_input::emit_text_input_ops;
use crate::{PaintOp, TextFocusState, SCROLLBAR_THUMB_COLOR};

const SCROLLBAR_THICKNESS: f32 = 6.0;
const SCROLLBAR_MARGIN: f32 = 3.0;
const SCROLLBAR_MIN_THUMB: f32 = 20.0;

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

fn emit_scrollview_scrollbars<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    id: NodeId,
    rect: LayoutRect,
    ops: &mut Vec<PaintOp>,
) {
    let Some((cw, ch)) = compute_scroll_content_size(store, layout_tree, id) else {
        return;
    };
    let has_v = ch > rect.height + 0.5;
    let has_h = cw > rect.width + 0.5;
    if !has_v && !has_h {
        return;
    }
    let (sx, sy) = layout_tree.scene().scroll_offset(id);
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
        ops.push(PaintOp::FillRoundedRect {
            rect: LayoutRect {
                x: thumb_x,
                y: thumb_y,
                width: SCROLLBAR_THICKNESS,
                height: thumb_h,
            },
            radius: SCROLLBAR_THICKNESS * 0.5,
            color: SCROLLBAR_THUMB_COLOR,
        });
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
        ops.push(PaintOp::FillRoundedRect {
            rect: LayoutRect {
                x: thumb_x,
                y: thumb_y,
                width: thumb_w,
                height: SCROLLBAR_THICKNESS,
            },
            radius: SCROLLBAR_THICKNESS * 0.5,
            color: SCROLLBAR_THUMB_COLOR,
        });
    }
}

#[inline]
fn map_text_align(align: NveTextAlign) -> TextAlign {
    match align {
        NveTextAlign::Start | NveTextAlign::Left | NveTextAlign::Justify => TextAlign::Left,
        NveTextAlign::Center => TextAlign::Center,
        NveTextAlign::End | NveTextAlign::Right => TextAlign::Right,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_node_into<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    focus: &TextFocusState,
    frame_hooks: &SkiaFrameHooks,
    table_registry: &SkiaTableRegistry,
    text_engine: &CosmicTextEngine,
    clip_bounds: LayoutRect,
    offset: (f32, f32),
    in_transform: bool,
    inherited_hidden: bool,
    parent_typo: ResolvedRow<'_, Typography>,
    sort_scratch: &mut Vec<NodeId>,
    ops: &mut Vec<PaintOp>,
    id: NodeId,
) {
    if !store.is_live(id) || nve_tables::tables::is_hidden_text_input(store, id) {
        return;
    }
    let mut patched_display: Option<Display> = None;
    let mut patched_bg_fill: Option<ResourceId> = None;
    let mut color_override = false;
    let mut font_size_override = false;
    layout_tree.visit_effective_state_patches(store, id, |pv| {
        if let Some(d) = pv.get_prop(Layout::DISPLAY) {
            patched_display = Some(d);
        }
        if let Some(rid) = pv.get_res_id(Visual::BACKGROUND_FILL) {
            patched_bg_fill = Some(rid);
        } else if pv.get_prop(Visual::BACKGROUND).is_some() {
            // Solid `bg(...)` override clears any base gradient unless a higher-priority rule
            // explicitly sets a non-none `BACKGROUND_FILL`.
            patched_bg_fill = Some(ResourceId::NONE);
        }
        if pv.get_prop(Typography::COLOR).is_some() {
            color_override = true;
        }
        if pv.get_prop(Typography::FONT_SIZE).is_some() {
            font_size_override = true;
        }
    });
    if patched_display == Some(Display::None)
        || (patched_display.is_none()
            && store
                .try_row::<Layout>(id)
                .is_some_and(|l| l.display() == Display::None))
    {
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

    let effective_tf = frame_hooks.resolve_transform(id, layout_tree.effective_transform(store, id));
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

    // 1. Box shadows
    if !effective_hidden && let Some(v) = store.try_row::<Visual>(id) {
        let shadow_res = v.box_shadows();
        if !shadow_res.is_none()
            && let Some(shadows) = store.resource::<BoxShadows>(shadow_res)
        {
            // Inset shadows are not rasterized by Skia yet (docs/BACKLOG.md); never paint them
            // as outer shadows.
            for sh in shadows.iter().filter(|sh| !sh.inset) {
                let shadow_bounds = LayoutRect {
                    x: rect.x + sh.offset_x - sh.spread_radius,
                    y: rect.y + sh.offset_y - sh.spread_radius,
                    width: rect.width + sh.spread_radius * 2.0,
                    height: rect.height + sh.spread_radius * 2.0,
                };
                if any_tf || intersects_clip(clip_bounds, shadow_bounds, sh.blur_radius * 1.5 + 1.0)
                {
                    ops.push(PaintOp::DrawShadow {
                        rect,
                        radius,
                        offset_x: sh.offset_x,
                        offset_y: sh.offset_y,
                        blur_radius: sh.blur_radius,
                        spread_radius: sh.spread_radius,
                        color: ColorRgba::from_u32(sh.color.0),
                    });
                }
            }
        }
    }

    // 2. Background gradient or solid fill
    if !effective_hidden && in_clip {
        let mut painted_gradient = false;
        let fill_res = match patched_bg_fill {
            Some(rid) => (!rid.is_none()).then_some(rid),
            None => store
                .try_row::<Visual>(id)
                .map(|v| v.background_fill())
                .filter(|rid| !rid.is_none()),
        };
        if let Some(fill_res) = fill_res
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

        // 3. Border stroke
        if style.border_width > 0.0 && style.border_color.a > 0 {
            ops.push(PaintOp::StrokeRoundedRect {
                rect,
                radius,
                width: style.border_width,
                color: style.border_color,
            });
        }
    }

    // 4. Typography resolution (branchless inheritance via `ResolvedRow<Typography>`;
    //    state/responsive/motion overrides win).
    let resolved_typo = ResolvedRow::<Typography>::resolve(store, id, &parent_typo);
    let typo = resolved_typo.get();
    let effective_weight = typo.font_weight();
    let effective_align = map_text_align(typo.text_align());
    let color_override = color_override || frame_hooks.overrides_text_color(id);
    let effective_color = if color_override {
        style.text_color
    } else {
        ColorRgba::from_u32(typo.color().0)
    };
    let effective_font_size = if font_size_override {
        style.font_size
    } else {
        typo.font_size()
    };

    // 5. Leaf & specialized core tables: Image, TextInput, TextContent, and registered SkiaTableSystems
    if !effective_hidden && in_clip {
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
                });
            }
        }

        if store.has::<TextInput>(id) {
            emit_text_input_ops(
                store,
                id,
                rect,
                radius,
                effective_color,
                effective_font_size,
                effective_weight,
                focus,
                text_engine,
                ops,
            );
        } else if let Some(tc) = store.try_row::<TextContent>(id) {
            let text = tc.text();
            if !text.is_empty() && effective_font_size > 0.0 && effective_color.a > 0 {
                ops.push(PaintOp::DrawText {
                    rect,
                    text: text.to_owned(),
                    font_size: effective_font_size,
                    font_weight: effective_weight,
                    text_align: effective_align,
                    color: effective_color,
                });
            }
        }

        // Uniform plugin dispatch (`nve-svg-skia`, `nve-canvas-skia`, 3P crates).
        if table_registry
            .iter()
            .iter()
            .any(|sys| store.has_slot(id, sys.slot()))
        {
            ops.push(PaintOp::Plugin {
                node_id: id,
                rect,
                inherited_text_color: Rgba(effective_color.to_u32()),
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
            let mut child_typo = resolved_typo;
            if color_override {
                child_typo.override_prop(Typography::COLOR, Rgba(effective_color.to_u32()));
            }
            if font_size_override {
                child_typo.override_prop(Typography::FONT_SIZE, effective_font_size);
            }

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
                        text_engine,
                        child_clip,
                        child_offset,
                        any_tf,
                        effective_hidden,
                        child_typo,
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
                        text_engine,
                        child_clip,
                        child_offset,
                        any_tf,
                        effective_hidden,
                        child_typo,
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
    if !effective_hidden && in_clip && store.has::<Scroll>(id) {
        emit_scrollview_scrollbars(store, layout_tree, id, rect, ops);
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
