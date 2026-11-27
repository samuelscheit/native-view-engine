//! Iterative nesting pre-check that runs before `syn` sees a spec.
//!
//! `syn` parses types and expressions recursively, so deeply nested input overflows the stack
//! before any frontend check can run: `*const` repeated 100k times, `Option<Option<...>>`,
//! `((((...))))`, `- - - 1`, `fn() -> fn() -> ...`, `|| || ...`, `return return ...`. This walk
//! uses an explicit stack and rejects input whose nesting exceeds [`MAX_NESTING`], with the
//! location of the token that crossed the limit.
//!
//! Nesting at a token = enclosing bracket groups + open `<` + the *chain* of recursion-inducing
//! tokens since the last separator (`,` `;` `=`) in the same group: prefix operators
//! (`*` `&` `-` `!` `|`), `->`, and keywords that prefix another type or expression.

use proc_macro2::{Spacing, TokenStream, TokenTree};

use super::types::error_at;
use crate::error::CodegenError;

/// Maximum nesting accepted from a spec (the same bound as type conversion).
pub(super) const MAX_NESTING: usize = 128;

/// Keywords after which `syn` recurses into another type or expression.
const CHAIN_KEYWORDS: &[&str] = &[
    "return", "break", "yield", "move", "async", "await", "box", "dyn", "impl", "static",
];

/// Nesting state of one bracket group being walked.
struct Frame {
    tokens: proc_macro2::token_stream::IntoIter,
    /// Nesting of the enclosing groups.
    base: usize,
    /// Open `<` not yet closed by a `>` in this group.
    angles: usize,
    /// Recursion-inducing tokens since the last separator in this group.
    chain: usize,
    /// The previous token was a `-` joined to the next punct (the start of `->`).
    after_minus: bool,
}

impl Frame {
    fn new(tokens: TokenStream, base: usize) -> Self {
        Self {
            tokens: tokens.into_iter(),
            base,
            angles: 0,
            chain: 0,
            after_minus: false,
        }
    }

    fn depth(&self) -> usize {
        self.base + self.angles + self.chain
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
        match &tree {
            TokenTree::Group(g) => {
                let inner = frame.depth() + 1;
                if inner > MAX_NESTING {
                    return Err(too_deep(g.span_open()));
                }
                stack.push(Frame::new(g.stream(), inner));
                continue;
            }
            TokenTree::Punct(p) => match p.as_char() {
                ',' | ';' | '=' => frame.chain = 0,
                '<' => frame.angles += 1,
                // `->` counts once, at its `-`.
                '>' if after_minus => {}
                '>' => frame.angles = frame.angles.saturating_sub(1),
                '*' | '&' | '!' | '|' => frame.chain += 1,
                '-' => {
                    frame.chain += 1;
                    frame.after_minus = p.spacing() == Spacing::Joint;
                }
                _ => {}
            },
            TokenTree::Ident(i) => {
                if CHAIN_KEYWORDS.contains(&i.to_string().as_str()) {
                    frame.chain += 1;
                }
            }
            TokenTree::Literal(_) => {}
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
