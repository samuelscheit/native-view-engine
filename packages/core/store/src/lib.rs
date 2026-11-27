//! Data-oriented word-table `NodeStore`.
//!
//! - Every node is a bare [`NodeId`] with attached tables; no component kinds. Backends derive a
//!   native realizing slot (`primary_slot`) from the attached tables via [`RealizationOrder`].
//! - Tables are `#[derive(NveTable)]` structs: fixed rows of `<= 32` `u32` words (dense or paged
//!   sparse), strings in one [`StrArena`], children in one flat [`SpanPool`], blobs in the
//!   refcounted, deduplicated [`ResourceStore`].
//! - Writes go through a [`Transaction`] (branchless word write + dirty mask + first-dirty append);
//!   [`Transaction::commit_into`] yields a reusable [`ChangeSet`] that in-process backends consume
//!   with `&NodeStore` (zero copies, zero encoding). Across process / `dlopen` / network
//!   boundaries [`wire`] encodes the same `ChangeSet` as table-major `NVE2` and applies it to a
//!   replica store.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate self as nve_store;

mod changes;
mod column;
mod commit;
pub mod events;
mod id;
mod inherited;
mod node_mut;
mod resource;
mod resource_ingress;
mod row;
mod row_patch;
mod span_pool;
mod store;
mod table;
mod tx;
mod value;
pub mod wire;

pub use changes::{ChangeSet, Command, TableChanges};
pub use events::SurfaceEventTracker;
pub use id::{Idx, IndexVec, NodeId, NodeIndex, ResourceId, TableSet, TableSetIter};
pub use inherited::{InheritedTable, ResolvedRow};
pub use nve_abi::{
    EventBatchBuf, EventEnum, EventFlags, EventId, EventSet, NveEvent, NveEventBatch, SurfaceId,
    TimestampUs, ValidatedEvent, ValidatedEvents,
};
pub use nve_abi::table::{
    effect, table_flags, FieldDesc, FieldKind, OptEncoding, Storage, TableDesc, TableKey, Word,
};
pub use nve_abi_macros::{NveTable, WordEnum};
pub use resource::{words_as_le_bytes, FlatResource, ResourceStore, MAX_RESOURCE_WORDS};
pub use resource_ingress::{
    padded_word_len, read_bytes_words, read_fixed_str_pack, read_fixed_str_pack_unchecked,
    read_str_words, read_str_words_unchecked, validate_bytes_words, validate_fixed_str_pack,
    validate_str_words, write_bytes_words, write_fixed_str_pack, write_str_words, ValidatedWords,
};
pub use row::{DirtyList, RowRef, RowWriter};
pub use row_patch::{
    prop_written, RowPatch, RowPatchView, TableOpsIter, TableOpsView, WordMask, WordOp, WordOpKind,
    WordOpsIter, MAX_PATCH_TABLES, WORD_OP_RESOURCE, WORD_OP_SCALAR,
};
pub use span_pool::{class_cap, ChildSpan, Span, SpanPool, StrArena, StrSpan};
pub use store::{NodeRef, NodeRow, NodeStore, RealizationOrder, StoreStats};
pub use table::{
    desc, slot_of, HasEvents, NveTable, Prop, RegistryError, Res, ResProp, ScalarField, Str,
    StrProp, TableField, TableSlot, MAX_STATIC_TABLE_SLOTS,
};
pub use node_mut::{NodeMut, ScopedTx, TempResources};
pub use tx::Transaction;
pub use value::{GridLine, Length, OptWord, Rgba, WordValue};

#[doc(hidden)]
pub mod __private {
    pub use crate::table::register_static;
    pub use crate::value::{
        bitflags_width, opt_bool_to_word, opt_i16_to_word, opt_i8_to_word, opt_node_to_word,
        opt_nz_u16_to_word, opt_nz_u32_to_word, opt_nz_u8_to_word, opt_surf_to_word,
        opt_u16_to_word, opt_u8_to_word,
    };
    pub use nve_abi::define_bitflags;
    pub use nve_abi::table::{
        compute_table_schema_hash, finalize_table_flags, pack_table, trim_bytes, trim_words,
        FieldDesc, FieldKind, OptEncoding, PackInput, PackKind, PackLayout, Storage, TableDesc,
        NO_VAL_WORD,
    };
    pub use nve_hash::{const_id, Key64};
}

#[cfg(test)]
mod inherited_edge_tests;
#[cfg(test)]
mod row_patch_tests;
#[cfg(test)]
mod inherited_tests;
#[cfg(test)]
mod table_packing_tests;
#[cfg(test)]
mod table_wire_tests;
#[cfg(test)]
mod tests;
