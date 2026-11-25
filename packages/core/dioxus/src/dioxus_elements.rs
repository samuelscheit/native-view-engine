//! Native-first `dioxus_elements` schema for Dioxus 0.7 `rsx!{}` without `dioxus-html` bloat.
//!
//! Supports both native names (`view`, `text`, `pressable`, `scrollview`, `image`, `textinput`,
//! `skia_canvas`) and familiar HTML aliases (`div`, `span`, `button`) with typed layout/style
//! attributes, component props, accessibility attributes, and native event bindings.

#![allow(non_upper_case_globals, non_snake_case, non_camel_case_types)]

use dioxus_core::{Attribute, AttributeValue, Event};

pub type AttributeDescription = (&'static str, Option<&'static str>, bool);

#[derive(Debug, Clone, Copy, Default)]
pub struct PressEventData {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Default)]
pub struct TextEventData {
    pub value: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScrollEventData {
    pub x: f32,
    pub y: f32,
    pub content_width: f32,
    pub content_height: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FocusEventData;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyEventData {
    pub key: String,
    pub meta: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubmitEventData {
    pub value: String,
}

pub type PressEvent = Event<PressEventData>;
pub type TextEvent = Event<TextEventData>;
pub type ScrollEvent = Event<ScrollEventData>;
pub type FocusEvent = Event<FocusEventData>;
pub type KeyEvent = Event<KeyEventData>;
pub type SubmitEvent = Event<SubmitEventData>;

macro_rules! define_attrs {
    ($($ident:ident => $name:literal),* $(,)?) => {
        $(
            pub const $ident: AttributeDescription = ($name, None, false);
        )*
    };
}

macro_rules! define_a11y_attrs {
    () => {
        define_attrs! {
            test_id => "testID",
            accessibility_label => "accessibilityLabel",
            accessibility_role => "accessibilityRole",
            accessibility_hint => "accessibilityHint",
            disabled => "disabled",
            tab_index => "tabIndex",
            aria_checked => "ariaChecked",
            aria_selected => "ariaSelected",
            aria_expanded => "ariaExpanded",
            aria_modal => "ariaModal",
            aria_live => "ariaLive",
            aria_controls => "ariaControls",
            aria_labelledby => "ariaLabelledBy",
            aria_describedby => "ariaDescribedBy",
            aria_valuenow => "ariaValueNow",
            aria_valuemin => "ariaValueMin",
            aria_valuemax => "ariaValueMax",
            aria_activedescendant => "ariaActiveDescendant",
            aria_autocomplete => "ariaAutoComplete",
            aria_multiselectable => "ariaMultiSelectable",
            aria_orientation => "ariaOrientation",
            aria_invalid => "ariaInvalid",
        }
    };
}

macro_rules! define_element {
    ($mod_name:ident, $tag:literal $(, { $($extra_ident:ident => $extra_name:literal),* $(,)? })?) => {
        pub mod $mod_name {
            use super::AttributeDescription;
            pub const TAG_NAME: &'static str = $tag;
            pub const NAME_SPACE: Option<&'static str> = None;

            define_attrs! {
                style => "style",
                nve_style => "nve_style",
                nve_show_on => "nve-show-on",
                width => "width",
                height => "height",
                min_width => "min-width",
                min_height => "min-height",
                max_width => "max-width",
                max_height => "max-height",
                flex_direction => "flex-direction",
                justify_content => "justify-content",
                align_items => "align-items",
                flex => "flex",
                flex_grow => "flex-grow",
                flex_shrink => "flex-shrink",
                gap => "gap",
                padding => "padding",
                padding_horizontal => "padding-horizontal",
                padding_vertical => "padding-vertical",
                margin => "margin",
                background => "background",
                background_color => "background-color",
                color => "color",
                border_color => "border-color",
                border_width => "border-width",
                border_radius => "border-radius",
                opacity => "opacity",
                font_size => "font-size",
                z_index => "z-index",
                $($( $extra_ident => $extra_name, )*)?
            }
            define_a11y_attrs!();
        }
    };
}

pub mod elements {
    use super::AttributeDescription;

    define_element!(view, "view");
    define_element!(div, "div");
    define_element!(text, "text");
    define_element!(span, "span");
    define_element!(p, "p");
    define_element!(pressable, "pressable");
    define_element!(button, "button");
    define_element!(scrollview, "scrollview");
    define_element!(image, "image", {
        src => "src",
        alt => "alt",
        resize_mode => "resizeMode",
    });
    define_element!(textinput, "textinput", {
        placeholder => "placeholder",
        value => "value",
        multiline => "multiline",
        secure_text_entry => "secureTextEntry",
        read_only => "readOnly",
        autocomplete => "autocomplete",
        input_mode => "inputmode",
        inputmode => "inputmode",
    });
    define_element!(input, "input", {
        placeholder => "placeholder",
        value => "value",
        multiline => "multiline",
        secure_text_entry => "secureTextEntry",
        read_only => "readOnly",
        autocomplete => "autocomplete",
        input_mode => "inputmode",
        inputmode => "inputmode",
    });
    define_element!(skia_canvas, "skia-canvas", {
        draw_list => "drawList",
    });
    define_element!(canvas, "skia-canvas", {
        draw_list => "drawList",
    });

    pub mod completions {
        pub mod CompleteWithBraces {}
    }
}

pub use elements::*;

macro_rules! define_event {
    ($fn_name:ident, $wire_name:literal, $event_ty:ty) => {
        pub fn $fn_name(handler: impl FnMut($event_ty) + 'static) -> Attribute {
            Attribute::new($wire_name, AttributeValue::listener(handler), None, false)
        }

        pub mod $fn_name {
            use super::*;
            pub fn call_with_explicit_closure(
                handler: impl FnMut($event_ty) + 'static,
            ) -> Attribute {
                Attribute::new($wire_name, AttributeValue::listener(handler), None, false)
            }
        }
    };
}

pub mod events {
    use super::*;

    define_event!(onpress, "onpress", PressEvent);
    define_event!(onclick, "onpress", PressEvent);
    define_event!(onchange, "onchange", TextEvent);
    define_event!(onscroll, "onscroll", ScrollEvent);
    define_event!(onfocus, "onfocus", FocusEvent);
    define_event!(onblur, "onblur", FocusEvent);
    define_event!(onkeydown, "onkeydown", KeyEvent);
    define_event!(onsubmit, "onsubmit", SubmitEvent);
}

pub mod GlobalAttributes {
    use super::AttributeDescription;
    define_attrs! {
        style => "style",
        nve_style => "nve_style",
        nve_show_on => "nve-show-on",
    }
    define_a11y_attrs!();
}

/// Engine backend traits re-exported for downstream UI crates and headless surface harnesses.
pub mod backend {
    pub use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};
}
