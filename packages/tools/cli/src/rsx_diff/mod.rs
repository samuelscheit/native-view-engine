//! Zero-dependency Rust lexer and span-based typed `rsx! { ... }` differ for Tier-1 (`< 10ms`)
//! hot-patching.
//!
//! Compares the cached baseline `.rs` source against the newly saved source:
//! - Ignores comments and whitespace (comment/whitespace-only edits return [`DiffOutcome::NoOp`]).
//! - Indexes every patchable span of the baseline: static text children and native-element style
//!   attribute values ([`attrs`]), with template coordinates.
//! - Requires every token outside those spans to match token for token; inside them only literal
//!   leaves may change, evaluated by the const evaluator ([`eval`]) into typed values.
//! - Emits [`DiffOutcome::Tier1Patch`] with the [`TypedRsxPatch`] and its faithful lowering to
//!   `HotPatchMsg::TemplatePatch` messages (one per text node, one per changed element).
//! - Falls back to [`DiffOutcome::RequireTier2Rebuild`] for any Rust logic, control flow, event
//!   handler, component prop, interpolated, non-constant or shape-changing value, identity
//!   (`key`/`test_id`), structural or syntax change, and for edits the string patch path cannot
//!   carry faithfully.

// Part 2 (`RowPatch` lowering) consumes `patch::TypedRsxPatch`, `patch::TemplateTarget`,
// `eval::TypedAttrValue` and `attrs::AttrSpec` through these crate-visible modules.
pub(crate) mod attrs;
pub(crate) mod cache;
pub(crate) mod eval;
mod lexer;
mod matcher;
mod parser;
pub(crate) mod patch;
mod strings;

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;

use nve_engine::HotPatchMsg;

pub use cache::RsxWatchCache;
use patch::TypedRsxPatch;

/// Classification of a watched `.rs` file save for 2-tier hot reload.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffOutcome {
    /// Comment-only, whitespace-only, or semantically identical save (no patch or rebuild needed).
    NoOp,
    /// Literal text/style value edits inside `rsx! { ... }`: the typed patch plus its
    /// `TemplatePatch` lowering, broadcast via the Tier-1 WebSocket.
    Tier1Patch {
        patch: TypedRsxPatch,
        messages: Vec<HotPatchMsg>,
    },
    /// Any logic, structural, interpolation, non-literal value or syntax change: Tier-2 build.
    RequireTier2Rebuild { reason: String },
}
