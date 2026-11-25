//! Core `TraitKey` trait, `BuiltinTraitSlot`, compact `BuiltinTraits` storage, and built-in
//! zero-allocation `TraitKey` implementations.

use super::types::{
    f32_changed, CanHover, ColorScheme, ContainerSize, DeviceFormFactor, HasHardwareKeyboard,
    HighContrast, HostToolkit, Locale, Orientation, PlatformOs, PointerKind, ReducedMotion,
    SafeAreaInsets, ScaleFactor, SizeClass, TextScale, UiDensity, VerticalSizeClass,
};

/// Number of built-in fast-path trait slots stored directly inside [`BuiltinTraits`].
pub const BUILTIN_TRAIT_COUNT: usize = 18;

/// Compile-time slot index for every built-in fast-path trait.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTraitSlot {
    PlatformOs = 0,
    HostToolkit = 1,
    SizeClass = 2,
    VerticalSizeClass = 3,
    ContainerSize = 4,
    Orientation = 5,
    SafeAreaInsets = 6,
    ScaleFactor = 7,
    PointerKind = 8,
    CanHover = 9,
    HasHardwareKeyboard = 10,
    ColorScheme = 11,
    ReducedMotion = 12,
    HighContrast = 13,
    TextScale = 14,
    UiDensity = 15,
    Locale = 16,
    DeviceFormFactor = 17,
}

impl BuiltinTraitSlot {
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn bit(self) -> u32 {
        1u32 << (self as u32)
    }

    pub const ALL: [Self; BUILTIN_TRAIT_COUNT] = [
        Self::PlatformOs,
        Self::HostToolkit,
        Self::SizeClass,
        Self::VerticalSizeClass,
        Self::ContainerSize,
        Self::Orientation,
        Self::SafeAreaInsets,
        Self::ScaleFactor,
        Self::PointerKind,
        Self::CanHover,
        Self::HasHardwareKeyboard,
        Self::ColorScheme,
        Self::ReducedMotion,
        Self::HighContrast,
        Self::TextScale,
        Self::UiDensity,
        Self::Locale,
        Self::DeviceFormFactor,
    ];
}

/// Extensible typed environment key modeled after SwiftUI `EnvironmentKey` and Compose
/// `CompositionLocal`.
///
/// Built-in NVE traits resolve in O(1) directly from [`BuiltinTraits`] with zero hashing and zero
/// heap allocation. User-defined keys only implement `Value` and `default_value()` and are stored
/// in a persistent copy-on-write `TypeId` overlay.
pub trait TraitKey: 'static {
    type Value: Clone + PartialEq + Send + Sync + 'static;

    /// Returns the default value when no scope or root store has overridden this trait.
    fn default_value() -> Self::Value;

    /// Internal fast-path slot identifier for built-in traits (`None` for custom user keys).
    #[doc(hidden)]
    const BUILTIN_SLOT: Option<BuiltinTraitSlot> = None;

    /// Internal O(1) direct field read for built-in traits.
    #[doc(hidden)]
    #[inline]
    fn read_builtin(_builtins: &BuiltinTraits) -> Option<Self::Value> {
        None
    }

    /// Internal O(1) direct field write for built-in traits.
    /// Returns `Some(changed)` for built-ins, or `None` for custom user keys.
    #[doc(hidden)]
    #[inline]
    fn write_builtin(_builtins: &mut BuiltinTraits, _value: &Self::Value) -> Option<bool> {
        None
    }
}

/// Compact, `Copy`, zero-allocation storage for all built-in NVE environment traits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuiltinTraits {
    pub platform_os: PlatformOs,
    pub device_form_factor: DeviceFormFactor,
    pub host_toolkit: HostToolkit,
    pub size_class: SizeClass,
    pub vertical_size_class: SizeClass,
    pub container_size: ContainerSize,
    pub orientation: Orientation,
    pub safe_area: SafeAreaInsets,
    pub scale_factor: f32,
    pub pointer_kind: PointerKind,
    pub can_hover: bool,
    pub has_hardware_keyboard: bool,
    pub color_scheme: ColorScheme,
    pub reduced_motion: bool,
    pub high_contrast: bool,
    pub text_scale: f32,
    pub ui_density: UiDensity,
    pub locale: Locale,
}

impl Default for BuiltinTraits {
    fn default() -> Self {
        Self {
            platform_os: PlatformOs::Unknown,
            device_form_factor: DeviceFormFactor::Desktop,
            host_toolkit: HostToolkit::Headless,
            size_class: SizeClass::Compact,
            vertical_size_class: SizeClass::Compact,
            container_size: ContainerSize::ZERO,
            orientation: Orientation::Portrait,
            safe_area: SafeAreaInsets::ZERO,
            scale_factor: 1.0,
            pointer_kind: PointerKind::Fine,
            can_hover: false,
            has_hardware_keyboard: false,
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            high_contrast: false,
            text_scale: 1.0,
            ui_density: UiDensity::Comfortable,
            locale: Locale::EN_US,
        }
    }
}

impl BuiltinTraits {
    /// Initializes built-in traits with the compile-time host OS and sensible platform input defaults.
    pub fn for_current_host(toolkit: HostToolkit) -> Self {
        Self::for_platform(PlatformOs::current(), toolkit)
    }

    /// Initializes built-in traits for an explicit [`PlatformOs`] and [`HostToolkit`].
    pub fn for_platform(os: PlatformOs, toolkit: HostToolkit) -> Self {
        let pointer_kind = match os {
            PlatformOs::TvOs | PlatformOs::AndroidTv => PointerKind::Remote,
            PlatformOs::WatchOs => PointerKind::Crown,
            PlatformOs::VisionOs => PointerKind::Gaze,
            PlatformOs::Ios | PlatformOs::Android => PointerKind::Coarse,
            PlatformOs::MacOs
            | PlatformOs::Windows
            | PlatformOs::Linux
            | PlatformOs::Web
            | PlatformOs::Unknown => PointerKind::Fine,
        };
        let can_hover = match os {
            PlatformOs::VisionOs => true,
            PlatformOs::TvOs
            | PlatformOs::AndroidTv
            | PlatformOs::WatchOs
            | PlatformOs::Ios
            | PlatformOs::Android => false,
            PlatformOs::MacOs
            | PlatformOs::Windows
            | PlatformOs::Linux
            | PlatformOs::Web
            | PlatformOs::Unknown => true,
        };
        let has_hardware_keyboard = !matches!(
            os,
            PlatformOs::Ios
                | PlatformOs::Android
                | PlatformOs::TvOs
                | PlatformOs::AndroidTv
                | PlatformOs::WatchOs
                | PlatformOs::VisionOs
        );
        Self {
            platform_os: os,
            device_form_factor: os.default_form_factor(),
            host_toolkit: toolkit,
            pointer_kind,
            can_hover,
            has_hardware_keyboard,
            ..Self::default()
        }
    }

    /// Copies a single built-in slot from `src` into `self`, returning `true` if the slot changed.
    #[inline]
    pub(crate) fn copy_slot_from(&mut self, slot: BuiltinTraitSlot, src: &Self) -> bool {
        match slot {
            BuiltinTraitSlot::PlatformOs => {
                let changed = self.platform_os != src.platform_os;
                self.platform_os = src.platform_os;
                changed
            }
            BuiltinTraitSlot::HostToolkit => {
                let changed = self.host_toolkit != src.host_toolkit;
                self.host_toolkit = src.host_toolkit;
                changed
            }
            BuiltinTraitSlot::SizeClass => {
                let changed = self.size_class != src.size_class;
                self.size_class = src.size_class;
                changed
            }
            BuiltinTraitSlot::VerticalSizeClass => {
                let changed = self.vertical_size_class != src.vertical_size_class;
                self.vertical_size_class = src.vertical_size_class;
                changed
            }
            BuiltinTraitSlot::ContainerSize => {
                let changed = f32_changed(self.container_size.width, src.container_size.width)
                    || f32_changed(self.container_size.height, src.container_size.height)
                    || self.container_size.class != src.container_size.class;
                self.container_size = src.container_size;
                changed
            }
            BuiltinTraitSlot::Orientation => {
                let changed = self.orientation != src.orientation;
                self.orientation = src.orientation;
                changed
            }
            BuiltinTraitSlot::SafeAreaInsets => {
                let changed = f32_changed(self.safe_area.top, src.safe_area.top)
                    || f32_changed(self.safe_area.right, src.safe_area.right)
                    || f32_changed(self.safe_area.bottom, src.safe_area.bottom)
                    || f32_changed(self.safe_area.left, src.safe_area.left);
                self.safe_area = src.safe_area;
                changed
            }
            BuiltinTraitSlot::ScaleFactor => {
                let changed = f32_changed(self.scale_factor, src.scale_factor);
                self.scale_factor = src.scale_factor;
                changed
            }
            BuiltinTraitSlot::PointerKind => {
                let changed = self.pointer_kind != src.pointer_kind;
                self.pointer_kind = src.pointer_kind;
                changed
            }
            BuiltinTraitSlot::CanHover => {
                let changed = self.can_hover != src.can_hover;
                self.can_hover = src.can_hover;
                changed
            }
            BuiltinTraitSlot::HasHardwareKeyboard => {
                let changed = self.has_hardware_keyboard != src.has_hardware_keyboard;
                self.has_hardware_keyboard = src.has_hardware_keyboard;
                changed
            }
            BuiltinTraitSlot::ColorScheme => {
                let changed = self.color_scheme != src.color_scheme;
                self.color_scheme = src.color_scheme;
                changed
            }
            BuiltinTraitSlot::ReducedMotion => {
                let changed = self.reduced_motion != src.reduced_motion;
                self.reduced_motion = src.reduced_motion;
                changed
            }
            BuiltinTraitSlot::HighContrast => {
                let changed = self.high_contrast != src.high_contrast;
                self.high_contrast = src.high_contrast;
                changed
            }
            BuiltinTraitSlot::TextScale => {
                let changed = f32_changed(self.text_scale, src.text_scale);
                self.text_scale = src.text_scale;
                changed
            }
            BuiltinTraitSlot::UiDensity => {
                let changed = self.ui_density != src.ui_density;
                self.ui_density = src.ui_density;
                changed
            }
            BuiltinTraitSlot::Locale => {
                let changed = self.locale != src.locale;
                self.locale = src.locale;
                changed
            }
            BuiltinTraitSlot::DeviceFormFactor => {
                let changed = self.device_form_factor != src.device_form_factor;
                self.device_form_factor = src.device_form_factor;
                changed
            }
        }
    }

    /// Compares a single built-in slot between two [`BuiltinTraits`] instances.
    #[inline]
    pub(crate) fn slot_eq(&self, other: &Self, slot: BuiltinTraitSlot) -> bool {
        match slot {
            BuiltinTraitSlot::PlatformOs => self.platform_os == other.platform_os,
            BuiltinTraitSlot::HostToolkit => self.host_toolkit == other.host_toolkit,
            BuiltinTraitSlot::SizeClass => self.size_class == other.size_class,
            BuiltinTraitSlot::VerticalSizeClass => {
                self.vertical_size_class == other.vertical_size_class
            }
            BuiltinTraitSlot::ContainerSize => {
                !f32_changed(self.container_size.width, other.container_size.width)
                    && !f32_changed(self.container_size.height, other.container_size.height)
                    && self.container_size.class == other.container_size.class
            }
            BuiltinTraitSlot::Orientation => self.orientation == other.orientation,
            BuiltinTraitSlot::SafeAreaInsets => {
                !f32_changed(self.safe_area.top, other.safe_area.top)
                    && !f32_changed(self.safe_area.right, other.safe_area.right)
                    && !f32_changed(self.safe_area.bottom, other.safe_area.bottom)
                    && !f32_changed(self.safe_area.left, other.safe_area.left)
            }
            BuiltinTraitSlot::ScaleFactor => !f32_changed(self.scale_factor, other.scale_factor),
            BuiltinTraitSlot::PointerKind => self.pointer_kind == other.pointer_kind,
            BuiltinTraitSlot::CanHover => self.can_hover == other.can_hover,
            BuiltinTraitSlot::HasHardwareKeyboard => {
                self.has_hardware_keyboard == other.has_hardware_keyboard
            }
            BuiltinTraitSlot::ColorScheme => self.color_scheme == other.color_scheme,
            BuiltinTraitSlot::ReducedMotion => self.reduced_motion == other.reduced_motion,
            BuiltinTraitSlot::HighContrast => self.high_contrast == other.high_contrast,
            BuiltinTraitSlot::TextScale => !f32_changed(self.text_scale, other.text_scale),
            BuiltinTraitSlot::UiDensity => self.ui_density == other.ui_density,
            BuiltinTraitSlot::Locale => self.locale == other.locale,
            BuiltinTraitSlot::DeviceFormFactor => {
                self.device_form_factor == other.device_form_factor
            }
        }
    }
}

macro_rules! impl_builtin_trait_key {
    ($key:ty, $val:ty, $slot:ident, $field:ident, $default:expr) => {
        impl TraitKey for $key {
            type Value = $val;

            #[inline]
            fn default_value() -> Self::Value {
                $default
            }

            const BUILTIN_SLOT: Option<BuiltinTraitSlot> = Some(BuiltinTraitSlot::$slot);

            #[inline]
            fn read_builtin(builtins: &BuiltinTraits) -> Option<Self::Value> {
                Some(builtins.$field)
            }

            #[inline]
            fn write_builtin(builtins: &mut BuiltinTraits, value: &Self::Value) -> Option<bool> {
                let changed = builtins.$field != *value;
                builtins.$field = *value;
                Some(changed)
            }
        }
    };
}

macro_rules! impl_builtin_f32_trait_key {
    ($key:ty, $slot:ident, $field:ident, $default:expr) => {
        impl TraitKey for $key {
            type Value = f32;

            #[inline]
            fn default_value() -> f32 {
                $default
            }

            const BUILTIN_SLOT: Option<BuiltinTraitSlot> = Some(BuiltinTraitSlot::$slot);

            #[inline]
            fn read_builtin(builtins: &BuiltinTraits) -> Option<f32> {
                Some(builtins.$field)
            }

            #[inline]
            fn write_builtin(builtins: &mut BuiltinTraits, value: &f32) -> Option<bool> {
                let changed = f32_changed(builtins.$field, *value);
                builtins.$field = *value;
                Some(changed)
            }
        }
    };
}

impl_builtin_trait_key!(PlatformOs, PlatformOs, PlatformOs, platform_os, PlatformOs::Unknown);
impl_builtin_trait_key!(
    DeviceFormFactor,
    DeviceFormFactor,
    DeviceFormFactor,
    device_form_factor,
    DeviceFormFactor::Desktop
);
impl_builtin_trait_key!(HostToolkit, HostToolkit, HostToolkit, host_toolkit, HostToolkit::Headless);
impl_builtin_trait_key!(SizeClass, SizeClass, SizeClass, size_class, SizeClass::Compact);
impl_builtin_trait_key!(
    VerticalSizeClass,
    SizeClass,
    VerticalSizeClass,
    vertical_size_class,
    SizeClass::Compact
);
impl_builtin_trait_key!(Orientation, Orientation, Orientation, orientation, Orientation::Portrait);
impl_builtin_trait_key!(PointerKind, PointerKind, PointerKind, pointer_kind, PointerKind::Fine);
impl_builtin_trait_key!(CanHover, bool, CanHover, can_hover, false);
impl_builtin_trait_key!(
    HasHardwareKeyboard,
    bool,
    HasHardwareKeyboard,
    has_hardware_keyboard,
    false
);
impl_builtin_trait_key!(ColorScheme, ColorScheme, ColorScheme, color_scheme, ColorScheme::Light);
impl_builtin_trait_key!(ReducedMotion, bool, ReducedMotion, reduced_motion, false);
impl_builtin_trait_key!(HighContrast, bool, HighContrast, high_contrast, false);
impl_builtin_trait_key!(UiDensity, UiDensity, UiDensity, ui_density, UiDensity::Comfortable);
impl_builtin_trait_key!(Locale, Locale, Locale, locale, Locale::EN_US);

impl_builtin_f32_trait_key!(ScaleFactor, ScaleFactor, scale_factor, 1.0);
impl_builtin_f32_trait_key!(TextScale, TextScale, text_scale, 1.0);

impl TraitKey for ContainerSize {
    type Value = ContainerSize;

    #[inline]
    fn default_value() -> ContainerSize {
        ContainerSize::ZERO
    }

    const BUILTIN_SLOT: Option<BuiltinTraitSlot> = Some(BuiltinTraitSlot::ContainerSize);

    #[inline]
    fn read_builtin(builtins: &BuiltinTraits) -> Option<ContainerSize> {
        Some(builtins.container_size)
    }

    #[inline]
    fn write_builtin(builtins: &mut BuiltinTraits, value: &ContainerSize) -> Option<bool> {
        let changed = f32_changed(builtins.container_size.width, value.width)
            || f32_changed(builtins.container_size.height, value.height)
            || builtins.container_size.class != value.class;
        builtins.container_size = *value;
        Some(changed)
    }
}

impl TraitKey for SafeAreaInsets {
    type Value = SafeAreaInsets;

    #[inline]
    fn default_value() -> SafeAreaInsets {
        SafeAreaInsets::ZERO
    }

    const BUILTIN_SLOT: Option<BuiltinTraitSlot> = Some(BuiltinTraitSlot::SafeAreaInsets);

    #[inline]
    fn read_builtin(builtins: &BuiltinTraits) -> Option<SafeAreaInsets> {
        Some(builtins.safe_area)
    }

    #[inline]
    fn write_builtin(builtins: &mut BuiltinTraits, value: &SafeAreaInsets) -> Option<bool> {
        let changed = f32_changed(builtins.safe_area.top, value.top)
            || f32_changed(builtins.safe_area.right, value.right)
            || f32_changed(builtins.safe_area.bottom, value.bottom)
            || f32_changed(builtins.safe_area.left, value.left);
        builtins.safe_area = *value;
        Some(changed)
    }
}
