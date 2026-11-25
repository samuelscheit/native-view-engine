//! Deterministic FNV-1a64 structural style hashing, base36 class naming, and volatility/leaf-text
//! classification shared by `stylesheet.rs` and `ssr.rs`.

use nve_abi::{ColorRgba, ComponentKind, Dimension, EdgeInsets, LayoutProps, StyleProps};

use crate::css::{canonical_f32, canonical_f32_bits, MirrorElement, RawStyleProps};

/// Point dimension threshold above which `Dimension::Points(v)` routes to the inline lane
/// (catches `VirtualList` spacers and large scroll/canvas surfaces).
pub const VOLATILE_DIMENSION_LIMIT: f32 = 2048.0;

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fnv1a_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

#[inline]
fn fnv1a_u32(hash: u64, v: u32) -> u64 {
    fnv1a_bytes(hash, &v.to_le_bytes())
}

#[inline]
fn fnv1a_f32(hash: u64, v: f32) -> u64 {
    fnv1a_u32(hash, canonical_f32_bits(v))
}

#[inline]
fn fnv1a_dim(hash: u64, d: Dimension) -> u64 {
    match d {
        Dimension::Auto => fnv1a_bytes(hash, &[0]),
        Dimension::Points(v) => fnv1a_f32(fnv1a_bytes(hash, &[1]), v),
        Dimension::Percent(v) => fnv1a_f32(fnv1a_bytes(hash, &[2]), v),
    }
}

#[inline]
fn fnv1a_edges(hash: u64, e: EdgeInsets) -> u64 {
    let h = fnv1a_f32(hash, e.top);
    let h = fnv1a_f32(h, e.right);
    let h = fnv1a_f32(h, e.bottom);
    fnv1a_f32(h, e.left)
}

#[inline]
fn fnv1a_color(hash: u64, c: ColorRgba) -> u64 {
    fnv1a_bytes(hash, &[c.r, c.g, c.b, c.a])
}

#[inline]
fn fnv1a_opt_str(hash: u64, s: Option<&str>) -> u64 {
    match s {
        Some(val) => {
            let h = fnv1a_bytes(hash, &[1]);
            let h = fnv1a_u32(h, val.len() as u32);
            fnv1a_bytes(h, val.as_bytes())
        }
        None => fnv1a_bytes(hash, &[0]),
    }
}

/// Formats a 64-bit hash as deterministic `s<base36>` class token (without leading `.`).
pub fn format_structural_class(hash: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = [0u8; 14];
    let mut n = hash;
    let mut i = buf.len();
    if n == 0 {
        return "s0".to_owned();
    }
    while n > 0 {
        i -= 1;
        buf[i] = DIGITS[(n % 36) as usize];
        n /= 36;
    }
    let mut out = String::with_capacity(1 + (buf.len() - i));
    out.push('s');
    out.push_str(core::str::from_utf8(&buf[i..]).unwrap_or("0"));
    out
}

/// Returns `true` when `dim` is a continuous/volatile point dimension (`> 2048px` or fractional)
/// that must route to the inline `style` lane rather than the structural stylesheet.
#[inline]
pub fn is_volatile_dimension(dim: Dimension) -> bool {
    match dim {
        Dimension::Points(v) => !v.is_finite() || v > VOLATILE_DIMENSION_LIMIT || v.fract() != 0.0,
        Dimension::Auto | Dimension::Percent(_) => false,
    }
}

/// Returns `true` when `opacity` is non-discrete (not an exact 5% step in `0.0..=1.0`,
/// e.g. smooth animation curves) and should route to the inline `style` lane.
#[inline]
pub fn is_volatile_opacity(opacity: f32) -> bool {
    let v = canonical_f32(opacity);
    if !(0.0..=1.0).contains(&v) {
        return true;
    }
    let scaled = v * 20.0;
    (scaled - scaled.round()).abs() > 1e-4
}

/// Detects Dioxus leaf `ComponentKind::Text` nodes whose typography matches their parent
/// `ComponentKind::Text` wrapper, qualifying them for `.nve-tl`.
pub fn is_inheriting_leaf_text(parent: Option<&MirrorElement>, node: &MirrorElement) -> bool {
    if node.kind != ComponentKind::Text || !node.children.is_empty() {
        return false;
    }
    let Some(p) = parent else {
        return false;
    };
    if p.kind != ComponentKind::Text {
        return false;
    }
    node.style.text_color == p.style.text_color
        && canonical_f32_bits(node.style.font_size) == canonical_f32_bits(p.style.font_size)
        && node.raw_style.effective_font_weight() == p.raw_style.effective_font_weight()
        && node.raw_style.effective_text_align() == p.raw_style.effective_text_align()
}

/// Computes deterministic FNV-1a64 hash over the structural (non-volatile) properties of a node.
pub fn hash_structural_style(
    layout: &LayoutProps,
    style: &StyleProps,
    raw: &RawStyleProps,
    leaf_text: bool,
) -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    let sanitize_dim = |d: Dimension| if is_volatile_dimension(d) { Dimension::Auto } else { d };
    h = fnv1a_dim(h, sanitize_dim(layout.width));
    h = fnv1a_dim(h, sanitize_dim(layout.height));
    h = fnv1a_dim(h, sanitize_dim(layout.min_width));
    h = fnv1a_dim(h, sanitize_dim(layout.min_height));
    h = fnv1a_dim(h, sanitize_dim(layout.max_width));
    h = fnv1a_dim(h, sanitize_dim(layout.max_height));
    h = fnv1a_bytes(
        h,
        &[
            layout.flex_direction as u8,
            layout.justify_content as u8,
            layout.align_items as u8,
        ],
    );
    h = fnv1a_f32(h, layout.flex_grow);
    h = fnv1a_f32(h, layout.flex_shrink);
    h = fnv1a_f32(h, layout.gap);
    h = fnv1a_edges(h, layout.padding);
    h = fnv1a_edges(h, layout.margin);

    let bg = if style.background.a == 0 {
        ColorRgba::TRANSPARENT
    } else {
        style.background
    };
    h = fnv1a_color(h, bg);
    if !leaf_text {
        h = fnv1a_color(h, style.text_color);
        h = fnv1a_f32(h, style.font_size);
    } else {
        h = fnv1a_color(h, ColorRgba::BLACK);
        h = fnv1a_f32(h, 16.0);
    }
    let bw = canonical_f32(style.border_width);
    if bw > 0.0 {
        h = fnv1a_f32(h, bw);
        h = fnv1a_color(h, style.border_color);
    } else {
        h = fnv1a_f32(h, 0.0);
        h = fnv1a_color(h, ColorRgba::TRANSPARENT);
    }
    h = fnv1a_f32(h, style.border_radius);
    let op = if is_volatile_opacity(style.opacity) {
        1.0
    } else {
        canonical_f32(style.opacity)
    };
    h = fnv1a_f32(h, op);
    h = fnv1a_u32(h, style.z_index as u32);

    h = fnv1a_opt_str(h, if leaf_text { None } else { raw.effective_font_weight() });
    h = fnv1a_opt_str(h, if leaf_text { None } else { raw.effective_text_align() });
    h = fnv1a_opt_str(h, raw.effective_box_shadow());
    h = fnv1a_opt_str(h, raw.effective_overflow());
    h = fnv1a_opt_str(h, raw.object_fit);
    h
}
