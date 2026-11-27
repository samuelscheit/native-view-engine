// Materializes a validated `NveBinValue` into a borrowed C `NveValue` tree.

use std::marker::PhantomData;

use super::bin_view::BinValueView;
use super::ffi_types::{
    NveMapEntry, NveValue, NveValueArray, NveValueData, NveValueMap, NveValueTag,
};
use super::ffi_views::NveValueRef;

/// `NveValue` tree of a validated `NveBinValue`, for C signatures that take `NveValue` (a hybrid
/// `Value` prop or event parameter) without wrapping the encoding in `Bytes`.
///
/// Strings, bytes, and map keys borrow the encoded bytes (`'a`); container slices point into two
/// flat arenas owned by the tree (one for array elements, one for map entries), sized exactly by a
/// counting pass and filled breadth-first, so they never reallocate once a pointer is taken.
pub struct NveValueTree<'a> {
    root: NveValue,
    _elems: Vec<NveValue>,
    _entries: Vec<NveMapEntry>,
    _bytes: PhantomData<&'a [u8]>,
}

impl<'a> NveValueTree<'a> {
    pub fn new(view: BinValueView<'a>) -> Self {
        let mut counts = Counts::default();
        counts.add(view);
        let elems = Vec::with_capacity(counts.elems);
        let entries = Vec::with_capacity(counts.entries);
        let mut builder = Builder {
            elems_base: elems.as_ptr(),
            entries_base: entries.as_ptr(),
            elems,
            entries,
            reserved_elems: 0,
            reserved_entries: 0,
            pending: Vec::with_capacity(counts.containers),
        };
        let root = builder.node(view);
        builder.fill();
        Self {
            root,
            _elems: builder.elems,
            _entries: builder.entries,
            _bytes: PhantomData,
        }
    }

    /// The root `NveValue`; every pointer reachable from it stays valid while `self` lives.
    #[inline]
    pub fn root(&self) -> &NveValue {
        &self.root
    }

    #[inline]
    pub fn as_value_ref(&self) -> NveValueRef<'_> {
        NveValueRef { raw: &self.root }
    }
}

#[derive(Default)]
struct Counts {
    elems: usize,
    entries: usize,
    containers: usize,
}

impl Counts {
    fn add(&mut self, view: BinValueView<'_>) {
        if let Some(items) = view.as_array() {
            self.containers += 1;
            self.elems += items.len();
            items.for_each(|item| self.add(item));
        } else if let Some(entries) = view.as_map() {
            self.containers += 1;
            self.entries += entries.len();
            entries.for_each(|(_, item)| self.add(item));
        }
    }
}

struct Builder<'a> {
    elems: Vec<NveValue>,
    entries: Vec<NveMapEntry>,
    /// Taken once from the exactly sized arenas; `Vec::push` within capacity keeps them valid.
    elems_base: *const NveValue,
    entries_base: *const NveMapEntry,
    reserved_elems: usize,
    reserved_entries: usize,
    /// Containers whose children are not built yet, in the order their arena blocks were reserved.
    pending: Vec<BinValueView<'a>>,
}

impl<'a> Builder<'a> {
    /// Builds a scalar, or reserves a container's arena block and queues its children.
    fn node(&mut self, view: BinValueView<'a>) -> NveValue {
        match view.tag() {
            NveValueTag::Null => NveValue::null(),
            NveValueTag::Bool => NveValue::bool(view.as_bool().unwrap_or_default()),
            NveValueTag::I64 => NveValue::i64(view.as_i64().unwrap_or_default()),
            NveValueTag::F64 => NveValue::f64(view.as_f64().unwrap_or_default()),
            NveValueTag::Str => NveValue::from_str(view.as_str().unwrap_or_default()),
            NveValueTag::Bytes => NveValue::from_bytes(view.as_bytes().unwrap_or_default()),
            NveValueTag::Array => {
                let len = view.array_len().unwrap_or_default();
                let ptr = self.elems_base.wrapping_add(self.reserved_elems);
                self.reserved_elems += len;
                self.pending.push(view);
                NveValue {
                    tag: NveValueTag::Array as u32,
                    data: NveValueData {
                        array_val: NveValueArray { ptr, len },
                    },
                }
            }
            NveValueTag::Map => {
                let len = view.map_len().unwrap_or_default();
                let ptr = self.entries_base.wrapping_add(self.reserved_entries);
                self.reserved_entries += len;
                self.pending.push(view);
                NveValue {
                    tag: NveValueTag::Map as u32,
                    data: NveValueData {
                        map_val: NveValueMap { ptr, len },
                    },
                }
            }
        }
    }

    /// Appends each queued container's children contiguously, breadth-first, so every container's
    /// block lands exactly where `node` reserved it.
    fn fill(&mut self) {
        let mut next = 0;
        while let Some(&container) = self.pending.get(next) {
            next += 1;
            if let Some(items) = container.as_array() {
                for item in items {
                    let value = self.node(item);
                    push_within_capacity(&mut self.elems, value);
                }
            } else if let Some(entries) = container.as_map() {
                for (key, item) in entries {
                    let value = self.node(item);
                    push_within_capacity(&mut self.entries, NveMapEntry::new(key, value));
                }
            }
        }
    }
}

/// The arenas are sized by `Counts` over the same validated view, so this always pushes; the
/// guard only makes a reallocation (which would dangle the reserved pointers) impossible.
#[inline]
fn push_within_capacity<T>(arena: &mut Vec<T>, value: T) {
    if arena.len() < arena.capacity() {
        arena.push(value);
    }
}
