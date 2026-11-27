//! Runs a spec's `syn` parse on a worker thread whose fixed stack provably covers `syn`'s
//! recursion for every input the *path bound* accepts, so no input (valid or adversarial) can
//! overflow a stack.
//!
//! `syn` parses, spans, and drops its AST recursively, and each recursive cycle consumes at least
//! one token (otherwise it would never terminate). At any token, the active recursion can only
//! hold tokens consumed earlier in the current bracket group, plus each enclosing group's tokens up
//! to that group. Their sum, maximized over every token, bounds the recursion depth. A count may
//! restart at a token after which `syn` provably holds no earlier token of the group on its stack:
//!
//! - `;` in any group: it ends a statement, an item, or an array type / repeat's element, all
//!   parsed in loops.
//! - `,` in a group with no open generic `<` (`->` / `=>` do not close one) and no `|` since the
//!   last restart (closure parameter lists also contain commas): list elements are parsed in loops.
//! - At file level, after an item's `{...}` body or an attribute `#[...]`, when the item has no
//!   `=`, `|`, or open `<` so far. Without `=` there is no expression at file level, so nothing
//!   recursive spans that group.
//!
//! The thread's stack is [`STACK_PER_TOKEN`] bytes per token of [`MAX_PATH_TOKENS`]: twice the worst
//! cost measured over every recursive expression, type, pattern, statement, item, and attribute
//! form with an unoptimized `syn` (`&&...&u8` and `((u8))`: 32 KB per token), the largest any
//! build of a dependent crate can use. (This workspace builds dependencies at `opt-level = 2`:
//! 5.2 KB per token at worst, `{{...}}`.) Lexing, the bound, the parse, and the drop all run on that
//! thread, so the caller's `proc_macro2` span table is never touched.

use std::thread::Builder;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

use super::types::error_at;
use crate::error::CodegenError;

/// Largest path bound accepted from a spec. A realistic spec stays far below it
/// (`tests/fixtures/realistic_hybrid.nve.rs`: 21).
pub(super) const MAX_PATH_TOKENS: usize = 2 * 1024;

/// Parse-thread stack per bounded token (twice the measured worst case, see the module docs).
const STACK_PER_TOKEN: usize = 64 * 1024;

/// Stack for everything besides `syn`'s token-driven recursion: lexing, lowering, type conversion
/// (bounded by its own depth limit), validation, and error formatting.
const BASE_STACK: usize = 4 * 1024 * 1024;

/// The parse thread's stack: 132 MiB of address space, of which a realistic spec touches a few.
const PARSE_STACK: usize = BASE_STACK + MAX_PATH_TOKENS * STACK_PER_TOKEN;

/// Lexes `body` on a parse thread, rejects it when its path bound exceeds [`MAX_PATH_TOKENS`],
/// and runs `parse` on the tokens there.
pub(super) fn parse_on_stack<T: Send>(
    body: &str,
    parse: impl FnOnce(TokenStream) -> Result<T, CodegenError> + Send,
) -> Result<T, CodegenError> {
    std::thread::scope(|scope| {
        let thread = Builder::new()
            .name("nve-hybrid-spec".into())
            .stack_size(PARSE_STACK)
            .spawn_scoped(scope, || {
                let tokens = lex(body)?;
                path_bound(tokens.clone())?;
                parse(tokens)
            })
            .map_err(|e| {
                CodegenError::validation(format!(
                    "cannot start the spec parser thread with a {} MiB stack: {e}",
                    PARSE_STACK >> 20
                ))
            })?;
        // A panic in the parser is a bug: re-raise it on the caller's thread unchanged.
        thread
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    })
}

fn lex(body: &str) -> Result<TokenStream, CodegenError> {
    body.parse().map_err(|e: proc_macro2::LexError| {
        error_at(
            e.span(),
            format!("{e}: unbalanced delimiter or unterminated string / comment"),
        )
    })
}

/// Walk state of one bracket group.
struct Frame {
    tokens: proc_macro2::token_stream::IntoIter,
    /// Path count at the group's own token (what a restart returns to).
    base: usize,
    /// Path count at the latest token of this group.
    consumed: usize,
    /// The file itself (item boundaries restart the count, see the module docs).
    root: bool,
    /// Generic `<` not yet closed by a `>` since the last restart.
    angles: usize,
    /// A `|` or `=` since the last restart.
    pipe: bool,
    assign: bool,
    /// The previous token was a joint `-` / `=` (so a `>` is `->` / `=>`) or a `#` / `#!`.
    arrow_lead: bool,
    attribute_lead: bool,
    /// Restart the parent's count when this group ends (a file-level item body or attribute).
    restarts_parent: bool,
}

impl Frame {
    fn new(tokens: TokenStream, base: usize, root: bool, restarts_parent: bool) -> Self {
        Self {
            tokens: tokens.into_iter(),
            base,
            consumed: base,
            root,
            angles: 0,
            pipe: false,
            assign: false,
            arrow_lead: false,
            attribute_lead: false,
            restarts_parent,
        }
    }

    fn restart(&mut self) {
        self.consumed = self.base;
        self.angles = 0;
        self.pipe = false;
        self.assign = false;
    }
}

/// The path bound of `tokens` (see the module docs), or a located error at the first token past
/// [`MAX_PATH_TOKENS`].
pub(super) fn path_bound(tokens: TokenStream) -> Result<usize, CodegenError> {
    let mut stack = vec![Frame::new(tokens, 0, true, false)];
    let mut max = 0;
    while let Some(frame) = stack.last_mut() {
        let Some(tree) = frame.tokens.next() else {
            if let Some(done) = stack.pop()
                && done.restarts_parent
                && let Some(parent) = stack.last_mut()
            {
                parent.restart();
            }
            continue;
        };
        frame.consumed += 1;
        let path = frame.consumed;
        if path > MAX_PATH_TOKENS {
            return Err(error_at(
                tree.span(),
                format!(
                    "nesting too deep: the parser could recurse through more than {MAX_PATH_TOKENS} tokens here; split this item or expression"
                ),
            ));
        }
        max = max.max(path);
        let (arrow_lead, attribute_lead) = (frame.arrow_lead, frame.attribute_lead);
        frame.arrow_lead = false;
        frame.attribute_lead = false;
        match tree {
            TokenTree::Group(g) => {
                let item_boundary = frame.root
                    && frame.angles == 0
                    && !frame.pipe
                    && !frame.assign
                    && match g.delimiter() {
                        Delimiter::Brace => true,
                        Delimiter::Bracket => attribute_lead,
                        Delimiter::Parenthesis | Delimiter::None => false,
                    };
                // The group's own stream is unshared once `g` is gone, so iterating it moves the
                // tokens instead of cloning them.
                let inner = g.stream();
                drop(g);
                stack.push(Frame::new(inner, path, false, item_boundary));
            }
            TokenTree::Punct(p) => match p.as_char() {
                ';' => frame.restart(),
                ',' if frame.angles == 0 && !frame.pipe => frame.consumed = frame.base,
                '<' => frame.angles += 1,
                '>' if !arrow_lead => frame.angles = frame.angles.saturating_sub(1),
                '|' => frame.pipe = true,
                '#' => frame.attribute_lead = true,
                '!' if attribute_lead => frame.attribute_lead = true,
                c => {
                    frame.assign |= c == '=';
                    frame.arrow_lead = p.spacing() == Spacing::Joint && matches!(c, '-' | '=');
                }
            },
            TokenTree::Ident(_) | TokenTree::Literal(_) => {}
        }
    }
    Ok(max)
}

#[cfg(test)]
mod tests {
    use super::{MAX_PATH_TOKENS, path_bound};

    fn bound(s: &str) -> Result<usize, String> {
        let tokens = s
            .parse()
            .map_err(|e: proc_macro2::LexError| e.to_string())?;
        path_bound(tokens).map_err(|e| e.to_string())
    }

    #[test]
    fn the_path_bound_counts_tokens_along_the_deepest_group_path() -> Result<(), String> {
        assert_eq!(bound("")?, 0);
        assert_eq!(bound("a b c")?, 3);
        // `a (b c) d`: at `c`, `a` and the group were consumed outside, `b c` inside.
        assert_eq!(bound("a (b c) d")?, 4);
        assert_eq!(bound("((((x))))")?, 5);
        // The cap is exact: `MAX_PATH_TOKENS` tokens are accepted, one more is a located error.
        let at_cap = "x ".repeat(MAX_PATH_TOKENS);
        assert_eq!(bound(&at_cap)?, MAX_PATH_TOKENS);
        let message = bound(&format!("{at_cap}\ny")).err().unwrap_or_default();
        assert!(
            message.contains("at 2:1") && message.contains("nesting too deep"),
            "{message}"
        );
        Ok(())
    }

    #[test]
    fn a_realistic_spec_stays_far_below_the_cap() -> Result<(), String> {
        let fixture = include_str!("../../../tests/fixtures/realistic_hybrid.nve.rs");
        let b = bound(fixture)?;
        eprintln!("realistic fixture path bound: {b}");
        assert!(b < MAX_PATH_TOKENS / 16, "{b}");
        Ok(())
    }

    #[test]
    fn counts_restart_only_where_syn_holds_no_earlier_token() -> Result<(), String> {
        // `;` everywhere; `,` outside generics and closures.
        assert_eq!(bound("a b; c")?, 3);
        assert_eq!(bound("(a b c, d)")?, 5);
        assert_eq!(bound("{ - - - x; y }")?, 6);
        assert_eq!(bound("[& & & u8; 4]")?, 6);
        // Inside `<...>` and after `|`, a comma is not a list boundary.
        assert_eq!(bound("(f::<a, b> = c)")?, 11);
        assert_eq!(bound("(|x, y| a, b)")?, 9);
        assert_eq!(bound("(Fn() -> a, b)")?, 7);
        // `->` does not close a generic `<`, so the comma after it is still inside it.
        assert_eq!(bound("(F<fn() -> u8, b>)")?, 11);
        // File-level items restart after their body / attributes, unless an expression is open.
        assert_eq!(bound("struct A { x: u8 } struct B { y: u8 }")?, 6);
        assert_eq!(bound("#[doc = \"a\"] #[doc = \"b\"] struct A;")?, 5);
        assert_eq!(bound("const X: u8 = A {} + A {} + A {};")?, 14);
        assert_eq!(bound("impl F<{ 1 }> for A {} x")?, 8);
        Ok(())
    }
}
