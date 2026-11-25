//! Zero-serialization Dioxus `rsx!` attribute wrapper ([`CanvasDrawList`]) carrying an
//! immutable [`DrawList`] (`Arc<[CanvasOp]>`) directly through `AttributeValue::Any` into
//! `NveMutationWriter` without base64 or binary `NVE_DRAW1` encode/decode round-trips.

use std::ops::Deref;
use std::rc::Rc;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_components::canvas_ir::{CanvasOp, DrawList};

/// Ergonomic `IntoAttributeValue` wrapper carrying a zero-serialization [`DrawList`] directly
/// into `<canvas draw_list: ...>` / `<skia_canvas draw_list: ...>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CanvasDrawList(pub DrawList);

impl CanvasDrawList {
    #[inline]
    pub fn into_inner(self) -> DrawList {
        self.0
    }
}

impl Deref for CanvasDrawList {
    type Target = DrawList;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<DrawList> for CanvasDrawList {
    #[inline]
    fn from(dl: DrawList) -> Self {
        Self(dl)
    }
}

impl From<Vec<CanvasOp>> for CanvasDrawList {
    #[inline]
    fn from(ops: Vec<CanvasOp>) -> Self {
        Self(DrawList::from_ops(ops))
    }
}

impl IntoAttributeValue for CanvasDrawList {
    #[inline]
    fn into_value(self) -> AttributeValue {
        AttributeValue::Any(Rc::new(self.0))
    }
}
