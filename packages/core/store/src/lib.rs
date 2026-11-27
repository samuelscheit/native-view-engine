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

extern crate self as nve_store;

mod changes;
mod column;
mod commit;
pub mod events;
mod id;
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
pub use nve_abi::table::{effect, table_flags, FieldDesc, FieldKind, Storage, TableDesc, TableKey, Word};
pub use nve_abi_macros::NveTable;
pub use resource::{register_codec, Resource, ResourceStore};
pub use row::{DirtyList, RowRef, RowWriter};
pub use span_pool::{class_cap, ChildSpan, Span, SpanPool, StrArena, StrSpan};
pub use store::{NodeRef, NodeRow, NodeStore, RealizationOrder, StoreStats};
pub use table::{
    desc, slot_of, NveTable, Prop, RegistryError, Res, ResProp, Str, StrProp, TableField,
    TableSlot, MAX_STATIC_TABLE_SLOTS,
};
pub use tx::{NodeMut, Transaction};
pub use value::{Length, Rgba, WordValue};

#[doc(hidden)]
pub mod __private {
    pub use crate::table::register_static;
    pub use nve_abi::table::{compute_table_schema_hash, FieldDesc, FieldKind, Storage, TableDesc};
    pub use nve_hash::{const_id, Key64};
}

#[cfg(test)]
mod tests;
