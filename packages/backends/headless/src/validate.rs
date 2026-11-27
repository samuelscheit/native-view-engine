//! Layout/style scalar validation, boolean raw-prop evaluation, and ASCII tree formatting
//! helpers for [`crate::HeadlessBackend`].

use nve_abi::{ComponentKind, Dimension, LayoutProps, NodeId, RawPropValueRef, StyleProps};
use nve_engine::BackendError;

use crate::{HeadlessNode, NodeSlotMap, MAX_TREE_DEPTH};

#[inline]
fn is_valid_scalar(v: f32) -> bool {
    nve_abi::num::is_valid_style_scalar(v)
}

#[inline]
fn is_valid_dim(d: Dimension) -> bool {
    match d {
        Dimension::Auto => true,
        Dimension::Points(v) | Dimension::Percent(v) => is_valid_scalar(v),
    }
}

pub(crate) fn validate_layout(l: &LayoutProps) -> Result<(), BackendError> {
    let ok = is_valid_dim(l.width)
        && is_valid_dim(l.height)
        && is_valid_dim(l.min_width)
        && is_valid_dim(l.min_height)
        && is_valid_dim(l.max_width)
        && is_valid_dim(l.max_height)
        && is_valid_scalar(l.flex_grow)
        && l.flex_grow >= 0.0
        && is_valid_scalar(l.flex_shrink)
        && l.flex_shrink >= 0.0
        && is_valid_scalar(l.gap)
        && l.gap >= 0.0
        && is_valid_scalar(l.padding.top)
        && is_valid_scalar(l.padding.right)
        && is_valid_scalar(l.padding.bottom)
        && is_valid_scalar(l.padding.left)
        && is_valid_scalar(l.margin.top)
        && is_valid_scalar(l.margin.right)
        && is_valid_scalar(l.margin.bottom)
        && is_valid_scalar(l.margin.left);
    if ok {
        Ok(())
    } else {
        Err(BackendError::Internal(
            "invalid non-finite or out-of-range layout scalar".into(),
        ))
    }
}

pub(crate) fn validate_style(s: &StyleProps) -> Result<(), BackendError> {
    let ok = is_valid_scalar(s.border_width)
        && is_valid_scalar(s.border_radius)
        && is_valid_scalar(s.opacity)
        && is_valid_scalar(s.font_size);
    if ok {
        Ok(())
    } else {
        Err(BackendError::Internal(
            "invalid non-finite or out-of-range style scalar".into(),
        ))
    }
}

#[inline]
pub(crate) fn is_truthy_raw_prop_ref(value: RawPropValueRef<'_>) -> bool {
    match value {
        RawPropValueRef::Bool(b) => b,
        RawPropValueRef::String(s) => {
            let t = s.trim();
            !t.is_empty()
                && !matches!(
                    t.to_ascii_lowercase().as_str(),
                    "false" | "0" | "no" | "off"
                )
        }
        RawPropValueRef::I64(n) => n != 0,
        RawPropValueRef::F64(n) => n != 0.0,
        RawPropValueRef::Null | RawPropValueRef::Bytes(_) => false,
    }
}

pub(crate) fn fmt_node(
    nodes: &NodeSlotMap<HeadlessNode>,
    id: NodeId,
    depth: usize,
    out: &mut String,
) {
    if depth > MAX_TREE_DEPTH {
        return;
    }
    let Some(node) = nodes.get(id) else {
        return;
    };
    let indent = "  ".repeat(depth);
    let tag = match (&node.kind, &node.custom_name) {
        (ComponentKind::Custom, Some(name)) => format!("Custom<{name}>"),
        _ => format!("{:?}", node.kind),
    };
    let r = node.computed_rect;
    out.push_str(&format!(
        "{indent}{tag}#{id} [{:.1}, {:.1}, {:.1}x{:.1}]",
        r.x, r.y, r.width, r.height
    ));
    if let Some(text) = node.text.as_deref() {
        out.push_str(&format!(" \"{text}\""));
    }
    if node.event_mask != 0 {
        out.push_str(&format!(" events=0x{:x}", node.event_mask));
    }
    out.push('\n');
    for &child in &node.children {
        fmt_node(nodes, child, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_css_parser_never_produces_layout_or_style_rejected_by_headless() {
        let adversarial = [
            "gap: -8px",
            "gap: 20000000px",
            "width: -10px",
            "width: 1e8px",
            "height: 1e8%",
            "padding: 20000000px",
            "margin-left: -20000000px",
            "flex-grow: -1",
            "flex-grow: 99999999",
            "flex-shrink: -2",
            "flex-shrink: 99999999",
            "flex: 99999999",
            "flex-basis: 1e8%",
            "border-width: 20000000px",
            "border-radius: -4px",
            "border-radius: 1e8px",
            "font-size: 0px",
            "font-size: -4px",
            "font-size: 1e8px",
            "opacity: 99999999",
        ];
        let mut l = LayoutProps::default();
        let mut s = StyleProps::default();
        for css in adversarial {
            assert!(nve_components::apply_typed_attr(&mut l, &mut s, "style", css));
            assert!(validate_layout(&l).is_ok(), "validate_layout failed for `{css}`: {l:?}");
            assert!(validate_style(&s).is_ok(), "validate_style failed for `{css}`: {s:?}");
        }
    }
}
