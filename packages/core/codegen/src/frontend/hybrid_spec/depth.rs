//! Iterative nesting pre-check that runs before `syn` sees a spec.
//!
//! `syn` parses, spans, and drops its AST recursively: prefix / infix / postfix chains
//! (`- - x`, `a = b = c`, `a.b.c`, `1 + 1 + ...`, `f()()`, `x?`, `x as T`, `if .. else if`),
//! nested groups, and `<...>` all recurse, so deep input overflows the stack before any frontend
//! check can run. Listing recursive tokens leaks; this walk bounds *every* token instead.
//!
//! Nesting at a token = enclosing bracket groups + open `<` + the tokens of the current
//! *segment*. A segment ends at `,` or `;` (`syn` parses list elements and statements in a loop)
//! and at an item keyword (`pub`, `struct`, ...: items are parsed in a loop too). Attributes
//! (`#[..]`, `#![..]`) are parsed in a loop as well and count nothing. Inside an open closure
//! parameter list (`|a, b|`) a `,` does not end the segment. Open `<` resets at `;`.

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

use super::types::error_at;
use crate::error::CodegenError;

/// Maximum nesting accepted from a spec (the same bound as type conversion).
pub(super) const MAX_NESTING: usize = 128;

/// Keywords that only start an item, after which `syn` starts afresh. (`const`, `fn`, and
/// `extern` also appear inside types such as `*const T` and `extern "C" fn() -> T`, so they cannot
/// end a segment.)
const ITEM_KEYWORDS: &[&str] = &[
    "pub", "struct", "enum", "union", "trait", "type", "use", "mod",
];

/// Nesting state of one bracket group being walked.
struct Frame {
    tokens: proc_macro2::token_stream::IntoIter,
    /// Nesting of the enclosing groups.
    base: usize,
    /// Open `<` not yet closed by a `>` in this group (reset at `;`).
    angles: usize,
    /// Tokens of the current segment.
    segment: usize,
    /// A closure parameter list `|...|` is open, so `,` does not end the segment.
    closure_params: bool,
    /// The previous token was a `-` joined to the next punct (the start of `->`).
    after_minus: bool,
    /// The previous token was `#` (or `#!`): the next bracket group is an attribute.
    attribute: bool,
}

impl Frame {
    fn new(tokens: TokenStream, base: usize) -> Self {
        Self {
            tokens: tokens.into_iter(),
            base,
            angles: 0,
            segment: 0,
            closure_params: false,
            after_minus: false,
            attribute: false,
        }
    }

    fn depth(&self) -> usize {
        self.base + self.angles + self.segment
    }
}

/// Rejects `tokens` when its nesting exceeds [`MAX_NESTING`] anywhere.
pub(super) fn check_nesting(tokens: TokenStream) -> Result<(), CodegenError> {
    let mut stack = vec![Frame::new(tokens, 0)];
    while let Some(frame) = stack.last_mut() {
        let Some(tree) = frame.tokens.next() else {
            stack.pop();
            continue;
        };
        let after_minus = std::mem::take(&mut frame.after_minus);
        let attribute = std::mem::take(&mut frame.attribute);
        match &tree {
            // `#[..]` / `#![..]`: parsed in a loop by `syn`, so its tokens count nothing here
            // (its contents are still walked, as a group).
            TokenTree::Group(g) if attribute && g.delimiter() == Delimiter::Bracket => {
                let inner = frame.base + 1;
                stack.push(Frame::new(g.stream(), inner));
                continue;
            }
            TokenTree::Group(g) => {
                frame.segment += 1;
                let inner = frame.depth();
                if inner > MAX_NESTING {
                    return Err(too_deep(g.span_open()));
                }
                stack.push(Frame::new(g.stream(), inner));
                continue;
            }
            TokenTree::Punct(p) => match p.as_char() {
                '#' => frame.attribute = true,
                '!' if attribute => frame.attribute = true,
                ';' => {
                    frame.segment = 0;
                    frame.angles = 0;
                    frame.closure_params = false;
                }
                ',' if !frame.closure_params => frame.segment = 0,
                '<' => {
                    frame.angles += 1;
                    frame.segment += 1;
                }
                // `->`: counted once, at its `-`.
                '>' if after_minus => {}
                '>' => {
                    frame.angles = frame.angles.saturating_sub(1);
                    frame.segment += 1;
                }
                '|' => {
                    // `||` is an empty parameter list (or `or`); a lone `|` opens or closes one.
                    if p.spacing() == Spacing::Alone {
                        frame.closure_params = !frame.closure_params;
                    }
                    frame.segment += 1;
                }
                '-' => {
                    frame.after_minus = p.spacing() == Spacing::Joint;
                    frame.segment += 1;
                }
                _ => frame.segment += 1,
            },
            TokenTree::Ident(i) if ITEM_KEYWORDS.iter().any(|kw| i == *kw) => {
                frame.segment = 0;
                frame.angles = 0;
                frame.closure_params = false;
            }
            TokenTree::Ident(_) | TokenTree::Literal(_) => frame.segment += 1,
        }
        if frame.depth() > MAX_NESTING {
            return Err(too_deep(tree.span()));
        }
    }
    Ok(())
}

fn too_deep(span: proc_macro2::Span) -> CodegenError {
    error_at(span, format!("spec nests deeper than {MAX_NESTING} levels"))
}
