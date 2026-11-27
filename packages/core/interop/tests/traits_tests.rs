//! Deep unit & edge-case tests for the Unified Extensible Environment (`TraitKey`,
//! `TraitEnvironment`, `TraitOverrides`, `TraitResolver`, `TraitStore`).

use std::cell::Cell;
use nve_interop::surface::{
    ColorScheme as SurfaceColorScheme, FrameworkKind, SafeAreaInsets, SurfaceEnvironment,
    ThemeContext,
};
use nve_interop::traits::{
    BuiltinTraitSlot, BuiltinTraits, CanHover, ColorScheme, ContainerSize, DeviceFormFactor,
    HasHardwareKeyboard, HighContrast, HostToolkit, Locale, Orientation, PlatformOs, PointerKind,
    ReducedMotion, ScaleFactor, SizeClass, TextScale, TraitEnvironment, TraitKey, TraitOverrides,
    TraitStore, UiDensity, VerticalSizeClass, BUILTIN_TRAIT_COUNT,
};

struct BrandAccent;
impl TraitKey for BrandAccent {
    type Value = u32;
    fn default_value() -> u32 {
        0x2563_EBFF
    }
}

struct FeatureTier;
impl TraitKey for FeatureTier {
    type Value = String;
    fn default_value() -> String {
        "free".to_owned()
    }
}

struct MaxColumns;
impl TraitKey for MaxColumns {
    type Value = usize;
    fn default_value() -> usize {
        1
    }
}

#[test]
fn builtin_traits_o1_access_locale_rtl_and_size_class_thresholds() {
    // Exact breakpoint boundaries for SizeClass
    assert_eq!(SizeClass::from_width(-10.0), SizeClass::Compact);
    assert_eq!(SizeClass::from_width(0.0), SizeClass::Compact);
    assert_eq!(SizeClass::from_width(599.99), SizeClass::Compact);
    assert_eq!(SizeClass::from_width(600.0), SizeClass::Medium);
    assert_eq!(SizeClass::from_width(1023.99), SizeClass::Medium);
    assert_eq!(SizeClass::from_width(1024.0), SizeClass::Expanded);
    assert_eq!(SizeClass::from_width(f32::NAN), SizeClass::Compact);
    assert_eq!(SizeClass::from_width(f32::INFINITY), SizeClass::Compact);

    // Locale inline zero-alloc storage, RTL detection, and multibyte truncation safety
    assert!(!Locale::EN_US.is_rtl());
    assert_eq!(Locale::EN_US.as_str(), "en-US");
    assert_eq!(Locale::new("   ").as_str(), "en-US");
    assert!(Locale::new("ar-SA").is_rtl());
    assert!(Locale::new("HE_IL").is_rtl());
    assert!(Locale::new("fa-IR").is_rtl());
    assert!(Locale::new("ur").is_rtl());
    assert!(!Locale::new("ja-JP").is_rtl());
    assert!(!Locale::new("fr-FR").is_rtl());
    let long_utf8 = Locale::new("zh-Hant-TW-🚀-extra-long-tag-over-23-bytes");
    assert!(long_utf8.as_str().len() <= 23);
    assert!(!long_utf8.is_rtl());

    // All 17 built-in traits round-trip cleanly through TraitEnvironment
    let env = TraitEnvironment::new()
        .with::<PlatformOs>(PlatformOs::Ios)
        .with::<HostToolkit>(HostToolkit::UiKit)
        .with::<SizeClass>(SizeClass::Medium)
        .with::<VerticalSizeClass>(SizeClass::Expanded)
        .with::<ContainerSize>(ContainerSize::new(720.0, 480.0))
        .with::<Orientation>(Orientation::Landscape)
        .with::<SafeAreaInsets>(SafeAreaInsets::new(59.0, 0.0, 34.0, 0.0))
        .with::<ScaleFactor>(3.0)
        .with::<PointerKind>(PointerKind::Coarse)
        .with::<CanHover>(false)
        .with::<HasHardwareKeyboard>(true)
        .with::<ColorScheme>(ColorScheme::Dark)
        .with::<ReducedMotion>(true)
        .with::<HighContrast>(true)
        .with::<TextScale>(1.35)
        .with::<UiDensity>(UiDensity::Spacious)
        .with::<Locale>(Locale::new("ar-AE"));

    assert_eq!(env.get::<PlatformOs>(), PlatformOs::Ios);
    assert_eq!(env.get::<HostToolkit>(), HostToolkit::UiKit);
    assert_eq!(env.get::<SizeClass>(), SizeClass::Medium);
    assert_eq!(env.get::<VerticalSizeClass>(), SizeClass::Expanded);
    assert_eq!(env.get::<ContainerSize>(), ContainerSize::new(720.0, 480.0));
    assert_eq!(env.get::<ContainerSize>().class, SizeClass::Medium);
    assert_eq!(env.get::<Orientation>(), Orientation::Landscape);
    assert_eq!(
        env.get::<SafeAreaInsets>(),
        SafeAreaInsets::new(59.0, 0.0, 34.0, 0.0)
    );
    assert_eq!(env.get::<ScaleFactor>(), 3.0);
    assert_eq!(env.get::<PointerKind>(), PointerKind::Coarse);
    assert!(!env.get::<CanHover>());
    assert!(env.get::<HasHardwareKeyboard>());
    assert_eq!(env.get::<ColorScheme>(), ColorScheme::Dark);
    assert!(env.get::<ReducedMotion>());
    assert!(env.get::<HighContrast>());
    assert_eq!(env.get::<TextScale>(), 1.35);
    assert_eq!(env.get::<UiDensity>(), UiDensity::Spacious);
    assert_eq!(env.get::<Locale>().as_str(), "ar-AE");
    assert!(env.get::<Locale>().is_rtl());
}

#[test]
fn custom_trait_key_isolation_cow_inheritance_and_overrides_merge() {
    let base = TraitEnvironment::new()
        .with::<ColorScheme>(ColorScheme::Light)
        .with::<BrandAccent>(0xFF00_88FF);

    // Derived scopes inherit base custom keys and shadow independently without mutating parent.
    let branch_a = base
        .with::<FeatureTier>("pro".to_owned())
        .with::<BrandAccent>(0x00FF_AAFF);
    let branch_b = base.with::<MaxColumns>(4);

    assert_eq!(base.get::<BrandAccent>(), 0xFF00_88FF);
    assert_eq!(base.get::<FeatureTier>(), "free");
    assert_eq!(base.get::<MaxColumns>(), 1);

    assert_eq!(branch_a.get::<BrandAccent>(), 0x00FF_AAFF);
    assert_eq!(branch_a.get::<FeatureTier>(), "pro");
    assert_eq!(branch_a.get::<MaxColumns>(), 1);

    assert_eq!(branch_b.get::<BrandAccent>(), 0xFF00_88FF);
    assert_eq!(branch_b.get::<FeatureTier>(), "free");
    assert_eq!(branch_b.get::<MaxColumns>(), 4);

    // TraitOverrides merge & equality
    let outer = TraitOverrides::new()
        .with::<ColorScheme>(ColorScheme::Dark)
        .with::<BrandAccent>(0x1111_11FF)
        .with::<FeatureTier>("team".to_owned());
    let inner = TraitOverrides::new()
        .with::<BrandAccent>(0x2222_22FF)
        .with::<UiDensity>(UiDensity::Compact);

    let merged = outer.merge(&inner);
    assert_eq!(merged.get::<ColorScheme>(), Some(ColorScheme::Dark));
    assert_eq!(merged.get::<BrandAccent>(), Some(0x2222_22FF));
    assert_eq!(merged.get::<FeatureTier>().as_deref(), Some("team"));
    assert_eq!(merged.get::<UiDensity>(), Some(UiDensity::Compact));
    assert_eq!(merged.get::<MaxColumns>(), None);

    let applied = base.with_overrides(&merged);
    assert_eq!(applied.get::<ColorScheme>(), ColorScheme::Dark);
    assert_eq!(applied.get::<BrandAccent>(), 0x2222_22FF);
    assert_eq!(applied.get::<FeatureTier>(), "team");
    assert_eq!(applied.get::<UiDensity>(), UiDensity::Compact);

    // Equality distinguishes different custom values even with same TypeId set.
    let same_merged = outer.merge(&inner);
    let diff_merged = outer.merge(&TraitOverrides::single::<BrandAccent>(0x3333_33FF));
    assert_eq!(merged, same_merged);
    assert_ne!(merged, diff_merged);
}

#[test]
fn trait_resolver_priority_and_strict_short_circuiting() {
    let env = TraitEnvironment::new()
        .with::<PlatformOs>(PlatformOs::Ios)
        .with::<SizeClass>(SizeClass::Compact)
        .with::<FeatureTier>("enterprise".to_owned())
        .with::<ColorScheme>(ColorScheme::Dark);

    let later_pred_calls = Cell::new(0usize);
    let later_value_calls = Cell::new(0usize);
    let fallback_calls = Cell::new(0usize);

    let padding = env
        .resolve::<f32>()
        .when_os(PlatformOs::Android, 24.0)
        .when_os(PlatformOs::Ios, 16.0)
        .when_size(SizeClass::Compact, 8.0)
        .when::<FeatureTier>(
            |tier| {
                later_pred_calls.set(later_pred_calls.get() + 1);
                tier == "enterprise"
            },
            32.0,
        )
        .when_with::<ColorScheme>(
            |_| {
                later_pred_calls.set(later_pred_calls.get() + 1);
                true
            },
            || {
                later_value_calls.set(later_value_calls.get() + 1);
                99.0
            },
        )
        .default_with(|| {
            fallback_calls.set(fallback_calls.get() + 1);
            12.0
        });

    assert_eq!(padding, 16.0);
    assert_eq!(
        later_pred_calls.get(),
        0,
        "short-circuiting must skip all subsequent predicates after first match"
    );
    assert_eq!(later_value_calls.get(), 0);
    assert_eq!(fallback_calls.get(), 0);

    // When earlier rules miss, custom predicate matches and fallback is skipped.
    let cols = env
        .resolve::<usize>()
        .when_os(PlatformOs::Windows, 4)
        .when_size(SizeClass::Expanded, 3)
        .when::<FeatureTier>(|t| t == "enterprise", 6)
        .default(1);
    assert_eq!(cols, 6);

    // When nothing matches, default is returned.
    let missed = env
        .resolve::<&'static str>()
        .when_os(PlatformOs::Linux, "linux")
        .when_size(SizeClass::Expanded, "expanded")
        .when::<MaxColumns>(|&c| c > 2, "many")
        .default("fallback");
    assert_eq!(missed, "fallback");
}

#[test]
fn trait_store_breakpoint_only_revisions_and_per_key_isolation() {
    let mut store = TraitStore::new();

    // Initial desktop viewport: 1200x800 => Expanded width, Medium height, Landscape orientation
    let init = store.update_viewport(1200.0, 800.0);
    assert!(init.size_class_changed);
    assert!(init.vertical_size_class_changed);
    assert!(init.orientation_changed);
    assert!(init.container_size_changed);
    assert_eq!(store.get::<SizeClass>(), SizeClass::Expanded);
    assert_eq!(store.get::<VerticalSizeClass>(), SizeClass::Medium);
    assert_eq!(store.get::<Orientation>(), Orientation::Landscape);

    let sc_rev = store.revision::<SizeClass>();
    let vsc_rev = store.revision::<VerticalSizeClass>();
    let ori_rev = store.revision::<Orientation>();
    let cs_rev = store.revision::<ContainerSize>();
    let theme_rev = store.revision::<ColorScheme>();

    // 1px drag: 1200x800 -> 1201x800 must NOT bump SizeClass, VerticalSizeClass, or Orientation!
    let drag = store.update_viewport(1201.0, 800.0);
    assert!(!drag.any_breakpoint_changed());
    assert!(!drag.size_class_changed);
    assert!(!drag.vertical_size_class_changed);
    assert!(!drag.orientation_changed);
    assert!(drag.container_size_changed);
    assert_eq!(store.revision::<SizeClass>(), sc_rev);
    assert_eq!(store.revision::<VerticalSizeClass>(), vsc_rev);
    assert_eq!(store.revision::<Orientation>(), ori_rev);
    assert_eq!(store.revision::<ColorScheme>(), theme_rev);
    assert_eq!(store.revision::<ContainerSize>(), cs_rev + 1);

    // Same exact viewport again bumps NOTHING (not even ContainerSize).
    let noop = store.update_viewport(1201.0, 800.0);
    assert!(!noop.any_changed());
    assert_eq!(store.revision::<ContainerSize>(), cs_rev + 1);

    // Shrink to 599x800 => Compact width, Medium height, Portrait orientation.
    let mobile = store.update_viewport(599.0, 800.0);
    assert!(mobile.size_class_changed);
    assert!(!mobile.vertical_size_class_changed);
    assert!(mobile.orientation_changed);
    assert_eq!(store.get::<SizeClass>(), SizeClass::Compact);
    assert_eq!(store.get::<Orientation>(), Orientation::Portrait);
    let rev_599 = store.revision::<SizeClass>();
    let ori_599 = store.revision::<Orientation>();

    // Cross 599 -> 600 boundary => DOES bump SizeClass, does NOT bump Orientation!
    let cross = store.update_viewport(600.0, 800.0);
    assert!(cross.size_class_changed);
    assert!(!cross.orientation_changed);
    assert_eq!(store.get::<SizeClass>(), SizeClass::Medium);
    assert_eq!(store.revision::<SizeClass>(), rev_599 + 1);
    assert_eq!(store.revision::<Orientation>(), ori_599);

    // Custom TraitKey per-key revision isolation via set & apply_overrides
    assert!(store.set::<BrandAccent>(0xAAAA_AAFF));
    assert!(store.set::<FeatureTier>("pro".to_owned()));
    let accent_rev = store.revision::<BrandAccent>();
    let tier_rev = store.revision::<FeatureTier>();

    // Setting same value is a no-op.
    assert!(!store.set::<BrandAccent>(0xAAAA_AAFF));
    assert_eq!(store.revision::<BrandAccent>(), accent_rev);

    // Batch override changing only FeatureTier must NOT bump BrandAccent's revision.
    let batch = TraitOverrides::new()
        .with::<BrandAccent>(0xAAAA_AAFF)
        .with::<FeatureTier>("enterprise".to_owned());
    let (changed_builtins, changed_custom) = store.apply_overrides(&batch);
    assert_eq!(changed_builtins, 0);
    assert_eq!(changed_custom.len(), 1);
    assert_eq!(store.revision::<BrandAccent>(), accent_rev);
    assert_eq!(store.revision::<FeatureTier>(), tier_rev + 1);
}

#[test]
fn surface_environment_bridges_cleanly_to_and_from_trait_environment() {
    let surface_env = SurfaceEnvironment {
        theme: ThemeContext {
            color_scheme: SurfaceColorScheme::HighContrastDark,
            accent_rgba: 0x60A5_FAFF,
            tokens: Default::default(),
        },
        safe_area: SafeAreaInsets::new(47.0, 0.0, 34.0, 0.0),
        scale_factor: 3.0,
        text_scale_factor: 1.25,
        locale: "he-IL".to_owned(),
        reduced_motion: true,
    };

    let trait_env = surface_env.to_trait_environment();
    assert_eq!(trait_env.get::<ColorScheme>(), ColorScheme::Dark);
    assert!(trait_env.get::<HighContrast>());
    assert_eq!(trait_env.get::<SafeAreaInsets>(), surface_env.safe_area);
    assert_eq!(trait_env.get::<ScaleFactor>(), 3.0);
    assert_eq!(trait_env.get::<TextScale>(), 1.25);
    assert_eq!(trait_env.get::<Locale>().as_str(), "he-IL");
    assert!(trait_env.get::<Locale>().is_rtl());
    assert!(trait_env.get::<ReducedMotion>());

    let roundtripped = SurfaceEnvironment::from_trait_environment(&trait_env);
    assert_eq!(
        roundtripped.theme.color_scheme,
        SurfaceColorScheme::HighContrastDark
    );
    assert_eq!(roundtripped.safe_area, surface_env.safe_area);
    assert_eq!(roundtripped.scale_factor, 3.0);
    assert_eq!(roundtripped.text_scale_factor, 1.25);
    assert_eq!(roundtripped.locale, "he-IL");
    assert!(roundtripped.reduced_motion);

    let mut store = TraitStore::from_builtins(BuiltinTraits::default());
    surface_env.apply_to_trait_store(&mut store);
    assert_eq!(store.get::<ColorScheme>(), ColorScheme::Dark);
    assert!(store.get::<HighContrast>());
    assert_eq!(store.get::<Locale>().as_str(), "he-IL");

    assert_eq!(FrameworkKind::WebDom.default_host_toolkit(), HostToolkit::WebDom);
    assert_eq!(FrameworkKind::SwiftUI.default_host_toolkit(), HostToolkit::SwiftUi);
    assert_eq!(
        FrameworkKind::JetpackCompose.default_host_toolkit(),
        HostToolkit::AndroidCompose
    );
}


#[test]
fn tv_watch_spatial_platform_os_form_factor_and_pointer_discriminants() {
    // Existing PlatformOs discriminants 0..=6 preserved + new 7..=10 variants
    assert_eq!(PlatformOs::MacOs as u8, 0);
    assert_eq!(PlatformOs::Ios as u8, 1);
    assert_eq!(PlatformOs::Windows as u8, 2);
    assert_eq!(PlatformOs::Android as u8, 3);
    assert_eq!(PlatformOs::Linux as u8, 4);
    assert_eq!(PlatformOs::Web as u8, 5);
    assert_eq!(PlatformOs::Unknown as u8, 6);
    assert_eq!(PlatformOs::TvOs as u8, 7);
    assert_eq!(PlatformOs::WatchOs as u8, 8);
    assert_eq!(PlatformOs::VisionOs as u8, 9);
    assert_eq!(PlatformOs::AndroidTv as u8, 10);

    // DeviceFormFactor discriminants and default
    assert_eq!(DeviceFormFactor::default(), DeviceFormFactor::Desktop);
    assert_eq!(DeviceFormFactor::Phone as u8, 0);
    assert_eq!(DeviceFormFactor::Tablet as u8, 1);
    assert_eq!(DeviceFormFactor::Desktop as u8, 2);
    assert_eq!(DeviceFormFactor::Tv as u8, 3);
    assert_eq!(DeviceFormFactor::Watch as u8, 4);
    assert_eq!(DeviceFormFactor::Spatial as u8, 5);

    // PointerKind discriminants
    assert_eq!(PointerKind::Fine as u8, 0);
    assert_eq!(PointerKind::Coarse as u8, 1);
    assert_eq!(PointerKind::None as u8, 2);
    assert_eq!(PointerKind::Remote as u8, 3);
    assert_eq!(PointerKind::Crown as u8, 4);
    assert_eq!(PointerKind::Gaze as u8, 5);

    // Classification helpers
    for os in [
        PlatformOs::MacOs,
        PlatformOs::Ios,
        PlatformOs::TvOs,
        PlatformOs::WatchOs,
        PlatformOs::VisionOs,
    ] {
        assert!(os.is_apple(), "{os:?} must be classified as Apple");
    }
    for os in [
        PlatformOs::Windows,
        PlatformOs::Android,
        PlatformOs::AndroidTv,
        PlatformOs::Linux,
        PlatformOs::Web,
        PlatformOs::Unknown,
    ] {
        assert!(!os.is_apple(), "{os:?} must not be classified as Apple");
    }

    assert!(PlatformOs::TvOs.is_tv());
    assert!(PlatformOs::AndroidTv.is_tv());
    assert!(!PlatformOs::Android.is_tv());
    assert!(!PlatformOs::Ios.is_tv());

    assert!(PlatformOs::WatchOs.is_watch());
    assert!(!PlatformOs::Ios.is_watch());

    assert!(PlatformOs::VisionOs.is_spatial());
    assert!(!PlatformOs::MacOs.is_spatial());

    // Default form factor mapping
    assert_eq!(PlatformOs::Ios.default_form_factor(), DeviceFormFactor::Phone);
    assert_eq!(
        PlatformOs::Android.default_form_factor(),
        DeviceFormFactor::Phone
    );
    assert_eq!(
        PlatformOs::MacOs.default_form_factor(),
        DeviceFormFactor::Desktop
    );
    assert_eq!(
        PlatformOs::Windows.default_form_factor(),
        DeviceFormFactor::Desktop
    );
    assert_eq!(
        PlatformOs::Linux.default_form_factor(),
        DeviceFormFactor::Desktop
    );
    assert_eq!(
        PlatformOs::Web.default_form_factor(),
        DeviceFormFactor::Desktop
    );
    assert_eq!(
        PlatformOs::Unknown.default_form_factor(),
        DeviceFormFactor::Desktop
    );
    assert_eq!(PlatformOs::TvOs.default_form_factor(), DeviceFormFactor::Tv);
    assert_eq!(
        PlatformOs::AndroidTv.default_form_factor(),
        DeviceFormFactor::Tv
    );
    assert_eq!(
        PlatformOs::WatchOs.default_form_factor(),
        DeviceFormFactor::Watch
    );
    assert_eq!(
        PlatformOs::VisionOs.default_form_factor(),
        DeviceFormFactor::Spatial
    );
}

#[test]
fn builtin_traits_19_slots_host_defaults_and_resolver_form_factors() {
    assert_eq!(BUILTIN_TRAIT_COUNT, 19);
    assert_eq!(BuiltinTraitSlot::ALL.len(), 19);
    assert_eq!(BuiltinTraitSlot::DeviceFormFactor.index(), 17);
    assert_eq!(BuiltinTraitSlot::DeviceFormFactor.bit(), 1u32 << 17);
    assert_eq!(BuiltinTraitSlot::KeyboardInsets.index(), 18);
    assert_eq!(BuiltinTraitSlot::KeyboardInsets.bit(), 1u32 << 18);

    // KeyboardInsets integrates into TraitStore with per-trait revision tracking and overrides.
    let mut kb_store = TraitStore::new();
    assert_eq!(kb_store.get::<nve_interop::KeyboardInsets>(), nve_interop::KeyboardInsets::HIDDEN);
    assert_eq!(kb_store.revision::<nve_interop::KeyboardInsets>(), 0);
    let shown = nve_interop::KeyboardInsets::new(336.0, true, 250, true);
    assert!(kb_store.set::<nve_interop::KeyboardInsets>(shown));
    assert_eq!(kb_store.revision::<nve_interop::KeyboardInsets>(), 1);
    assert!(!kb_store.set::<nve_interop::KeyboardInsets>(shown));
    assert_eq!(kb_store.revision::<nve_interop::KeyboardInsets>(), 1);
    assert_eq!(
        nve_interop::effective_bottom_inset(34.0, kb_store.get::<nve_interop::KeyboardInsets>(), false),
        336.0
    );
    assert_eq!(
        nve_interop::effective_bottom_inset(34.0, kb_store.get::<nve_interop::KeyboardInsets>(), true),
        0.0
    );

    // Host input & form-factor defaults per platform
    let tvos = BuiltinTraits::for_platform(PlatformOs::TvOs, HostToolkit::SwiftUi);
    assert_eq!(tvos.device_form_factor, DeviceFormFactor::Tv);
    assert_eq!(tvos.pointer_kind, PointerKind::Remote);
    assert!(!tvos.can_hover);
    assert!(!tvos.has_hardware_keyboard);

    let android_tv = BuiltinTraits::for_platform(PlatformOs::AndroidTv, HostToolkit::AndroidCompose);
    assert_eq!(android_tv.device_form_factor, DeviceFormFactor::Tv);
    assert_eq!(android_tv.pointer_kind, PointerKind::Remote);
    assert!(!android_tv.can_hover);
    assert!(!android_tv.has_hardware_keyboard);

    let watchos = BuiltinTraits::for_platform(PlatformOs::WatchOs, HostToolkit::SwiftUi);
    assert_eq!(watchos.device_form_factor, DeviceFormFactor::Watch);
    assert_eq!(watchos.pointer_kind, PointerKind::Crown);
    assert!(!watchos.can_hover);
    assert!(!watchos.has_hardware_keyboard);

    let visionos = BuiltinTraits::for_platform(PlatformOs::VisionOs, HostToolkit::SwiftUi);
    assert_eq!(visionos.device_form_factor, DeviceFormFactor::Spatial);
    assert_eq!(visionos.pointer_kind, PointerKind::Gaze);
    assert!(visionos.can_hover);
    assert!(!visionos.has_hardware_keyboard);

    let current = BuiltinTraits::for_current_host(HostToolkit::Headless);
    assert_eq!(current.platform_os, PlatformOs::current());
    assert_eq!(
        current.device_form_factor,
        PlatformOs::current().default_form_factor()
    );

    // TraitResolver when_form_factor / when_tv / when_watch / when_spatial + short-circuiting
    let tv_env = TraitEnvironment::from_builtins(tvos);
    assert_eq!(
        tv_env
            .resolve::<&'static str>()
            .when_watch("watch")
            .when_spatial("spatial")
            .when_tv("tv-10ft")
            .default("other"),
        "tv-10ft"
    );

    let watch_env = TraitEnvironment::from_builtins(watchos);
    assert_eq!(
        watch_env
            .resolve::<u32>()
            .when_tv(48)
            .when_watch(12)
            .when_spatial(36)
            .default(16),
        12
    );

    let spatial_env = TraitEnvironment::from_builtins(visionos);
    assert_eq!(
        spatial_env
            .resolve::<&'static str>()
            .when_form_factor(DeviceFormFactor::Tablet, "tablet")
            .when_spatial("glass-depth")
            .when_form_factor(DeviceFormFactor::Spatial, "later-unused")
            .default("flat"),
        "glass-depth"
    );

    let tablet_env = TraitEnvironment::new().with::<DeviceFormFactor>(DeviceFormFactor::Tablet);
    assert_eq!(
        tablet_env
            .resolve::<&'static str>()
            .when_tv("tv")
            .when_watch("watch")
            .when_spatial("spatial")
            .when_form_factor(DeviceFormFactor::Tablet, "split-pane")
            .default("default"),
        "split-pane"
    );

    // TraitStore revision isolation & TraitOverrides roundtrip for DeviceFormFactor
    let mut store = TraitStore::new();
    assert_eq!(store.get::<DeviceFormFactor>(), DeviceFormFactor::Desktop);
    assert_eq!(store.revision::<DeviceFormFactor>(), 0);

    // No-op setting same value must not bump revision
    assert!(!store.set::<DeviceFormFactor>(DeviceFormFactor::Desktop));
    assert_eq!(store.revision::<DeviceFormFactor>(), 0);

    let os_rev_before = store.revision::<PlatformOs>();
    assert!(store.set::<DeviceFormFactor>(DeviceFormFactor::Tv));
    assert_eq!(store.get::<DeviceFormFactor>(), DeviceFormFactor::Tv);
    assert_eq!(store.revision::<DeviceFormFactor>(), 1);
    assert_eq!(store.revision::<PlatformOs>(), os_rev_before);

    // TraitOverrides merge & apply_overrides on DeviceFormFactor
    let ov_outer = TraitOverrides::new()
        .with::<DeviceFormFactor>(DeviceFormFactor::Watch)
        .with::<PointerKind>(PointerKind::Crown);
    let ov_inner = TraitOverrides::single::<DeviceFormFactor>(DeviceFormFactor::Spatial);
    let merged = ov_outer.merge(&ov_inner);
    assert_eq!(merged.get::<DeviceFormFactor>(), Some(DeviceFormFactor::Spatial));
    assert_eq!(merged.get::<PointerKind>(), Some(PointerKind::Crown));

    let (changed_mask, changed_custom) = store.apply_overrides(&merged);
    assert!(changed_custom.is_empty());
    assert_ne!(changed_mask & BuiltinTraitSlot::DeviceFormFactor.bit(), 0);
    assert_ne!(changed_mask & BuiltinTraitSlot::PointerKind.bit(), 0);
    assert_eq!(store.get::<DeviceFormFactor>(), DeviceFormFactor::Spatial);
    assert_eq!(store.get::<PointerKind>(), PointerKind::Crown);
    assert_eq!(store.revision::<DeviceFormFactor>(), 2);

    // Applying identical overrides again changes nothing
    let (noop_mask, _) = store.apply_overrides(&merged);
    assert_eq!(noop_mask, 0);
    assert_eq!(store.revision::<DeviceFormFactor>(), 2);
}
