//! Zero-allocation short-circuiting conditional [`TraitResolver`].

use super::builtin::TraitKey;
use super::environment::TraitEnvironment;
use super::types::{
    ColorScheme, DeviceFormFactor, HostToolkit, Orientation, PlatformOs, PointerKind, SizeClass,
    UiDensity, VerticalSizeClass,
};

/// Zero-allocation short-circuiting conditional trait resolver created by [`TraitEnvironment::resolve`].
#[must_use = "call `.default(fallback)` or `.finish()` to obtain the resolved value"]
pub struct TraitResolver<'a, T> {
    pub(crate) env: &'a TraitEnvironment,
    pub(crate) resolved: Option<T>,
}

impl<'a, T> TraitResolver<'a, T> {
    #[inline]
    pub fn when_os(mut self, os: PlatformOs, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.platform_os == os {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_os_with(mut self, os: PlatformOs, f: impl FnOnce() -> T) -> Self {
        if self.resolved.is_none() && self.env.builtins.platform_os == os {
            self.resolved = Some(f());
        }
        self
    }

    #[inline]
    pub fn when_toolkit(mut self, toolkit: HostToolkit, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.host_toolkit == toolkit {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_form_factor(mut self, factor: DeviceFormFactor, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.device_form_factor == factor {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_tv(self, value: T) -> Self {
        self.when_form_factor(DeviceFormFactor::Tv, value)
    }

    #[inline]
    pub fn when_watch(self, value: T) -> Self {
        self.when_form_factor(DeviceFormFactor::Watch, value)
    }

    #[inline]
    pub fn when_spatial(self, value: T) -> Self {
        self.when_form_factor(DeviceFormFactor::Spatial, value)
    }

    #[inline]
    pub fn when_size(mut self, size: SizeClass, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.size_class == size {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_size_with(mut self, size: SizeClass, f: impl FnOnce() -> T) -> Self {
        if self.resolved.is_none() && self.env.builtins.size_class == size {
            self.resolved = Some(f());
        }
        self
    }

    #[inline]
    pub fn when_vertical_size(mut self, size: SizeClass, value: T) -> Self {
        if self.resolved.is_none() && self.env.get::<VerticalSizeClass>() == size {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_container_size(mut self, size: SizeClass, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.container_size.class == size {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_orientation(mut self, orientation: Orientation, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.orientation == orientation {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_color_scheme(mut self, scheme: ColorScheme, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.color_scheme == scheme {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_dark(self, value: T) -> Self {
        self.when_color_scheme(ColorScheme::Dark, value)
    }

    #[inline]
    pub fn when_light(self, value: T) -> Self {
        self.when_color_scheme(ColorScheme::Light, value)
    }

    #[inline]
    pub fn when_pointer(mut self, pointer: PointerKind, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.pointer_kind == pointer {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_density(mut self, density: UiDensity, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.ui_density == density {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_reduced_motion(mut self, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.reduced_motion {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_high_contrast(mut self, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.high_contrast {
            self.resolved = Some(value);
        }
        self
    }

    #[inline]
    pub fn when_rtl(mut self, value: T) -> Self {
        if self.resolved.is_none() && self.env.builtins.locale.is_rtl() {
            self.resolved = Some(value);
        }
        self
    }

    /// Evaluates `pred` against trait `K` only if no earlier rule has matched (strict short-circuit).
    #[inline]
    pub fn when<K: TraitKey>(mut self, pred: impl FnOnce(&K::Value) -> bool, value: T) -> Self {
        if self.resolved.is_none() {
            let current = self.env.get::<K>();
            if pred(&current) {
                self.resolved = Some(value);
            }
        }
        self
    }

    /// Evaluates `pred` and lazily computes `f` only if no earlier rule has matched.
    #[inline]
    pub fn when_with<K: TraitKey>(
        mut self,
        pred: impl FnOnce(&K::Value) -> bool,
        f: impl FnOnce() -> T,
    ) -> Self {
        if self.resolved.is_none() {
            let current = self.env.get::<K>();
            if pred(&current) {
                self.resolved = Some(f());
            }
        }
        self
    }

    /// Matches when trait `K` equals `expected`.
    #[inline]
    pub fn when_eq<K: TraitKey>(mut self, expected: K::Value, value: T) -> Self {
        if self.resolved.is_none() && self.env.get::<K>() == expected {
            self.resolved = Some(value);
        }
        self
    }

    /// Finishes the resolver chain, returning `fallback` if no rule matched.
    #[inline]
    pub fn default(self, fallback: T) -> T {
        self.resolved.unwrap_or(fallback)
    }

    /// Finishes the resolver chain, lazily computing `fallback` only if no rule matched.
    #[inline]
    pub fn default_with(self, fallback: impl FnOnce() -> T) -> T {
        self.resolved.unwrap_or_else(fallback)
    }

    /// Returns `Some(matched)` or `None` if no rule matched.
    #[inline]
    pub fn finish(self) -> Option<T> {
        self.resolved
    }
}
