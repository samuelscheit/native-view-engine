//! One docs host session, shared by every host path (static SSR, browser hydration, native
//! module): the `nve-system-env` appearance the host publishes, the host services the mounted
//! surface subscribes through, and the user's theme mode shared with the surface.

use std::sync::Arc;

use nve_abi::SurfaceId;
use nve_clipboard::MemoryClipboard;
use nve_dioxus::DioxusSurface;
use nve_docs::{DocsViewport, ThemeModeCell, create_docs_surface_with_pages, docs_theme_config, system_appearance};
use nve_host_services::MemoryHostServices;
use nve_mdx::CompiledSite;
use nve_system_env::{LocaleSnapshot, MemorySystemEnv, NativeSystemEnv, SystemEnvBackend, ThemeSnapshot};
use nve_ui::{Theme, ThemeMode};

use crate::landing::nve_site_custom_pages;
use crate::playgrounds::nve_docs_playground_registry;
use crate::site::nve_docs_site_config_with_theme;

/// `localStorage` key of the persisted user [`ThemeMode`] (read before first paint by
/// [`crate::ssr::THEME_MODE_BOOT_SCRIPT`]).
pub const THEME_MODE_STORAGE_KEY: &str = "nve-docs-theme-mode";

/// A docs host: publish appearance changes with [`Self::publish_theme`], read the user's mode
/// with [`Self::theme_mode`], and mount (or remount) surfaces with [`Self::mount`]. Remounts keep
/// the user's mode because every surface shares the host's [`ThemeModeCell`].
pub struct DocsHost {
    system_env: Arc<MemorySystemEnv>,
    services: Arc<MemoryHostServices>,
    theme_mode: ThemeModeCell,
    default_dark: bool,
}

impl DocsHost {
    /// A host reporting `theme` (a `None` color scheme falls back to `default_dark`).
    pub fn new(theme: ThemeSnapshot, mode: ThemeMode, default_dark: bool) -> Self {
        let system_env = Arc::new(MemorySystemEnv::with_initial(theme, LocaleSnapshot::default()));
        let services = MemoryHostServices::new();
        services.register(MemoryClipboard::new());
        services.register_arc(system_env.clone());
        Self { system_env, services: Arc::new(services), theme_mode: ThemeModeCell::new(mode), default_dark }
    }

    /// A host without an OS appearance (SSR, static builds, tests): `default_dark` decides.
    pub fn headless(default_dark: bool) -> Self {
        Self::new(ThemeSnapshot { color_scheme: None, ..ThemeSnapshot::default() }, ThemeMode::System, default_dark)
    }

    /// A host seeded from the OS appearance (`NativeSystemEnv`); later OS changes arrive through
    /// [`Self::publish_theme`]. Falls back to [`Self::headless`] where the OS reports nothing.
    pub fn native() -> Self {
        match NativeSystemEnv::new().get_theme() {
            Ok(theme) => Self::new(theme, ThemeMode::System, true),
            Err(_) => Self::headless(true),
        }
    }

    /// Publishes a host appearance change to every mounted surface (no rebuild).
    pub fn publish_theme(&self, theme: ThemeSnapshot) {
        self.system_env.set_theme_and_publish(self.services.typed_events(), theme);
    }

    /// The user's color-scheme preference, as last set on a mounted surface.
    pub fn theme_mode(&self) -> ThemeMode {
        self.theme_mode.get()
    }

    /// Sets the user's preference before mounting (e.g. restored from storage).
    pub fn restore_theme_mode(&self, mode: ThemeMode) {
        self.theme_mode.set(mode);
    }

    /// The theme mounted surfaces resolve: user mode over the host appearance.
    pub fn theme(&self) -> Theme {
        docs_theme_config(self.theme_mode()).resolve(system_appearance(self.services.as_ref(), self.default_dark))
    }

    pub fn services(&self) -> &Arc<MemoryHostServices> {
        &self.services
    }

    /// Mounts the official docs app for `site` at `route` on this host.
    pub fn mount(
        &self,
        surface_id: SurfaceId,
        site: Arc<CompiledSite>,
        route: &str,
        base_path: &str,
        viewport: DocsViewport,
    ) -> DioxusSurface {
        create_docs_surface_with_pages(
            surface_id,
            route,
            viewport,
            nve_docs_site_config_with_theme(base_path, self.default_dark).with_theme_mode(self.theme_mode.clone()),
            site,
            nve_docs_playground_registry(),
            nve_site_custom_pages(),
            Arc::clone(&self.services) as _,
        )
    }
}
