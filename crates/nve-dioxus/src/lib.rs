//! Dioxus (`dioxus-core 0.7`) frontend adapter for `native-view-engine`.
//!
//! Implements `dioxus_core::WriteMutations` to emit a single flat, typed binary `CommandBuffer`
//! per frame — feeding Fabric (iOS/Android), Skia (Desktop/Mobile), Web (WASM), and Headless
//! backends identically.

pub mod dioxus_elements;
pub use dioxus_elements as elements;
mod events;
pub mod hot_patch;
pub mod native_view;
pub mod signal_archive;
mod surface;
#[cfg(test)]
mod tests;
pub mod traits;
mod writer;

pub use elements::{
    FocusEvent, FocusEventData, KeyEvent, KeyEventData, ScrollEvent, ScrollEventData, SubmitEvent,
    SubmitEventData,
};
pub use native_view::{
    use_native_view_handle, use_native_view_registry, use_provide_native_view_registry,
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodeNativeCommand, DecodeNativeProps,
    DecodedWebElementProps, EncodeNativeCommand, EncodeNativeProps, NativeCommandDecoder,
    NativeCommandEncoder, NativePropDecoder, NativePropEncoder, NativeValueRef, NativeView,
    NativeViewCodecError, NativeViewHandle, NativeViewProvider, NativeViewRegistry, NativeViewSpec,
    NoCommand, QueuedNativeCommand, VideoPlayer, VideoPlayerCommand, VideoPlayerProps, WebElement,
    WebElementProps, WebView, WebViewCommand, WebViewProps, NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC,
    NATIVE_VIEW_KIND_PROP_KEY, NATIVE_VIEW_PROPS_PROP_KEY,
};
pub use signal_archive::{
    signal_slot_id, BinValueSlot, HydrationReport, SignalArchive, SignalArchiveError, SignalCodec,
    SignalDelta, SignalDeltaOp, SignalRegistry, SignalTableSnapshot, SignalValue, SignalValueTag,
    NVE_SIGNAL_ARCHIVE_MAGIC, NVE_SIGNAL_ARCHIVE_VERSION, NVE_SIGNAL_DELTA_MAGIC,
};
pub use surface::DioxusSurface;
pub use traits::{
    use_environment, use_provide_existing_trait_store, use_provide_trait_store, use_trait,
    use_trait_store, BuiltinTraitSlot, BuiltinTraits, CanHover, ColorScheme, ContainerSize,
    HasHardwareKeyboard, HighContrast, HostToolkit, Locale, Orientation, PlatformOnly, PlatformOs,
    PointerKind, ReactiveTraitStore, ReducedMotion, SafeAreaInsets, ScaleFactor, SizeClass,
    TextScale, TraitEnvironment, TraitKey, TraitOverrides, TraitProvider, TraitResolver,
    TraitScope, TraitStore, TraitSwitch, TraitValueScope, UiDensity, VerticalSizeClass,
    ViewportChangeMask,
};
pub use writer::NveMutationWriter;

use dioxus_core::Event;

/// Alias matching `elements::events::onblur` (`FocusEvent`) so blur listeners share one canonical payload type.
pub type BlurEventData = FocusEventData;
pub type BlurEvent = FocusEvent;

/// Payload for generic native `Custom` events dispatched into Dioxus custom event listeners.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomEventData {
    pub name: String,
    pub data: Vec<u8>,
}

pub type CustomEvent = Event<CustomEventData>;

pub mod prelude {
    pub use crate::elements::{
        self as dioxus_elements, events::*, KeyEvent, KeyEventData, PressEvent, SubmitEvent,
        SubmitEventData, TextEvent,
    };
    pub use crate::native_view::{
        use_native_view_handle, use_native_view_registry, use_provide_native_view_registry,
        BlurMaterial, BlurSurface, BlurSurfaceProps, NativeView, NativeViewCodecError,
        NativeViewHandle, NativeViewProvider, NativeViewRegistry, NativeViewSpec, VideoPlayer,
        VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView, WebViewCommand,
        WebViewProps,
    };
    pub use crate::traits::{
        use_environment, use_provide_existing_trait_store, use_provide_trait_store, use_trait,
        use_trait_store, CanHover, ColorScheme, ContainerSize, HasHardwareKeyboard, HighContrast,
        HostToolkit, Locale, Orientation, PlatformOnly, PlatformOs, PointerKind,
        ReactiveTraitStore, ReducedMotion, SafeAreaInsets, ScaleFactor, SizeClass, TextScale,
        TraitEnvironment, TraitKey, TraitOverrides, TraitProvider, TraitScope, TraitStore,
        TraitSwitch, TraitValueScope, UiDensity, VerticalSizeClass,
    };
    pub use crate::use_signal;
    pub use crate::{
        BlurEvent, BlurEventData, CustomEvent, CustomEventData, FocusEvent, FocusEventData,
        ScrollEvent, ScrollEventData,
    };
    pub use dioxus_core;
    pub use dioxus_core::{Element, VirtualDom};
    pub use dioxus_core_macro::{component, rsx, Props};
    pub use dioxus_signals;
    pub use dioxus_signals::{ReadableExt, Signal, WritableExt};
}

/// Lightweight `use_signal` hook backed directly by `dioxus_core::use_hook` + `dioxus_signals::Signal`.
pub fn use_signal<T: 'static>(init: impl FnOnce() -> T) -> dioxus_signals::Signal<T> {
    dioxus_core::use_hook(|| dioxus_signals::Signal::new(init()))
}
