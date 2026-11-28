use std::collections::BTreeMap;

pub use nve_abi::{ColorScheme, ContrastLevel};
use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize};

#[cfg(feature = "layout")]
use crate::layout::IntrinsicSize;
#[cfg(feature = "window")]
pub use crate::window::SurfacePresentation;

/// UI framework hosting or rendered within an orchestrated surface.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, nve_abi::ReprEnum)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkKind {
    Nve = 0,
    ReactNativeFabric = 1,
    Flutter = 2,
    SwiftUI = 3,
    JetpackCompose = 4,
    WebDom = 5,
}

impl FrameworkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nve => "nve",
            Self::ReactNativeFabric => "react_native_fabric",
            Self::Flutter => "flutter",
            Self::SwiftUI => "swiftui",
            Self::JetpackCompose => "jetpack_compose",
            Self::WebDom => "web_dom",
        }
    }

    /// Maps this surface framework to its default [`crate::traits::HostToolkit`] trait value.
    #[cfg(feature = "traits")]
    #[inline]
    pub const fn default_host_toolkit(self) -> crate::traits::HostToolkit {
        match self {
            Self::Nve => crate::traits::HostToolkit::Skia,
            Self::ReactNativeFabric => crate::traits::HostToolkit::UiKit,
            Self::Flutter => crate::traits::HostToolkit::Flutter,
            Self::SwiftUI => crate::traits::HostToolkit::SwiftUi,
            Self::JetpackCompose => crate::traits::HostToolkit::AndroidCompose,
            Self::WebDom => crate::traits::HostToolkit::WebDom,
        }
    }
}

/// Lifecycle state of an orchestrated surface across framework boundaries.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Mounted = 0,
    Paused = 1,
    Resumed = 2,
    Destroyed = 3,
}

impl LifecycleState {
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match (self, next) {
            (Self::Destroyed, _) => false,
            (Self::Mounted, Self::Resumed | Self::Paused | Self::Destroyed) => true,
            (Self::Resumed, Self::Paused | Self::Destroyed) => true,
            (Self::Paused, Self::Resumed | Self::Destroyed) => true,
            _ => false,
        }
    }
}

/// Theme state propagated hierarchically across framework boundaries.
///
/// `color_scheme: None` explicitly means "follow the host system scheme" (never silently coerced
/// to light when applied to a live [`crate::traits::TraitStore`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeContext {
    pub color_scheme: Option<ColorScheme>,
    #[serde(default)]
    pub contrast: ContrastLevel,
    /// Accent color packed as `0xRRGGBBAA`.
    pub accent_rgba: u32,
    /// Arbitrary semantic design tokens (e.g. `"surface.bg" -> "#0F172A"`).
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
}

impl Default for ThemeContext {
    fn default() -> Self {
        Self {
            color_scheme: Some(ColorScheme::Light),
            contrast: ContrastLevel::Standard,
            accent_rgba: 0x2563_EBFF,
            tokens: BTreeMap::new(),
        }
    }
}

impl ThemeContext {
    pub fn dark() -> Self {
        Self {
            color_scheme: Some(ColorScheme::Dark),
            contrast: ContrastLevel::Standard,
            accent_rgba: 0x60A5_FAFF,
            tokens: BTreeMap::new(),
        }
    }

    pub fn system() -> Self {
        Self {
            color_scheme: None,
            contrast: ContrastLevel::Standard,
            accent_rgba: 0x2563_EBFF,
            tokens: BTreeMap::new(),
        }
    }

    /// Resolves the effective [`ColorScheme`] using `system` when `self.color_scheme` is `None`.
    #[inline]
    pub const fn resolve_color_scheme(&self, system: ColorScheme) -> ColorScheme {
        match self.color_scheme {
            Some(scheme) => scheme,
            None => system,
        }
    }

    #[inline]
    pub const fn is_dark(&self) -> bool {
        matches!(self.color_scheme, Some(ColorScheme::Dark))
    }

    #[inline]
    pub const fn is_dark_with_system(&self, system: ColorScheme) -> bool {
        self.resolve_color_scheme(system).is_dark()
    }

    #[inline]
    pub const fn is_high_contrast(&self) -> bool {
        self.contrast.is_high()
    }
}

/// Safe-area insets (in logical points/dp) propagated from host windows into child surfaces.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct SafeAreaInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl SafeAreaInsets {
    pub const ZERO: Self = Self {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    #[inline]
    pub(crate) fn validate(self) -> Result<(), crate::error::InteropError> {
        if self.top.is_finite()
            && self.right.is_finite()
            && self.bottom.is_finite()
            && self.left.is_finite()
        {
            Ok(())
        } else {
            Err(crate::error::InteropError::InvalidEnvironment(
                "safe_area insets must be finite".into(),
            ))
        }
    }
}

/// Inherited environment context propagated across framework boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceEnvironment {
    pub theme: ThemeContext,
    pub safe_area: SafeAreaInsets,
    pub scale_factor: f32,
    pub text_scale_factor: f32,
    pub locale: String,
    pub reduced_motion: bool,
}

impl Default for SurfaceEnvironment {
    fn default() -> Self {
        Self {
            theme: ThemeContext::default(),
            safe_area: SafeAreaInsets::ZERO,
            scale_factor: 1.0,
            text_scale_factor: 1.0,
            locale: "en-US".to_owned(),
            reduced_motion: false,
        }
    }
}

impl SurfaceEnvironment {
    /// Rejects non-finite numeric fields (`NaN`/`±Inf`) at ingress so equality pruning stays sound.
    #[inline]
    pub(crate) fn validate(&self) -> Result<(), crate::error::InteropError> {
        if !self.scale_factor.is_finite() {
            return Err(crate::error::InteropError::InvalidEnvironment(
                "scale_factor must be finite".into(),
            ));
        }
        if !self.text_scale_factor.is_finite() {
            return Err(crate::error::InteropError::InvalidEnvironment(
                "text_scale_factor must be finite".into(),
            ));
        }
        self.safe_area.validate()
    }
}

#[cfg(feature = "traits")]
impl SurfaceEnvironment {
    /// Converts this cross-framework surface environment into a zero-alloc
    /// [`crate::traits::TraitEnvironment`] snapshot, resolving `color_scheme: None` against `system`.
    pub fn to_trait_environment_with_system(
        &self,
        system: ColorScheme,
    ) -> crate::traits::TraitEnvironment {
        let builtins = crate::traits::BuiltinTraits {
            color_scheme: self.theme.resolve_color_scheme(system),
            high_contrast: self.theme.is_high_contrast(),
            safe_area: self.safe_area,
            scale_factor: self.scale_factor,
            text_scale: self.text_scale_factor,
            locale: crate::traits::Locale::new(&self.locale),
            reduced_motion: self.reduced_motion,
            ..crate::traits::BuiltinTraits::default()
        };
        crate::traits::TraitEnvironment::from_builtins(builtins)
    }

    /// Converts this cross-framework surface environment into a zero-alloc
    /// [`crate::traits::TraitEnvironment`] snapshot (falling back to [`ColorScheme::Light`] if
    /// `theme.color_scheme` is `None`).
    pub fn to_trait_environment(&self) -> crate::traits::TraitEnvironment {
        self.to_trait_environment_with_system(ColorScheme::Light)
    }

    /// Applies this surface environment onto a live [`crate::traits::TraitStore`], bumping only
    /// the revision counters for traits whose effective value actually changed.
    ///
    /// When `self.theme.color_scheme` is `None` (follow system), the store's existing host
    /// [`ColorScheme`] is preserved instead of being overwritten with `Light`.
    pub fn apply_to_trait_store(&self, store: &mut crate::traits::TraitStore) {
        if let Some(scheme) = self.theme.color_scheme {
            store.set::<ColorScheme>(scheme);
        }
        store.set::<crate::traits::HighContrast>(self.theme.is_high_contrast());
        store.set::<SafeAreaInsets>(self.safe_area);
        store.set::<crate::traits::ScaleFactor>(self.scale_factor);
        store.set::<crate::traits::TextScale>(self.text_scale_factor);
        store.set::<crate::traits::Locale>(crate::traits::Locale::new(&self.locale));
        store.set::<crate::traits::ReducedMotion>(self.reduced_motion);
    }

    /// Reconstructs a [`SurfaceEnvironment`] from a [`crate::traits::TraitEnvironment`].
    pub fn from_trait_environment(env: &crate::traits::TraitEnvironment) -> Self {
        let b = env.builtins();
        let mut theme = if b.color_scheme.is_dark() {
            ThemeContext::dark()
        } else {
            ThemeContext::default()
        };
        theme.color_scheme = Some(b.color_scheme);
        theme.contrast = if b.high_contrast {
            ContrastLevel::High
        } else {
            ContrastLevel::Standard
        };
        Self {
            theme,
            safe_area: b.safe_area,
            scale_factor: b.scale_factor,
            text_scale_factor: b.text_scale,
            locale: b.locale.as_str().to_owned(),
            reduced_motion: b.reduced_motion,
        }
    }
}

/// Optional per-surface overrides that stop parent propagation for the overridden fields.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EnvironmentOverrides {
    pub theme: Option<ThemeContext>,
    pub safe_area: Option<SafeAreaInsets>,
    pub scale_factor: Option<f32>,
    #[serde(default)]
    pub text_scale_factor: Option<f32>,
    pub locale: Option<String>,
    #[serde(default)]
    pub reduced_motion: Option<bool>,
}

impl EnvironmentOverrides {
    /// Rejects non-finite override values (`NaN`/`±Inf`) at ingress.
    #[inline]
    pub(crate) fn validate(&self) -> Result<(), crate::error::InteropError> {
        if let Some(scale) = self.scale_factor
            && !scale.is_finite()
        {
            return Err(crate::error::InteropError::InvalidEnvironment(
                "override scale_factor must be finite".into(),
            ));
        }
        if let Some(ts) = self.text_scale_factor
            && !ts.is_finite()
        {
            return Err(crate::error::InteropError::InvalidEnvironment(
                "override text_scale_factor must be finite".into(),
            ));
        }
        if let Some(sa) = self.safe_area {
            sa.validate()?;
        }
        Ok(())
    }

    /// Applies any explicitly set override fields onto `env`.
    pub(crate) fn apply_to(&self, env: &mut SurfaceEnvironment) {
        if let Some(theme) = &self.theme {
            env.theme = theme.clone();
        }
        if let Some(safe_area) = self.safe_area {
            env.safe_area = safe_area;
        }
        if let Some(scale) = self.scale_factor {
            env.scale_factor = scale;
        }
        if let Some(text_scale) = self.text_scale_factor {
            env.text_scale_factor = text_scale;
        }
        if let Some(locale) = &self.locale {
            env.locale = locale.clone();
        }
        if let Some(reduced_motion) = self.reduced_motion {
            env.reduced_motion = reduced_motion;
        }
    }

    /// Zero-allocation comparison checking whether `child_env == self.apply(parent_env)`.
    #[inline]
    pub(crate) fn matches_resolved(
        &self,
        child_env: &SurfaceEnvironment,
        parent_env: &SurfaceEnvironment,
    ) -> bool {
        let exp_theme = self.theme.as_ref().unwrap_or(&parent_env.theme);
        let exp_safe = self.safe_area.unwrap_or(parent_env.safe_area);
        let exp_scale = self.scale_factor.unwrap_or(parent_env.scale_factor);
        let exp_text_scale = self.text_scale_factor.unwrap_or(parent_env.text_scale_factor);
        let exp_locale = self.locale.as_deref().unwrap_or(parent_env.locale.as_str());
        let exp_reduced = self.reduced_motion.unwrap_or(parent_env.reduced_motion);
        child_env.theme == *exp_theme
            && child_env.safe_area == exp_safe
            && child_env.scale_factor == exp_scale
            && child_env.text_scale_factor == exp_text_scale
            && child_env.locale == exp_locale
            && child_env.reduced_motion == exp_reduced
    }

    /// Seals `self.apply(parent_env) == target_env` by recording overrides for any field that
    /// differs from `parent_env` (while keeping already-pinned fields updated to `target_env`
    /// without cloning equal `ThemeContext`/`String` values).
    pub(crate) fn seal_against_parent(
        &mut self,
        target_env: &SurfaceEnvironment,
        parent_env: &SurfaceEnvironment,
    ) {
        if self.theme.as_ref() != Some(&target_env.theme)
            && (self.theme.is_some() || target_env.theme != parent_env.theme)
        {
            self.theme = Some(target_env.theme.clone());
        }
        if self.safe_area.is_some() || target_env.safe_area != parent_env.safe_area {
            self.safe_area = Some(target_env.safe_area);
        }
        if self.scale_factor.is_some() || target_env.scale_factor != parent_env.scale_factor {
            self.scale_factor = Some(target_env.scale_factor);
        }
        if self.text_scale_factor.is_some()
            || target_env.text_scale_factor != parent_env.text_scale_factor
        {
            self.text_scale_factor = Some(target_env.text_scale_factor);
        }
        if self.locale.as_deref() != Some(target_env.locale.as_str())
            && (self.locale.is_some() || target_env.locale != parent_env.locale)
        {
            self.locale = Some(target_env.locale.clone());
        }
        if self.reduced_motion.is_some()
            || target_env.reduced_motion != parent_env.reduced_motion
        {
            self.reduced_motion = Some(target_env.reduced_motion);
        }
    }
}

/// Registration parameters for a new surface in [`super::SurfaceOrchestrator`].
///
/// Surface IDs are allocated exclusively by [`super::SurfaceOrchestrator`]'s generational slab
/// allocator (`index:24 | generation:8`) so stale handles can never be resurrected.
#[derive(Debug, Clone)]
pub struct SurfaceDescriptor {
    pub parent: Option<SurfaceId>,
    pub framework: FrameworkKind,
    pub component_name: String,
    pub environment: Option<SurfaceEnvironment>,
    pub overrides: EnvironmentOverrides,
    #[cfg(feature = "layout")]
    pub initial_size: IntrinsicSize,
    #[cfg(feature = "window")]
    pub presentation: SurfacePresentation,
}

impl SurfaceDescriptor {
    pub fn new(framework: FrameworkKind, component_name: impl Into<String>) -> Self {
        Self {
            parent: None,
            framework,
            component_name: component_name.into(),
            environment: None,
            overrides: EnvironmentOverrides::default(),
            #[cfg(feature = "layout")]
            initial_size: IntrinsicSize::ZERO,
            #[cfg(feature = "window")]
            presentation: SurfacePresentation::Embedded,
        }
    }

    pub fn with_parent(mut self, parent: SurfaceId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn with_environment(mut self, env: SurfaceEnvironment) -> Self {
        self.environment = Some(env);
        self
    }

    pub fn with_overrides(mut self, overrides: EnvironmentOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    #[cfg(feature = "layout")]
    pub fn with_initial_size(mut self, size: IntrinsicSize) -> Self {
        self.initial_size = size;
        self
    }

    #[cfg(feature = "window")]
    pub fn with_presentation(mut self, presentation: SurfacePresentation) -> Self {
        self.presentation = presentation;
        self
    }
}

/// Metadata stored for every registered surface in the hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceRecord {
    pub id: SurfaceId,
    pub parent: Option<SurfaceId>,
    pub children: Vec<SurfaceId>,
    pub framework: FrameworkKind,
    pub component_name: String,
    pub state: LifecycleState,
    pub environment: SurfaceEnvironment,
    pub overrides: EnvironmentOverrides,
    #[cfg(feature = "layout")]
    pub viewport_size: IntrinsicSize,
    #[cfg(feature = "window")]
    #[serde(default)]
    pub presentation: SurfacePresentation,
}

/// Record describing one surface torn down during cascading destruction (bottom-up order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeardownRecord {
    pub surface_id: SurfaceId,
    pub framework: FrameworkKind,
    pub component_name: String,
    pub previous_state: LifecycleState,
}

/// Lifecycle/environment notification emitted by [`super::SurfaceOrchestrator`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceEvent {
    Mounted {
        surface_id: SurfaceId,
        parent: Option<SurfaceId>,
        framework: FrameworkKind,
    },
    LifecycleChanged {
        surface_id: SurfaceId,
        from: LifecycleState,
        to: LifecycleState,
    },
    EnvironmentPropagated {
        surface_id: SurfaceId,
        environment: SurfaceEnvironment,
    },
    Reparented {
        surface_id: SurfaceId,
        old_parent: Option<SurfaceId>,
        new_parent: Option<SurfaceId>,
    },
    Destroyed {
        surface_id: SurfaceId,
        framework: FrameworkKind,
    },
}
