//! Unified Extensible Environment & `TraitKey` System.
//!
//! Combines SwiftUI `EnvironmentValues`, Jetpack Compose `CompositionLocal`, and CSS
//! `@media`/`@container` queries into a zero-JSON, zero-allocation fast-path trait engine:
//! - 18 built-in traits live directly inside a compact `Copy` [`BuiltinTraits`] struct (`O(1)`
//!   field access, zero hashing/boxing).
//! - User-defined [`TraitKey`]s live in a persistent copy-on-write `TypeId` overlay map.
//! - [`TraitStore`] tracks per-trait `u64` revision counters with breakpoint-only viewport updates
//!   so 1px window drags within the same [`SizeClass`]/[`Orientation`] never trigger re-renders.

mod builtin;
mod environment;
mod resolver;
mod store;
mod types;

pub use builtin::{BuiltinTraitSlot, BuiltinTraits, TraitKey, BUILTIN_TRAIT_COUNT};
pub use environment::{TraitEnvironment, TraitOverrides};
pub use resolver::TraitResolver;
pub use store::{TraitStore, ViewportChangeMask};
pub use types::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, CanHover, ColorScheme,
    ContainerSize, DeviceFormFactor, HasHardwareKeyboard, HighContrast, HostToolkit,
    KeyboardInsets, Locale, Orientation, PlatformOs, PointerKind, ReducedMotion, SafeAreaInsets,
    ScaleFactor, SizeClass, TextScale, UiDensity, VerticalSizeClass,
    VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};
