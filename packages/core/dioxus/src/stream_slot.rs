//! Declarative Shadow DOM `<slot>` streaming marker component ([`NveStreamSlot`] / [`SuspenseSlot`])
//! for Stage-1 zero-JS out-of-order async SSR streaming and Stage-2 zero-JS live slot updates.

use dioxus_core_macro::Props;

use crate::prelude::*;

/// Raw DOM attribute identifying a Declarative Shadow DOM streaming slot boundary.
pub const NVE_STREAM_SLOT_ATTR: &str = "data-nve-stream-slot";

/// Raw DOM attribute distinguishing mutable output slots from stable input slots.
pub const NVE_STREAM_SLOT_ROLE_ATTR: &str = "data-nve-slot-role";

/// Prefix applied to Declarative Shadow DOM `<slot name="nve-slot-{id}">` markers.
pub const NVE_STREAM_SLOT_NAME_PREFIX: &str = "nve-slot-";

/// Role of a streaming slot boundary in Stage-1 async SSR and Stage-2 Zero-JS Live Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamSlotRole {
    /// Replaceable async/live output slot (receives out-of-order Light DOM chunks).
    #[default]
    Output,
    /// Stable input slot hosting active `<Form>` / `<input>` controls (never overwritten live).
    Input,
}

impl StreamSlotRole {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Input => "input",
        }
    }
}

/// Declarative async/streaming slot boundary emitting `data-nve-stream-slot="{slot_id}"` around
/// fallback children so `nve-backend-web` emits `<slot name="nve-slot-{slot_id}">...</slot>` inside
/// Declarative Shadow DOM during Stage-1 SSR and streams Light DOM replacements out-of-order.
#[component]
pub fn NveStreamSlot(
    slot_id: String,
    #[props(default)] role: StreamSlotRole,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let role_str = role.as_str();
    rsx! {
        view {
            test_id: test_id,
            "data-nve-stream-slot": "{slot_id}",
            "data-nve-slot-role": "{role_str}",
            style: style,
            {children}
        }
    }
}

/// Ergonomic alias for [`NveStreamSlot`] (`SuspenseSlot { slot_id: "feed", ... }`).
pub use self::NveStreamSlot as SuspenseSlot;
pub use self::NveStreamSlotProps as SuspenseSlotProps;
