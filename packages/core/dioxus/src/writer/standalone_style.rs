//! Recorded standalone style attributes (`width`, `color`, `opacity`, ...) and the recycling
//! pool that keeps the writer's per-node side-table buffers (standalone style lists, their
//! `String`s, inline `style` strings, `TableAttr` lists) allocation-free in steady state:
//! removed nodes return their buffers, newly styled nodes (e.g. re-instantiated templates) take
//! them back.

use crate::TableAttr;

/// Saved value of a standalone layout/visual/typography attribute so clearing `style="..."`
/// restores standalone attributes instead of wiping them.
#[derive(Debug, Clone)]
pub(crate) enum StandaloneStyleVal {
    Float(f64),
    Int(i64),
    Str(String),
}

/// Borrowed incoming [`StandaloneStyleVal`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum StandaloneStyleRef<'v> {
    Float(f64),
    Int(i64),
    Str(&'v str),
}

/// One node's standalone style attributes in canonical CSS precedence order.
pub(crate) type StandaloneStyleList = Vec<(String, StandaloneStyleVal)>;

/// Short buffers (standalone keys and values) are rarely longer than this; longer ones are
/// dropped on recycle instead of inflating every later key to the peak length.
const MAX_SHORT_CAP: usize = 64;
/// Inline `style="..."` strings above this capacity are dropped on recycle.
const MAX_INLINE_CAP: usize = 4 << 10;
/// Lists above this many entries (capacity) are dropped on recycle.
const MAX_LIST_CAP: usize = 32;

/// One node's active `TableAttr` writers by attribute name.
pub(crate) type TableAttrList = Vec<(&'static str, TableAttr)>;
/// Pooled buffers per kind: enough to remount a few hundred styled nodes without allocating,
/// while an unmounted large view never pins its whole style footprint.
const MAX_POOLED: usize = 512;

/// Recycled [`StandaloneStyleList`]s, [`TableAttrList`]s, short `String`s (keys, values), and
/// inline `style` strings, kept apart so mixed sizes never swap through one LIFO. Bounded in
/// count and per-buffer capacity; zero-capacity buffers are never pooled (they own nothing).
#[derive(Debug, Default)]
pub(crate) struct BufferPool {
    lists: Vec<StandaloneStyleList>,
    table_lists: Vec<TableAttrList>,
    strings: Vec<String>,
    inline: Vec<String>,
}

/// Returns `v` (empty) to `pool` unless it owns nothing, is oversized, or the pool is full.
fn give_list<T>(pool: &mut Vec<Vec<T>>, mut v: Vec<T>) {
    if v.capacity() != 0 && v.capacity() <= MAX_LIST_CAP && pool.len() < MAX_POOLED {
        v.clear();
        pool.push(v);
    }
}

/// Pops a recycled buffer holding a copy of `s`.
fn take(pool: &mut Vec<String>, s: &str) -> String {
    let mut out = pool.pop().unwrap_or_default();
    out.push_str(s);
    out
}

/// Returns `s` (cleared) to `pool` unless it owns nothing, is oversized, or the pool is full.
fn give(pool: &mut Vec<String>, mut s: String, max_cap: usize) {
    if s.capacity() != 0 && s.capacity() <= max_cap && pool.len() < MAX_POOLED {
        s.clear();
        pool.push(s);
    }
}

impl BufferPool {
    /// An owned copy of a standalone key or value in a recycled buffer.
    pub(crate) fn string(&mut self, s: &str) -> String {
        take(&mut self.strings, s)
    }

    /// An owned copy of an inline `style="..."` string in a recycled buffer.
    pub(crate) fn inline(&mut self, s: &str) -> String {
        take(&mut self.inline, s)
    }

    pub(crate) fn value(&mut self, val: StandaloneStyleRef<'_>) -> StandaloneStyleVal {
        match val {
            StandaloneStyleRef::Float(f) => StandaloneStyleVal::Float(f),
            StandaloneStyleRef::Int(i) => StandaloneStyleVal::Int(i),
            StandaloneStyleRef::Str(s) => StandaloneStyleVal::Str(self.string(s)),
        }
    }

    /// Overwrites `slot` with `val`, reusing an existing `Str` buffer in place.
    pub(crate) fn assign(&mut self, slot: &mut StandaloneStyleVal, val: StandaloneStyleRef<'_>) {
        if let (StandaloneStyleVal::Str(s), StandaloneStyleRef::Str(v)) = (&mut *slot, val) {
            s.clear();
            s.push_str(v);
            return;
        }
        let old = core::mem::replace(slot, self.value(val));
        self.recycle_value(old);
    }

    pub(crate) fn list(&mut self) -> StandaloneStyleList {
        self.lists.pop().unwrap_or_default()
    }

    pub(crate) fn recycle_string(&mut self, s: String) {
        give(&mut self.strings, s, MAX_SHORT_CAP);
    }

    pub(crate) fn recycle_inline(&mut self, s: String) {
        give(&mut self.inline, s, MAX_INLINE_CAP);
    }

    pub(crate) fn recycle_entry(&mut self, (key, val): (String, StandaloneStyleVal)) {
        self.recycle_string(key);
        self.recycle_value(val);
    }

    pub(crate) fn recycle_list(&mut self, mut list: StandaloneStyleList) {
        for entry in list.drain(..) {
            self.recycle_entry(entry);
        }
        give_list(&mut self.lists, list);
    }

    pub(crate) fn table_list(&mut self) -> TableAttrList {
        self.table_lists.pop().unwrap_or_default()
    }

    pub(crate) fn recycle_table_list(&mut self, list: TableAttrList) {
        give_list(&mut self.table_lists, list);
    }

    fn recycle_value(&mut self, val: StandaloneStyleVal) {
        if let StandaloneStyleVal::Str(s) = val {
            self.recycle_string(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_is_bounded_split_by_kind_and_skips_empty_or_oversized_buffers() {
        let mut pool = BufferPool::default();
        // Zero-capacity buffers own nothing: never pooled.
        pool.recycle_string(String::new());
        pool.recycle_inline(String::new());
        pool.recycle_list(Vec::new());
        assert!(pool.strings.is_empty() && pool.inline.is_empty() && pool.lists.is_empty());

        // Oversized buffers are dropped, not pinned.
        pool.recycle_string(String::with_capacity(MAX_SHORT_CAP + 1));
        pool.recycle_inline(String::with_capacity(MAX_INLINE_CAP + 1));
        pool.recycle_list(Vec::with_capacity(MAX_LIST_CAP + 1));
        assert!(pool.strings.is_empty() && pool.inline.is_empty() && pool.lists.is_empty());

        // A long inline string never feeds a short key, and recycled buffers come back cleared.
        let long = "background-color: #1e293b; border-radius: 4px";
        pool.recycle_inline(long.to_owned());
        pool.recycle_string("gap".to_owned());
        assert_eq!(pool.string("4px"), "4px");
        let inline = pool.inline("opacity: 0.5");
        assert_eq!(inline, "opacity: 0.5");
        assert!(inline.capacity() >= long.len(), "inline buffer reused");

        // Count bound.
        for _ in 0..MAX_POOLED + 10 {
            pool.recycle_string(String::with_capacity(8));
            pool.recycle_list(Vec::with_capacity(4));
        }
        assert_eq!(
            (pool.strings.len(), pool.lists.len()),
            (MAX_POOLED, MAX_POOLED)
        );
    }
}
