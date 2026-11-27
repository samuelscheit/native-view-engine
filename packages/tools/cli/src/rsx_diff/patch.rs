//! Typed Tier-1 patch model ([`TypedRsxPatch`]) and its lossless lowering to today's
//! `HotPatchMsg::TemplatePatch` wire form (CSS attribute strings).

use std::fmt;

use nve_engine::HotPatchMsg;

use super::attrs::{AttrSpec, ValueClasses};
use super::eval::TypedAttrValue;

/// The `rsx!` invocation a node is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxBlockCoord {
    /// File-order ordinal of the `rsx!` invocation (nested macros included).
    pub ordinal: u32,
    /// 1-based line and char column of the `rsx` identifier in the baseline source.
    pub line: u32,
    pub column: u32,
}

/// Where a typed change applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateTarget {
    pub block: RsxBlockCoord,
    /// Baseline token index of the element's opening `{` (attributes) or the text literal.
    pub anchor_tok: u32,
    /// Runtime child-index path from the outermost enclosing `rsx!` root (control-flow bodies and
    /// direct child `rsx!` invocations continue their parent's path).
    pub node_path: Vec<usize>,
    /// Static `test_id` of the element (attributes) or of the nearest enclosing element (text).
    pub test_id: Option<String>,
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
    /// File-order index of the text child among all static text children.
    pub text_index: usize,
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

    /// Lowers to `TemplatePatch` messages: one for all text changes, then one per changed
    /// element (so text and attribute targeting never mask each other). Fails for values the
    /// string wire form cannot carry (theme tokens need the app's theme).
    pub fn to_template_patches(
        &self,
        template_id: &str,
        generation: u64,
    ) -> Result<Vec<HotPatchMsg>, String> {
        let mut messages = Vec::new();
        if let Some(first) = self.text_changes.first() {
            let uniform_owner = self
                .text_changes
                .iter()
                .all(|c| c.target.test_id == first.target.test_id);
            messages.push(HotPatchMsg::TemplatePatch {
                generation,
                template_id: template_id.to_owned(),
                attr_updates: Vec::new(),
                text_updates: self
                    .text_changes
                    .iter()
                    .map(|c| (c.text_index, c.new.clone()))
                    .collect(),
                target_test_id: uniform_owner
                    .then(|| first.target.test_id.clone())
                    .flatten(),
                node_path: if self.text_changes.len() == 1 {
                    first.target.node_path.clone()
                } else {
                    Vec::new()
                },
                text_replacements: self
                    .text_changes
                    .iter()
                    .map(|c| (c.old.clone(), c.new.clone()))
                    .collect(),
            });
        }
        for element in self
            .attr_changes
            .chunk_by(|a, b| a.target.anchor_tok == b.target.anchor_tok)
        {
            let attr_updates = element
                .iter()
                .map(|c| Ok((c.attr.name.to_owned(), css_value(&c.new, c.attr)?)))
                .collect::<Result<Vec<_>, String>>()?;
            let target = &element[0].target;
            messages.push(HotPatchMsg::TemplatePatch {
                generation,
                template_id: template_id.to_owned(),
                attr_updates,
                text_updates: Vec::new(),
                target_test_id: target.test_id.clone(),
                node_path: target.node_path.clone(),
                text_replacements: Vec::new(),
            });
        }
        Ok(messages)
    }
}

impl fmt::Display for TemplateTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let RsxBlockCoord {
            ordinal,
            line,
            column,
        } = self.block;
        write!(
            f,
            "rsx#{ordinal} @{line}:{column} path {:?}",
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
        let (old, new, idx) = (&self.old, &self.new, self.text_index);
        write!(f, "text #{idx}: {old:?} -> {new:?} ({})", self.target)
    }
}

/// CSS text of `value` for attribute `spec`, exactly as the string attribute path parses it.
pub fn css_value(value: &TypedAttrValue, spec: &AttrSpec) -> Result<String, String> {
    let accepts = spec.accepts;
    let unsupported = || format!("`{}` does not accept {value}", spec.name);
    match value {
        TypedAttrValue::String(s) => Ok(s.clone()),
        TypedAttrValue::Float(v) => css_number(v.to_string(), spec).ok_or_else(unsupported),
        TypedAttrValue::Int(v) => css_number(v.to_string(), spec).ok_or_else(unsupported),
        TypedAttrValue::LengthPx(v) if accepts.intersects(ValueClasses::LENGTH) => {
            Ok(format!("{v}px"))
        }
        TypedAttrValue::LengthPct(v) if accepts.intersects(ValueClasses::LENGTH) => {
            Ok(format!("{v}%"))
        }
        TypedAttrValue::Auto if accepts.intersects(ValueClasses::LENGTH) => Ok("auto".to_owned()),
        TypedAttrValue::Rgba { r, g, b, a } if accepts.intersects(ValueClasses::COLOR) => {
            Ok(if *a == u8::MAX {
                format!("#{r:02x}{g:02x}{b:02x}")
            } else {
                format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
            })
        }
        TypedAttrValue::Enum { variant, .. } if accepts.intersects(ValueClasses::KEYWORD) => {
            Ok(css_keyword(variant))
        }
        TypedAttrValue::ThemeToken(path) => Err(format!(
            "theme token `{path}` needs the running app's theme (typed RowPatch lowering)"
        )),
        _ => Err(unsupported()),
    }
}

/// A bare number: unitless where the attribute takes numbers, else a `px` length.
fn css_number(number: String, spec: &AttrSpec) -> Option<String> {
    if spec.accepts.intersects(ValueClasses::NUMBER) {
        Some(number)
    } else if spec.accepts.intersects(ValueClasses::LENGTH) {
        Some(number + "px")
    } else {
        None
    }
}

/// CSS keyword of a `PascalCase` enum variant: `SpaceBetween` -> `space-between`,
/// `NoWrap` -> `nowrap` (the one CSS keyword that is not kebab-case).
fn css_keyword(variant: &str) -> String {
    if variant.eq_ignore_ascii_case("nowrap") {
        return "nowrap".to_owned();
    }
    let mut out = String::with_capacity(variant.len() + 4);
    for (i, c) in variant.char_indices() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
