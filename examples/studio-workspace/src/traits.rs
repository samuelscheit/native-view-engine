//! Unified Environment [`TraitKey`] definitions and breakpoint-only responsive layout resolution
//! (`packages/core/interop::traits` + `packages/core/dioxus::traits`) for **Aether Studio Workspace**.
//!
//! Replaces ad-hoc manual pixel width checks with [`SizeClass`], [`ColorScheme`], [`UiDensity`],
//! [`Locale`], and Studio custom [`TraitKey`]s ([`StudioAccentTrait`], [`StudioCompactModeTrait`]).

use nve_dioxus::prelude::*;
use nve_dioxus::traits::{use_environment, use_trait, ReactiveTraitStore};
use nve_interop::traits::{
    ColorScheme, HighContrast, Locale, ReducedMotion, ScaleFactor, SizeClass, TextScale,
    TraitEnvironment, TraitKey, TraitOverrides, TraitStore, UiDensity, ViewportChangeMask,
};
use nve_system_env::{ColorScheme as SdkColorScheme, ContrastLevel, LocaleSnapshot, ThemeSnapshot};

use crate::sdk::StudioSystemEnv;
use crate::state::{AccentTheme, DensityMode, WorkspaceSnapshot};
use crate::theme::{LayoutMetrics, BORDER_DEFAULT, CONTROL_BG, TEXT_MUTED};

/// Custom user [`TraitKey`] carrying the active Studio [`AccentTheme`].
///
/// Can be overridden cleanly for any subtree with `<TraitScope overrides: studio_accent_override(...)>`.
pub struct StudioAccentTrait;

impl TraitKey for StudioAccentTrait {
    type Value = AccentTheme;

    fn default_value() -> AccentTheme {
        AccentTheme::Cyan
    }
}

/// Custom user [`TraitKey`] allowing a subtree or preview container to force compact layout mode
/// independently of the root viewport [`SizeClass`].
pub struct StudioCompactModeTrait;

impl TraitKey for StudioCompactModeTrait {
    type Value = bool;

    fn default_value() -> bool {
        false
    }
}

/// Maps Studio [`DensityMode`] onto the built-in [`UiDensity`] environment trait.
pub const fn studio_density_to_trait(density: DensityMode) -> UiDensity {
    match density {
        DensityMode::Normal => UiDensity::Comfortable,
        DensityMode::Compact => UiDensity::Compact,
    }
}

/// Maps an `nve-system-env` [`SdkColorScheme`] snapshot onto the built-in [`ColorScheme`] trait.
pub const fn sdk_color_scheme_to_trait(scheme: SdkColorScheme) -> ColorScheme {
    match scheme {
        SdkColorScheme::Light => ColorScheme::Light,
        SdkColorScheme::Dark | SdkColorScheme::Unspecified => ColorScheme::Dark,
    }
}

/// Maps an `nve-system-env` [`LocaleSnapshot`] onto the inline zero-allocation [`Locale`] trait.
pub fn sdk_locale_to_trait(locale: &LocaleSnapshot) -> Locale {
    Locale::new(&locale.tag)
}

/// Builds a [`TraitOverrides`] layer overriding [`StudioAccentTrait`] for `<TraitScope>`.
pub fn studio_accent_override(accent: AccentTheme) -> TraitOverrides {
    TraitOverrides::single::<StudioAccentTrait>(accent)
}

/// Builds a [`TraitOverrides`] layer overriding [`StudioCompactModeTrait`] for `<TraitScope>`.
pub fn studio_compact_override(compact: bool) -> TraitOverrides {
    TraitOverrides::single::<StudioCompactModeTrait>(compact)
}

/// Breakpoint-resolved layout decisions derived strictly from [`TraitEnvironment::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedStudioLayout {
    pub size_class: SizeClass,
    pub is_mobile: bool,
    pub compact_header: bool,
    pub center_pad: u32,
}

/// Resolves Aether Studio responsive layout from the active [`TraitEnvironment`].
///
/// Uses discrete [`SizeClass`], [`UiDensity`], and [`StudioCompactModeTrait`] via `env.resolve()`
/// so continuous 1px desktop drags within the same breakpoint never churn layout state.
pub fn resolve_studio_layout(env: &TraitEnvironment, m: LayoutMetrics) -> ResolvedStudioLayout {
    let size_class = env.get::<SizeClass>();
    let is_mobile = env
        .resolve::<bool>()
        .when::<StudioCompactModeTrait>(|&forced| forced, true)
        .when_size(SizeClass::Compact, true)
        .default(false);
    let compact_header = env
        .resolve::<bool>()
        .when::<StudioCompactModeTrait>(|&forced| forced, true)
        .when_size(SizeClass::Compact, true)
        .when_size(SizeClass::Medium, true)
        .when_size(SizeClass::Expanded, false)
        .default(false);
    let center_pad = env
        .resolve::<u32>()
        .when_size(SizeClass::Compact, m.main_pad.min(10))
        .when_eq::<UiDensity>(UiDensity::Compact, m.main_pad.min(10))
        .default(m.main_pad);

    ResolvedStudioLayout {
        size_class,
        is_mobile,
        compact_header,
        center_pad,
    }
}

/// Applies Studio snapshot + `nve-system-env` [`ThemeSnapshot`] / [`LocaleSnapshot`] onto a [`TraitStore`].
pub fn apply_studio_traits(
    store: &mut TraitStore,
    snapshot: &WorkspaceSnapshot,
    theme: &ThemeSnapshot,
    locale: &LocaleSnapshot,
) -> ViewportChangeMask {
    let mask = store.update_viewport(
        snapshot.viewport_width as f32,
        snapshot.viewport_height as f32,
    );
    store.set::<ColorScheme>(sdk_color_scheme_to_trait(theme.color_scheme));
    store.set::<HighContrast>(theme.contrast == ContrastLevel::High);
    store.set::<ReducedMotion>(theme.reduced_motion);
    store.set::<ScaleFactor>(theme.scale_factor().max(0.5));
    store.set::<TextScale>(theme.font_scale().max(0.5));
    store.set::<Locale>(sdk_locale_to_trait(locale));
    store.set::<UiDensity>(studio_density_to_trait(snapshot.studio.density));
    store.set::<StudioAccentTrait>(snapshot.studio.accent);
    store.set::<StudioCompactModeTrait>(false);
    mask
}

/// Builds a fresh [`TraitStore`] seeded from the workspace snapshot and synced host system env.
pub fn build_studio_trait_store(
    snapshot: &WorkspaceSnapshot,
    system_env: &StudioSystemEnv,
) -> TraitStore {
    let mut store = TraitStore::new();
    apply_studio_traits(&mut store, snapshot, &system_env.theme, &system_env.locale);
    store
}

/// Synchronizes a Dioxus [`ReactiveTraitStore`] without writing signals for unchanged breakpoints.
pub fn sync_reactive_trait_store(
    reactive: &ReactiveTraitStore,
    snapshot: &WorkspaceSnapshot,
    system_env: &StudioSystemEnv,
) -> ViewportChangeMask {
    let mask = reactive.update_viewport(
        snapshot.viewport_width as f32,
        snapshot.viewport_height as f32,
    );
    reactive.set::<ColorScheme>(sdk_color_scheme_to_trait(system_env.theme.color_scheme));
    reactive.set::<HighContrast>(system_env.theme.contrast == ContrastLevel::High);
    reactive.set::<ReducedMotion>(system_env.theme.reduced_motion);
    reactive.set::<ScaleFactor>(system_env.theme.scale_factor().max(0.5));
    reactive.set::<TextScale>(system_env.theme.font_scale().max(0.5));
    reactive.set::<Locale>(sdk_locale_to_trait(&system_env.locale));
    reactive.set::<UiDensity>(studio_density_to_trait(snapshot.studio.density));
    reactive.set::<StudioAccentTrait>(snapshot.studio.accent);
    mask
}

/// Wrapper around [`TraitStore`] that compares logical trait values for deterministic equality in
/// [`crate::state::WorkspaceState`].
#[derive(Debug, Clone)]
pub struct StudioTraitBox(pub TraitStore);

impl PartialEq for StudioTraitBox {
    fn eq(&self, other: &Self) -> bool {
        self.0.get::<SizeClass>() == other.0.get::<SizeClass>()
            && self.0.get::<ColorScheme>() == other.0.get::<ColorScheme>()
            && self.0.get::<UiDensity>() == other.0.get::<UiDensity>()
            && self.0.get::<Locale>() == other.0.get::<Locale>()
            && self.0.get::<StudioAccentTrait>() == other.0.get::<StudioAccentTrait>()
            && self.0.get::<StudioCompactModeTrait>() == other.0.get::<StudioCompactModeTrait>()
    }
}

impl Eq for StudioTraitBox {}

/// Compact label for [`SizeClass`] displayed in the Studio Environment readout.
pub const fn size_class_label(sc: SizeClass) -> &'static str {
    match sc {
        SizeClass::Compact => "Compact",
        SizeClass::Medium => "Medium",
        SizeClass::Expanded => "Expanded",
    }
}

/// Compact label for [`ColorScheme`] displayed in the Studio Environment readout.
pub const fn color_scheme_label(cs: ColorScheme) -> &'static str {
    match cs {
        ColorScheme::Light => "Light",
        ColorScheme::Dark => "Dark",
    }
}

/// Live trait-aware chip consuming built-in (`SizeClass`, `ColorScheme`, `Locale`) and custom
/// (`StudioAccentTrait`, `StudioCompactModeTrait`) keys via `use_trait` / `use_environment`.
///
/// Works both inside the docked Studio panel and inside `<TraitScope>` override subtrees.
#[component]
pub fn StudioTraitPreviewChip(#[props(default)] test_id: Option<String>) -> Element {
    let size_class = use_trait::<SizeClass>();
    let color_scheme = use_trait::<ColorScheme>();
    let locale = use_trait::<Locale>();
    let accent = use_trait::<StudioAccentTrait>();
    let forced_compact = use_trait::<StudioCompactModeTrait>();
    let env = use_environment();

    let mode_tag = env
        .resolve::<&'static str>()
        .when::<StudioCompactModeTrait>(|&c| c, "CompactScope")
        .when_size(SizeClass::Compact, "Compact")
        .when_size(SizeClass::Medium, "Medium")
        .default("Expanded");

    let accent_hex = accent.hex();
    let accent_lbl = accent.label();
    let scheme_lbl = color_scheme_label(color_scheme);
    let sc_lbl = size_class_label(size_class);
    let loc_str = locale.as_str().to_owned();
    let summary = format!("{sc_lbl} | {scheme_lbl} | {loc_str} | {accent_lbl}");
    let tid = test_id.unwrap_or_else(|| "studio-trait-chip".into());
    let compact_attr = if forced_compact { "true" } else { "false" };

    rsx! {
        view {
            test_id: "{tid}",
            nve_table: {
                let pairs = vec![
                    ("trait-size-class".to_owned(), sc_lbl.to_owned()),
                    ("trait-mode".to_owned(), mode_tag.to_owned()),
                    ("trait-accent".to_owned(), accent_lbl.to_owned()),
                    ("trait-compact-scope".to_owned(), compact_attr.to_owned()),
                ];
                nve_dioxus::table_attr_fn(move |tx, node| {
                    tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                        nve_web_html::DataAttrList::from_pairs(pairs.clone()),
                    ));
                })
            },
            style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 5px; padding-vertical: 3px; background-color: {CONTROL_BG}; border-radius: 6px; border-width: 1px; border-color: {BORDER_DEFAULT};",
            text {
                style: "color: {accent_hex}; font-size: 9px;",
                "{summary}"
            }
            text {
                style: "color: {TEXT_MUTED}; font-size: 8px;",
                "{mode_tag}"
            }
        }
    }
}
