//! CSS Flexbox/style translation, Option 5 Hybrid compact formatting, semantic HTML5 tag
//! resolution, and [`MirrorElement`].

use crate::layout_ext_css::{write_compact_layout_ext_into, write_compact_transform_into};
use crate::raw_style::RawStyleProps;
use crate::style_hash::{is_volatile_dimension, is_volatile_opacity};

use nve_abi::num::{write_f32, write_i64, write_u64};
use nve_abi::{
    ColorRgba, ComponentKind, Dimension, EdgeInsets, EventMask, FlexAlign, FlexDirection,
    FlexJustify, LayoutProps, NodeId, RawPropValue, StyleProps,
};
use nve_abi::hash::HashMap;
use nve_components::canvas_ir::DrawList;
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
use nve_components::transform::Transform2D;

pub use crate::semantic_attrs::{
    component_to_html_tag, is_semantic_tag_prop, resolve_semantic_tag, SemanticTag,
};

/// Static `.nve` base reset stylesheet emitted once on mount or SSR.
///
/// Uses `font-family:inherit` (never `font:inherit`) on buttons/inputs so inherited `font-size`
/// is never clobbered by the CSS `font` shorthand reset, and preserves UA `display:none` on
/// closed `<dialog>` and `[popover]` overlays so native top-layer elements never paint inline.
pub const NVE_BASE_RESET_CSS: &str = concat!(
    ".nve{display:flex;box-sizing:border-box;flex-direction:column;justify-content:flex-start;align-items:stretch;flex:0 1 auto;margin:0;padding:0;border:0 solid transparent;color:#000;font-size:16px}",
    ".nve-s{overflow:auto}",
    ".nve-b{cursor:pointer;appearance:none;background:transparent;font-family:inherit}",
    ".nve-i{font-family:inherit;outline:none}",
    ".nve-p{cursor:pointer}",
    ".nve-tl{color:inherit;font-size:inherit;font-weight:inherit;text-align:inherit}",
    ".nve:is(dialog:not([open]),[popover]:not(:popover-open)){display:none}",
    "dialog.nve::backdrop{background:rgba(15,23,42,.55);backdrop-filter:blur(2px)}",
    "[dir=\"rtl\"] [data-nve-mirror=\"directional\"],[dir=\"rtl\"][data-nve-mirror=\"directional\"]{transform:scaleX(-1)}"
);

/// Canonicalizes an `f32` to deterministic IEEE-754 bits (folding `-0.0` and non-finite to `0`).
#[inline]
pub fn canonical_f32_bits(v: f32) -> u32 {
    if !v.is_finite() || v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

#[inline]
pub(crate) fn canonical_f32(v: f32) -> f32 {
    if !v.is_finite() || v == 0.0 {
        0.0
    } else {
        v
    }
}

/// Formats a compact CSS numeric value (`nve_abi::num` fixed point: `0` instead of `-0`, no
/// trailing `.0`, at most 3 decimals).
#[inline]
pub(crate) fn write_compact_num(v: f32, out: &mut String) {
    write_f32(out, v);
}

/// Appends `v` followed by `unit`, or a bare `0` when `v` formats as zero.
#[inline]
fn write_compact_unit(v: f32, unit: &str, out: &mut String) {
    let start = out.len();
    write_f32(out, v);
    if &out[start..] != "0" {
        out.push_str(unit);
    }
}

/// Formats a compact CSS pixel length (`0` without unit, otherwise `{v}px`).
#[inline]
pub(crate) fn write_compact_px(v: f32, out: &mut String) {
    write_compact_unit(v, "px", out);
}

/// Appends `v` followed by `unit`, always keeping the unit (full, non-compact CSS).
#[inline]
fn write_full_unit(v: f32, unit: &str, out: &mut String) {
    write_f32(out, v);
    out.push_str(unit);
}

fn push_hex_byte(b: u8, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(HEX[usize::from(b >> 4)] as char);
    out.push(HEX[usize::from(b & 15)] as char);
}

/// Appends `rgba(r,g,b,a)` with the alpha channel as an `nve_abi::num` fixed-point fraction.
pub(crate) fn write_rgba(c: ColorRgba, out: &mut String) {
    out.push_str("rgba(");
    for ch in [c.r, c.g, c.b] {
        write_u64(out, u64::from(ch));
        out.push(',');
    }
    write_f32(out, f32::from(c.a) / 255.0);
    out.push(')');
}

/// Formats a compact [`Dimension`] into `out`.
#[inline]
pub(crate) fn write_compact_dim(d: Dimension, out: &mut String) {
    match d {
        Dimension::Auto => out.push_str("auto"),
        Dimension::Points(v) => write_compact_px(v, out),
        Dimension::Percent(v) => write_compact_unit(v, "%", out),
    }
}

/// Formats a [`ColorRgba`] compactly: `#rrggbb` when `a == 255`, `transparent` when `a == 0`,
/// otherwise `rgba(r,g,b,a)`.
#[inline]
pub fn write_compact_color(c: ColorRgba, out: &mut String) {
    if c.a == 255 {
        out.push('#');
        for ch in [c.r, c.g, c.b] {
            push_hex_byte(ch, out);
        }
    } else if c.a == 0 {
        out.push_str("transparent");
    } else {
        write_rgba(c, out);
    }
}

/// Appends compact 1/2/3/4-value CSS shorthand for `padding` or `margin`, omitting all-zero edges.
pub fn write_compact_edges(prop: &str, e: EdgeInsets, out: &mut String) {
    let t = canonical_f32(e.top);
    let r = canonical_f32(e.right);
    let b = canonical_f32(e.bottom);
    let l = canonical_f32(e.left);
    if t == 0.0 && r == 0.0 && b == 0.0 && l == 0.0 {
        return;
    }
    out.push_str(prop);
    out.push(':');
    if t == r && r == b && b == l {
        write_compact_px(t, out);
    } else if t == b && r == l {
        write_compact_px(t, out);
        out.push(' ');
        write_compact_px(r, out);
    } else if r == l {
        write_compact_px(t, out);
        out.push(' ');
        write_compact_px(r, out);
        out.push(' ');
        write_compact_px(b, out);
    } else {
        write_compact_px(t, out);
        out.push(' ');
        write_compact_px(r, out);
        out.push(' ');
        write_compact_px(b, out);
        out.push(' ');
        write_compact_px(l, out);
    }
    out.push(';');
}

#[inline]
fn write_dim_full(d: Dimension, out: &mut String) {
    match d {
        Dimension::Auto => out.push_str("auto"),
        Dimension::Points(v) => write_full_unit(v, "px", out),
        Dimension::Percent(v) => write_full_unit(v, "%", out),
    }
}

/// Appends full Flexbox CSS declarations for `l` directly into `out` (used by `render_html`).
pub fn layout_to_css_into(l: &LayoutProps, out: &mut String) {
    let dir = match l.flex_direction {
        FlexDirection::Column => "column",
        FlexDirection::Row => "row",
        FlexDirection::ColumnReverse => "column-reverse",
        FlexDirection::RowReverse => "row-reverse",
    };
    let justify = match l.justify_content {
        FlexJustify::FlexStart => "flex-start",
        FlexJustify::Center => "center",
        FlexJustify::FlexEnd => "flex-end",
        FlexJustify::SpaceBetween => "space-between",
        FlexJustify::SpaceAround => "space-around",
        FlexJustify::SpaceEvenly => "space-evenly",
    };
    let align = match l.align_items {
        FlexAlign::Stretch => "stretch",
        FlexAlign::FlexStart => "flex-start",
        FlexAlign::Center => "center",
        FlexAlign::FlexEnd => "flex-end",
        FlexAlign::Baseline => "baseline",
    };

    out.push_str("display:flex;box-sizing:border-box;flex-direction:");
    out.push_str(dir);
    out.push_str(";justify-content:");
    out.push_str(justify);
    out.push_str(";align-items:");
    out.push_str(align);
    out.push_str(";flex-grow:");
    write_f32(out, l.flex_grow);
    out.push_str(";flex-shrink:");
    write_f32(out, l.flex_shrink);
    out.push_str(";gap:");
    write_full_unit(l.gap, "px", out);
    out.push_str(";width:");
    write_dim_full(l.width, out);
    out.push_str(";height:");
    write_dim_full(l.height, out);
    for (prop, e) in [(";padding:", l.padding), (";margin:", l.margin)] {
        out.push_str(prop);
        for (i, v) in [e.top, e.right, e.bottom, e.left].into_iter().enumerate() {
            if i != 0 {
                out.push(' ');
            }
            write_full_unit(v, "px", out);
        }
    }
    out.push(';');

    if l.min_width != Dimension::Auto {
        out.push_str("min-width:");
        write_dim_full(l.min_width, out);
        out.push(';');
    }
    if l.min_height != Dimension::Auto {
        out.push_str("min-height:");
        write_dim_full(l.min_height, out);
        out.push(';');
    }
    if l.max_width != Dimension::Auto {
        out.push_str("max-width:");
        write_dim_full(l.max_width, out);
        out.push(';');
    }
    if l.max_height != Dimension::Auto {
        out.push_str("max-height:");
        write_dim_full(l.max_height, out);
        out.push(';');
    }
}

pub fn layout_to_css(l: &LayoutProps) -> String {
    let mut out = String::with_capacity(256);
    layout_to_css_into(l, &mut out);
    out
}

/// Appends full visual style CSS declarations for `s` directly into `out` (used by `render_html`).
pub fn style_to_css_into(s: &StyleProps, out: &mut String) {
    out.push_str("background-color:");
    write_rgba(s.background, out);
    out.push_str(";color:");
    write_rgba(s.text_color, out);
    out.push_str(";border:");
    write_full_unit(s.border_width, "px solid ", out);
    write_rgba(s.border_color, out);
    out.push_str(";border-radius:");
    write_full_unit(s.border_radius, "px", out);
    out.push_str(";opacity:");
    write_f32(out, s.opacity);
    out.push_str(";font-size:");
    write_full_unit(s.font_size, "px", out);
    out.push_str(";z-index:");
    write_i64(out, i64::from(s.z_index));
    out.push(';');
}

pub fn style_to_css(s: &StyleProps) -> String {
    let mut out = String::with_capacity(192);
    style_to_css_into(s, &mut out);
    out
}

fn kind_default_css(kind: ComponentKind, event_mask: u64) -> &'static str {
    match kind {
        ComponentKind::ScrollView => "overflow:auto;",
        ComponentKind::Pressable => "cursor:pointer;appearance:none;background:transparent;font-family:inherit;",
        ComponentKind::TextInput => "font-family:inherit;outline:none;",
        _ if (event_mask & EventMask::PRESS) != 0 => "cursor:pointer;",
        _ => "",
    }
}

/// Cross-target DOM mirror node with zero stored `layout_css`/`style_css` heap bloat.
#[derive(Debug, Clone)]
pub struct MirrorElement {
    pub kind: ComponentKind,
    pub tag: &'static str,
    pub wrap_code: bool,
    pub layout: LayoutProps,
    pub layout_ext: Option<Box<LayoutExt>>,
    pub style: StyleProps,
    pub background_fill: Option<BackgroundFill>,
    pub transform: Option<Transform2D>,
    pub raw_style: RawStyleProps,
    pub patch_css: String,
    pub class_name: String,
    pub inline_style: String,
    pub has_inline_style: bool,
    pub last_style_frame: u64,
    pub consecutive_style_frames: u8,
    pub text: Option<String>,
    pub children: Vec<NodeId>,
    pub event_mask: u64,
    pub raw_props: HashMap<String, RawPropValue>,
    pub draw_ops: DrawList,
    pub draw_op_count: usize,
    pub svg: Option<SvgPicture>,
    pub show_on: Option<MediaQuery>,
    pub responsive_rules: Vec<ResponsiveRule>,
    pub cursor: Option<CursorKind>,
    pub state_rules: Vec<StateStyleRule>,
    pub transitions: Vec<TransitionSpec>,
}

impl MirrorElement {
    /// The `data-nve-draw-ops` value: the decoded op count of a canvas with a non-empty draw
    /// list. One decision for the mirror, SSR and the live DOM, which removes the attribute when
    /// this is `None`.
    pub fn draw_ops_attr(&self) -> Option<usize> {
        (self.kind == ComponentKind::SkiaCanvas && self.draw_op_count > 0).then_some(self.draw_op_count)
    }

    pub(crate) fn new(kind: ComponentKind) -> Self {
        Self {
            kind,
            tag: component_to_html_tag(kind),
            wrap_code: false,
            layout: LayoutProps::default(),
            layout_ext: None,
            style: StyleProps::default(),
            background_fill: None,
            transform: None,
            raw_style: RawStyleProps::default(),
            patch_css: String::new(),
            class_name: String::new(),
            inline_style: String::new(),
            has_inline_style: false,
            last_style_frame: 0,
            consecutive_style_frames: 0,
            text: None,
            children: Vec::new(),
            event_mask: EventMask::NONE,
            raw_props: HashMap::default(),
            draw_ops: DrawList::empty(),
            draw_op_count: 0,
            svg: None,
            show_on: None,
            responsive_rules: Vec::new(),
            cursor: None,
            state_rules: Vec::new(),
            transitions: Vec::new(),
        }
    }

    /// Records a style/layout mutation on `frame_id` to detect consecutive-frame volatility.
    #[inline]
    pub(crate) fn record_style_mutation(&mut self, frame_id: u64) {
        if frame_id == 0 || self.last_style_frame == frame_id {
            return;
        }
        if self.last_style_frame > 0 && self.last_style_frame.saturating_add(1) == frame_id {
            self.consecutive_style_frames = self.consecutive_style_frames.saturating_add(1);
        } else {
            self.consecutive_style_frames = 1;
        }
        self.last_style_frame = frame_id;
    }

    /// Returns `true` when this node mutated styles in `>= 2` consecutive frames.
    #[inline]
    pub fn is_consecutive_frame_volatile(&self) -> bool {
        self.consecutive_style_frames >= 2
    }

    /// Appends full inline `style` text into `out` on demand without storing cached strings.
    #[inline]
    pub fn combined_css_into(&self, out: &mut String) {
        out.push_str(kind_default_css(self.kind, self.event_mask));
        layout_to_css_into(&self.layout, out);
        write_compact_layout_ext_into(self.layout_ext.as_deref(), out);
        style_to_css_into(&self.style, out);
        crate::paint_svg::write_compact_background_fill_into(self.background_fill.as_ref(), out);
        write_compact_transform_into(self.transform, out);
        write_compact_cursor_into(self.cursor, out);
        TransitionSpec::write_css_list_into(&self.transitions, out);
        self.raw_style.write_compact_into(false, out);
        out.push_str(&self.patch_css);
    }

    /// Full inline `style` text combining kind defaults, typed layout, typed style, `RawStyleProps`,
    /// and Tier-1 patches.
    pub fn combined_css(&self) -> String {
        let mut out = String::with_capacity(384 + self.patch_css.len());
        self.combined_css_into(&mut out);
        out
    }
}

/// Emits compact non-default layout declarations into `out`, optionally skipping volatile dims.
pub fn write_compact_layout_into(l: &LayoutProps, skip_volatile: bool, out: &mut String) {
    match l.flex_direction {
        FlexDirection::Column => {}
        FlexDirection::Row => out.push_str("flex-direction:row;"),
        FlexDirection::ColumnReverse => out.push_str("flex-direction:column-reverse;"),
        FlexDirection::RowReverse => out.push_str("flex-direction:row-reverse;"),
    }
    match l.justify_content {
        FlexJustify::FlexStart => {}
        FlexJustify::Center => out.push_str("justify-content:center;"),
        FlexJustify::FlexEnd => out.push_str("justify-content:flex-end;"),
        FlexJustify::SpaceBetween => out.push_str("justify-content:space-between;"),
        FlexJustify::SpaceAround => out.push_str("justify-content:space-around;"),
        FlexJustify::SpaceEvenly => out.push_str("justify-content:space-evenly;"),
    }
    match l.align_items {
        FlexAlign::Stretch => {}
        FlexAlign::FlexStart => out.push_str("align-items:flex-start;"),
        FlexAlign::Center => out.push_str("align-items:center;"),
        FlexAlign::FlexEnd => out.push_str("align-items:flex-end;"),
        FlexAlign::Baseline => out.push_str("align-items:baseline;"),
    }
    let fg = canonical_f32(l.flex_grow);
    if fg != 0.0 {
        out.push_str("flex-grow:");
        write_compact_num(fg, out);
        out.push(';');
    }
    let fs = canonical_f32(l.flex_shrink);
    if (fs - 1.0).abs() > f32::EPSILON {
        out.push_str("flex-shrink:");
        write_compact_num(fs, out);
        out.push(';');
    }
    let gap = canonical_f32(l.gap);
    if gap != 0.0 {
        out.push_str("gap:");
        write_compact_px(gap, out);
        out.push(';');
    }

    for (prop, dim) in [
        ("width:", l.width),
        ("height:", l.height),
        ("min-width:", l.min_width),
        ("min-height:", l.min_height),
        ("max-width:", l.max_width),
        ("max-height:", l.max_height),
    ] {
        if dim != Dimension::Auto && !(skip_volatile && is_volatile_dimension(dim)) {
            out.push_str(prop);
            write_compact_dim(dim, out);
            out.push(';');
        }
    }

    write_compact_edges("padding", l.padding, out);
    write_compact_edges("margin", l.margin, out);
}

/// Emits compact non-default visual & raw style declarations into `out`.
pub fn write_compact_style_into(
    s: &StyleProps,
    raw: &RawStyleProps,
    leaf_text: bool,
    skip_volatile_opacity: bool,
    out: &mut String,
) {
    if s.background.a > 0 {
        out.push_str("background-color:");
        write_compact_color(s.background, out);
        out.push(';');
    }
    if !leaf_text && s.text_color != ColorRgba::BLACK {
        out.push_str("color:");
        write_compact_color(s.text_color, out);
        out.push(';');
    }
    let bw = canonical_f32(s.border_width);
    if bw > 0.0 {
        out.push_str("border:");
        write_compact_px(bw, out);
        out.push_str(" solid ");
        write_compact_color(s.border_color, out);
        out.push(';');
    }
    let br = canonical_f32(s.border_radius);
    if br != 0.0 {
        out.push_str("border-radius:");
        write_compact_px(br, out);
        out.push(';');
    }
    let op = canonical_f32(s.opacity);
    if (op - 1.0).abs() > f32::EPSILON && !(skip_volatile_opacity && is_volatile_opacity(op)) {
        out.push_str("opacity:");
        write_compact_num(op, out);
        out.push(';');
    }
    let fs = canonical_f32(s.font_size);
    if !leaf_text && (fs - 16.0).abs() > f32::EPSILON {
        out.push_str("font-size:");
        write_compact_px(fs, out);
        out.push(';');
    }
    if s.z_index != 0 {
        out.push_str("z-index:");
        write_i64(out, i64::from(s.z_index));
        out.push(';');
    }
    raw.write_compact_into(leaf_text, out);
}

/// Emits `cursor:<kw>;` when an explicit non-default (`!= CursorKind::Auto`) cursor is set.
#[inline]
pub fn write_compact_cursor_into(cursor: Option<CursorKind>, out: &mut String) {
    if let Some(c) = cursor
        && c != CursorKind::Auto
    {
        out.push_str("cursor:");
        out.push_str(c.to_css_keyword());
        out.push(';');
    }
}

/// Emits ONLY volatile dimensions, non-discrete opacity, and Tier-1 `patch_css` into `out`.
pub fn write_volatile_inline_into(
    l: &LayoutProps,
    s: &StyleProps,
    patch_css: &str,
    out: &mut String,
) {
    for (prop, dim) in [
        ("width:", l.width),
        ("height:", l.height),
        ("min-width:", l.min_width),
        ("min-height:", l.min_height),
        ("max-width:", l.max_width),
        ("max-height:", l.max_height),
    ] {
        if is_volatile_dimension(dim) {
            out.push_str(prop);
            write_compact_dim(dim, out);
            out.push(';');
        }
    }
    if is_volatile_opacity(s.opacity) && (canonical_f32(s.opacity) - 1.0).abs() > f32::EPSILON {
        out.push_str("opacity:");
        write_compact_num(s.opacity, out);
        out.push(';');
    }
    if !patch_css.is_empty() {
        out.push_str(patch_css);
    }
}
