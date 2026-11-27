//! Uniform, zero-module-coupling table attachment for Dioxus RSX (`nve_table`).
//!
//! Capability modules (`nve-svg`, `nve-canvas`, `nve-blur`, `nve-webview`, `nve-video`,
//! `nve-camera`, `nve-web-element`, `nve-web-html`, or any 3rd-party crate) implement
//! [`TableWriter`] or wrap a closure with [`table_attr_fn`] and pass it via `nve_table: ...`.
//! `NveMutationWriter` performs a single downcast to [`TableAttr`] and invokes
//! `TableWriter::write_to(tx, node_id)` directly on the live `NodeStore` transaction.

use std::rc::Rc;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_store::{NodeId, Transaction};

/// Attribute wire name recognized by [`crate::NveMutationWriter`] for extensible [`TableAttr`]s.
pub const NVE_TABLE_ATTR: &str = "nve_table";

/// Trait implemented by capability crates and 3rd-party components to attach/update typed
/// `NodeStore` word tables directly inside a Dioxus commit transaction.
pub trait TableWriter: 'static {
    /// Writes or updates one or more word tables on `node` inside `tx`.
    fn write_to(&self, tx: &mut Transaction<'_>, node: NodeId);

    /// Optional cleanup invoked when the attribute is removed (`AttributeValue::None`).
    fn clear_from(&self, _tx: &mut Transaction<'_>, _node: NodeId) {}
}

/// Dioxus `IntoAttributeValue` payload carrying an arbitrary [`TableWriter`] into RSX.
#[derive(Clone)]
pub struct TableAttr {
    writer: Rc<dyn TableWriter>,
    kind_id: core::any::TypeId,
}

impl TableAttr {
    #[inline]
    pub fn new<W: TableWriter>(writer: W) -> Self {
        Self {
            writer: Rc::new(writer),
            kind_id: core::any::TypeId::of::<W>(),
        }
    }

    /// Returns `true` when `other` wraps the same concrete [`TableWriter`] type.
    #[inline]
    pub fn same_writer_kind(&self, other: &Self) -> bool {
        self.kind_id == other.kind_id
    }

    #[inline]
    pub fn write_to(&self, tx: &mut Transaction<'_>, node: NodeId) {
        self.writer.write_to(tx, node);
    }

    #[inline]
    pub fn clear_from(&self, tx: &mut Transaction<'_>, node: NodeId) {
        self.writer.clear_from(tx, node);
    }
}

impl std::fmt::Debug for TableAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TableAttr(..)")
    }
}

impl PartialEq for TableAttr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.writer, &other.writer)
    }
}

impl IntoAttributeValue for TableAttr {
    #[inline]
    fn into_value(self) -> AttributeValue {
        AttributeValue::Any(Rc::new(self))
    }
}

struct FnTableWriter<F>(F);

impl<F> TableWriter for FnTableWriter<F>
where
    F: Fn(&mut Transaction<'_>, NodeId) + 'static,
{
    #[inline]
    fn write_to(&self, tx: &mut Transaction<'_>, node: NodeId) {
        (self.0)(tx, node);
    }
}

/// Wraps a closure `Fn(&mut Transaction<'_>, NodeId)` into an RSX-ready [`TableAttr`].
#[inline]
pub fn table_attr_fn<F>(f: F) -> TableAttr
where
    F: Fn(&mut Transaction<'_>, NodeId) + 'static,
{
    TableAttr::new(FnTableWriter(f))
}

struct FnClearTableWriter<W, C> {
    write: W,
    clear: C,
}

impl<W, C> TableWriter for FnClearTableWriter<W, C>
where
    W: Fn(&mut Transaction<'_>, NodeId) + 'static,
    C: Fn(&mut Transaction<'_>, NodeId) + 'static,
{
    #[inline]
    fn write_to(&self, tx: &mut Transaction<'_>, node: NodeId) {
        (self.write)(tx, node);
    }

    #[inline]
    fn clear_from(&self, tx: &mut Transaction<'_>, node: NodeId) {
        (self.clear)(tx, node);
    }
}

/// Wraps paired `(write, clear)` closures into an RSX-ready [`TableAttr`] that cleans up its
/// table state when replaced or removed.
#[inline]
pub fn table_attr_fn_with_clear<W, C>(write: W, clear: C) -> TableAttr
where
    W: Fn(&mut Transaction<'_>, NodeId) + 'static,
    C: Fn(&mut Transaction<'_>, NodeId) + 'static,
{
    TableAttr::new(FnClearTableWriter { write, clear })
}
