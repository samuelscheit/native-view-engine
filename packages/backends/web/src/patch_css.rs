//! CSS declaration emission for zero-copy table-generic [`RowPatchView`]s.

use crate::css::{Keyword, WriteCss};
use nve_abi::num::{write_i64, write_u64};
use nve_abi::{ColorRgba, Dimension, EdgeInsets};
use nve_tables::tables::{
    AlignItems, BackgroundFillRes, Direction, Display, FlexDirection, FlexWrap, GridItem,
    GridLayout, GridTracks, Interaction, JustifyContent, Layout, Overflow, Position, TextAlign,
    Transform, Typography, Visual,
};
use nve_tables::transform::Transform2D;
use nve_store::{
    prop_written, GridLine, Length, NodeStore, NveTable, ResourceId, RowPatchView, Word,
};

use crate::css::{
    canonical_f32, write_compact_dim, write_compact_edges, write_compact_num,

};

#[inline]
fn length_to_dim(l: Length) -> Dimension {
    if l.is_points() {
        Dimension::Points(l.value())
    } else if l.is_percent() {
        Dimension::Percent(l.value())
    } else {
        Dimension::Auto
    }
}

fn write_override_edges(prop: &str, e: EdgeInsets, out: &mut String) {
    let t = canonical_f32(e.top);
    let r = canonical_f32(e.right);
    let b = canonical_f32(e.bottom);
    let l = canonical_f32(e.left);
    if t == 0.0 && r == 0.0 && b == 0.0 && l == 0.0 {
        out.push_str(prop);
        out.push_str(":0;");
    } else {
        out.push_str(prop);
        out.push(':');
        write_compact_edges(e, out);
        out.push(';');
    }
}

fn write_grid_line_css(g: GridLine, out: &mut String) {
    match g {
        GridLine::Auto => out.push_str("auto"),
        GridLine::Line(n) => write_i64(out, i64::from(n)),
        GridLine::Span(n) => {
            out.push_str("span ");
            write_u64(out, u64::from(n.max(1)));
        }
    }
}

struct PatchedTables<'a> {
    lw: &'a [Word],
    lm: &'a [u32],
    has_l: bool,
    vw: &'a [Word],
    vm: &'a [u32],
    has_v: bool,
    tw: &'a [Word],
    tm: &'a [u32],
    has_t: bool,
    tfw: &'a [Word],
    has_tf: bool,
    giw: &'a [Word],
    gim: &'a [u32],
    has_gi: bool,
    iw: &'a [Word],
    im: &'a [u32],
    has_i: bool,
    bg_fill_id: Option<ResourceId>,
    cols_id: Option<ResourceId>,
    rows_id: Option<ResourceId>,
}

fn emit_layout_decls(lw: &[Word], lm: &[u32], out: &mut String) {
    if prop_written(lm, Layout::FLEX_DIRECTION) {
        out.push_str("flex-direction:");
        out.push_str(match Layout::FLEX_DIRECTION.get(lw) {
            FlexDirection::Column => "column",
            FlexDirection::Row => "row",
            FlexDirection::ColumnReverse => "column-reverse",
            FlexDirection::RowReverse => "row-reverse",
        });
        out.push(';');
    }
    if prop_written(lm, Layout::JUSTIFY_CONTENT) {
        out.push_str("justify-content:");
        out.push_str(match Layout::JUSTIFY_CONTENT.get(lw) {
            JustifyContent::FlexStart => "flex-start",
            JustifyContent::Center => "center",
            JustifyContent::FlexEnd => "flex-end",
            JustifyContent::SpaceBetween => "space-between",
            JustifyContent::SpaceAround => "space-around",
            JustifyContent::SpaceEvenly => "space-evenly",
        });
        out.push(';');
    }
    if prop_written(lm, Layout::ALIGN_ITEMS) {
        out.push_str("align-items:");
        out.push_str(match Layout::ALIGN_ITEMS.get(lw) {
            AlignItems::Stretch => "stretch",
            AlignItems::FlexStart => "flex-start",
            AlignItems::Center => "center",
            AlignItems::FlexEnd => "flex-end",
            AlignItems::Baseline => "baseline",
        });
        out.push(';');
    }
    if prop_written(lm, Layout::FLEX_GROW) {
        out.push_str("flex-grow:");
        write_compact_num(canonical_f32(Layout::FLEX_GROW.get(lw)).max(0.0), out);
        out.push(';');
    }
    if prop_written(lm, Layout::FLEX_SHRINK) {
        out.push_str("flex-shrink:");
        write_compact_num(canonical_f32(Layout::FLEX_SHRINK.get(lw)).max(0.0), out);
        out.push(';');
    }
    let has_rg = prop_written(lm, Layout::ROW_GAP);
    let has_cg = prop_written(lm, Layout::COLUMN_GAP);
    if has_rg && has_cg && Layout::ROW_GAP.get(lw) == Layout::COLUMN_GAP.get(lw) {
        out.push_str("gap:");
        crate::css::write_px(canonical_f32(Layout::ROW_GAP.get(lw).value()).max(0.0), out);
        out.push(';');
    } else {
        if has_cg {
            out.push_str("column-gap:");
            crate::css::write_px(canonical_f32(Layout::COLUMN_GAP.get(lw).value()).max(0.0), out);
            out.push(';');
        }
        if has_rg {
            out.push_str("row-gap:");
            crate::css::write_px(canonical_f32(Layout::ROW_GAP.get(lw).value()).max(0.0), out);
            out.push(';');
        }
    }
    for (prop_name, prop) in [
        ("width:", Layout::WIDTH),
        ("height:", Layout::HEIGHT),
        ("min-width:", Layout::MIN_WIDTH),
        ("max-width:", Layout::MAX_WIDTH),
        ("min-height:", Layout::MIN_HEIGHT),
        ("max-height:", Layout::MAX_HEIGHT),
    ] {
        if prop_written(lm, prop) {
            out.push_str(prop_name);
            write_compact_dim(length_to_dim(prop.get(lw)), out);
            out.push(';');
        }
    }
    let has_pt = prop_written(lm, Layout::PADDING_TOP);
    let has_pr = prop_written(lm, Layout::PADDING_RIGHT);
    let has_pb = prop_written(lm, Layout::PADDING_BOTTOM);
    let has_pl = prop_written(lm, Layout::PADDING_LEFT);
    let pad_logical =
        prop_written(lm, Layout::PADDING_INLINE_LOGICAL) && Layout::PADDING_INLINE_LOGICAL.get(lw);
    if pad_logical {
        if has_pt {
            out.push_str("padding-top:");
            crate::css::write_px(canonical_f32(Layout::PADDING_TOP.get(lw).value()).max(0.0), out);
            out.push(';');
        }
        if has_pb {
            out.push_str("padding-bottom:");
            crate::css::write_px(canonical_f32(Layout::PADDING_BOTTOM.get(lw).value()).max(0.0), out);
            out.push(';');
        }
        if has_pl {
            out.push_str("padding-inline-start:");
            crate::css::write_px(canonical_f32(Layout::PADDING_LEFT.get(lw).value()).max(0.0), out);
            out.push(';');
        }
        if has_pr {
            out.push_str("padding-inline-end:");
            crate::css::write_px(canonical_f32(Layout::PADDING_RIGHT.get(lw).value()).max(0.0), out);
            out.push(';');
        }
    } else if has_pt && has_pr && has_pb && has_pl {
        write_override_edges(
            "padding",
            EdgeInsets {
                top: Layout::PADDING_TOP.get(lw).value(),
                right: Layout::PADDING_RIGHT.get(lw).value(),
                bottom: Layout::PADDING_BOTTOM.get(lw).value(),
                left: Layout::PADDING_LEFT.get(lw).value(),
            },
            out,
        );
    } else {
        for (name, written, val) in [
            ("padding-top:", has_pt, Layout::PADDING_TOP.get(lw).value()),
            ("padding-right:", has_pr, Layout::PADDING_RIGHT.get(lw).value()),
            ("padding-bottom:", has_pb, Layout::PADDING_BOTTOM.get(lw).value()),
            ("padding-left:", has_pl, Layout::PADDING_LEFT.get(lw).value()),
        ] {
            if written {
                out.push_str(name);
                crate::css::write_px(canonical_f32(val).max(0.0), out);
                out.push(';');
            }
        }
    }
    let has_mt = prop_written(lm, Layout::MARGIN_TOP);
    let has_mr = prop_written(lm, Layout::MARGIN_RIGHT);
    let has_mb = prop_written(lm, Layout::MARGIN_BOTTOM);
    let has_ml = prop_written(lm, Layout::MARGIN_LEFT);
    let mar_logical =
        prop_written(lm, Layout::MARGIN_INLINE_LOGICAL) && Layout::MARGIN_INLINE_LOGICAL.get(lw);
    if mar_logical {
        if has_mt {
            out.push_str("margin-top:");
            crate::css::write_px(canonical_f32(Layout::MARGIN_TOP.get(lw).value()), out);
            out.push(';');
        }
        if has_mb {
            out.push_str("margin-bottom:");
            crate::css::write_px(canonical_f32(Layout::MARGIN_BOTTOM.get(lw).value()), out);
            out.push(';');
        }
        if has_ml {
            out.push_str("margin-inline-start:");
            crate::css::write_px(canonical_f32(Layout::MARGIN_LEFT.get(lw).value()), out);
            out.push(';');
        }
        if has_mr {
            out.push_str("margin-inline-end:");
            crate::css::write_px(canonical_f32(Layout::MARGIN_RIGHT.get(lw).value()), out);
            out.push(';');
        }
    } else if has_mt && has_mr && has_mb && has_ml {
        write_override_edges(
            "margin",
            EdgeInsets {
                top: Layout::MARGIN_TOP.get(lw).value(),
                right: Layout::MARGIN_RIGHT.get(lw).value(),
                bottom: Layout::MARGIN_BOTTOM.get(lw).value(),
                left: Layout::MARGIN_LEFT.get(lw).value(),
            },
            out,
        );
    } else {
        for (name, written, val) in [
            ("margin-top:", has_mt, Layout::MARGIN_TOP.get(lw).value()),
            ("margin-right:", has_mr, Layout::MARGIN_RIGHT.get(lw).value()),
            ("margin-bottom:", has_mb, Layout::MARGIN_BOTTOM.get(lw).value()),
            ("margin-left:", has_ml, Layout::MARGIN_LEFT.get(lw).value()),
        ] {
            if written {
                out.push_str(name);
                crate::css::write_px(canonical_f32(val), out);
                out.push(';');
            }
        }
    }
    if prop_written(lm, Layout::FLEX_WRAP) {
        out.push_str("flex-wrap:");
        out.push_str(match Layout::FLEX_WRAP.get(lw) {
            FlexWrap::NoWrap => "nowrap",
            FlexWrap::Wrap => "wrap",
            FlexWrap::WrapReverse => "wrap-reverse",
        });
        out.push(';');
    }
    if prop_written(lm, Layout::DISPLAY) {
        out.push_str("display:");
        out.push_str(match Layout::DISPLAY.get(lw) {
            Display::Flex => "flex",
            Display::None => "none",
            Display::Grid => "grid",
            Display::Block => "block",
            Display::Inline => "inline",
            Display::InlineBlock => "inline-block",
            Display::InlineFlex => "inline-flex",
            Display::Contents => "contents",
        });
        out.push(';');
    }
    if prop_written(lm, Layout::FLEX_BASIS) {
        out.push_str("flex-basis:");
        write_compact_dim(length_to_dim(Layout::FLEX_BASIS.get(lw)), out);
        out.push(';');
    }
    if prop_written(lm, Layout::ASPECT_RATIO)
        && let Some(ratio) = Layout::ASPECT_RATIO.get(lw)
        && ratio.is_finite()
        && ratio > 0.0
    {
        out.push_str("aspect-ratio:");
        write_compact_num(ratio, out);
        out.push(';');
    }
    if prop_written(lm, Layout::POSITION) {
        out.push_str("position:");
        out.push_str(match Layout::POSITION.get(lw) {
            Position::Relative => "relative",
            Position::Absolute => "absolute",
            Position::Sticky => "sticky",
            Position::Fixed => "fixed",
        });
        out.push(';');
    }
    for (prop_name, prop) in [
        ("top:", Layout::INSET_TOP),
        ("right:", Layout::INSET_RIGHT),
        ("bottom:", Layout::INSET_BOTTOM),
        ("left:", Layout::INSET_LEFT),
    ] {
        if prop_written(lm, prop) {
            out.push_str(prop_name);
            write_compact_dim(length_to_dim(prop.get(lw)), out);
            out.push(';');
        }
    }
    if prop_written(lm, Layout::DIRECTION) {
        match Layout::DIRECTION.get(lw) {
            Direction::Ltr => out.push_str("direction:ltr;"),
            Direction::Rtl => out.push_str("direction:rtl;"),
            Direction::Inherit => {}
        }
    }
}

fn emit_row_patch_decls(t: PatchedTables<'_>, store: Option<&NodeStore>, out: &mut String) {
    if t.has_l {
        emit_layout_decls(t.lw, t.lm, out);
    }
    if t.has_v {
        if prop_written(t.vm, Visual::BACKGROUND) {
            out.push_str("background-color:");
            ColorRgba::from_u32(Visual::BACKGROUND.get(t.vw).0).write_css(out);
            out.push(';');
        }
        let has_bw = prop_written(t.vm, Visual::BORDER_WIDTH);
        let has_bc = prop_written(t.vm, Visual::BORDER_COLOR);
        match (has_bw, has_bc) {
            (true, true) => {
                out.push_str("border:");
                crate::css::write_px(canonical_f32(Visual::BORDER_WIDTH.get(t.vw)).max(0.0), out);
                out.push_str(" solid ");
                ColorRgba::from_u32(Visual::BORDER_COLOR.get(t.vw).0).write_css(out);
                out.push(';');
            }
            (true, false) => {
                out.push_str("border-width:");
                crate::css::write_px(canonical_f32(Visual::BORDER_WIDTH.get(t.vw)).max(0.0), out);
                out.push(';');
            }
            (false, true) => {
                out.push_str("border-color:");
                ColorRgba::from_u32(Visual::BORDER_COLOR.get(t.vw).0).write_css(out);
                out.push(';');
            }
            (false, false) => {}
        }
        if prop_written(t.vm, Visual::BORDER_RADIUS) {
            out.push_str("border-radius:");
            crate::css::write_px(canonical_f32(Visual::BORDER_RADIUS.get(t.vw)).max(0.0), out);
            out.push(';');
        }
        if prop_written(t.vm, Visual::OPACITY) {
            out.push_str("opacity:");
            write_compact_num(canonical_f32(Visual::OPACITY.get(t.vw)).clamp(0.0, 1.0), out);
            out.push(';');
        }
        if prop_written(t.vm, Visual::Z_INDEX) {
            out.push_str("z-index:");
            write_i64(out, i64::from(Visual::Z_INDEX.get(t.vw)));
            out.push(';');
        }
        let has_ox = prop_written(t.vm, Visual::OVERFLOW_X);
        let has_oy = prop_written(t.vm, Visual::OVERFLOW_Y);
        let overflow_kw = |o: Overflow| match o {
            Overflow::Visible => "visible",
            Overflow::Hidden => "hidden",
            Overflow::Scroll => "scroll",
            Overflow::Auto => "auto",
        };
        match (has_ox, has_oy) {
            (true, true) => {
                let ox = Visual::OVERFLOW_X.get(t.vw);
                let oy = Visual::OVERFLOW_Y.get(t.vw);
                if ox == oy {
                    out.push_str("overflow:");
                    out.push_str(overflow_kw(ox));
                    out.push(';');
                } else {
                    out.push_str("overflow-x:");
                    out.push_str(overflow_kw(ox));
                    out.push_str(";overflow-y:");
                    out.push_str(overflow_kw(oy));
                    out.push(';');
                }
            }
            (true, false) => {
                out.push_str("overflow-x:");
                out.push_str(overflow_kw(Visual::OVERFLOW_X.get(t.vw)));
                out.push(';');
            }
            (false, true) => {
                out.push_str("overflow-y:");
                out.push_str(overflow_kw(Visual::OVERFLOW_Y.get(t.vw)));
                out.push(';');
            }
            (false, false) => {}
        }
    }

    if let (Some(store), Some(bg_id)) = (store, t.bg_fill_id)
        && !bg_id.is_none()
        && let Some(fill) = store.resource::<BackgroundFillRes>(bg_id)
    {
        crate::css::write_background_decl(&fill, true, out);
    }

    if t.has_t {
        if prop_written(t.tm, Typography::COLOR) {
            out.push_str("color:");
            ColorRgba::from_u32(Typography::COLOR.get(t.tw).0).write_css(out);
            out.push(';');
        }
        if prop_written(t.tm, Typography::FONT_SIZE) {
            out.push_str("font-size:");
            crate::css::write_px(canonical_f32(Typography::FONT_SIZE.get(t.tw)).max(1.0), out);
            out.push(';');
        }
        if prop_written(t.tm, Typography::TEXT_ALIGN) {
            out.push_str("text-align:");
            out.push_str(match Typography::TEXT_ALIGN.get(t.tw) {
                TextAlign::Start => "start",
                TextAlign::End => "end",
                TextAlign::Left => "left",
                TextAlign::Right => "right",
                TextAlign::Center => "center",
                TextAlign::Justify => "justify",
            });
            out.push(';');
        }
        if prop_written(t.tm, Typography::FONT_WEIGHT) {
            out.push_str("font-weight:");
            write_u64(out, u64::from(Typography::FONT_WEIGHT.get(t.tw).clamp(1, 1000)));
            out.push(';');
        }
    }

    if let (Some(store), Some(cid)) = (store, t.cols_id)
        && !cid.is_none()
        && let Some(tracks) = store.resource::<GridTracks>(cid)
        && !tracks.is_empty()
    {
        out.push_str("grid-template-columns:");
        tracks.write_css(out);
        out.push(';');
    }
    if let (Some(store), Some(rid)) = (store, t.rows_id)
        && !rid.is_none()
        && let Some(tracks) = store.resource::<GridTracks>(rid)
        && !tracks.is_empty()
    {
        out.push_str("grid-template-rows:");
        tracks.write_css(out);
        out.push(';');
    }

    if t.has_gi {
        if prop_written(t.gim, GridItem::COLUMN_START) || prop_written(t.gim, GridItem::COLUMN_END) {
            let s = GridItem::COLUMN_START.get(t.giw);
            let e = GridItem::COLUMN_END.get(t.giw);
            out.push_str("grid-column:");
            write_grid_line_css(s, out);
            if e != GridLine::Auto {
                out.push('/');
                write_grid_line_css(e, out);
            }
            out.push(';');
        }
        if prop_written(t.gim, GridItem::ROW_START) || prop_written(t.gim, GridItem::ROW_END) {
            let s = GridItem::ROW_START.get(t.giw);
            let e = GridItem::ROW_END.get(t.giw);
            out.push_str("grid-row:");
            write_grid_line_css(s, out);
            if e != GridLine::Auto {
                out.push('/');
                write_grid_line_css(e, out);
            }
            out.push(';');
        }
    }

    if t.has_tf {
        let tf = Transform2D {
            translate_x: Transform::TRANSLATE_X.get(t.tfw),
            translate_y: Transform::TRANSLATE_Y.get(t.tfw),
            scale_x: Transform::SCALE_X.get(t.tfw),
            scale_y: Transform::SCALE_Y.get(t.tfw),
            rotate_deg: Transform::ROTATE_DEG.get(t.tfw),
            origin_x: Transform::ORIGIN_X.get(t.tfw),
            origin_y: Transform::ORIGIN_Y.get(t.tfw),
        };
        crate::css::write_transform_decls(&tf, true, out);
    }

    if t.has_i && prop_written(t.im, Interaction::CURSOR) {
        out.push_str("cursor:");
        out.push_str(Interaction::CURSOR.get(t.iw).keyword());
        out.push(';');
    }
}

const _: () = {
    assert!(Layout::DESC.words() <= 32);
    assert!(Visual::DESC.words() <= 16);
    assert!(Typography::DESC.words() <= 16);
    assert!(Transform::DESC.words() <= 8);
    assert!(GridItem::DESC.words() <= 8);
    assert!(Interaction::DESC.words() <= 8);
};

macro_rules! impl_write_patch_decls {
    ($fn_name:ident, $doc:literal, $param:ty) => {
        #[doc = $doc]
        pub fn $fn_name(patch: $param, store: Option<&NodeStore>, out: &mut String) {
            let ln = Layout::DESC.words();
            let vn = Visual::DESC.words();
            let tn = Typography::DESC.words();
            let tfn = Transform::DESC.words();
            let gin = GridItem::DESC.words();
            let inn = Interaction::DESC.words();

            let (mut lw, mut lm) = ([0u32; 32], [0u32; 32]);
            lw[..ln].copy_from_slice(Layout::DESC.defaults);
            let has_l = patch.apply_table_with_mask::<Layout>(&mut lw[..ln], &mut lm[..ln]);

            let (mut vw, mut vm) = ([0u32; 16], [0u32; 16]);
            vw[..vn].copy_from_slice(Visual::DESC.defaults);
            let has_v = patch.apply_table_with_mask::<Visual>(&mut vw[..vn], &mut vm[..vn]);

            let (mut tw, mut tm) = ([0u32; 16], [0u32; 16]);
            tw[..tn].copy_from_slice(Typography::DESC.defaults);
            let has_t = patch.apply_table_with_mask::<Typography>(&mut tw[..tn], &mut tm[..tn]);

            let mut tfw = [0u32; 8];
            tfw[..tfn].copy_from_slice(Transform::DESC.defaults);
            let has_tf = patch.apply_table::<Transform>(&mut tfw[..tfn]);

            let (mut giw, mut gim) = ([0u32; 8], [0u32; 8]);
            giw[..gin].copy_from_slice(GridItem::DESC.defaults);
            let has_gi = patch.apply_table_with_mask::<GridItem>(&mut giw[..gin], &mut gim[..gin]);

            let (mut iw, mut im) = ([0u32; 8], [0u32; 8]);
            iw[..inn].copy_from_slice(Interaction::DESC.defaults);
            let has_i = patch.apply_table_with_mask::<Interaction>(&mut iw[..inn], &mut im[..inn]);

            emit_row_patch_decls(
                PatchedTables {
                    lw: &lw[..ln],
                    lm: &lm[..ln],
                    has_l,
                    vw: &vw[..vn],
                    vm: &vm[..vn],
                    has_v,
                    tw: &tw[..tn],
                    tm: &tm[..tn],
                    has_t,
                    tfw: &tfw[..tfn],
                    has_tf,
                    giw: &giw[..gin],
                    gim: &gim[..gin],
                    has_gi,
                    iw: &iw[..inn],
                    im: &im[..inn],
                    has_i,
                    bg_fill_id: patch.get_res_id(Visual::BACKGROUND_FILL),
                    cols_id: patch.get_res_id(GridLayout::TEMPLATE_COLUMNS),
                    rows_id: patch.get_res_id(GridLayout::TEMPLATE_ROWS),
                },
                store,
                out,
            );
        }
    };
}

impl_write_patch_decls!(
    write_row_patch_view_decls,
    "Emits compact CSS declarations for every patched field in a zero-copy [`RowPatchView`], resolving embedded child resources (`BackgroundFillRes`, `GridTracks`) from `store` when given.",
    RowPatchView<'_>
);
