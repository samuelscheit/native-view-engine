//! Zero-alloc-friendly, bounded-recursion ICU MessageFormat parser and formatter,
//! wired generically over [`I18nBackend`] so plural selection and `#` number
//! formatting automatically use the active platform backend (`Intl.*` on Web,
//! `BuiltinI18nBackend` on SSR/headless/fallback).

mod parser;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use crate::backend::{I18nBackend, NativeI18nBackend};
use crate::error::I18nError;
use crate::format::NumberFormatOptions;
use crate::locale::LocaleId;
use crate::plural::{PluralCategory, PluralKind, PluralOperands};

use parser::{
    ExactDecimalKey, MessageNode, NumberArgStyle, PluralBranch, PluralSelector, parse_template,
};

/// Maximum permitted nesting depth for ICU MessageFormat blocks.
pub const MAX_NESTING_DEPTH: usize = 16;

/// Argument value passed into ICU MessageFormat formatting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MessageValue<'a> {
    /// String value (used for `{name}` and `{arg, select, ...}`).
    Str(&'a str),
    /// Signed integer value (exact operands, no float rounding).
    Int(i64),
    /// Unsigned integer value.
    UInt(u64),
    /// Floating-point value using shortest decimal representation.
    Float(f64),
    /// Floating-point value with explicit visible fractional digits (e.g. `1.0` with `1`).
    FixedFloat { value: f64, fraction_digits: u32 },
    /// Exact decimal string (e.g. `"1.0"`, `"1.20"`) preserving visible trailing zeros.
    DecimalStr(&'a str),
}

impl<'a> From<&'a str> for MessageValue<'a> {
    #[inline]
    fn from(v: &'a str) -> Self {
        Self::Str(v)
    }
}

impl<'a> From<&'a String> for MessageValue<'a> {
    #[inline]
    fn from(v: &'a String) -> Self {
        Self::Str(v.as_str())
    }
}

impl From<i64> for MessageValue<'_> {
    #[inline]
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<i32> for MessageValue<'_> {
    #[inline]
    fn from(v: i32) -> Self {
        Self::Int(i64::from(v))
    }
}

impl From<u64> for MessageValue<'_> {
    #[inline]
    fn from(v: u64) -> Self {
        Self::UInt(v)
    }
}

impl From<usize> for MessageValue<'_> {
    #[inline]
    fn from(v: usize) -> Self {
        Self::UInt(v as u64)
    }
}

impl From<f64> for MessageValue<'_> {
    #[inline]
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

/// Zero-alloc lookup abstraction for ICU MessageFormat arguments.
pub trait ArgLookup {
    fn get_arg(&self, name: &str) -> Option<MessageValue<'_>>;
}

impl ArgLookup for () {
    #[inline]
    fn get_arg(&self, _name: &str) -> Option<MessageValue<'_>> {
        None
    }
}

impl ArgLookup for [(&str, MessageValue<'_>)] {
    #[inline]
    fn get_arg(&self, name: &str) -> Option<MessageValue<'_>> {
        self.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }
}

impl<const N: usize> ArgLookup for [(&str, MessageValue<'_>); N] {
    #[inline]
    fn get_arg(&self, name: &str) -> Option<MessageValue<'_>> {
        self.as_slice().get_arg(name)
    }
}

impl<T: ArgLookup + ?Sized> ArgLookup for &T {
    #[inline]
    fn get_arg(&self, name: &str) -> Option<MessageValue<'_>> {
        (**self).get_arg(name)
    }
}

impl ArgLookup for BTreeMap<&str, MessageValue<'_>> {
    #[inline]
    fn get_arg(&self, name: &str) -> Option<MessageValue<'_>> {
        self.get(name).copied()
    }
}

impl ArgLookup for HashMap<&str, MessageValue<'_>> {
    #[inline]
    fn get_arg(&self, name: &str) -> Option<MessageValue<'_>> {
        self.get(name).copied()
    }
}

/// Ergonomic builder for message arguments.
#[derive(Debug, Default, Clone)]
pub struct MessageArgs<'a> {
    entries: Vec<(&'a str, MessageValue<'a>)>,
}

impl<'a> MessageArgs<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, name: &'a str, value: impl Into<MessageValue<'a>>) -> Self {
        let val = value.into();
        if let Some((_, existing)) = self.entries.iter_mut().find(|(k, _)| *k == name) {
            *existing = val;
        } else {
            self.entries.push((name, val));
        }
        self
    }
}

impl ArgLookup for MessageArgs<'_> {
    #[inline]
    fn get_arg(&self, name: &str) -> Option<MessageValue<'_>> {
        self.entries.as_slice().get_arg(name)
    }
}

/// Parsed, borrowed ICU MessageFormat AST.
#[derive(Debug, Clone, PartialEq)]
pub struct MessagePattern<'a> {
    nodes: Vec<MessageNode<'a>>,
}

/// Parses an ICU MessageFormat template string into a borrowed [`MessagePattern`].
pub fn parse_message(template: &str) -> Result<MessagePattern<'_>, I18nError> {
    let nodes = parse_template(template)?;
    Ok(MessagePattern { nodes })
}

/// Parses and formats an ICU MessageFormat template using [`NativeI18nBackend`].
pub fn format_message<A: ArgLookup + ?Sized>(
    locale: &LocaleId,
    template: &str,
    args: &A,
) -> Result<String, I18nError> {
    format_message_with_backend(&NativeI18nBackend, locale, template, args)
}

/// Parses and formats an ICU MessageFormat template using an explicit [`I18nBackend`].
pub fn format_message_with_backend<B: I18nBackend + ?Sized, A: ArgLookup + ?Sized>(
    backend: &B,
    locale: &LocaleId,
    template: &str,
    args: &A,
) -> Result<String, I18nError> {
    let pattern = parse_message(template)?;
    let mut out = String::with_capacity(template.len());
    pattern.format_into_with_backend(backend, locale, args, &mut out)?;
    Ok(out)
}

/// Parses and formats an ICU MessageFormat template directly into `out` using [`NativeI18nBackend`].
pub fn format_message_into<A: ArgLookup + ?Sized>(
    locale: &LocaleId,
    template: &str,
    args: &A,
    out: &mut String,
) -> Result<(), I18nError> {
    format_message_into_with_backend(&NativeI18nBackend, locale, template, args, out)
}

/// Parses and formats an ICU MessageFormat template directly into `out` using `backend`.
pub fn format_message_into_with_backend<B: I18nBackend + ?Sized, A: ArgLookup + ?Sized>(
    backend: &B,
    locale: &LocaleId,
    template: &str,
    args: &A,
    out: &mut String,
) -> Result<(), I18nError> {
    let pattern = parse_message(template)?;
    pattern.format_into_with_backend(backend, locale, args, out)
}

impl MessagePattern<'_> {
    /// Formats this parsed pattern using [`NativeI18nBackend`].
    pub fn format<A: ArgLookup + ?Sized>(
        &self,
        locale: &LocaleId,
        args: &A,
    ) -> Result<String, I18nError> {
        self.format_with_backend(&NativeI18nBackend, locale, args)
    }

    /// Formats this parsed pattern using an explicit [`I18nBackend`].
    pub fn format_with_backend<B: I18nBackend + ?Sized, A: ArgLookup + ?Sized>(
        &self,
        backend: &B,
        locale: &LocaleId,
        args: &A,
    ) -> Result<String, I18nError> {
        let mut out = String::new();
        self.format_into_with_backend(backend, locale, args, &mut out)?;
        Ok(out)
    }

    /// Formats this parsed pattern directly into `out` using [`NativeI18nBackend`].
    pub fn format_into<A: ArgLookup + ?Sized>(
        &self,
        locale: &LocaleId,
        args: &A,
        out: &mut String,
    ) -> Result<(), I18nError> {
        self.format_into_with_backend(&NativeI18nBackend, locale, args, out)
    }

    /// Formats this parsed pattern directly into `out` using an explicit [`I18nBackend`].
    pub fn format_into_with_backend<B: I18nBackend + ?Sized, A: ArgLookup + ?Sized>(
        &self,
        backend: &B,
        locale: &LocaleId,
        args: &A,
        out: &mut String,
    ) -> Result<(), I18nError> {
        eval_nodes(backend, &self.nodes, locale, args, None, out)
    }
}

#[derive(Debug, Clone, Copy)]
enum PluralNumberContext {
    Int(i64),
    Float { value: f64, fraction_digits: Option<u32> },
}

fn eval_nodes<B: I18nBackend + ?Sized, A: ArgLookup + ?Sized>(
    backend: &B,
    nodes: &[MessageNode<'_>],
    locale: &LocaleId,
    args: &A,
    plural_ctx: Option<PluralNumberContext>,
    out: &mut String,
) -> Result<(), I18nError> {
    for node in nodes {
        match node {
            MessageNode::Literal(text) => out.push_str(text),
            MessageNode::Apostrophe => out.push('\''),
            MessageNode::Pound => match plural_ctx {
                Some(PluralNumberContext::Int(v)) => {
                    out.push_str(&backend.format_integer(locale, v));
                }
                Some(PluralNumberContext::Float {
                    value,
                    fraction_digits,
                }) => {
                    let opts = match fraction_digits {
                        Some(prec) => NumberFormatOptions {
                            min_fraction_digits: Some(prec),
                            max_fraction_digits: Some(prec),
                            ..NumberFormatOptions::default()
                        },
                        None => NumberFormatOptions::default(),
                    };
                    out.push_str(&backend.format_number(locale, value, opts));
                }
                None => out.push('#'),
            },
            MessageNode::Argument(name) => {
                let value = args.get_arg(name).ok_or_else(|| I18nError::MissingArgument {
                    name: (*name).to_owned(),
                })?;
                write_value_plain(backend, value, locale, out);
            }
            MessageNode::NumberArg { name, style } => {
                let value = args.get_arg(name).ok_or_else(|| I18nError::MissingArgument {
                    name: (*name).to_owned(),
                })?;
                write_value_number(backend, name, value, *style, locale, out)?;
            }
            MessageNode::Plural {
                name,
                kind,
                offset,
                branches,
            } => {
                let value = args.get_arg(name).ok_or_else(|| I18nError::MissingArgument {
                    name: (*name).to_owned(),
                })?;
                eval_plural(backend, name, value, *kind, *offset, branches, locale, args, out)?;
            }
            MessageNode::Select { name, branches } => {
                let value = args.get_arg(name).ok_or_else(|| I18nError::MissingArgument {
                    name: (*name).to_owned(),
                })?;
                let key = match value {
                    MessageValue::Str(s) => s,
                    _ => {
                        return Err(I18nError::InvalidArgumentValue {
                            name: (*name).to_owned(),
                            expected: "a string value for select",
                        });
                    }
                };
                let chosen = branches
                    .iter()
                    .find(|b| b.key == key)
                    .or_else(|| branches.iter().find(|b| b.key == "other"))
                    .expect("validated select block always contains 'other'");
                eval_nodes(backend, &chosen.body, locale, args, None, out)?;
            }
        }
    }
    Ok(())
}

fn write_value_plain<B: I18nBackend + ?Sized>(
    backend: &B,
    value: MessageValue<'_>,
    locale: &LocaleId,
    out: &mut String,
) {
    match value {
        MessageValue::Str(s) => out.push_str(s),
        MessageValue::Int(v) => {
            let _ = write!(out, "{v}");
        }
        MessageValue::UInt(v) => {
            let _ = write!(out, "{v}");
        }
        MessageValue::Float(v) => {
            out.push_str(&backend.format_number(locale, v, NumberFormatOptions::default()));
        }
        MessageValue::FixedFloat {
            value,
            fraction_digits,
        } => {
            let opts = NumberFormatOptions {
                min_fraction_digits: Some(fraction_digits),
                max_fraction_digits: Some(fraction_digits),
                ..NumberFormatOptions::default()
            };
            out.push_str(&backend.format_number(locale, value, opts));
        }
        MessageValue::DecimalStr(s) => out.push_str(s),
    }
}

fn write_value_number<B: I18nBackend + ?Sized>(
    backend: &B,
    name: &str,
    value: MessageValue<'_>,
    style: NumberArgStyle,
    locale: &LocaleId,
    out: &mut String,
) -> Result<(), I18nError> {
    let opts = match style {
        NumberArgStyle::Default => NumberFormatOptions::default(),
        NumberArgStyle::Integer => NumberFormatOptions {
            min_fraction_digits: Some(0),
            max_fraction_digits: Some(0),
            ..NumberFormatOptions::default()
        },
    };
    match value {
        MessageValue::Int(v) => out.push_str(&backend.format_integer(locale, v)),
        MessageValue::UInt(v) => {
            if let Ok(iv) = i64::try_from(v) {
                out.push_str(&backend.format_integer(locale, iv));
            } else {
                out.push_str(&backend.format_number(locale, v as f64, opts));
            }
        }
        MessageValue::Float(v) => out.push_str(&backend.format_number(locale, v, opts)),
        MessageValue::FixedFloat {
            value,
            fraction_digits,
        } => {
            let eff_opts = match style {
                NumberArgStyle::Integer => opts,
                NumberArgStyle::Default => NumberFormatOptions {
                    min_fraction_digits: Some(fraction_digits),
                    max_fraction_digits: Some(fraction_digits),
                    ..NumberFormatOptions::default()
                },
            };
            out.push_str(&backend.format_number(locale, value, eff_opts));
        }
        MessageValue::DecimalStr(s) => {
            let parsed = s.trim().parse::<f64>().map_err(|_| I18nError::InvalidNumber {
                input: s.to_owned(),
            })?;
            out.push_str(&backend.format_number(locale, parsed, opts));
        }
        MessageValue::Str(_) => {
            return Err(I18nError::InvalidArgumentValue {
                name: name.to_owned(),
                expected: "a numeric value for number format",
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn eval_plural<B: I18nBackend + ?Sized, A: ArgLookup + ?Sized>(
    backend: &B,
    name: &str,
    value: MessageValue<'_>,
    kind: PluralKind,
    offset: i64,
    branches: &[PluralBranch<'_>],
    locale: &LocaleId,
    args: &A,
    out: &mut String,
) -> Result<(), I18nError> {
    let (exact_key, adjusted_operands, ctx) = match value {
        MessageValue::Int(v) => {
            let raw_op = PluralOperands::from_int(v);
            let adj = v.saturating_sub(offset);
            (
                ExactDecimalKey::from_operands(v < 0, raw_op),
                PluralOperands::from_int(adj),
                PluralNumberContext::Int(adj),
            )
        }
        MessageValue::UInt(v) => {
            let raw_op = PluralOperands::from_u64(v);
            let adj_i64 = i64::try_from(v)
                .map(|iv| iv.saturating_sub(offset))
                .unwrap_or(i64::MAX);
            let adj_op = if offset == 0 {
                raw_op
            } else {
                PluralOperands::from_int(adj_i64)
            };
            (
                ExactDecimalKey::from_operands(false, raw_op),
                adj_op,
                PluralNumberContext::Int(adj_i64),
            )
        }
        MessageValue::Float(v) => {
            let raw_op = PluralOperands::from_f64(v)?;
            let adj = v - (offset as f64);
            let adj_op = PluralOperands::from_f64(adj)?;
            (
                ExactDecimalKey::from_operands(v < 0.0, raw_op),
                adj_op,
                PluralNumberContext::Float {
                    value: adj,
                    fraction_digits: None,
                },
            )
        }
        MessageValue::FixedFloat {
            value: v,
            fraction_digits,
        } => {
            let raw_op = PluralOperands::from_f64_with_precision(v, fraction_digits)?;
            let adj = v - (offset as f64);
            let adj_op = PluralOperands::from_f64_with_precision(adj, fraction_digits)?;
            (
                ExactDecimalKey::from_operands(v < 0.0, raw_op),
                adj_op,
                PluralNumberContext::Float {
                    value: adj,
                    fraction_digits: Some(fraction_digits),
                },
            )
        }
        MessageValue::DecimalStr(s) => {
            let trimmed = s.trim();
            let negative = trimmed.starts_with('-');
            let raw_op = PluralOperands::parse(trimmed)?;
            let frac_digits = (raw_op.v > 0).then_some(raw_op.v);
            let (adj_op, ctx) = if offset == 0 {
                if raw_op.v == 0 && raw_op.e == 0 {
                    let signed = if negative {
                        -(raw_op.i as i64)
                    } else {
                        raw_op.i as i64
                    };
                    (raw_op, PluralNumberContext::Int(signed))
                } else {
                    let signed_n = if negative { -raw_op.n } else { raw_op.n };
                    (
                        raw_op,
                        PluralNumberContext::Float {
                            value: signed_n,
                            fraction_digits: frac_digits,
                        },
                    )
                }
            } else {
                let signed_n = if negative { -raw_op.n } else { raw_op.n };
                let adj = signed_n - (offset as f64);
                let adj_op = match frac_digits {
                    Some(p) => PluralOperands::from_f64_with_precision(adj, p)?,
                    None => PluralOperands::from_f64(adj)?,
                };
                (
                    adj_op,
                    PluralNumberContext::Float {
                        value: adj,
                        fraction_digits: frac_digits,
                    },
                )
            };
            (ExactDecimalKey::from_operands(negative, raw_op), adj_op, ctx)
        }
        MessageValue::Str(_) => {
            return Err(I18nError::InvalidArgumentValue {
                name: name.to_owned(),
                expected: "a numeric value for plural/selectordinal",
            });
        }
    };

    if let Some(branch) = branches
        .iter()
        .find(|b| b.selector == PluralSelector::Exact(exact_key))
    {
        return eval_nodes(backend, &branch.body, locale, args, Some(ctx), out);
    }

    let cat = backend.plural_category(locale, kind, adjusted_operands);
    let chosen = branches
        .iter()
        .find(|b| b.selector == PluralSelector::Category(cat))
        .or_else(|| {
            branches
                .iter()
                .find(|b| b.selector == PluralSelector::Category(PluralCategory::Other))
        })
        .expect("validated plural block always contains 'other'");

    eval_nodes(backend, &chosen.body, locale, args, Some(ctx), out)
}
