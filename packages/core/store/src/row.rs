//! Branchless dirty tracking ([`DirtyList`]) and table row writer ([`RowWriter`]).

use nve_abi::table::Word;

use crate::id::{NodeIndex, ResourceId};
use crate::resource::{FlatResource, ResourceStore};
use crate::span_pool::{StrArena, StrSpan};
use crate::store::NodeRow;
use crate::table::TableMeta;

/// Sealed zero-copy view of one table row paired with its owning [`StrArena`].
///
/// External crates cannot construct `RowRef`, guaranteeing that `words` and `text` belong to the
/// same [`crate::NodeStore`] and share lifetime `'a` so Rust's borrow checker statically prevents
/// mutating the arena while any borrowed `&'a str` is live.
#[derive(Clone, Copy)]
pub struct RowRef<'a> {
    words: &'a [Word],
    text: &'a StrArena,
    str_words: u32,
}

impl<'a> RowRef<'a> {
    #[inline(always)]
    pub(crate) const fn new(words: &'a [Word], text: &'a StrArena, str_words: u32) -> Self {
        Self {
            words,
            text,
            str_words,
        }
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn word(self, idx: usize) -> Word {
        self.words[idx]
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn words(self) -> &'a [Word] {
        self.words
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn str_at(self, word: usize) -> &'a str {
        assert!(
            word < 31 && (self.str_words & (1u32 << word)) != 0,
            "RowRef::str_at: word {word} is not a declared string word (str_words={:#x})",
            self.str_words
        );
        self.text
            .str_unchecked(StrSpan::new(self.words[word], self.words[word + 1]))
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn str_span_at(self, word: usize) -> StrSpan {
        assert!(
            word < 31 && (self.str_words & (1u32 << word)) != 0,
            "RowRef::str_span_at: word {word} is not a declared string word (str_words={:#x})",
            self.str_words
        );
        StrSpan::new(self.words[word], self.words[word + 1])
    }
}

/// Append-only per-frame dirty node-index list with a branchless conditional push:
/// `buf[len] = n; len += cond` with no data-dependent branch.
#[derive(Debug, Default, Clone)]
pub struct DirtyList {
    buf: Vec<NodeIndex>,
    len: usize,
}

impl DirtyList {
    #[inline(always)]
    pub(crate) fn push_if(&mut self, value: NodeIndex, cond: bool) {
        if self.len == self.buf.len() {
            self.grow();
        }
        self.buf[self.len] = value;
        self.len += cond as usize;
    }

    #[cold]
    fn grow(&mut self) {
        let n = (self.buf.len() * 2).max(64);
        self.buf.resize(n, NodeIndex::NULL);
    }

    #[inline]
    pub fn as_slice(&self) -> &[NodeIndex] {
        &self.buf[..self.len]
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
}

/// Mutable access to one row of one table for one node, tracking dirty words and node effects.
///
/// Generated `*Mut` types wrap this. Every scalar write is: masked word update, change bit into
/// the row dirty mask, first-dirty append into the table's list, effect bits into the node row,
/// first-touched append into the store's list — all branch-free.
pub struct RowWriter<'a> {
    pub(crate) row: &'a mut [Word],
    pub(crate) mask: &'a mut u32,
    pub(crate) dirty: &'a mut DirtyList,
    pub(crate) node_index: NodeIndex,
    pub(crate) node: &'a mut NodeRow,
    pub(crate) touched: &'a mut DirtyList,
    pub(crate) desc: &'a TableMeta,
    pub(crate) text: &'a mut StrArena,
    pub(crate) resources: &'a mut ResourceStore,
}

impl<'a> RowWriter<'a> {
    #[inline(always)]
    fn mark_with_effects(&mut self, word: usize, changed: u32, effect_bits: u8) {
        let was = *self.mask;
        *self.mask = was | (changed << word);
        self.dirty
            .push_if(self.node_index, (was == 0) & (changed != 0));
        let fx = effect_bits & 0u8.wrapping_sub(changed as u8);
        let had = self.node.effects;
        self.node.effects = had | fx;
        self.touched
            .push_if(self.node_index, (had == 0) & (fx != 0));
    }

    #[inline(always)]
    fn mark(&mut self, word: usize, changed: u32) {
        let effect_bits = self.desc.word_effects()[word];
        self.mark_with_effects(word, changed, effect_bits);
    }

    /// Branchlessly sets explicit bit `bit` (`0..32`) in word 0 (`__explicit: u32`) of an
    /// inherited table and attributes `field_effects` whenever the bit flips `0 -> 1` (including
    /// when writing the table's default value to override an ancestor).
    #[doc(hidden)]
    #[inline(always)]
    pub fn set_explicit_bit(&mut self, bit: u32, field_effects: u8) {
        assert!(
            self.desc.inherited()
                && bit < 32
                && ((self.desc.str_words() | self.desc.resource_words()) & 1) == 0,
            "RowWriter::set_explicit_bit requires an inherited table with bit < 32 on `{}`",
            self.desc.name()
        );
        let bit_mask = 1u32 << bit;
        let old = self.row[0];
        let new = old | bit_mask;
        self.row[0] = new;
        self.mark_with_effects(0, (old != new) as u32, field_effects);
    }

    /// Branchlessly clears explicit bit `bit` (`0..32`) in word 0 (`__explicit: u32`) of an
    /// inherited table and attributes `field_effects` whenever the bit flips `1 -> 0`.
    #[doc(hidden)]
    #[inline(always)]
    pub fn clear_explicit_bit(&mut self, bit: u32, field_effects: u8) {
        assert!(
            self.desc.inherited()
                && bit < 32
                && ((self.desc.str_words() | self.desc.resource_words()) & 1) == 0,
            "RowWriter::clear_explicit_bit requires an inherited table with bit < 32 on `{}`",
            self.desc.name()
        );
        let bit_mask = 1u32 << bit;
        let old = self.row[0];
        let new = old & !bit_mask;
        self.row[0] = new;
        self.mark_with_effects(0, (old != new) as u32, field_effects);
    }

    /// Applies a wire patch to word 0 (`__explicit: u32`) of an inherited table, ORing the exact
    /// field effects of every flipped bit into the node's effects.
    #[inline]
    pub(crate) fn put_explicit_word(&mut self, new_mask: u32) {
        debug_assert!(self.desc.inherited());
        let old = self.row[0];
        let diff = old ^ new_mask;
        if diff == 0 {
            return;
        }
        self.row[0] = new_mask;
        let fx = if let Some(sd) = self.desc.as_static() {
            let mut acc = 0u8;
            let mut d = diff;
            while d != 0 {
                let b = d.trailing_zeros() as usize;
                d &= d - 1;
                if let Some(f) = sd.fields.get(b) {
                    acc |= f.effects;
                }
            }
            acc
        } else {
            self.desc.effects()
        };
        self.mark_with_effects(0, 1, fx);
    }

    /// Writes `bits` into `word` under `mask` (`bits` already shifted into place by the caller's
    /// `shift`). Panics if `word` belongs to a string or resource slot.
    #[doc(hidden)]
    #[inline(always)]
    pub fn put(&mut self, word: usize, shift: u32, mask: u32, bits: u32) {
        let str_w = self.desc.str_words();
        let protected = str_w | (str_w << 1) | self.desc.resource_words();
        assert!(
            word < 32 && (protected & (1u32 << word)) == 0,
            "RowWriter::put cannot overwrite string/resource word {word} on `{}`",
            self.desc.name()
        );
        let w = &mut self.row[word];
        let old = *w;
        let new = (old & !mask) | ((bits << shift) & mask);
        *w = new;
        self.mark(word, (old != new) as u32);
    }

    /// Current sealed row view (used by generated `*Mut::get`).
    #[doc(hidden)]
    #[inline(always)]
    pub fn as_row_ref(&self) -> RowRef<'_> {
        RowRef::new(self.row, self.text, self.desc.str_words())
    }

    /// Current raw words of the row.
    #[inline(always)]
    pub fn words(&self) -> &[Word] {
        self.row
    }

    /// Writes a two-word string field at `word`.
    #[doc(hidden)]
    #[inline]
    pub fn put_str(&mut self, word: usize, text: &str) {
        assert!(
            word < 31 && (self.desc.str_words() & (1u32 << word)) != 0,
            "RowWriter::put_str: word {word} is not a declared string word on `{}`",
            self.desc.name()
        );
        let old = StrSpan::new(self.row[word], self.row[word + 1]);
        if self.text.str_unchecked(old) == text {
            return;
        }
        let new = self.text.replace_str(old, text);
        self.row[word] = new.off();
        self.row[word + 1] = new.len();
        self.mark(word, 1);
        self.mark(word + 1, 1);
    }

    /// Text of a string field at `word`.
    #[inline]
    pub fn str(&self, word: usize) -> &str {
        assert!(
            word < 31 && (self.desc.str_words() & (1u32 << word)) != 0,
            "RowWriter::str: word {word} is not a declared string word on `{}`",
            self.desc.name()
        );
        self.text
            .str_unchecked(StrSpan::new(self.row[word], self.row[word + 1]))
    }

    #[inline]
    fn swap_resource(&mut self, word: usize, id: ResourceId) {
        let old = ResourceId(self.row[word]);
        self.row[word] = id.0;
        self.mark(word, (old != id) as u32);
        self.resources.release(old);
    }

    /// Writes (interning + deduplicating) or clears a resource field at `word`.
    #[doc(hidden)]
    #[inline]
    pub fn put_resource<R: FlatResource>(&mut self, word: usize, value: Option<&R>) {
        assert!(
            word < 32 && (self.desc.resource_words() & (1u32 << word)) != 0,
            "RowWriter::put_resource: word {word} is not a declared resource word on `{}`",
            self.desc.name()
        );
        let id = match value {
            Some(v) => self.resources.intern(v),
            None => ResourceId::NONE,
        };
        self.swap_resource(word, id);
    }

    /// Writes a resource via a zero-alloc flat-word closure into `ResourceStore`'s scratch buffer.
    #[doc(hidden)]
    #[inline]
    pub fn put_resource_with<R: FlatResource>(
        &mut self,
        word: usize,
        write: impl FnOnce(&mut Vec<u32>) -> bool,
    ) {
        assert!(
            word < 32 && (self.desc.resource_words() & (1u32 << word)) != 0,
            "RowWriter::put_resource_with: word {word} is not a declared resource word on `{}`",
            self.desc.name()
        );
        let id = self
            .resources
            .try_intern_with::<R>(write)
            .unwrap_or(ResourceId::NONE);
        self.swap_resource(word, id);
    }

    /// Writes an ephemeral (non-deduplicated, rewritten-in-place) resource at `word`.
    #[doc(hidden)]
    #[inline]
    pub fn put_resource_ephemeral<R: FlatResource>(&mut self, word: usize, value: &R) {
        assert!(
            word < 32 && (self.desc.resource_words() & (1u32 << word)) != 0,
            "RowWriter::put_resource_ephemeral: word {word} is not a declared resource word on `{}`",
            self.desc.name()
        );
        let old = ResourceId(self.row[word]);
        let id = self.resources.set_ephemeral(old, value);
        if id == old && !id.is_none() {
            self.mark(word, 1);
        } else {
            self.swap_resource(word, id);
        }
    }

    /// Writes an ephemeral resource via a zero-alloc flat-word closure into `ResourceStore`.
    #[doc(hidden)]
    #[inline]
    pub fn put_resource_ephemeral_with<R: FlatResource>(
        &mut self,
        word: usize,
        write: impl FnOnce(&mut Vec<u32>) -> bool,
    ) {
        assert!(
            word < 32 && (self.desc.resource_words() & (1u32 << word)) != 0,
            "RowWriter::put_resource_ephemeral_with: word {word} is not a declared resource word on `{}`",
            self.desc.name()
        );
        let old = ResourceId(self.row[word]);
        let id = self
            .resources
            .try_set_ephemeral_with::<R>(old, write)
            .unwrap_or(ResourceId::NONE);
        if id == old && !id.is_none() {
            self.mark(word, 1);
        } else {
            self.swap_resource(word, id);
        }
    }

    /// Zero-copy typed view of the resource field at `word`.
    #[inline]
    pub fn resource<R: FlatResource>(&self, word: usize) -> Option<R::View<'_>> {
        self.resources.get::<R>(ResourceId(self.row[word]))
    }

    /// Points a resource field at a live resource of type `R` (or `ResourceId::NONE`), taking a
    /// ref. Returns `false` without mutating if `word` is not a resource word or `id` is stale,
    /// dead (zero refs), or of a mismatched resource kind/type.
    #[doc(hidden)]
    #[inline]
    pub fn put_resource_id<R: FlatResource>(&mut self, word: usize, id: ResourceId) -> bool {
        if word >= 32 || (self.desc.resource_words() & (1u32 << word)) == 0 {
            return false;
        }
        if !id.is_none() && !self.resources.is_live_of::<R>(id) {
            return false;
        }
        if ResourceId(self.row[word]) == id {
            return true;
        }
        self.resources.retain(id);
        self.swap_resource(word, id);
        true
    }

    /// Wire/internal counterpart to [`Self::put_resource_id`] accepting any defined resource id
    /// (including `0`-ref wire-defined entries validated by `WireDecoder` earlier in the frame).
    #[inline]
    pub(crate) fn put_resource_id_wire(&mut self, word: usize, id: ResourceId) {
        if word >= 32 || (self.desc.resource_words() & (1u32 << word)) == 0 {
            return;
        }
        if !id.is_none() && !self.resources.is_defined(id) {
            return;
        }
        if ResourceId(self.row[word]) == id {
            return;
        }
        self.resources.retain(id);
        self.swap_resource(word, id);
    }
}
