//! Field-granular branchless inheritance for `#[nve_table(inherited)]` tables.
//!
//! An inherited table reserves hidden word `0` as `__explicit: u32` (`word_effects[0] == 0`),
//! with bit `i` recording whether field `i` was explicitly written on that row. Resolution blends
//! each field's words/bits from `own` over `parent` without data-dependent branches.

use nve_abi::table::Word;

use crate::id::NodeId;
use crate::row::RowRef;
use crate::span_pool::StrArena;
use crate::store::{NodeRef, NodeStore};
use crate::table::{NveTable, Prop};
use crate::value::WordValue;

/// Marker and branchless word-resolution contract generated for `#[nve_table(inherited)]` tables.
///
/// # Safety
///
/// Implementors (exclusively `#[derive(NveTable)]` with `#[nve_table(inherited)]`) must guarantee:
/// - `Self::DESC.inherited` is `true` and word `0` is the `__explicit: u32` mask.
/// - `Self::Words` has length equal to `Self::DESC.words()`.
/// - `default_words()` matches `Self::DESC.defaults` with word `0 == 0`.
/// - `resolve_inherited_arrays` blends every field's words/bitfields according to `own[0]`
///   without out-of-bounds access or invalid string/resource words.
pub unsafe trait InheritedTable: NveTable {
    /// Stack-allocated fixed word array (`[Word; WORD_COUNT]`) for zero-allocation resolution.
    type Words: Copy + AsRef<[Word]> + AsMut<[Word]>;

    /// Default resolved row words (word `0` = `0`, remaining words = `Self::DESC.defaults`).
    fn default_words() -> Self::Words;

    /// Branchlessly blends `own` words over `parent` words into `out` using `own[0]` as the
    /// per-field explicit bitmask (`out[0] = own[0] | parent[0]`).
    fn resolve_inherited_arrays(own: &Self::Words, parent: &Self::Words, out: &mut Self::Words);

    /// Resolves a borrowed row slice `own` over `parent` into `out` (returns `false` without
    /// mutating `out` if `own.len() < Self::DESC.words()`).
    fn resolve_inherited_words(own: &[Word], parent: &Self::Words, out: &mut Self::Words) -> bool;
}

/// Zero-allocation stack-resident resolved snapshot of an [`InheritedTable`] row tied to a
/// borrowing [`NodeStore`]'s string arena.
///
/// `Copy` (no heap allocation, fits in a few registers / one cache line for typical tables like
/// `Typography`), enabling direct pass-by-value down recursive or iterative tree walks:
///
/// ```
/// use nve_store::{InheritedTable, NodeId, NodeStore, NveTable, ResolvedRow, Rgba};
///
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.inherited.basic", storage = "sparse", inherited)]
/// struct ThemeText {
///     #[effect(paint)]
///     #[default(Rgba::BLACK)]
///     color: Rgba,
///     #[effect(measure)]
///     #[default(16.0)]
///     size: f32,
/// }
///
/// let mut store = NodeStore::new();
/// let (parent, child) = {
///     let mut tx = store.tx();
///     let p = tx.create();
///     let c = tx.create();
///     tx.set_children(NodeId::ROOT, &[p]);
///     tx.set_children(p, &[c]);
///     tx.edit::<ThemeText>(p).color(Rgba::WHITE).size(24.0);
///     tx.edit::<ThemeText>(c).color(Rgba::BLACK); // explicit table default overrides parent!
///     (p, c)
/// };
///
/// let root_row = ResolvedRow::<ThemeText>::root(&store);
/// let p_row = ResolvedRow::<ThemeText>::resolve(&store, parent, &root_row);
/// let c_row = ResolvedRow::<ThemeText>::resolve(&store, child, &p_row);
/// assert_eq!(p_row.get().color(), Rgba::WHITE);
/// assert_eq!(c_row.get().color(), Rgba::BLACK);
/// assert_eq!(c_row.get().size(), 24.0);
/// ```
///
/// Exact compile-failure diagnostics for magic-sentinel defaults, split-bool pairs, and
/// `inherited` + `events` conflicts are snapshot-tested via `trybuild` in `tests/compile_fail_ui.rs`.
pub struct ResolvedRow<'a, T: InheritedTable> {
    words: T::Words,
    text: &'a StrArena,
}

impl<T: InheritedTable> Clone for ResolvedRow<'_, T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: InheritedTable> Copy for ResolvedRow<'_, T> {}

impl<'a, T: InheritedTable> ResolvedRow<'a, T> {
    /// Root resolved row initialized to `T`'s table defaults.
    #[inline(always)]
    pub fn root(store: &'a NodeStore) -> Self {
        Self {
            words: T::default_words(),
            text: store.text_arena(),
        }
    }

    /// Wraps an already-resolved stack word array (e.g. cached in a layout pass) with `store`'s
    /// text arena so typed getters (`get()`) including string fields work zero-copy.
    #[inline(always)]
    pub fn from_words(words: T::Words, store: &'a NodeStore) -> Self {
        Self {
            words,
            text: store.text_arena(),
        }
    }

    /// Extracts the underlying `'static + Copy` resolved word array.
    #[inline(always)]
    pub fn into_words(self) -> T::Words {
        self.words
    }

    /// Resolves `T` on `id` over `parent` in `O(words)` with zero heap allocations and zero
    /// per-field branches. If `id` does not have `T` attached, inherits `*parent` unchanged.
    #[inline]
    pub fn resolve(store: &'a NodeStore, id: NodeId, parent: &Self) -> Self {
        match store.words(id, T::slot()) {
            Some(own) => {
                let mut words = parent.words;
                T::resolve_inherited_words(own, &parent.words, &mut words);
                Self {
                    words,
                    text: store.text_arena(),
                }
            }
            None => *parent,
        }
    }

    /// Resolves `T` on an already-validated [`NodeRef`] without repeating liveness checks.
    #[inline]
    pub fn resolve_ref(node: NodeRef<'a>, parent: &Self) -> Self {
        match node.words(T::slot()) {
            Some(own) => {
                let mut words = parent.words;
                T::resolve_inherited_words(own, &parent.words, &mut words);
                Self {
                    words,
                    text: node.text_arena(),
                }
            }
            None => *parent,
        }
    }

    /// Zero-copy typed reader over the resolved words.
    #[inline(always)]
    pub fn get(&self) -> T::Ref<'_> {
        T::from_row_ref(RowRef::new(self.words.as_ref(), self.text, T::DESC.str_words))
    }

    /// Raw resolved words (word `0` holds the cumulative explicit mask).
    #[inline(always)]
    pub fn words(&self) -> &[Word] {
        self.words.as_ref()
    }

    /// Overrides one packed scalar field in this resolved snapshot (for transient state/motion
    /// overrides propagated to children without mutating the store).
    #[inline(always)]
    pub fn override_prop<V: WordValue>(&mut self, prop: Prop<T, V>, value: V) {
        let w = &mut self.words.as_mut()[prop.word as usize];
        let bits = value.to_masked_bits(prop.mask.count_ones());
        *w = (*w & !prop.mask) | ((bits << prop.shift) & prop.mask);
    }
}
