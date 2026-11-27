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
mod node_mut;
mod node_ref;
mod resource;
mod row;
mod span_pool;
mod store;
mod table;
mod tx;
mod value;
pub mod wire;

pub use changes::{ChangeSet, Command, TableChanges};
pub use events::{TableEventBus, TypedTableEvent};
pub use id::{Idx, IndexVec, NodeId, NodeIndex, ResourceId, TableSet, TableSetIter};
pub use node_mut::NodeMut;
pub use node_ref::NodeRef;
pub use nve_abi::table::{
    FieldDesc, FieldKind, Storage, TableDesc, TableKey, ValueSetDesc, Word, effect, table_flags,
};
pub use nve_abi_macros::NveTable;
pub use resource::{Resource, ResourceStore, register_codec};
pub use row::{DirtyList, RowRef, RowWriter};
pub use span_pool::{ChildSpan, Span, SpanPool, StrArena, StrSpan, class_cap};
pub use store::{NodeRow, NodeStore, RealizationOrder, StoreStats, TableStorage};
pub use table::{
    MAX_STATIC_TABLE_SLOTS, NveTable, Prop, RegistryError, Res, ResProp, Str, StrProp, TableField,
    TableSlot, desc, slot_of, tables_schema_hash,
};
pub use tx::Transaction;
pub use value::{Length, Rgba, WordValue};

#[doc(hidden)]
pub mod __private {
    pub use crate::table::register_static;
    pub use nve_abi::table::{
        FieldDesc, FieldKind, Storage, TableDesc, ValueSetDesc, compute_table_schema_hash,
    };
    pub use nve_hash::{Key64, const_id};
}

#[cfg(test)]
mod tests;
