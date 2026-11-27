//! Compact 29-bit longhand field mask + 8-bit shorthand mask for Layer-2 standalone typed
//! attributes ([`StandaloneLayer`]). Guarantees deterministic precedence
//! (`shorthand` < `axis shorthand` < `longhand`) regardless of attribute order, Dioxus
//! static-vs-dynamic split, or `Some -> None -> Some` toggles, with zero string parsing on compose.

use nve_abi::{Dimension, LayoutProps, StyleProps};
use nve_components::{apply_typed_attr_kind, ExtStyleDecl, TypedAttrKind};

use super::StandaloneLayer;

pub(super) const SH_FLEX: u8 = 1 << 0;
pub(super) const SH_PADDING: u8 = 1 << 1;
pub(super) const SH_PAD_H: u8 = 1 << 2;
pub(super) const SH_PAD_V: u8 = 1 << 3;
pub(super) const SH_MARGIN: u8 = 1 << 4;
pub(super) const SH_MAR_H: u8 = 1 << 5;
pub(super) const SH_MAR_V: u8 = 1 << 6;
pub(super) const SH_BORDER: u8 = 1 << 7;

const MASK_WIDTH: u32 = 1 << 0;
const MASK_HEIGHT: u32 = 1 << 1;
const MASK_MIN_WIDTH: u32 = 1 << 2;
const MASK_MIN_HEIGHT: u32 = 1 << 3;
const MASK_MAX_WIDTH: u32 = 1 << 4;
const MASK_MAX_HEIGHT: u32 = 1 << 5;
const MASK_FLEX_DIR: u32 = 1 << 6;
const MASK_JUSTIFY: u32 = 1 << 7;
const MASK_ALIGN: u32 = 1 << 8;
const MASK_FLEX_GROW: u32 = 1 << 9;
const MASK_FLEX_SHRINK: u32 = 1 << 10;
const MASK_FLEX_BASIS: u32 = 1 << 11;
const MASK_GAP: u32 = 1 << 12;
const MASK_PAD_T: u32 = 1 << 13;
const MASK_PAD_R: u32 = 1 << 14;
const MASK_PAD_B: u32 = 1 << 15;
const MASK_PAD_L: u32 = 1 << 16;
const MASK_MAR_T: u32 = 1 << 17;
const MASK_MAR_R: u32 = 1 << 18;
const MASK_MAR_B: u32 = 1 << 19;
const MASK_MAR_L: u32 = 1 << 20;
const MASK_BACKGROUND: u32 = 1 << 21;
const MASK_TEXT_COLOR: u32 = 1 << 22;
const MASK_BORDER_COLOR: u32 = 1 << 23;
const MASK_BORDER_WIDTH: u32 = 1 << 24;
const MASK_BORDER_RADIUS: u32 = 1 << 25;
const MASK_OPACITY: u32 = 1 << 26;
const MASK_FONT_SIZE: u32 = 1 << 27;
const MASK_Z_INDEX: u32 = 1 << 28;

pub(super) const MASK_ANY_STYLE: u32 = MASK_BACKGROUND
    | MASK_TEXT_COLOR
    | MASK_BORDER_COLOR
    | MASK_BORDER_WIDTH
    | MASK_BORDER_RADIUS
    | MASK_OPACITY
    | MASK_FONT_SIZE
    | MASK_Z_INDEX;

/// Shorthand/axis bit for `kind` (`0` for non-shorthand / longhand attributes).
#[inline]
pub(super) const fn shorthand_bit(kind: TypedAttrKind) -> u8 {
    use TypedAttrKind as K;
    match kind {
        K::Flex => SH_FLEX,
        K::Padding => SH_PADDING,
        K::PaddingHorizontal => SH_PAD_H,
        K::PaddingVertical => SH_PAD_V,
        K::Margin => SH_MARGIN,
        K::MarginHorizontal => SH_MAR_H,
        K::MarginVertical => SH_MAR_V,
        K::Border => SH_BORDER,
        _ => 0,
    }
}

/// Mask of all field bits affected by `kind`.
#[inline]
pub(super) const fn mask_for_attr(kind: TypedAttrKind) -> u32 {
    use TypedAttrKind as K;
    match kind {
        K::Width => MASK_WIDTH,
        K::Height => MASK_HEIGHT,
        K::MinWidth => MASK_MIN_WIDTH,
        K::MinHeight => MASK_MIN_HEIGHT,
        K::MaxWidth => MASK_MAX_WIDTH,
        K::MaxHeight => MASK_MAX_HEIGHT,
        K::FlexDirection => MASK_FLEX_DIR,
        K::JustifyContent => MASK_JUSTIFY,
        K::AlignItems => MASK_ALIGN,
        K::Flex => MASK_FLEX_GROW | MASK_FLEX_SHRINK | MASK_FLEX_BASIS,
        K::FlexBasis => MASK_FLEX_BASIS,
        K::FlexGrow => MASK_FLEX_GROW,
        K::FlexShrink => MASK_FLEX_SHRINK,
        K::Gap => MASK_GAP,
        K::Padding => MASK_PAD_T | MASK_PAD_R | MASK_PAD_B | MASK_PAD_L,
        K::PaddingTop => MASK_PAD_T,
        K::PaddingRight => MASK_PAD_R,
        K::PaddingBottom => MASK_PAD_B,
        K::PaddingLeft => MASK_PAD_L,
        K::PaddingHorizontal => MASK_PAD_L | MASK_PAD_R,
        K::PaddingVertical => MASK_PAD_T | MASK_PAD_B,
        K::Margin => MASK_MAR_T | MASK_MAR_R | MASK_MAR_B | MASK_MAR_L,
        K::MarginTop => MASK_MAR_T,
        K::MarginRight => MASK_MAR_R,
        K::MarginBottom => MASK_MAR_B,
        K::MarginLeft => MASK_MAR_L,
        K::MarginHorizontal => MASK_MAR_L | MASK_MAR_R,
        K::MarginVertical => MASK_MAR_T | MASK_MAR_B,
        K::Border => MASK_BORDER_WIDTH | MASK_BORDER_COLOR,
        K::Background => MASK_BACKGROUND,
        K::Color => MASK_TEXT_COLOR,
        K::BorderColor => MASK_BORDER_COLOR,
        K::BorderWidth => MASK_BORDER_WIDTH,
        K::BorderRadius => MASK_BORDER_RADIUS,
        K::Opacity => MASK_OPACITY,
        K::FontSize => MASK_FONT_SIZE,
        K::ZIndex => MASK_Z_INDEX,
    }
}

/// Returns `true` when applying `kind` onto a direct fast-path node would overlap a different
/// shorthand/axis/longhand attribute in the same family, requiring promotion to [`StandaloneLayer`].
#[inline]
pub(super) const fn conflicts_with_direct(
    standalone_mask: u32,
    direct_shorthands: u8,
    kind: TypedAttrKind,
) -> bool {
    let bits = mask_for_attr(kind);
    let sh = shorthand_bit(kind);
    if sh != 0 {
        // Shorthand/axis conflicts if ANY of its target fields are already set by a different attr.
        (standalone_mask & bits) != 0 && (direct_shorthands & sh) == 0
    } else {
        // Longhand conflicts if any enclosing shorthand/axis bit is active for its family.
        let family_sh = match kind {
            TypedAttrKind::FlexGrow | TypedAttrKind::FlexShrink | TypedAttrKind::FlexBasis => {
                SH_FLEX
            }
            TypedAttrKind::PaddingTop | TypedAttrKind::PaddingBottom => SH_PADDING | SH_PAD_V,
            TypedAttrKind::PaddingLeft | TypedAttrKind::PaddingRight => SH_PADDING | SH_PAD_H,
            TypedAttrKind::MarginTop | TypedAttrKind::MarginBottom => SH_MARGIN | SH_MAR_V,
            TypedAttrKind::MarginLeft | TypedAttrKind::MarginRight => SH_MARGIN | SH_MAR_H,
            TypedAttrKind::BorderWidth | TypedAttrKind::BorderColor => SH_BORDER,
            _ => 0,
        };
        (direct_shorthands & family_sh) != 0
    }
}

/// Builds a [`StandaloneLayer`] from a non-conflicting direct fast-path snapshot.
pub(super) fn promote_direct_to_layer(
    mut standalone_mask: u32,
    direct_shorthands: u8,
    layout: LayoutProps,
    style: StyleProps,
    flex_basis: Dimension,
    border_current_color: bool,
) -> StandaloneLayer {
    let mut layer = StandaloneLayer {
        shorthand_mask: direct_shorthands,
        ..Default::default()
    };
    if (direct_shorthands & SH_FLEX) != 0 {
        layer.sh_flex_grow = layout.flex_grow;
        layer.sh_flex_shrink = layout.flex_shrink;
        layer.sh_flex_basis = flex_basis;
        standalone_mask &= !(MASK_FLEX_GROW | MASK_FLEX_SHRINK | MASK_FLEX_BASIS);
    }
    if (direct_shorthands & SH_PADDING) != 0 {
        layer.sh_padding = layout.padding;
        standalone_mask &= !(MASK_PAD_T | MASK_PAD_R | MASK_PAD_B | MASK_PAD_L);
    }
    if (direct_shorthands & SH_PAD_H) != 0 {
        layer.sh_pad_h = layout.padding.left;
        standalone_mask &= !(MASK_PAD_L | MASK_PAD_R);
    }
    if (direct_shorthands & SH_PAD_V) != 0 {
        layer.sh_pad_v = layout.padding.top;
        standalone_mask &= !(MASK_PAD_T | MASK_PAD_B);
    }
    if (direct_shorthands & SH_MARGIN) != 0 {
        layer.sh_margin = layout.margin;
        standalone_mask &= !(MASK_MAR_T | MASK_MAR_R | MASK_MAR_B | MASK_MAR_L);
    }
    if (direct_shorthands & SH_MAR_H) != 0 {
        layer.sh_mar_h = layout.margin.left;
        standalone_mask &= !(MASK_MAR_L | MASK_MAR_R);
    }
    if (direct_shorthands & SH_MAR_V) != 0 {
        layer.sh_mar_v = layout.margin.top;
        standalone_mask &= !(MASK_MAR_T | MASK_MAR_B);
    }
    if (direct_shorthands & SH_BORDER) != 0 {
        layer.sh_border_width = style.border_width;
        layer.sh_border_color = style.border_color;
        layer.sh_border_current_color = border_current_color;
        standalone_mask &= !(MASK_BORDER_WIDTH | MASK_BORDER_COLOR);
    }
    layer.mask = standalone_mask;
    layer.layout = layout;
    layer.style = style;
    layer.flex_basis = flex_basis;
    layer.border_current_color = border_current_color;
    layer
}

/// Applies one standalone typed attribute into its exact tier (shorthand / axis / longhand) inside
/// [`StandaloneLayer`]. Returns `true` iff `value` parsed cleanly.
pub(super) fn apply_to_standalone_layer(
    layer: &mut StandaloneLayer,
    kind: TypedAttrKind,
    value: &str,
) -> bool {
    let sh = shorthand_bit(kind);
    if sh != 0 {
        let mut tmp_l = LayoutProps::default();
        let mut tmp_s = StyleProps::default();
        let mut parsed_basis = None;
        let mut border_current = None;
        let applied = apply_typed_attr_kind(&mut tmp_l, &mut tmp_s, kind, value, |decl| match decl {
            ExtStyleDecl::FlexBasis(b) => parsed_basis = Some(b),
            ExtStyleDecl::BorderCurrentColor(flag) => border_current = Some(flag),
            ExtStyleDecl::ExplicitTextColor | ExtStyleDecl::ExplicitStyle | ExtStyleDecl::Raw(..) => {}
        });
        if !applied {
            return false;
        }
        layer.shorthand_mask |= sh;
        match kind {
            TypedAttrKind::Flex => {
                layer.sh_flex_grow = tmp_l.flex_grow;
                layer.sh_flex_shrink = tmp_l.flex_shrink;
                layer.sh_flex_basis = parsed_basis.unwrap_or(Dimension::Auto);
            }
            TypedAttrKind::Padding => layer.sh_padding = tmp_l.padding,
            TypedAttrKind::PaddingHorizontal => layer.sh_pad_h = tmp_l.padding.left,
            TypedAttrKind::PaddingVertical => layer.sh_pad_v = tmp_l.padding.top,
            TypedAttrKind::Margin => layer.sh_margin = tmp_l.margin,
            TypedAttrKind::MarginHorizontal => layer.sh_mar_h = tmp_l.margin.left,
            TypedAttrKind::MarginVertical => layer.sh_mar_v = tmp_l.margin.top,
            TypedAttrKind::Border => {
                layer.sh_border_width = tmp_s.border_width;
                layer.sh_border_color = tmp_s.border_color;
                layer.sh_border_current_color = border_current.unwrap_or(false);
            }
            _ => {}
        }
        return true;
    }

    let mut parsed_basis = None;
    let mut border_current = None;
    let applied = apply_typed_attr_kind(
        &mut layer.layout,
        &mut layer.style,
        kind,
        value,
        |decl| match decl {
            ExtStyleDecl::FlexBasis(b) => parsed_basis = Some(b),
            ExtStyleDecl::BorderCurrentColor(flag) => border_current = Some(flag),
            ExtStyleDecl::ExplicitTextColor | ExtStyleDecl::ExplicitStyle | ExtStyleDecl::Raw(..) => {}
        },
    );
    if !applied {
        return false;
    }
    layer.mask |= mask_for_attr(kind);
    if let Some(basis) = parsed_basis {
        layer.flex_basis = basis;
    }
    if let Some(flag) = border_current {
        layer.border_current_color = flag;
    }
    true
}

/// Removes one standalone typed attribute from its exact tier in [`StandaloneLayer`], returning
/// `true` iff it had been active.
pub(super) fn remove_from_standalone_layer(layer: &mut StandaloneLayer, kind: TypedAttrKind) -> bool {
    let sh = shorthand_bit(kind);
    if sh != 0 {
        if (layer.shorthand_mask & sh) == 0 {
            return false;
        }
        layer.shorthand_mask &= !sh;
        return true;
    }
    let bits = mask_for_attr(kind);
    if (layer.mask & bits) == 0 {
        return false;
    }
    layer.mask &= !bits;
    clear_attr_fields(
        kind,
        &mut layer.layout,
        &mut layer.style,
        &mut layer.flex_basis,
        &mut layer.border_current_color,
    );
    true
}

pub(super) fn clear_attr_fields(
    kind: TypedAttrKind,
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    flex_basis: &mut Dimension,
    border_current_color: &mut bool,
) {
    use TypedAttrKind as K;
    let dl = LayoutProps::default();
    let ds = StyleProps::default();
    match kind {
        K::Width => layout.width = dl.width,
        K::Height => layout.height = dl.height,
        K::MinWidth => layout.min_width = dl.min_width,
        K::MinHeight => layout.min_height = dl.min_height,
        K::MaxWidth => layout.max_width = dl.max_width,
        K::MaxHeight => layout.max_height = dl.max_height,
        K::FlexDirection => layout.flex_direction = dl.flex_direction,
        K::JustifyContent => layout.justify_content = dl.justify_content,
        K::AlignItems => layout.align_items = dl.align_items,
        K::Flex => {
            layout.flex_grow = dl.flex_grow;
            layout.flex_shrink = dl.flex_shrink;
            *flex_basis = Dimension::Auto;
        }
        K::FlexBasis => *flex_basis = Dimension::Auto,
        K::FlexGrow => layout.flex_grow = dl.flex_grow,
        K::FlexShrink => layout.flex_shrink = dl.flex_shrink,
        K::Gap => layout.gap = dl.gap,
        K::Padding => layout.padding = dl.padding,
        K::PaddingTop => layout.padding.top = 0.0,
        K::PaddingRight => layout.padding.right = 0.0,
        K::PaddingBottom => layout.padding.bottom = 0.0,
        K::PaddingLeft => layout.padding.left = 0.0,
        K::PaddingHorizontal => {
            layout.padding.left = 0.0;
            layout.padding.right = 0.0;
        }
        K::PaddingVertical => {
            layout.padding.top = 0.0;
            layout.padding.bottom = 0.0;
        }
        K::Margin => layout.margin = dl.margin,
        K::MarginTop => layout.margin.top = 0.0,
        K::MarginRight => layout.margin.right = 0.0,
        K::MarginBottom => layout.margin.bottom = 0.0,
        K::MarginLeft => layout.margin.left = 0.0,
        K::MarginHorizontal => {
            layout.margin.left = 0.0;
            layout.margin.right = 0.0;
        }
        K::MarginVertical => {
            layout.margin.top = 0.0;
            layout.margin.bottom = 0.0;
        }
        K::Border => {
            style.border_width = ds.border_width;
            style.border_color = ds.border_color;
            *border_current_color = false;
        }
        K::Background => style.background = ds.background,
        K::Color => style.text_color = ds.text_color,
        K::BorderColor => {
            style.border_color = ds.border_color;
            *border_current_color = false;
        }
        K::BorderWidth => style.border_width = ds.border_width,
        K::BorderRadius => style.border_radius = ds.border_radius,
        K::Opacity => style.opacity = ds.opacity,
        K::FontSize => style.font_size = ds.font_size,
        K::ZIndex => style.z_index = ds.z_index,
    }
}

/// Overlays Layer 2 in deterministic 3-tier order: shorthands -> axis shorthands -> longhands.
pub(super) fn overlay_standalone_layer(
    layer: &StandaloneLayer,
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    override_basis: &mut Option<Dimension>,
    border_current_color: &mut bool,
    explicit_text_color: &mut bool,
    style_touched: &mut bool,
) {
    let sh = layer.shorthand_mask;
    if sh != 0 {
        // Tier 2a: full shorthands.
        if (sh & SH_FLEX) != 0 {
            layout.flex_grow = layer.sh_flex_grow;
            layout.flex_shrink = layer.sh_flex_shrink;
            *override_basis = Some(layer.sh_flex_basis);
        }
        if (sh & SH_PADDING) != 0 {
            layout.padding = layer.sh_padding;
        }
        if (sh & SH_MARGIN) != 0 {
            layout.margin = layer.sh_margin;
        }
        if (sh & SH_BORDER) != 0 {
            *style_touched = true;
            style.border_width = layer.sh_border_width;
            *border_current_color = layer.sh_border_current_color;
            if !layer.sh_border_current_color {
                style.border_color = layer.sh_border_color;
            }
        }
        // Tier 2b: axis shorthands.
        if (sh & SH_PAD_H) != 0 {
            layout.padding.left = layer.sh_pad_h;
            layout.padding.right = layer.sh_pad_h;
        }
        if (sh & SH_PAD_V) != 0 {
            layout.padding.top = layer.sh_pad_v;
            layout.padding.bottom = layer.sh_pad_v;
        }
        if (sh & SH_MAR_H) != 0 {
            layout.margin.left = layer.sh_mar_h;
            layout.margin.right = layer.sh_mar_h;
        }
        if (sh & SH_MAR_V) != 0 {
            layout.margin.top = layer.sh_mar_v;
            layout.margin.bottom = layer.sh_mar_v;
        }
    }

    // Tier 2c: longhands & single-field attributes.
    let m = layer.mask;
    if m == 0 {
        return;
    }
    if (m & MASK_WIDTH) != 0 { layout.width = layer.layout.width; }
    if (m & MASK_HEIGHT) != 0 { layout.height = layer.layout.height; }
    if (m & MASK_MIN_WIDTH) != 0 { layout.min_width = layer.layout.min_width; }
    if (m & MASK_MIN_HEIGHT) != 0 { layout.min_height = layer.layout.min_height; }
    if (m & MASK_MAX_WIDTH) != 0 { layout.max_width = layer.layout.max_width; }
    if (m & MASK_MAX_HEIGHT) != 0 { layout.max_height = layer.layout.max_height; }
    if (m & MASK_FLEX_DIR) != 0 { layout.flex_direction = layer.layout.flex_direction; }
    if (m & MASK_JUSTIFY) != 0 { layout.justify_content = layer.layout.justify_content; }
    if (m & MASK_ALIGN) != 0 { layout.align_items = layer.layout.align_items; }
    if (m & MASK_FLEX_GROW) != 0 { layout.flex_grow = layer.layout.flex_grow; }
    if (m & MASK_FLEX_SHRINK) != 0 { layout.flex_shrink = layer.layout.flex_shrink; }
    if (m & MASK_FLEX_BASIS) != 0 { *override_basis = Some(layer.flex_basis); }
    if (m & MASK_GAP) != 0 { layout.gap = layer.layout.gap; }
    if (m & MASK_PAD_T) != 0 { layout.padding.top = layer.layout.padding.top; }
    if (m & MASK_PAD_R) != 0 { layout.padding.right = layer.layout.padding.right; }
    if (m & MASK_PAD_B) != 0 { layout.padding.bottom = layer.layout.padding.bottom; }
    if (m & MASK_PAD_L) != 0 { layout.padding.left = layer.layout.padding.left; }
    if (m & MASK_MAR_T) != 0 { layout.margin.top = layer.layout.margin.top; }
    if (m & MASK_MAR_R) != 0 { layout.margin.right = layer.layout.margin.right; }
    if (m & MASK_MAR_B) != 0 { layout.margin.bottom = layer.layout.margin.bottom; }
    if (m & MASK_MAR_L) != 0 { layout.margin.left = layer.layout.margin.left; }
    if (m & MASK_ANY_STYLE) != 0 {
        *style_touched = true;
        if (m & MASK_BACKGROUND) != 0 { style.background = layer.style.background; }
        if (m & MASK_TEXT_COLOR) != 0 {
            style.text_color = layer.style.text_color;
            *explicit_text_color = true;
        }
        if (m & MASK_BORDER_COLOR) != 0 {
            *border_current_color = layer.border_current_color;
            if !layer.border_current_color {
                style.border_color = layer.style.border_color;
            }
        }
        if (m & MASK_BORDER_WIDTH) != 0 { style.border_width = layer.style.border_width; }
        if (m & MASK_BORDER_RADIUS) != 0 { style.border_radius = layer.style.border_radius; }
        if (m & MASK_OPACITY) != 0 { style.opacity = layer.style.opacity; }
        if (m & MASK_FONT_SIZE) != 0 { style.font_size = layer.style.font_size; }
        if (m & MASK_Z_INDEX) != 0 { style.z_index = layer.style.z_index; }
    }
}
