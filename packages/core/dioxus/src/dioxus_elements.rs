//! Native-first `dioxus_elements` schema for Dioxus 0.7 `rsx!{}` without `dioxus-html` bloat.
//!
//! Supports both native names (`view`, `text`, `pressable`, `scrollview`, `image`, `textinput`)
//! and familiar HTML aliases (`div`, `span`, `p`, `button`, `a`, `input`, `form`, `dialog`,
//! `details`, `summary`), plus uniform extensible `nve_table` attachment for capability modules.

#![allow(non_upper_case_globals, non_snake_case, non_camel_case_types)]

use core::marker::PhantomData;

use dioxus_core::{Attribute, AttributeValue, Event};
use nve_abi::table::TableKey;
use nve_abi::{EventFlags, EventId, EventSet, NveEvent, SurfaceId, TimestampUs};
use nve_store::{HasEvents, NodeId, NodeRef, NodeStore};

pub type AttributeDescription = (&'static str, Option<&'static str>, bool);

#[derive(Debug, Clone, Copy, Default)]
pub struct PressEventData {
    pub x: f32,
    pub y: f32,
}

/// Typed pointer identifier handle (`#[repr(transparent)]`) distinguishing active pointers.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerId(pub u32);

impl PointerId {
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl From<u32> for PointerId {
    #[inline]
    fn from(value: u32) -> Self {
        Self(value)
    }
}

/// Full pointer event payload (`onpointerdown`, `onpointermove`, `onpointerup`, `onpointercancel`,
/// `onpointerenter`, `onpointerleave`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PointerEventData {
    pub x: f32,
    pub y: f32,
    pub pointer_id: PointerId,
    pub button: u8,
    pub buttons: u16,
    /// Pointer device kind (`NVE_POINTER_TYPE_MOUSE = 0`, `PEN = 1`, `TOUCH = 2`).
    pub pointer_type: u8,
    pub captured: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompositionEventData {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableEventData {
    pub table_key: TableKey,
    pub event_id: EventId,
    pub payload: Vec<u8>,
}

/// Generic 64-byte [`NveEvent`] payload carrying the fixed POD header plus a reusable byte buffer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NveEventData {
    pub raw: NveEvent,
    pub bytes: Vec<u8>,
}

impl NveEventData {
    #[inline(always)] pub const fn table(&self) -> TableKey { self.raw.table }
    #[inline(always)] pub const fn surface(&self) -> SurfaceId { self.raw.surface }
    #[inline(always)] pub const fn node(&self) -> NodeId { self.raw.node }
    #[inline(always)] pub const fn timestamp(&self) -> TimestampUs { self.raw.timestamp }
    #[inline(always)] pub const fn event(&self) -> EventId { self.raw.event }
    #[inline(always)] pub const fn flags(&self) -> EventFlags { self.raw.flags }
    #[inline(always)] pub const fn x(&self) -> f32 { self.raw.payload_f32(0) }
    #[inline(always)] pub const fn y(&self) -> f32 { self.raw.payload_f32(1) }
    #[inline(always)] pub const fn width(&self) -> f32 { self.raw.payload_f32(2) }
    #[inline(always)] pub const fn height(&self) -> f32 { self.raw.payload_f32(3) }
    #[inline(always)] pub fn bytes(&self) -> &[u8] { &self.bytes }
    #[inline(always)] pub fn text(&self) -> &str {
        if self.raw.flags.has_utf8_text() {
            core::str::from_utf8(&self.bytes).unwrap_or("")
        } else {
            ""
        }
    }
}

/// Zero-cost typed view over a node's [`HasEvents`] table row and active listener [`EventSet`].
#[derive(Clone, Copy)]
pub struct ComponentView<'a, T: HasEvents> {
    node: NodeRef<'a>,
    _marker: PhantomData<fn() -> T>,
}

impl<'a, T: HasEvents> ComponentView<'a, T> {
    #[inline(always)]
    pub const fn new(node: NodeRef<'a>) -> Self {
        Self { node, _marker: PhantomData }
    }

    #[inline(always)]
    pub fn from_store(store: &'a NodeStore, id: NodeId) -> Option<Self> {
        store.get(id).map(Self::new)
    }

    #[inline(always)]
    pub fn node_id(self) -> NodeId {
        self.node.id()
    }

    #[inline(always)]
    pub fn node(self) -> NodeRef<'a> {
        self.node
    }

    #[inline(always)]
    pub fn try_row(self) -> Option<T::Ref<'a>> {
        self.node.try_row::<T>()
    }

    #[inline(always)]
    pub fn row(self) -> T::Ref<'a> {
        self.node.row::<T>()
    }

    #[inline(always)]
    pub fn listeners(self) -> EventSet<T::Event> {
        self.node
            .words(T::slot())
            .and_then(|w| w.get(T::DESC.events_word as usize).copied())
            .map_or(EventSet::EMPTY, EventSet::from_raw)
    }

    #[inline(always)]
    pub fn is_listening(self, ev: T::Event) -> bool {
        self.listeners().contains(ev)
    }
}

pub type PressEvent = Event<PressEventData>;
pub type PointerEvent = Event<PointerEventData>;
pub type TextEvent = Event<TextEventData>;
pub type ScrollEvent = Event<ScrollEventData>;
pub type FocusEvent = Event<FocusEventData>;
pub type KeyEvent = Event<KeyEventData>;
pub type SubmitEvent = Event<SubmitEventData>;
pub type CompositionEvent = Event<CompositionEventData>;
pub type TableEvent = Event<TableEventData>;
pub type RawNveEvent = Event<NveEventData>;

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
            id => "id",
            slot => "slot",
            test_id => "testID",
            accessibility_label => "accessibilityLabel",
            accessibility_role => "accessibilityRole",
            accessibility_hint => "accessibilityHint",
            disabled => "disabled",
            tab_index => "tabIndex",
            aria_checked => "ariaChecked",
            aria_selected => "ariaSelected",
            aria_expanded => "ariaExpanded",
            aria_pressed => "ariaPressed",
            aria_hidden => "ariaHidden",
            aria_busy => "ariaBusy",
            aria_level => "ariaLevel",
            aria_modal => "ariaModal",
            aria_live => "ariaLive",
            aria_controls => "ariaControls",
            aria_labelledby => "ariaLabelledBy",
            aria_describedby => "ariaDescribedBy",
            aria_valuenow => "ariaValueNow",
            aria_valuemin => "ariaValueMin",
            aria_valuemax => "ariaValueMax",
            aria_valuetext => "ariaValueText",
            aria_activedescendant => "ariaActiveDescendant",
            aria_autocomplete => "ariaAutoComplete",
            aria_multiselectable => "ariaMultiSelectable",
            aria_orientation => "ariaOrientation",
            aria_invalid => "ariaInvalid",
            aria_disabled => "ariaDisabled",
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
                nve_table => "nve_table",
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
                flex_basis => "flex-basis",
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
    define_element!(a, "a", {
        href => "href",
        target => "target",
        rel => "rel",
    });
    define_element!(pressable, "pressable", {
        r#type => "type",
        button_type => "type",
        name => "name",
        value => "value",
    });
    define_element!(button, "button", {
        r#type => "type",
        button_type => "type",
        name => "name",
        value => "value",
    });
    define_element!(scrollview, "scrollview", {
        requested_scroll_x => "requestedScrollX",
        requested_scroll_y => "requestedScrollY",
        requested_scroll_seq => "requestedScrollSeq",
    });
    define_element!(image, "image", {
        src => "src",
        alt => "alt",
        resize_mode => "resizeMode",
    });
    define_element!(textinput, "textinput", {
        name => "name",
        r#type => "type",
        input_type => "type",
        placeholder => "placeholder",
        value => "value",
        multiline => "multiline",
        secure_text_entry => "secureTextEntry",
        read_only => "readOnly",
        required => "required",
        minlength => "minlength",
        maxlength => "maxlength",
        pattern => "pattern",
        autocomplete => "autocomplete",
        input_mode => "inputmode",
        inputmode => "inputmode",
    });
    define_element!(input, "input", {
        name => "name",
        r#type => "type",
        input_type => "type",
        placeholder => "placeholder",
        value => "value",
        multiline => "multiline",
        secure_text_entry => "secureTextEntry",
        read_only => "readOnly",
        required => "required",
        minlength => "minlength",
        maxlength => "maxlength",
        pattern => "pattern",
        autocomplete => "autocomplete",
        input_mode => "inputmode",
        inputmode => "inputmode",
    });

    pub mod completions {
        pub mod CompleteWithBraces {}
    }
}

pub use elements::*;

/// Single-source compile-time declaration of every built-in RSX event attribute.
macro_rules! rsx_events {
    ($( $fn_name:ident => ($wire_name:literal, $event_ty:ty) ),* $(,)?) => {
        pub mod events {
            use super::*;
            $(
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
            )*
        }
    };
}

rsx_events! {
    onpress => ("onpress", PressEvent),
    onclick => ("onpress", PressEvent),
    onlongpress => ("onlongpress", PressEvent),
    onpointerdown => ("onpointerdown", PointerEvent),
    onpointermove => ("onpointermove", PointerEvent),
    onpointerup => ("onpointerup", PointerEvent),
    onpointercancel => ("onpointercancel", PointerEvent),
    onpointerenter => ("onpointerenter", PointerEvent),
    onpointerleave => ("onpointerleave", PointerEvent),
    onwheel => ("onwheel", ScrollEvent),
    onchange => ("onchange", TextEvent),
    onscroll => ("onscroll", ScrollEvent),
    onfocus => ("onfocus", FocusEvent),
    onblur => ("onblur", FocusEvent),
    onkeydown => ("onkeydown", KeyEvent),
    onkeyup => ("onkeyup", KeyEvent),
    onsubmit => ("onsubmit", SubmitEvent),
    oncompositionstart => ("oncompositionstart", CompositionEvent),
    oncompositionupdate => ("oncompositionupdate", CompositionEvent),
    oncompositionend => ("oncompositionend", CompositionEvent),
    onlayout => ("onlayout", RawNveEvent),
    ontableevent => ("ontableevent", TableEvent),
    // User/browser-initiated `<dialog>` / `[popover]` closes (bridged by capability plugins).
    onclose => ("onclose", crate::CustomEvent),
}

pub mod GlobalAttributes {
    use super::AttributeDescription;
    define_attrs! {
        style => "style",
        nve_style => "nve_style",
        nve_show_on => "nve-show-on",
        nve_table => "nve_table",
    }
    define_a11y_attrs!();
}

/// Engine backend traits re-exported for downstream UI crates and headless surface harnesses.
pub mod backend {
    pub use nve_engine::{BackendApplyStats, BackendError, LayoutRect, StoreBackend, Viewport};
}
