//! Synchronized `nve-router` navigation, unified Environment `TraitStore` queries, and
//! `nve-sdk` Clipboard / SystemEnv actions for [`WorkspaceState`].

use nve_interop::traits::{
    ColorScheme, Locale, SizeClass, TraitEnvironment, TraitStore, ViewportChangeMask,
};
use nve_router::Routable;
use nve_sdk::SdkError;

use crate::routes::StudioRoute;
use crate::sdk::{StudioHostServices, StudioSystemEnv};
use crate::traits::{apply_studio_traits, build_studio_trait_store, StudioTraitBox};

use super::WorkspaceState;

impl WorkspaceState {
    /// Returns the active typed [`StudioRoute`] from the `nve-router` history stack.
    pub fn current_route(&self) -> &StudioRoute {
        self.router.current_route()
    }

    /// Returns the canonical path string for the current route (e.g. `"/board"`, `"/doc/1"`).
    pub fn current_route_path(&self) -> String {
        self.router.current_path()
    }

    /// Returns the `aether://studio/...` deep-link URI for the current route.
    pub fn current_deep_link(&self) -> String {
        self.router.current_deep_link()
    }

    /// Returns `true` if the `nve-router` stack can step backward.
    pub fn can_navigate_back(&self) -> bool {
        self.router.can_go_back()
    }

    /// Returns `true` if the `nve-router` stack can step forward.
    pub fn can_navigate_forward(&self) -> bool {
        self.router.can_go_forward()
    }

    /// Applies a resolved [`StudioRoute`] onto `WorkspaceSnapshot` + `focused_task_id`.
    fn apply_route_to_state(&mut self, route: &StudioRoute, label: String) {
        let target_view = route.active_view();
        let target_doc = route.doc_id();
        let target_task = route.task_id();
        self.focused_task_id = target_task;

        let needs_doc_change = target_doc.is_some_and(|id| {
            self.current.active_doc_id != id && self.current.docs.iter().any(|d| d.id == id)
        });
        let needs_view_change = self.current.active_view != target_view;

        if needs_view_change || needs_doc_change {
            self.commit_action(label, |s| {
                s.active_view = target_view;
                s.mobile_nav_open = false;
                if let Some(doc_id) = target_doc
                    && s.docs.iter().any(|d| d.id == doc_id)
                {
                    s.active_doc_id = doc_id;
                    s.doc_editing_block_id = None;
                    s.doc_edit_draft.clear();
                }
            });
        } else {
            self.current.mobile_nav_open = false;
            self.frame_count = self.frame_count.saturating_add(1);
        }
    }

    /// Navigates to a typed [`StudioRoute`], pushing it onto the Back/Forward stack and
    /// synchronizing `active_view`, `active_doc_id`, and `focused_task_id`.
    pub fn navigate_to(&mut self, route: StudioRoute) -> bool {
        if !self.router.push(route.clone()) {
            self.focused_task_id = route.task_id();
            return false;
        }
        let label = format!("Route -> {}", route.to_path());
        self.apply_route_to_state(&route, label);
        true
    }

    /// Parses and navigates to a deep-link URI (`/docs`, `/task/2`, `/doc/2`,
    /// `aether://studio/stream`, or `https://studio.nve.dev/metrics`).
    pub fn navigate_uri(&mut self, uri: &str) -> bool {
        let Ok(route) = self.router.apply_deep_link(uri) else {
            return false;
        };
        let label = format!("DeepLink -> {}", route.to_path());
        self.apply_route_to_state(&route, label);
        true
    }

    /// Navigates one entry backward in the `nve-router` history stack without discarding forward
    /// entries, keeping `WorkspaceState::active_view` cleanly synchronized.
    pub fn navigate_back(&mut self) -> bool {
        let Some(route) = self.router.back() else {
            return false;
        };
        let label = format!("Back -> {}", route.to_path());
        self.apply_route_to_state(&route, label);
        true
    }

    /// Navigates one entry forward in the `nve-router` history stack.
    pub fn navigate_forward(&mut self) -> bool {
        let Some(route) = self.router.forward() else {
            return false;
        };
        let label = format!("Forward -> {}", route.to_path());
        self.apply_route_to_state(&route, label);
        true
    }

    /// Synchronizes the router and trait store after a time-travel scrub.
    pub(super) fn sync_derived_after_scrub(&mut self) {
        self.router.sync_to_view(self.current.active_view);
        self.focused_task_id = self.router.current_route().task_id();
        self.refresh_traits();
    }

    /// Returns the underlying [`TraitStore`] synced with viewport, studio tokens, and host env.
    pub fn trait_store(&self) -> &TraitStore {
        &self.trait_store.0
    }

    /// Returns a cloned snapshot of the current [`TraitEnvironment`].
    pub fn trait_environment(&self) -> TraitEnvironment {
        self.trait_store.0.environment().clone()
    }

    /// Returns the discrete horizontal [`SizeClass`] from the unified [`TraitStore`].
    pub fn size_class(&self) -> SizeClass {
        self.trait_store.0.get::<SizeClass>()
    }

    /// Returns the synced host [`ColorScheme`] from the unified [`TraitStore`].
    pub fn color_scheme(&self) -> ColorScheme {
        self.trait_store.0.get::<ColorScheme>()
    }

    /// Returns the synced host [`Locale`] from the unified [`TraitStore`].
    pub fn locale(&self) -> Locale {
        self.trait_store.0.get::<Locale>()
    }

    /// Updates viewport dimensions through [`TraitStore::update_viewport`] and returns the exact
    /// breakpoint [`ViewportChangeMask`] so callers can skip layout churn on 1px desktop drags.
    pub fn update_viewport_with_mask(&mut self, width: f32, height: f32) -> ViewportChangeMask {
        let w = if width.is_finite() && width > 0.0 {
            width.round() as u32
        } else {
            return ViewportChangeMask::default();
        };
        let h = if height.is_finite() && height > 0.0 {
            height.round() as u32
        } else {
            self.current.viewport_height
        };

        let was_compact = self.size_class() == SizeClass::Compact;
        let mask = self.trait_store.0.update_viewport(w as f32, h as f32);
        let is_compact = self.size_class() == SizeClass::Compact;

        self.current.viewport_width = w;
        self.current.viewport_height = h;
        self.current.stream.viewport_height = h.saturating_sub(280).max(180);

        if was_compact && !is_compact {
            self.current.mobile_nav_open = false;
            self.current.mobile_studio_open = false;
        }
        if self.history.len() == 1 && self.cursor == 0 {
            self.history[0].viewport_width = w;
            self.history[0].viewport_height = h;
            self.history[0].stream.viewport_height = self.current.stream.viewport_height;
        }
        mask
    }

    /// Re-synchronizes non-viewport traits (accent, density, color scheme, locale) onto
    /// `self.trait_store`.
    pub(crate) fn refresh_traits(&mut self) {
        apply_studio_traits(
            &mut self.trait_store.0,
            &self.current,
            &self.system_env.theme,
            &self.system_env.locale,
        );
    }

    /// Rebuilds the [`StudioTraitBox`] from scratch (used on `NWS1` state restore).
    pub(crate) fn rebuild_trait_box(
        snapshot: &super::WorkspaceSnapshot,
        system_env: &StudioSystemEnv,
    ) -> StudioTraitBox {
        StudioTraitBox(build_studio_trait_store(snapshot, system_env))
    }

    /// Applies an updated host [`StudioSystemEnv`] and synchronizes `TraitStore`.
    pub fn apply_system_env(&mut self, env: StudioSystemEnv) -> bool {
        if self.system_env == env {
            return false;
        }
        self.system_env = env;
        self.refresh_traits();
        self.frame_count = self.frame_count.saturating_add(1);
        true
    }

    /// Pulls live theme (`NVT2`) and locale (`NVL1`) from [`StudioHostServices`] via
    /// `nve_sdk::SystemEnvClient` and synchronizes them into [`TraitStore`].
    pub fn sync_system_env(&mut self, services: &StudioHostServices) -> bool {
        let Ok(env) = services.fetch_system_env() else {
            return false;
        };
        self.apply_system_env(env)
    }

    /// Copies the active document's Markdown export via `nve_sdk::ClipboardClient` and records a
    /// non-intrusive status notice without mutating time-travel history.
    pub fn copy_active_doc_markdown(
        &mut self,
        services: &StudioHostServices,
    ) -> Result<String, SdkError> {
        let md = self.active_doc_markdown();
        services.copy_text(&md)?;
        self.last_clipboard_notice = Some(format!("Copied MD ({}B)", md.len()));
        self.frame_count = self.frame_count.saturating_add(1);
        Ok(md)
    }

    /// Copies the active deep-link route URL (`aether://studio/...`) via `nve_sdk::ClipboardClient`.
    pub fn copy_current_route_url(
        &mut self,
        services: &StudioHostServices,
    ) -> Result<String, SdkError> {
        let url = self.current_deep_link();
        services.copy_text(&url)?;
        self.last_clipboard_notice = Some(format!("Copied {}", self.current_route_path()));
        self.frame_count = self.frame_count.saturating_add(1);
        Ok(url)
    }
}
