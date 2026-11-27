//! Field-granular branchless inheritance for `#[nve_table(inherited)]` tables.
//!
//! An inherited table reserves hidden word `0` as `__explicit: u32` (`word_effects[0] == 0`),
//! with bit `i` recording whether field `i` was explicitly written on that row. Resolution blends
//! each field's words/bits from `own` over `parent` without data-dependent branches.

use nve_abi::table::Word;

use crate::id::NodeId;
use crate::row::RowRef;
use crate::span_pool::{StrArena, StrSpan};
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
/// - `inherit_only` preserves inherited fields from `parent`, resets `#[no_inherit]` fields to
///   `Self::DESC.defaults`, and masks word `0` with `Self::INHERITED_FIELD_MASK` without
///   introducing invalid string/resource words.
/// - `resolve_inherited_arrays` blends every field's words/bitfields according to `own[0]`
///   without out-of-bounds access or invalid string/resource words.
pub unsafe trait InheritedTable: NveTable {
    /// Stack-allocated fixed word array (`[Word; WORD_COUNT]`) for zero-allocation resolution.
    type Words: Copy + AsRef<[Word]> + AsMut<[Word]>;

    /// Bitmask of fields (bits `0..fields.len()`) participating in inheritance (`0` for
    /// `#[no_inherit]` fields).
    const INHERITED_FIELD_MASK: u32;

    /// Default resolved row words (word `0` = `0`, remaining words = `Self::DESC.defaults`).
    fn default_words() -> Self::Words;

    /// Projects `parent` to an unattached child row: preserves inherited fields and resets every
    /// `#[no_inherit]` field to its table default (identity `*parent` when no `#[no_inherit]`
    /// fields exist).
    fn inherit_only(parent: &Self::Words) -> Self::Words;

    /// Branchlessly blends `own` words over `parent` words using `own[0]` as the per-field
    /// explicit bitmask (`out[0] = own[0] | (parent[0] & Self::INHERITED_FIELD_MASK)`).
    /// `#[no_inherit]` fields always take `own` rather than `parent`.
    fn resolve_inherited_arrays(own: &Self::Words, parent: &Self::Words) -> Self::Words;

    /// Resolves a borrowed row slice `own` over `parent` (`None` if `own.len() < Self::DESC.words()`).
    fn resolve_inherited_words(own: &[Word], parent: &Self::Words) -> Option<Self::Words>;
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
    ///
    /// # Safety
    ///
    /// Every two-word string slot flagged in `T::DESC.str_words` inside `words` must hold a valid
    /// UTF-8 `(offset, len)` span inside `store.text_arena()` (for example, words produced by
    /// `ResolvedRow::{root, resolve, resolve_ref}` on the same unmodified `store` or
    /// `T::default_words()`).
    #[inline(always)]
    pub unsafe fn from_words(words: T::Words, store: &'a NodeStore) -> Self {
        Self {
            words,
            text: store.text_arena(),
        }
    }

    /// Safe validated counterpart to [`Self::from_words`]: verifies every string span flagged in
    /// `T::DESC.str_words` against `store.text_arena()` (compiles away when `str_words == 0`).
    #[inline]
    pub fn try_from_words(words: T::Words, store: &'a NodeStore) -> Option<Self> {
        let mut sm = T::DESC.str_words;
        let slice = words.as_ref();
        let arena = store.text_arena();
        while sm != 0 {
            let w = sm.trailing_zeros() as usize;
            sm &= sm - 1;
            let off = *slice.get(w)?;
            let len = *slice.get(w + 1)?;
            arena.try_str(StrSpan::new(off, len))?;
        }
        Some(Self { words, text: arena })
    }

    /// Extracts the underlying `'static + Copy` resolved word array.
    #[inline(always)]
    pub fn into_words(self) -> T::Words {
        self.words
    }

    /// Resolves `T` on `id` over `parent` in `O(words)` with zero heap allocations and zero
    /// per-field branches. Panics if `parent` belongs to a different [`NodeStore`].
    #[inline]
    pub fn resolve(store: &'a NodeStore, id: NodeId, parent: &Self) -> Self {
        assert!(
            core::ptr::eq(parent.text, store.text_arena()),
            "ResolvedRow::resolve: parent row belongs to a different NodeStore"
        );
        match store.words(id, T::slot()) {
            Some(own) => Self {
                words: T::resolve_inherited_words(own, &parent.words)
                    .unwrap_or_else(|| T::inherit_only(&parent.words)),
                text: store.text_arena(),
            },
            None => Self {
                words: T::inherit_only(&parent.words),
                text: parent.text,
            },
        }
    }

    /// Resolves `T` on an already-validated [`NodeRef`] without repeating liveness checks.
    /// Panics if `parent` belongs to a different [`NodeStore`].
    #[inline]
    pub fn resolve_ref(node: NodeRef<'a>, parent: &Self) -> Self {
        assert!(
            core::ptr::eq(parent.text, node.text_arena()),
            "ResolvedRow::resolve_ref: parent row belongs to a different NodeStore"
        );
        match node.words(T::slot()) {
            Some(own) => Self {
                words: T::resolve_inherited_words(own, &parent.words)
                    .unwrap_or_else(|| T::inherit_only(&parent.words)),
                text: node.text_arena(),
            },
            None => Self {
                words: T::inherit_only(&parent.words),
                text: parent.text,
            },
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
    /// overrides propagated to children without mutating the store) and marks its explicit bit in
    /// word `0`.
    #[inline(always)]
    pub fn override_prop<V: WordValue>(&mut self, prop: Prop<T, V>, value: V) {
        let words = self.words.as_mut();
        let w = &mut words[prop.word as usize];
        let bits = value.to_masked_bits(prop.mask.count_ones());
        *w = (*w & !prop.mask) | ((bits << prop.shift) & prop.mask);
        if prop.explicit_bit < 32 {
            words[0] |= 1u32 << prop.explicit_bit;
        }
    }
}
