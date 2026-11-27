//! Zero-copy borrowed views ([`RowPatchView`], [`TableOpsView`]), iterators, [`prop_written`],
//! and the [`FlatResource`] implementation for [`RowPatch`].

use nve_abi::table::{TableKey, Word, MAX_TABLE_WORDS};
use nve_abi::Key64;

use crate::id::ResourceId;
use crate::resource::FlatResource;
use crate::resource_ingress::ValidatedWords;
use crate::table::{NveTable, Prop, ResProp, ScalarField};

use super::{RowPatch, WordMask, WordOp, WordOpKind, MAX_PATCH_TABLES, WORD_OP_RESOURCE};

/// Zero-copy borrowed view over flat `RowPatch` `u32` words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowPatchView<'a>(&'a [u32]);

impl<'a> RowPatchView<'a> {
    pub const EMPTY: Self = Self(&[0]);

    /// Validates `words` via [`RowPatch::validate`] and returns a zero-copy view on success.
    #[inline]
    pub fn from_words(words: &'a [u32]) -> Option<Self> {
        RowPatch::validate(words).then_some(Self(words))
    }

    #[doc(hidden)]
    #[inline(always)]
    pub const fn from_validated_words(words: &'a [u32]) -> Self {
        Self(words)
    }

    #[inline(always)]
    pub const fn raw_words(self) -> &'a [u32] {
        self.0
    }

    #[inline(always)]
    pub const fn table_count(self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            self.0[0] as usize
        }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.table_count() == 0
    }

    #[inline]
    pub fn tables(self) -> TableOpsIter<'a> {
        let count = self.table_count();
        TableOpsIter {
            words: if self.0.len() > 1 { &self.0[1..] } else { &[] },
            pos: 0,
            remaining: count,
        }
    }

    /// Finds the operations for table `key`, if present.
    #[inline]
    pub fn table_ops(self, key: TableKey) -> Option<TableOpsView<'a>> {
        self.tables().find(|t| t.key() == key)
    }

    #[inline]
    pub fn has_table<T: NveTable>(self) -> bool {
        self.table_ops(T::DESC.key).is_some()
    }

    /// Branchless masked apply onto `out` for table `T`. Returns `true` if `T` was present.
    #[inline]
    pub fn apply_table<T: NveTable>(self, out: &mut [Word]) -> bool {
        let Some(t) = self.table_ops(T::DESC.key) else {
            return false;
        };
        t.apply_checked::<T>(out, None);
        true
    }

    /// Masked apply onto `out` for table `T`, also recording every written bit in `written_masks`.
    #[inline]
    pub fn apply_table_with_mask<T: NveTable>(
        self,
        out: &mut [Word],
        written_masks: &mut [u32],
    ) -> bool {
        let Some(t) = self.table_ops(T::DESC.key) else {
            return false;
        };
        t.apply_checked::<T>(out, Some(written_masks));
        true
    }

    /// Reads `prop` directly from this flat view if all its words/bits are patched.
    pub fn get_prop<T: NveTable, V: ScalarField>(self, prop: Prop<T, V>) -> Option<V> {
        let t = self.table_ops(T::DESC.key)?;
        let mut row = [0u32; MAX_TABLE_WORDS];
        let mut masks = [0u32; MAX_TABLE_WORDS];
        let n = T::DESC.words();
        row[..n].copy_from_slice(T::DESC.defaults);
        t.apply_checked::<T>(&mut row[..n], Some(&mut masks[..n]));
        if !prop_written(&masks[..n], prop) {
            return None;
        }
        Some(prop.get(&row[..n]))
    }

    /// Reads the patched [`ResourceId`] for `prop`, if set.
    pub fn get_res_id<T: NveTable, R: FlatResource>(
        self,
        prop: ResProp<T, R>,
    ) -> Option<ResourceId> {
        let t = self.table_ops(T::DESC.key)?;
        t.ops()
            .find(|op| op.word == prop.word && op.is_resource())
            .map(|op| ResourceId(op.set_bits))
    }

    /// Visits every non-none embedded [`ResourceId`] across all tables in this patch.
    pub fn visit_resources(self, visit: &mut dyn FnMut(ResourceId)) {
        for t in self.tables() {
            for op in t.ops() {
                if op.is_resource() {
                    let id = ResourceId(op.set_bits);
                    if !id.is_none() {
                        visit(id);
                    }
                }
            }
        }
    }
}

/// Zero-copy view of one table's masked word operations inside a [`RowPatchView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableOpsView<'a> {
    key: TableKey,
    op_words: &'a [u32],
}

impl<'a> TableOpsView<'a> {
    #[inline(always)]
    pub const fn key(self) -> TableKey {
        self.key
    }

    #[inline(always)]
    pub const fn len(self) -> usize {
        self.op_words.len() / 3
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.op_words.is_empty()
    }

    #[inline]
    pub fn ops(self) -> WordOpsIter<'a> {
        WordOpsIter {
            words: self.op_words,
            pos: 0,
        }
    }

    #[inline]
    fn apply_checked<T: NveTable>(
        self,
        out: &mut [Word],
        written_masks: Option<&mut [u32]>,
    ) {
        apply_ops_iter::<T>(self.ops(), out, written_masks);
    }
}

/// Zero-alloc iterator over [`TableOpsView`] sections in a [`RowPatchView`].
#[derive(Debug, Clone)]
pub struct TableOpsIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for TableOpsIter<'a> {
    type Item = TableOpsView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let key_lo = *self.words.get(self.pos)?;
        let key_hi = *self.words.get(self.pos + 1)?;
        let op_count = *self.words.get(self.pos + 2)? as usize;
        let ops_start = self.pos + 3;
        let ops_end = ops_start.checked_add(op_count.checked_mul(3)?)?;
        let op_words = self.words.get(ops_start..ops_end)?;
        self.pos = ops_end;
        self.remaining -= 1;
        Some(TableOpsView {
            key: Key64::from_halves(key_lo, key_hi),
            op_words,
        })
    }
}

/// Zero-alloc iterator over [`WordOp`]s inside one [`TableOpsView`].
#[derive(Debug, Clone)]
pub struct WordOpsIter<'a> {
    words: &'a [u32],
    pos: usize,
}

impl Iterator for WordOpsIter<'_> {
    type Item = WordOp;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let wf = *self.words.get(self.pos)?;
        let clear_mask = WordMask(*self.words.get(self.pos + 1)?);
        let set_bits = *self.words.get(self.pos + 2)?;
        self.pos += 3;
        let word = wf & 0xFFFF;
        let flags = (wf >> 16) as u16;
        if word as usize >= MAX_TABLE_WORDS || flags > WORD_OP_RESOURCE {
            return None;
        }
        let kind = if flags == WORD_OP_RESOURCE {
            WordOpKind::Resource
        } else {
            WordOpKind::Scalar
        };
        Some(WordOp::from_validated_raw(
            word as u8,
            kind,
            clear_mask,
            set_bits,
        ))
    }
}

#[inline]
fn apply_ops_iter<T: NveTable>(
    ops: impl Iterator<Item = WordOp>,
    out: &mut [Word],
    mut written_masks: Option<&mut [u32]>,
) {
    let str_mask = T::DESC.str_words | (T::DESC.str_words << 1);
    let res_mask = T::DESC.resource_words;
    for op in ops {
        let w = op.word() as usize;
        if w >= out.len() || w >= MAX_TABLE_WORDS {
            continue;
        }
        let bit = 1u32 << w;
        if (str_mask & bit) != 0 {
            continue;
        }
        if op.is_resource() != ((res_mask & bit) != 0) {
            continue;
        }
        out[w] = op.clear_mask().apply(out[w], op.set_bits());
        if let Some(wm) = written_masks.as_deref_mut()
            && w < wm.len()
        {
            wm[w] |= op.clear_mask().raw();
        }
    }
}

#[inline]
pub(super) fn apply_ops_checked<T: NveTable>(
    ops: &[WordOp],
    out: &mut [Word],
    written_masks: Option<&mut [u32]>,
) {
    apply_ops_iter::<T>(ops.iter().copied(), out, written_masks);
}

/// Returns `true` iff every word/bit required by `prop` is covered by `written_masks`.
#[inline]
pub fn prop_written<T: NveTable, V: ScalarField>(written_masks: &[u32], prop: Prop<T, V>) -> bool {
    let w = prop.word as usize;
    if w >= written_masks.len() || (written_masks[w] & prop.mask) != prop.mask {
        return false;
    }
    if prop.val_word != nve_abi::table::NO_VAL_WORD {
        let vw = prop.val_word as usize;
        return vw < written_masks.len() && written_masks[vw] == u32::MAX;
    }
    if V::BITS == 64 {
        let w1 = w + 1;
        return w1 < written_masks.len() && written_masks[w1] == u32::MAX;
    }
    true
}

impl FlatResource for RowPatch {
    const KIND: Key64 = Key64::of("nve.res.row_patch");
    type View<'a> = RowPatchView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.tables.len() > MAX_PATCH_TABLES {
            return false;
        }
        let mut needed = 1usize;
        for tp in &self.tables {
            if tp.ops.is_empty() || tp.ops.len() > MAX_TABLE_WORDS {
                return false;
            }
            needed += 3 + tp.ops.len() * 3;
        }
        out.reserve(needed);
        out.push(self.tables.len() as u32);
        for tp in &self.tables {
            out.push(tp.key.lo());
            out.push(tp.key.hi());
            out.push(tp.ops.len() as u32);
            for op in &tp.ops {
                out.push(u32::from(op.word()) | (u32::from(op.kind() as u16) << 16));
                out.push(op.clear_mask().raw());
                out.push(op.set_bits());
            }
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&table_count_u32) = words.first() else {
            return false;
        };
        let table_count = table_count_u32 as usize;
        if table_count > MAX_PATCH_TABLES {
            return false;
        }
        let mut pos = 1usize;
        let mut prev_key: Option<u64> = None;
        for _ in 0..table_count {
            let Some(&key_lo) = words.get(pos) else {
                return false;
            };
            let Some(&key_hi) = words.get(pos + 1) else {
                return false;
            };
            let Some(&op_count_u32) = words.get(pos + 2) else {
                return false;
            };
            let key = Key64::from_halves(key_lo, key_hi).0;
            if prev_key.is_some_and(|pk| key <= pk) {
                return false;
            }
            prev_key = Some(key);
            let op_count = op_count_u32 as usize;
            if op_count == 0 || op_count > MAX_TABLE_WORDS {
                return false;
            }
            pos += 3;
            let Some(ops_end) = op_count.checked_mul(3).and_then(|n| pos.checked_add(n)) else {
                return false;
            };
            if ops_end > words.len() {
                return false;
            }
            let mut prev_word: Option<u32> = None;
            for _ in 0..op_count {
                let wf = words[pos];
                let clear_mask = words[pos + 1];
                let set_bits = words[pos + 2];
                pos += 3;
                let word_idx = wf & 0xFFFF;
                let flags = (wf >> 16) as u16;
                if word_idx as usize >= MAX_TABLE_WORDS
                    || prev_word.is_some_and(|pw| word_idx <= pw)
                {
                    return false;
                }
                prev_word = Some(word_idx);
                if flags > WORD_OP_RESOURCE || clear_mask == 0 || (set_bits & !clear_mask) != 0 {
                    return false;
                }
                if flags == WORD_OP_RESOURCE && clear_mask != u32::MAX {
                    return false;
                }
            }
        }
        pos == words.len()
    }

    #[inline(always)]
    fn view<'a>(words: ValidatedWords<'a, Self>) -> Self::View<'a> {
        RowPatchView(words.as_slice())
    }

    fn child_resources(words: &[u32], visit: &mut dyn FnMut(ResourceId)) {
        RowPatchView(words).visit_resources(visit);
    }
}
