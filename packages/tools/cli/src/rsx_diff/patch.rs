//! Typed Tier-1 patch model ([`TypedRsxPatch`]) and its faithful lowering to today's
//! `HotPatchMsg::TemplatePatch` wire form (CSS attribute strings).

use std::fmt;

use nve_abi::num::is_valid_style_scalar;
use nve_engine::HotPatchMsg;

use super::attrs::{AttrKind, AttrSpec, NumberRule};
use super::eval::TypedAttrValue;

/// The template (an `rsx!` invocation or a control-flow body) a node is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateCoord {
    /// File-order ordinal of the template.
    pub ordinal: u32,
    /// 1-based line and char column of the template anchor (`rsx` identifier or body `{`) in
    /// the baseline source.
    pub line: u32,
    pub column: u32,
}

/// Where a typed change applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateTarget {
    pub template: TemplateCoord,
    /// Child-index path from the template root.
    pub node_path: Vec<usize>,
    /// Static `test_id` of the element (attributes) or of the nearest native ancestor (text).
    pub test_id: Option<String>,
    /// Baseline token index of the element's `{` (attributes) or the text literal.
    pub(super) anchor_tok: u32,
}

/// One changed native-element style attribute value.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedAttrChange {
    pub target: TemplateTarget,
    pub attr: &'static AttrSpec,
    pub old: TypedAttrValue,
    pub new: TypedAttrValue,
}

/// One changed static text child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChange {
    pub target: TemplateTarget,
    pub old: String,
    pub new: String,
}

/// Every typed change of one Tier-1 save, in source order, ready for `RowPatch` lowering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypedRsxPatch {
    pub attr_changes: Vec<TypedAttrChange>,
    pub text_changes: Vec<TextChange>,
}

impl TypedRsxPatch {
    pub fn is_empty(&self) -> bool {
        self.attr_changes.is_empty() && self.text_changes.is_empty()
    }

    /// Lowers to `TemplatePatch` messages: one per changed text node (exact old -> new text,
    /// scoped to its owner's `test_id`), then one per changed element. Fails (Tier-2) for edits
    /// the string path cannot apply faithfully: typed-only attributes, theme tokens, values the
    /// CSS parser would read differently, and attribute edits on elements without a static
    /// `test_id` (the web and native consumers would otherwise patch every node).
    pub fn to_template_patches(
        &self,
        template_id: &str,
        generation: u64,
    ) -> Result<Vec<HotPatchMsg>, String> {
        let message = |target: &TemplateTarget, attrs, texts| HotPatchMsg::TemplatePatch {
            generation,
            template_id: template_id.to_owned(),
            attr_updates: attrs,
            text_updates: Vec::new(),
            target_test_id: target.test_id.clone(),
            node_path: target.node_path.clone(),
            text_replacements: texts,
        };
        let mut messages: Vec<HotPatchMsg> = self
            .text_changes
            .iter()
            .map(|c| message(&c.target, Vec::new(), vec![(c.old.clone(), c.new.clone())]))
            .collect();

        // Group attribute changes per element, keeping first-appearance order.
        let mut elements: Vec<(&TemplateTarget, Vec<(String, String)>)> = Vec::new();
        for change in &self.attr_changes {
            let name = change.attr.name;
            if change.target.test_id.is_none() {
                return Err(format!(
                    "`{name}` edit on an element without a static `test_id` cannot be targeted \
                     until template identity lands (typed RowPatch lowering)"
                ));
            }
            let update = (name.to_owned(), css_value(&change.new, change.attr)?);
            match elements
                .iter_mut()
                .find(|(t, _)| t.anchor_tok == change.target.anchor_tok)
            {
                Some((_, updates)) => updates.push(update),
                None => elements.push((&change.target, vec![update])),
            }
        }
        messages.extend(
            elements
                .into_iter()
                .map(|(target, updates)| message(target, updates, Vec::new())),
        );
        Ok(messages)
    }
}

impl fmt::Display for TemplateTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let TemplateCoord {
            ordinal,
            line,
            column,
        } = self.template;
        write!(
            f,
            "template#{ordinal} @{line}:{column} path {:?}",
            self.node_path
        )?;
        match &self.test_id {
            Some(id) => write!(f, " test_id {id:?}"),
            None => Ok(()),
        }
    }
}

impl fmt::Display for TypedAttrChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.attr.name;
        write!(f, "{name}: {} -> {} ({})", self.old, self.new, self.target)
    }
}

impl fmt::Display for TextChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "text: {:?} -> {:?} ({})",
            self.old, self.new, self.target
        )
    }
}

/// CSS text of `value` for attribute `spec` such that `nve_tables::css` stores exactly what
/// the compiled app stores for the same RSX value. Float literals mirror the typed numeric path
/// (`apply_numeric_attr`: `f32` conversion and clamps); strings pass through unchanged.
pub fn css_value(value: &TypedAttrValue, spec: &AttrSpec) -> Result<String, String> {
    let name = spec.name;
    if !spec.css {
        return Err(format!(
            "`{name}` has no string TemplatePatch form yet (typed RowPatch lowering)"
        ));
    }
    let out_of_range = || format!("`{name}` cannot carry {value}: the CSS parser rejects it");
    let unsupported = || {
        Err(format!(
            "`{name}` cannot carry {value} on the string patch path"
        ))
    };
    match (spec.kind, value) {
        (_, TypedAttrValue::String(s)) => Ok(s.clone()),
        (_, TypedAttrValue::ThemeToken(path)) => Err(format!(
            "theme token `{path}` needs the running app's theme (typed RowPatch lowering)"
        )),
        (
            AttrKind::Dimension { float: Some(rule) }
            | AttrKind::Pixels { float: Some(rule) }
            | AttrKind::Number(rule),
            TypedAttrValue::Float(v),
        ) => {
            let css = css_float(rule, *v);
            let scalar = css.trim_end_matches("px").parse::<f32>().unwrap_or(f32::NAN);
            in_domain(spec, scalar).then_some(css).ok_or_else(out_of_range)
        }
        (AttrKind::Number(NumberRule::ZIndex), TypedAttrValue::Int(v)) => {
            Ok((*v).clamp(i32::MIN.into(), i32::MAX.into()).to_string())
        }
        (AttrKind::Dimension { .. } | AttrKind::Pixels { .. }, TypedAttrValue::LengthPx(v)) => {
            in_domain(spec, *v)
                .then(|| format!("{v}px"))
                .ok_or_else(out_of_range)
        }
        (AttrKind::Dimension { .. }, TypedAttrValue::LengthPct(v)) => in_domain(spec, *v)
            .then(|| format!("{v}%"))
            .ok_or_else(out_of_range),
        (AttrKind::Dimension { .. }, TypedAttrValue::Auto) => Ok("auto".to_owned()),
        (AttrKind::Color, TypedAttrValue::Rgba { r, g, b, a }) => Ok(if *a == u8::MAX {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }),
        (AttrKind::Keyword(kw), TypedAttrValue::Enum { ty, variant })
            if ty.as_deref().is_none_or(|t| t == kw.ty) =>
        {
            match kw.css(variant) {
                Some(Some(keyword)) => Ok(keyword.to_owned()),
                _ => unsupported(),
            }
        }
        _ => unsupported(),
    }
}

/// Whether the CSS parser accepts scalar `v` for `spec`: within `MAX_STYLE_SCALAR`, and above the
/// attribute's floor (`>= min`, or `> 0` when the floor is positive, e.g. `font-size`; pixel
/// attributes without a float rule are non-negative).
fn in_domain(spec: &AttrSpec, v: f32) -> bool {
    let min = match spec.kind {
        AttrKind::Dimension { float } | AttrKind::Pixels { float } => match float {
            Some(NumberRule::Px { min }) => min,
            _ => 0.0,
        },
        _ => f32::NEG_INFINITY,
    };
    is_valid_style_scalar(v) && if min > 0.0 { v > 0.0 } else { v >= min }
}

/// A float literal as the typed path stores it (`v as f32`, non-finite as `0`, then clamped).
fn css_float(rule: NumberRule, v: f64) -> String {
    let f = if v.is_finite() { v as f32 } else { 0.0 };
    match rule {
        NumberRule::Px { min } => format!("{}px", f.max(min)),
        NumberRule::Unitless { min, max } => f.clamp(min, max).to_string(),
        NumberRule::ZIndex => (f as i32).to_string(),
    }
}
