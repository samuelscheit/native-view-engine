//! Branchless dirty tracking ([`DirtyList`]) and table row writer ([`RowWriter`]).

use nve_abi::table::Word;

use crate::id::{NodeIndex, ResourceId};
use crate::resource::{Resource, ResourceStore};
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
    pub(crate) fn try_reserve(
        &mut self,
        min_cap: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        if self.buf.len() < min_cap {
            self.buf.try_reserve(min_cap - self.buf.len())?;
            self.buf.resize(min_cap, NodeIndex::NULL);
        }
        Ok(())
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
    fn mark(&mut self, word: usize, changed: u32) {
        let was = *self.mask;
        *self.mask = was | (changed << word);
        self.dirty
            .push_if(self.node_index, (was == 0) & (changed != 0));
        let fx = self.desc.word_effects()[word] & 0u8.wrapping_sub(changed as u8);
        let had = self.node.effects;
        self.node.effects = had | fx;
        self.touched
            .push_if(self.node_index, (had == 0) & (fx != 0));
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
    pub fn put_resource<R: Resource>(&mut self, word: usize, value: Option<R>) {
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

    /// Writes an ephemeral (non-deduplicated, rewritten-in-place) resource at `word`.
    #[doc(hidden)]
    #[inline]
    pub fn put_resource_ephemeral<R: Resource>(&mut self, word: usize, value: R) {
        assert!(
            word < 32 && (self.desc.resource_words() & (1u32 << word)) != 0,
            "RowWriter::put_resource_ephemeral: word {word} is not a declared resource word on `{}`",
            self.desc.name()
        );
        let old = ResourceId(self.row[word]);
        let id = self.resources.set_ephemeral(old, value);
        if id == old {
            self.mark(word, 1);
        } else {
            self.swap_resource(word, id);
        }
    }

    /// Typed value of the resource field at `word`.
    #[inline]
    pub fn resource<R: Resource>(&self, word: usize) -> Option<&R> {
        self.resources.get(ResourceId(self.row[word]))
    }

    /// Points a resource field at an existing id (replica apply), taking a reference to it.
    #[inline]
    pub(crate) fn put_resource_id(&mut self, word: usize, id: ResourceId) {
        if ResourceId(self.row[word]) == id {
            return;
        }
        self.resources.retain(id);
        self.swap_resource(word, id);
    }
}
