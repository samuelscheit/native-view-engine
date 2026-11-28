//! Typed Tier-1 patch model ([`TypedRsxPatch`]) and direct binary `nve_store::RowPatch`
//! word lowering (`HotPatchMsg::TemplatePatch`) with zero CSS string emission.

use std::fmt;

use nve_engine::{hot_patch_template_key, HotPatchMsg};
use nve_store::{FlatResource, Length, NveTable, Prop, RowPatch, ScalarField, WordValue};
use nve_tables::tables::{
    AlignItems, AlignSelf, BoxSizing, CursorKind, Display, FlexDirection, FlexWrap, FontStyle,
    Interaction, JustifyContent, Layout, Overflow, OverflowWrap, Position, TextAlign,
    Typography, VerticalAlign, Visibility, Visual, WhiteSpace, WordBreak,
};

use super::attrs::{AttrKind, AttrSpec, KeywordEnum};
use super::eval::TypedAttrValue;
use super::value_lower::{
    eval_color, eval_font_weight, eval_length, eval_line_height, eval_px_scalar,
    eval_text_decoration, eval_unitless, eval_z_index,
};

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

/// Every typed change of one Tier-1 save, in source order, lowered straight to binary `RowPatch`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypedRsxPatch {
    pub attr_changes: Vec<TypedAttrChange>,
    pub text_changes: Vec<TextChange>,
}

impl TypedRsxPatch {
    pub fn is_empty(&self) -> bool {
        self.attr_changes.is_empty() && self.text_changes.is_empty()
    }

    /// Lowers to binary `HotPatchMsg::TemplatePatch` messages (one per changed text node, then
    /// one per changed element with flat `RowPatch` `u32` words). Targets any native element via
    /// `(template_key, node_path)` without requiring a `test_id`. Fails (Tier-2) only for theme
    /// tokens, non-scalar string/resource columns (`font-family`, `box-shadow`, `border`), or
    /// out-of-domain values.
    pub fn to_template_patches(
        &self,
        rel_file: &str,
        generation: u64,
    ) -> Result<Vec<HotPatchMsg>, String> {
        let mut messages = Vec::with_capacity(self.text_changes.len() + self.attr_changes.len());
        for c in &self.text_changes {
            messages.push(HotPatchMsg::TemplatePatch {
                generation,
                template_key: hot_patch_template_key(rel_file, c.target.template.ordinal),
                node_path: encode_node_path(&c.target.node_path)?,
                target_test_id: c.target.test_id.clone(),
                row_patch_words: Vec::new(),
                text_updates: Vec::new(),
                text_replacements: vec![(c.old.clone(), c.new.clone())],
            });
        }

        // Group attribute changes per element, keeping first-appearance order.
        let mut elements: Vec<(&TemplateTarget, RowPatch)> = Vec::new();
        for change in &self.attr_changes {
            let slot = match elements
                .iter_mut()
                .find(|(t, _)| t.anchor_tok == change.target.anchor_tok)
            {
                Some((_, rp)) => rp,
                None => {
                    let idx = elements.len();
                    elements.push((&change.target, RowPatch::new()));
                    &mut elements[idx].1
                }
            };
            encode_attr_into_row_patch(slot, change.attr, &change.new)?;
        }

        for (target, rp) in elements {
            let mut words = Vec::new();
            if !rp.write_words(&mut words) {
                return Err("RowPatch exceeded flat word limit".to_owned());
            }
            messages.push(HotPatchMsg::TemplatePatch {
                generation,
                template_key: hot_patch_template_key(rel_file, target.template.ordinal),
                node_path: encode_node_path(&target.node_path)?,
                target_test_id: target.test_id.clone(),
                row_patch_words: words,
                text_updates: Vec::new(),
                text_replacements: Vec::new(),
            });
        }
        Ok(messages)
    }
}

fn encode_node_path(path: &[usize]) -> Result<Vec<u16>, String> {
    let mut out = Vec::with_capacity(path.len());
    for &step in path {
        out.push(
            u16::try_from(step)
                .map_err(|_| format!("node_path index {step} exceeds u16 wire limit"))?,
        );
    }
    Ok(out)
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

/// Encodes one typed RSX attribute value directly into `patch` (`nve_store::RowPatch`).
/// Returns `Err(reason)` when the edit requires a Tier-2 rebuild (theme token, string/resource
/// column, or out-of-domain scalar).
pub fn encode_attr_into_row_patch(
    patch: &mut RowPatch,
    spec: &AttrSpec,
    value: &TypedAttrValue,
) -> Result<(), String> {
    let name = spec.name;
    if let TypedAttrValue::ThemeToken(path) = value {
        return Err(format!(
            "theme token `{path}` needs the running app's theme (Tier-2)"
        ));
    }
    if matches!(spec.kind, AttrKind::Text) {
        return Err(format!(
            "`{name}` is a non-scalar string/resource attribute (Tier-2)"
        ));
    }
    match name {
        // Box sizing & flex lengths
        "width" => set_length(patch, Layout::WIDTH, spec, value),
        "height" => set_length(patch, Layout::HEIGHT, spec, value),
        "min-width" => set_length(patch, Layout::MIN_WIDTH, spec, value),
        "min-height" => set_length(patch, Layout::MIN_HEIGHT, spec, value),
        "max-width" => set_length(patch, Layout::MAX_WIDTH, spec, value),
        "max-height" => set_length(patch, Layout::MAX_HEIGHT, spec, value),
        "flex-basis" => set_length(patch, Layout::FLEX_BASIS, spec, value),
        "top" => set_length(patch, Layout::INSET_TOP, spec, value),
        "right" => set_length(patch, Layout::INSET_RIGHT, spec, value),
        "bottom" => set_length(patch, Layout::INSET_BOTTOM, spec, value),
        "left" => set_length(patch, Layout::INSET_LEFT, spec, value),
        "row-gap" => set_length(patch, Layout::ROW_GAP, spec, value),
        "column-gap" => set_length(patch, Layout::COLUMN_GAP, spec, value),
        "gap" => {
            let l = eval_length(spec, value)?;
            patch.set(Layout::ROW_GAP, l).set(Layout::COLUMN_GAP, l);
            Ok(())
        }
        "padding" => {
            let l = eval_length(spec, value)?;
            patch
                .set(Layout::PADDING_TOP, l)
                .set(Layout::PADDING_RIGHT, l)
                .set(Layout::PADDING_BOTTOM, l)
                .set(Layout::PADDING_LEFT, l);
            Ok(())
        }
        "padding-horizontal" => {
            let l = eval_length(spec, value)?;
            patch.set(Layout::PADDING_LEFT, l).set(Layout::PADDING_RIGHT, l);
            Ok(())
        }
        "padding-vertical" => {
            let l = eval_length(spec, value)?;
            patch.set(Layout::PADDING_TOP, l).set(Layout::PADDING_BOTTOM, l);
            Ok(())
        }
        "padding-top" => set_length(patch, Layout::PADDING_TOP, spec, value),
        "padding-right" => set_length(patch, Layout::PADDING_RIGHT, spec, value),
        "padding-bottom" => set_length(patch, Layout::PADDING_BOTTOM, spec, value),
        "padding-left" => set_length(patch, Layout::PADDING_LEFT, spec, value),
        "margin" => {
            let l = eval_length(spec, value)?;
            patch
                .set(Layout::MARGIN_TOP, l)
                .set(Layout::MARGIN_RIGHT, l)
                .set(Layout::MARGIN_BOTTOM, l)
                .set(Layout::MARGIN_LEFT, l);
            Ok(())
        }
        "margin-top" => set_length(patch, Layout::MARGIN_TOP, spec, value),
        "margin-right" => set_length(patch, Layout::MARGIN_RIGHT, spec, value),
        "margin-bottom" => set_length(patch, Layout::MARGIN_BOTTOM, spec, value),
        "margin-left" => set_length(patch, Layout::MARGIN_LEFT, spec, value),
        // Layout scalars
        "flex" | "flex-grow" => {
            patch.set(Layout::FLEX_GROW, eval_unitless(spec, value)?);
            Ok(())
        }
        "flex-shrink" => {
            patch.set(Layout::FLEX_SHRINK, eval_unitless(spec, value)?);
            Ok(())
        }
        "aspect-ratio" => {
            let v = eval_unitless(spec, value)?;
            patch.set(Layout::ASPECT_RATIO, (v > 0.0).then_some(v));
            Ok(())
        }
        // Layout keyword enums
        "display" => set_kw::<Layout, Display>(patch, Layout::DISPLAY, spec, value),
        "position" => set_kw::<Layout, Position>(patch, Layout::POSITION, spec, value),
        "flex-direction" => {
            set_kw::<Layout, FlexDirection>(patch, Layout::FLEX_DIRECTION, spec, value)
        }
        "flex-wrap" => set_kw::<Layout, FlexWrap>(patch, Layout::FLEX_WRAP, spec, value),
        "justify-content" => {
            set_kw::<Layout, JustifyContent>(patch, Layout::JUSTIFY_CONTENT, spec, value)
        }
        "align-items" => set_kw::<Layout, AlignItems>(patch, Layout::ALIGN_ITEMS, spec, value),
        "align-self" => set_kw::<Layout, AlignSelf>(patch, Layout::ALIGN_SELF, spec, value),
        "box-sizing" => set_kw::<Layout, BoxSizing>(patch, Layout::BOX_SIZING, spec, value),
        // Visual
        "background" | "background-color" => {
            patch.set(Visual::BACKGROUND, eval_color(spec, value)?);
            Ok(())
        }
        "border-color" => {
            patch.set(Visual::BORDER_COLOR, eval_color(spec, value)?);
            Ok(())
        }
        "border-width" => {
            patch.set(Visual::BORDER_WIDTH, eval_px_scalar(spec, value)?);
            Ok(())
        }
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            Err(format!("`{name}` has no per-edge Visual column (Tier-2)"))
        }
        "border-radius" => {
            patch.set(Visual::BORDER_RADIUS, eval_px_scalar(spec, value)?);
            Ok(())
        }
        "opacity" => {
            patch.set(Visual::OPACITY, eval_unitless(spec, value)?);
            Ok(())
        }
        "z-index" => {
            patch.set(Visual::Z_INDEX, eval_z_index(spec, value)?);
            Ok(())
        }
        "visibility" => set_kw::<Visual, Visibility>(patch, Visual::VISIBILITY, spec, value),
        "overflow" => {
            let disc = eval_keyword(spec, value)?;
            let o = Overflow::from_bits(u32::from(disc));
            patch.set(Visual::OVERFLOW_X, o).set(Visual::OVERFLOW_Y, o);
            Ok(())
        }
        "overflow-x" => set_kw::<Visual, Overflow>(patch, Visual::OVERFLOW_X, spec, value),
        "overflow-y" => set_kw::<Visual, Overflow>(patch, Visual::OVERFLOW_Y, spec, value),
        // Typography
        "color" => {
            patch.set(Typography::COLOR, eval_color(spec, value)?);
            Ok(())
        }
        "font-size" => {
            patch.set(Typography::FONT_SIZE, eval_px_scalar(spec, value)?);
            Ok(())
        }
        "letter-spacing" => {
            patch.set(Typography::LETTER_SPACING, eval_px_scalar(spec, value)?);
            Ok(())
        }
        "font-weight" => {
            patch.set(Typography::FONT_WEIGHT, eval_font_weight(spec, value)?);
            Ok(())
        }
        "line-height" => {
            patch.set(Typography::LINE_HEIGHT, eval_line_height(spec, value)?);
            Ok(())
        }
        "text-decoration" => {
            patch.set(
                Typography::TEXT_DECORATION,
                eval_text_decoration(spec, value)?,
            );
            Ok(())
        }
        "font-style" => set_kw::<Typography, FontStyle>(patch, Typography::FONT_STYLE, spec, value),
        "text-align" => set_kw::<Typography, TextAlign>(patch, Typography::TEXT_ALIGN, spec, value),
        "white-space" => {
            set_kw::<Typography, WhiteSpace>(patch, Typography::WHITE_SPACE, spec, value)
        }
        "overflow-wrap" => {
            set_kw::<Typography, OverflowWrap>(patch, Typography::OVERFLOW_WRAP, spec, value)
        }
        "word-break" => set_kw::<Typography, WordBreak>(patch, Typography::WORD_BREAK, spec, value),
        "vertical-align" => {
            set_kw::<Typography, VerticalAlign>(patch, Typography::VERTICAL_ALIGN, spec, value)
        }
        // Interaction
        "cursor" => set_kw::<Interaction, CursorKind>(patch, Interaction::CURSOR, spec, value),
        _ => Err(format!("`{name}` has no binary RowPatch column mapping")),
    }
}

fn set_length(
    patch: &mut RowPatch,
    prop: Prop<Layout, Length>,
    spec: &AttrSpec,
    value: &TypedAttrValue,
) -> Result<(), String> {
    patch.set(prop, eval_length(spec, value)?);
    Ok(())
}

fn set_kw<T: NveTable, E: WordValue + ScalarField>(
    patch: &mut RowPatch,
    prop: Prop<T, E>,
    spec: &AttrSpec,
    value: &TypedAttrValue,
) -> Result<(), String> {
    let disc = eval_keyword(spec, value)?;
    patch.set(prop, E::from_bits(u32::from(disc)));
    Ok(())
}

fn eval_keyword(spec: &AttrSpec, value: &TypedAttrValue) -> Result<u8, String> {
    let AttrKind::Keyword(kw) = spec.kind else {
        return Err(format!("`{}` is not a keyword attribute", spec.name));
    };
    eval_keyword_from(kw, spec.name, value)
}

fn eval_keyword_from(kw: &KeywordEnum, name: &str, value: &TypedAttrValue) -> Result<u8, String> {
    match value {
        TypedAttrValue::Enum { ty, variant } if ty.as_deref().is_none_or(|t| t == kw.ty) => kw
            .discriminant(variant)
            .ok_or_else(|| format!("`{name}` does not accept {value}")),
        TypedAttrValue::String(s) => kw
            .from_keyword_str(s)
            .ok_or_else(|| format!("`{name}` cannot parse keyword {s:?}")),
        _ => Err(format!("`{name}` cannot carry {value}")),
    }
}


