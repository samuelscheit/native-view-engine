//! Scanner for `nve_style: TypedStyle::...` literal builder chains on native RSX elements.
//!
//! Only scans chains rooted at `[path::]* TypedStyle::{new, row, col}()` followed purely by
//! `.method(args...)` calls. Uses a single caller-owned scratch buffer (zero per-element
//! allocations) and suppresses earlier overridden setters in-place.

use super::attrs::{AttrSpec, lookup_typed_style_arg, opaque_typed_style_method_bits};
use super::lexer::Lex;

/// Maximum arguments inspected on any patchable `TypedStyle` method (`padding_xy`, `margin_xy`, `border`).
const MAX_CHAIN_ARGS: usize = 2;

/// Candidate argument span extracted from one `TypedStyle` builder chain.
#[derive(Debug, Clone, Copy)]
pub(super) struct ChainArgSpan {
    pub(super) start: u32,
    pub(super) end: u32,
    pub(super) spec: &'static AttrSpec,
}

/// Scans `lex[start..end]` as a `TypedStyle::{new,row,col}().method(...)...` builder chain into
/// `steps` (cleared on entry), suppressing any earlier setter whose property slots are overwritten
/// by a later setter, shorthand, or `.merge*` call in the same chain.
///
/// Returns `true` iff `lex[start..end]` was a valid `TypedStyle` builder chain (kept spans live in
/// `steps.iter().filter_map(|(s, _)| *s)` in source order). Returns `false` for non-chain values
/// (e.g. `nve_style: s1`) so the caller falls back to scanning for nested `rsx!`.
pub(super) fn scan_typed_style_chain(
    lex: Lex<'_>,
    start: usize,
    end: usize,
    steps: &mut Vec<(Option<ChainArgSpan>, u64)>,
) -> bool {
    steps.clear();
    let Some(mut cur) = match_typed_style_ctor(lex, start, end) else {
        return false;
    };

    while cur < end {
        if !lex.is_punct(cur, b'.') {
            steps.clear();
            return false;
        }
        cur += 1;
        let Some(method) = lex.ident(cur) else {
            steps.clear();
            return false;
        };
        cur += 1;
        if cur >= end || !lex.is_punct(cur, b'(') {
            steps.clear();
            return false;
        }
        let close = lex.close(cur);
        if close >= end {
            steps.clear();
            return false;
        }
        let opaque_bits = opaque_typed_style_method_bits(method);
        if opaque_bits != 0 {
            steps.push((None, opaque_bits));
        } else if let Some((args, argc)) = split_chain_args(lex, cur + 1, close) {
            for (idx, &(a_start, a_end)) in args[..argc].iter().enumerate() {
                if let Some(spec) = lookup_typed_style_arg(method, idx) {
                    steps.push((
                        Some(ChainArgSpan {
                            start: a_start as u32,
                            end: a_end as u32,
                            spec,
                        }),
                        spec.bits,
                    ));
                }
            }
        }
        cur = close + 1;
    }

    // Reverse pass in-place: clear any earlier setter shadowed by a later call or `.merge*`.
    let mut later_written = 0u64;
    for (span, bits) in steps.iter_mut().rev() {
        if (*bits & later_written) != 0 {
            *span = None;
        }
        later_written |= *bits;
    }
    true
}

/// Matches `[path::]* TypedStyle :: (new | row | col) ()`, returning the token index after `)`.
fn match_typed_style_ctor(lex: Lex<'_>, start: usize, end: usize) -> Option<usize> {
    let mut cur = start;
    if cur + 1 < end && lex.is_punct(cur, b':') && lex.is_punct(cur + 1, b':') {
        cur += 2;
    }
    let mut segs = ("", "");
    loop {
        let id = lex.ident(cur)?;
        segs = (segs.1, id);
        cur += 1;
        if cur + 1 < end && lex.is_punct(cur, b':') && lex.is_punct(cur + 1, b':') {
            cur += 2;
            continue;
        }
        break;
    }
    if segs.0 != "TypedStyle" || !matches!(segs.1, "new" | "row" | "col") {
        return None;
    }
    if cur >= end || !lex.is_punct(cur, b'(') {
        return None;
    }
    let close = lex.close(cur);
    if close >= end || close != cur + 1 {
        return None;
    }
    Some(close + 1)
}

fn split_chain_args(
    lex: Lex<'_>,
    start: usize,
    end: usize,
) -> Option<([(usize, usize); MAX_CHAIN_ARGS], usize)> {
    let mut out = [(0usize, 0usize); MAX_CHAIN_ARGS];
    let mut argc = 0usize;
    let mut arg_start = start;
    let mut cur = start;
    loop {
        let at_end = cur >= end;
        if at_end || lex.is_punct(cur, b',') {
            if cur > arg_start {
                if argc >= MAX_CHAIN_ARGS {
                    return None;
                }
                out[argc] = (arg_start, cur);
                argc += 1;
            } else if !at_end || (argc == 0 && start != end) {
                return None;
            }
            if at_end {
                return Some((out, argc));
            }
            cur += 1;
            arg_start = cur;
            continue;
        }
        let opens = lex.is_punct(cur, b'(') || lex.is_punct(cur, b'[') || lex.is_punct(cur, b'{');
        cur = if opens { lex.close(cur) + 1 } else { cur + 1 };
    }
}
