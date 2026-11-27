//! Runs a spec's `syn` parse on a worker thread whose stack is sized from a sound bound on `syn`'s
//! recursion, so no input (valid or adversarial) can overflow a stack.
//!
//! `syn` parses, spans, and drops its AST recursively, and each recursive cycle consumes at least
//! one token (otherwise it would never terminate). At any token, the active recursion can only
//! have consumed the earlier tokens of the current bracket group and, in each enclosing group, the
//! tokens up to and including that group. Their sum, maximized over every token, is the *path
//! bound* this module computes iteratively. The parse thread then gets [`STACK_PER_TOKEN`] bytes
//! of stack per bounded token: twice the worst debug-build cost measured over every recursive
//! expression, type, pattern, statement, item, and attribute form (`&&...&u8` and `((u8))`:
//! 32 KB per token). No grammar knowledge is involved, so no grammar trick can escape the bound.
//!
//! Both passes run on short-lived threads, so the caller's `proc_macro2` span table is never
//! touched: each thread's span table is freed when the thread exits.

use std::thread::{Builder, Scope};

use proc_macro2::{TokenStream, TokenTree};

use super::types::error_at;
use crate::error::CodegenError;

/// Largest path bound accepted from a spec: a [`STACK_PER_TOKEN`] stack of 1 GiB at most. A
/// realistic spec stays in the hundreds (`tests/fixtures/realistic_hybrid.nve.rs`: 155).
pub(super) const MAX_PATH_TOKENS: usize = 16 * 1024;

/// Parse-thread stack per bounded token (twice the measured worst case, see the module docs).
const STACK_PER_TOKEN: usize = 64 * 1024;

/// Stack for everything besides `syn`'s token-driven recursion: lowering, type conversion
/// (bounded by its own depth limit), validation, and error formatting.
const BASE_STACK: usize = 4 * 1024 * 1024;

/// The bound pass is iterative; its stack only has to hold `proc_macro2`'s lexer.
const BOUND_STACK: usize = 2 * 1024 * 1024;

/// Lexes `body`, mapping a lex error to its location.
pub(super) fn lex(body: &str) -> Result<TokenStream, CodegenError> {
    body.parse().map_err(|e: proc_macro2::LexError| {
        error_at(
            e.span(),
            format!("{e}: unbalanced delimiter or unterminated string / comment"),
        )
    })
}

/// Runs `parse(body)` on a thread with enough stack for `syn` to parse, lower, and drop `body`,
/// after rejecting a `body` whose path bound exceeds [`MAX_PATH_TOKENS`].
pub(super) fn on_sized_stack<T: Send>(
    body: &str,
    parse: impl FnOnce(&str) -> Result<T, CodegenError> + Send,
) -> Result<T, CodegenError> {
    std::thread::scope(|scope| {
        let bound = run(scope, BOUND_STACK, || path_bound(body))??;
        run(scope, BASE_STACK + bound * STACK_PER_TOKEN, || parse(body))?
    })
}

fn run<'scope, T: Send + 'scope>(
    scope: &'scope Scope<'scope, '_>,
    stack: usize,
    f: impl FnOnce() -> T + Send + 'scope,
) -> Result<T, CodegenError> {
    let thread = Builder::new()
        .name("nve-hybrid-spec".into())
        .stack_size(stack)
        .spawn_scoped(scope, f)
        .map_err(|e| {
            CodegenError::validation(format!(
                "cannot start the spec parser thread with a {} MiB stack: {e}",
                stack >> 20
            ))
        })?;
    // A panic in the parser is a bug: re-raise it on the caller's thread unchanged.
    Ok(thread
        .join()
        .unwrap_or_else(|payload| std::panic::resume_unwind(payload)))
}

/// The path bound of `body` (see the module docs), or a located error at the first token past
/// [`MAX_PATH_TOKENS`].
pub(super) fn path_bound(body: &str) -> Result<usize, CodegenError> {
    // (remaining tokens of a group, tokens consumed on the path up to and including this group)
    let mut stack = vec![(lex(body)?.into_iter(), 0)];
    let mut max = 0;
    while let Some((tokens, consumed)) = stack.last_mut() {
        let Some(tree) = tokens.next() else {
            stack.pop();
            continue;
        };
        *consumed += 1;
        let path = *consumed;
        if path > MAX_PATH_TOKENS {
            return Err(error_at(
                tree.span(),
                format!(
                    "nesting too deep: the parser could recurse through more than {MAX_PATH_TOKENS} tokens here; split this item or expression"
                ),
            ));
        }
        max = max.max(path);
        if let TokenTree::Group(g) = tree {
            stack.push((g.stream().into_iter(), path));
        }
    }
    Ok(max)
}

#[cfg(test)]
mod tests {
    use super::{MAX_PATH_TOKENS, path_bound};

    #[test]
    fn the_path_bound_counts_tokens_along_the_deepest_group_path() -> Result<(), String> {
        let bound = |s: &str| path_bound(s).map_err(|e| e.to_string());
        assert_eq!(bound("")?, 0);
        assert_eq!(bound("a b c")?, 3);
        // `a (b c) d`: at `c`, `a` and the group were consumed outside, `b c` inside.
        assert_eq!(bound("a (b c) d")?, 4);
        // Tokens after a closed group do not stack on its contents.
        assert_eq!(bound("(x x x x) y")?, 5);
        assert_eq!(bound("((((x))))")?, 5);
        // The cap is exact: `MAX_PATH_TOKENS` tokens are accepted, one more is a located error.
        let at_cap = "x ".repeat(MAX_PATH_TOKENS);
        assert_eq!(bound(&at_cap)?, MAX_PATH_TOKENS);
        let over = format!("{at_cap}\ny");
        let message = bound(&over).err().unwrap_or_default();
        assert!(
            message.contains("line 2") || message.contains("2:1"),
            "{message}"
        );
        assert!(message.contains("nesting too deep"), "{message}");
        Ok(())
    }
}
