//! Declarative Shadow DOM `<slot>` streaming marker component ([`NveStreamSlot`] / [`SuspenseSlot`])
//! for Stage-1 zero-JS out-of-order async SSR streaming and Stage-2 zero-JS live slot updates.

use dioxus_core_macro::Props;

use crate::prelude::*;

pub use nve_tables::tables::{
    StreamSlotRole, NVE_STREAM_SLOT_ATTR, NVE_STREAM_SLOT_ROLE_ATTR, NVE_SUSPENSE_ATTR,
};

/// Prefix applied to Declarative Shadow DOM `<slot name="nve-slot-{id}">` markers.
pub const NVE_STREAM_SLOT_NAME_PREFIX: &str = "nve-slot-";

/// Declarative async/streaming slot boundary emitting `data-nve-stream-slot="{slot_id}"` around
/// fallback children so `nve-backend-web` emits `<slot name="nve-slot-{slot_id}">...</slot>` inside
/// Declarative Shadow DOM during Stage-1 SSR and streams Light DOM replacements out-of-order.
#[component]
pub fn NveStreamSlot(
    slot_id: String,
    #[props(default = StreamSlotRole::Output)] role: StreamSlotRole,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let role_attr = role.as_attr();
    rsx! {
        view {
            test_id: test_id,
            "data-nve-stream-slot": "{slot_id}",
            "data-nve-slot-role": role_attr,
            nve_style: nve_style,
            {children}
        }
    }
}

/// Ergonomic alias for [`NveStreamSlot`] (`SuspenseSlot { slot_id: "feed", ... }`).
pub use self::NveStreamSlot as SuspenseSlot;
pub use self::NveStreamSlotProps as SuspenseSlotProps;
