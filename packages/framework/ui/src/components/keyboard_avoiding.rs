//! Reactive soft-keyboard inset hooks ([`use_keyboard_insets`], [`use_effective_bottom_inset`])
//! and [`KeyboardAvoidingView`] layout container built directly on `nve_dioxus::use_trait`.

use dioxus_core_macro::Props;
use nve_abi::EdgeInsets;
use nve_dioxus::prelude::*;
pub use nve_dioxus::{effective_bottom_inset, KeyboardInsets};

/// Subscribes strictly to the [`KeyboardInsets`] trait from the nearest [`TraitScope`] or
/// [`TraitProvider`] (zero re-renders on unrelated trait updates).
#[inline]
pub fn use_keyboard_insets() -> KeyboardInsets {
    use_trait::<KeyboardInsets>()
}

/// Computes the reactive effective bottom inset (in logical points/dp) from the active
/// [`SafeAreaInsets`] and [`KeyboardInsets`] traits using [`effective_bottom_inset`].
///
/// - When `os_adjusts_resize == true` (Android `adjustResize`), returns `0.0` while the keyboard
///   is visible (zero double-padding) and `safe_area.bottom` when hidden.
/// - When `os_adjusts_resize == false` (iOS, Android edge-to-edge, Web `visualViewport`),
///   returns `safe_area.bottom.max(keyboard.height)` while open and `safe_area.bottom` when closed.
#[inline]
pub fn use_effective_bottom_inset(os_adjusts_resize: bool) -> f32 {
    let safe_area = use_trait::<SafeAreaInsets>();
    let keyboard = use_keyboard_insets();
    effective_bottom_inset(safe_area.bottom, keyboard, os_adjusts_resize)
}

/// Flex column container that automatically applies bottom padding matching the effective soft
/// keyboard + safe-area inset so sticky bottom chrome (chat composers, forms, action bars) stays
/// cleanly above both the virtual keyboard and bottom home indicator with zero double-padding.
#[component]
pub fn KeyboardAvoidingView(
    #[props(default)] os_adjusts_resize: bool,
    #[props(default = true)] include_safe_area: bool,
    #[props(default)] extra_bottom_padding: f32,
    #[props(default)] style: Option<String>,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let keyboard = use_keyboard_insets();
    let safe_bottom = if include_safe_area {
        use_trait::<SafeAreaInsets>().bottom
    } else {
        0.0
    };
    let extra = if extra_bottom_padding.is_finite() {
        extra_bottom_padding.max(0.0)
    } else {
        0.0
    };
    let bottom = effective_bottom_inset(safe_bottom, keyboard, os_adjusts_resize) + extra;

    let container_style = TypedStyle::new()
        .flex_col()
        .width_pct(100.0)
        .padding_insets(EdgeInsets {
            top: 0.0,
            right: 0.0,
            bottom,
            left: 0.0,
        });

    rsx! {
        view {
            test_id: test_id,
            nve_style: container_style,
            style: style,
            {children}
        }
    }
}
