//! Table-generic, 4-byte-aligned sparse word-patch format ([`RowPatch`] / [`RowPatchView`]).
//!
//! Replaces bespoke hand-maintained patch structs (`StylePatch`) with a single uniform word-level
//! override representation that works for **every** `#[derive(NveTable)]` table (built-in or 3P
//! module), preserves packed bitfield siblings via `(clear_mask, set_bits)`, sets inherited
//! word-0 explicit bits automatically, and tracks embedded `ResourceId` overrides as refcounted
//! child dependencies in [`crate::ResourceStore`].
//!
//! ## Flat Word Layout (`[u32]`, little-endian)
//!
//! ```text
//! words[0] = table_count (0..=MAX_PATCH_TABLES)
//! For each table (strictly ascending TableKey order):
//!     table_key_lo: u32
//!     table_key_hi: u32
//!     op_count: u32 (1..=MAX_TABLE_WORDS)
//!     For each op (strictly ascending word_idx < 32):
//!         word_and_flags: u32  // bits 0..16 = word_idx, bits 16..32 = flags (0=scalar, 1=resource)
//!         clear_mask: u32      // non-zero; u32::MAX when flags == WORD_OP_RESOURCE
//!         set_bits: u32        // invariant: (set_bits & !clear_mask) == 0
//! ```

use nve_abi::table::{TableKey, Word, MAX_TABLE_WORDS};

use crate::id::ResourceId;
use crate::resource::FlatResource;
use crate::table::{NveTable, Prop, ResProp, ScalarField};

#[path = "row_patch_view.rs"]
mod view;
pub(crate) use view::effective_inherited_explicit_clear;
pub use view::{prop_written, RowPatchView, TableOpsIter, TableOpsView, WordOpsIter};
use view::apply_ops_checked;

/// Maximum distinct tables allowed in a single [`RowPatch`].
pub const MAX_PATCH_TABLES: usize = 32;

/// Non-zero bitmask of modified bits inside a single `NveTable` word.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WordMask(pub u32);

impl WordMask {
    /// Mask selecting all 32 bits of a word.
    pub const ALL: Self = Self(u32::MAX);

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn apply(self, current: Word, set_bits: Word) -> Word {
        (current & !self.0) | (set_bits & self.0)
    }
}

/// Discriminator distinguishing scalar/bitfield word updates from [`ResourceId`] word updates.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordOpKind {
    Scalar = 0,
    Resource = 1,
}

/// Scalar / packed-bitfield word op wire tag.
pub const WORD_OP_SCALAR: u16 = WordOpKind::Scalar as u16;
/// `ResourceId` word op wire tag (`clear_mask == WordMask::ALL`; `set_bits` is a `ResourceId`).
pub const WORD_OP_RESOURCE: u16 = WordOpKind::Resource as u16;

/// One validated masked word update targeting a single word inside an `NveTable` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordOp {
    word: u8,
    kind: WordOpKind,
    clear_mask: WordMask,
    set_bits: Word,
}

impl WordOp {
    /// Constructs a validated scalar/bitfield word op, returning `None` if `word >= MAX_TABLE_WORDS`,
    /// `clear_mask == 0`, or `(set_bits & !clear_mask) != 0`.
    #[inline(always)]
    pub const fn scalar(word: u8, clear_mask: WordMask, set_bits: Word) -> Option<Self> {
        if (word as usize) >= MAX_TABLE_WORDS
            || clear_mask.0 == 0
            || (set_bits & !clear_mask.0) != 0
        {
            return None;
        }
        Some(Self {
            word,
            kind: WordOpKind::Scalar,
            clear_mask,
            set_bits,
        })
    }

    /// Constructs a validated [`ResourceId`] word op (`clear_mask == WordMask::ALL`).
    #[inline(always)]
    pub const fn resource(word: u8, id: ResourceId) -> Option<Self> {
        if (word as usize) >= MAX_TABLE_WORDS {
            return None;
        }
        Some(Self {
            word,
            kind: WordOpKind::Resource,
            clear_mask: WordMask::ALL,
            set_bits: id.0,
        })
    }

    #[inline(always)]
    pub(super) const fn from_validated_raw(
        word: u8,
        kind: WordOpKind,
        clear_mask: WordMask,
        set_bits: Word,
    ) -> Self {
        Self {
            word,
            kind,
            clear_mask,
            set_bits,
        }
    }

    #[inline(always)]
    pub const fn word(self) -> u8 {
        self.word
    }

    #[inline(always)]
    pub const fn kind(self) -> WordOpKind {
        self.kind
    }

    #[inline(always)]
    pub const fn clear_mask(self) -> WordMask {
        self.clear_mask
    }

    #[inline(always)]
    pub const fn set_bits(self) -> Word {
        self.set_bits
    }

    #[inline(always)]
    pub const fn is_resource(self) -> bool {
        matches!(self.kind, WordOpKind::Resource)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TablePatch {
    key: TableKey,
    ops: Vec<WordOp>,
}

/// Owned, ergonomic table-generic word patch builder that encodes directly into flat `u32` words
/// and implements [`FlatResource`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowPatch {
    tables: Vec<TablePatch>,
}

impl RowPatch {
    #[inline]
    pub const fn new() -> Self {
        Self { tables: Vec::new() }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    #[inline]
    pub fn table_count(&self) -> usize {
        self.tables.len()
    }

    #[inline]
    pub fn has_table<T: NveTable>(&self) -> bool {
        self.tables.binary_search_by_key(&T::DESC.key.0, |t| t.key.0).is_ok()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.tables.clear();
    }

    fn push_raw_op(
        &mut self,
        key: TableKey,
        word: u8,
        kind: WordOpKind,
        clear_mask: WordMask,
        set_bits: Word,
    ) {
        debug_assert!((word as usize) < MAX_TABLE_WORDS);
        debug_assert!(clear_mask.0 != 0);
        debug_assert_eq!(set_bits & !clear_mask.0, 0);

        let t_idx = match self.tables.binary_search_by_key(&key.0, |t| t.key.0) {
            Ok(i) => i,
            Err(i) => {
                // Allow unbounded in-memory table merges (e.g. combining multiple valid wire
                // `StateRules`); `write_words` enforces `<= MAX_PATCH_TABLES` fallibly on encode.
                self.tables.insert(
                    i,
                    TablePatch {
                        key,
                        ops: Vec::with_capacity(4),
                    },
                );
                i
            }
        };
        let ops = &mut self.tables[t_idx].ops;
        match ops.binary_search_by_key(&word, |o| o.word) {
            Ok(oi) => {
                let existing = &mut ops[oi];
                existing.kind = kind;
                existing.clear_mask = WordMask(existing.clear_mask.0 | clear_mask.0);
                existing.set_bits = clear_mask.apply(existing.set_bits, set_bits);
            }
            Err(oi) => {
                ops.insert(
                    oi,
                    WordOp::from_validated_raw(word, kind, clear_mask, set_bits),
                );
            }
        }
    }

    /// Records a typed scalar, narrow-optional, wide-optional, or 64-bit property override for
    /// table `T`, coalescing packed sibling fields cleanly and setting word-0's explicit bit on
    /// `#[nve_table(inherited)]` tables.
    #[inline]
    pub fn set<T: NveTable, V: ScalarField>(&mut self, prop: Prop<T, V>, value: V) -> &mut Self {
        let key = T::DESC.key;
        value.encode_patch_ops(
            prop.word as usize,
            prop.shift as u32,
            prop.mask,
            prop.val_word as usize,
            &mut |w, clear_mask, set_bits| {
                self.push_raw_op(
                    key,
                    w as u8,
                    WordOpKind::Scalar,
                    WordMask(clear_mask),
                    set_bits,
                );
            },
        );
        if prop.explicit_bit < 32 {
            let bit = 1u32 << prop.explicit_bit;
            self.push_raw_op(key, 0, WordOpKind::Scalar, WordMask(bit), bit);
        }
        self
    }

    /// Fluent by-value counterpart to [`Self::set`].
    #[inline]
    pub fn with<T: NveTable, V: ScalarField>(mut self, prop: Prop<T, V>, value: V) -> Self {
        self.set(prop, value);
        self
    }

    /// Records a [`ResourceId`] override for a `Res<R>` field of table `T` (marking it as a
    /// refcounted child resource edge when `id != ResourceId::NONE`).
    #[inline]
    pub fn set_res_id<T: NveTable, R: FlatResource>(
        &mut self,
        prop: ResProp<T, R>,
        id: ResourceId,
    ) -> &mut Self {
        let key = T::DESC.key;
        self.push_raw_op(key, prop.word, WordOpKind::Resource, WordMask::ALL, id.0);
        if prop.explicit_bit < 32 {
            let bit = 1u32 << prop.explicit_bit;
            self.push_raw_op(key, 0, WordOpKind::Scalar, WordMask(bit), bit);
        }
        self
    }

    /// Fluent by-value counterpart to [`Self::set_res_id`].
    #[inline]
    pub fn with_res_id<T: NveTable, R: FlatResource>(
        mut self,
        prop: ResProp<T, R>,
        id: ResourceId,
    ) -> Self {
        self.set_res_id(prop, id);
        self
    }

    /// Merges all operations from `other` on top of `self` (last-writer-wins cascade).
    pub fn merge(&mut self, other: &Self) {
        for tp in &other.tables {
            for op in &tp.ops {
                self.push_raw_op(tp.key, op.word, op.kind, op.clear_mask, op.set_bits);
            }
        }
    }

    /// Merges all operations from a zero-copy [`RowPatchView`] on top of `self`.
    pub fn merge_view(&mut self, other: RowPatchView<'_>) {
        for table in other.tables() {
            let key = table.key();
            for op in table.ops() {
                self.push_raw_op(key, op.word(), op.kind(), op.clear_mask(), op.set_bits());
            }
        }
    }

    /// Applies every op targeting table `T` onto `out` (skipping string words and mismatched
    /// resource/scalar flags). Returns `true` if `T` was present in this patch.
    pub fn apply_table<T: NveTable>(&self, out: &mut [Word]) -> bool {
        let Ok(idx) = self.tables.binary_search_by_key(&T::DESC.key.0, |t| t.key.0) else {
            return false;
        };
        apply_ops_checked::<T>(&self.tables[idx].ops, out, None);
        true
    }

    /// Applies ops targeting table `T` onto `out` and ORs each op's `clear_mask` into
    /// `written_masks[w]` so backends (e.g. Web CSS) know exactly which packed bits were patched.
    pub fn apply_table_with_mask<T: NveTable>(
        &self,
        out: &mut [Word],
        written_masks: &mut [u32],
    ) -> bool {
        let Ok(idx) = self.tables.binary_search_by_key(&T::DESC.key.0, |t| t.key.0) else {
            return false;
        };
        apply_ops_checked::<T>(&self.tables[idx].ops, out, Some(written_masks));
        true
    }

    /// Reads `prop` if all bits/words belonging to `prop` were set in this patch.
    pub fn get_prop<T: NveTable, V: ScalarField>(&self, prop: Prop<T, V>) -> Option<V> {
        let idx = self.tables.binary_search_by_key(&T::DESC.key.0, |t| t.key.0).ok()?;
        let mut row = [0u32; MAX_TABLE_WORDS];
        let mut masks = [0u32; MAX_TABLE_WORDS];
        let n = T::DESC.words();
        row[..n].copy_from_slice(T::DESC.defaults);
        apply_ops_checked::<T>(&self.tables[idx].ops, &mut row[..n], Some(&mut masks[..n]));
        if !prop_written(&masks[..n], prop) {
            return None;
        }
        Some(prop.get(&row[..n]))
    }

    /// Reads the patched [`ResourceId`] for `prop`, if set in this patch.
    pub fn get_res_id<T: NveTable, R: FlatResource>(
        &self,
        prop: ResProp<T, R>,
    ) -> Option<ResourceId> {
        let idx = self.tables.binary_search_by_key(&T::DESC.key.0, |t| t.key.0).ok()?;
        let oi = self.tables[idx]
            .ops
            .binary_search_by_key(&prop.word, |o| o.word)
            .ok()?;
        let op = self.tables[idx].ops[oi];
        op.is_resource().then_some(ResourceId(op.set_bits))
    }

    /// Visits every non-none [`ResourceId`] referenced by this owned patch.
    pub fn visit_resources(&self, mut visit: impl FnMut(ResourceId)) {
        for tp in &self.tables {
            for op in &tp.ops {
                if op.is_resource() {
                    let id = ResourceId(op.set_bits);
                    if !id.is_none() {
                        visit(id);
                    }
                }
            }
        }
    }

    /// Decodes a [`RowPatchView`] back into an owned [`RowPatch`].
    pub fn from_view(view: RowPatchView<'_>) -> Self {
        let mut out = Self::new();
        out.merge_view(view);
        out
    }
}
