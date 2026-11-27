//! Dioxus 0.7 `rsx!` shell for **Aether Workspace** + docked **NVE Studio** meta-workbench.
//!
//! Powered by shared `nve-ui` components on the unified OKLCH [`nve_ui::Theme`] (Studio brand,
//! OS-following polarity, per-workspace accent), `nve-router` typed navigation, `nve-host-services`
//! capability modules (`nve-clipboard`, `nve-system-env`), the Environment [`TraitProvider`], and a
//! fine-grained [`StudioStore`] of per-domain signals: every view is a component that subscribes
//! only to the domains it reads.

pub mod store;
pub(crate) mod style;

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_dioxus::traits::{use_environment, use_trait, ReactiveTraitStore, TraitProvider};
use nve_interop::traits::{ColorScheme, HighContrast, SizeClass};
use nve_interop::ContrastLevel;
use nve_ui::{Appearance, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, ThemeProvider};

use crate::sdk::StudioHostServices;
use crate::state::{ActiveView, ChannelKind, WorkspaceState};
use crate::theme::{badge_min_w, studio_theme, LayoutMetrics, STRETCH_COL};
use crate::traits::{resolve_studio_layout, size_class_label, StudioAccentTrait};
use crate::ui::style::data_attrs;
use crate::ui_board::BoardView;
use crate::ui_channels::ChannelView;
use crate::ui_chrome::{DesktopSidebar, MobileChrome, RouteNavBar};
use crate::ui_docs::DocsView;
use crate::ui_islands_view::IslandsView;
use crate::ui_metrics::MetricsView;
use crate::ui_palette::CommandPalette;
use crate::ui_stream::Stream100kPanel;
use crate::ui_studio::StudioPanel;
use crate::ui_workbench::WorkbenchView;
use store::{use_studio, StudioStore};
use style::use_studio_style;

thread_local! {
    /// Hands a module-owned `Arc<Mutex<WorkspaceState>>` into the `DioxusSurface` root hook
    /// on first mount without any cross-instance global contention.
    static PENDING_STORE: RefCell<Option<Arc<Mutex<WorkspaceState>>>> = const { RefCell::new(None) };
    /// Hands a module-owned `Arc<StudioHostServices>` into the `DioxusSurface` root hook.
    static PENDING_HOST_SERVICES: RefCell<Option<Arc<StudioHostServices>>> = const { RefCell::new(None) };
}

pub(crate) fn set_pending_store(store: Arc<Mutex<WorkspaceState>>) {
    PENDING_STORE.with(|slot| *slot.borrow_mut() = Some(store));
}

pub(crate) fn clear_pending_store() {
    PENDING_STORE.with(|slot| slot.borrow_mut().take());
}

pub(crate) fn set_pending_host_services(services: Arc<StudioHostServices>) {
    PENDING_HOST_SERVICES.with(|slot| *slot.borrow_mut() = Some(services));
}

pub(crate) fn clear_pending_host_services() {
    PENDING_HOST_SERVICES.with(|slot| slot.borrow_mut().take());
}

#[component]
pub fn StudioWorkspaceApp() -> Element {
    let reactive_traits = dioxus_core::use_hook(|| {
        let backing = PENDING_STORE
            .with(|slot| slot.borrow_mut().take())
            .unwrap_or_else(|| Arc::new(Mutex::new(WorkspaceState::default())));
        let services = PENDING_HOST_SERVICES
            .with(|slot| slot.borrow_mut().take())
            .unwrap_or_else(|| Arc::new(StudioHostServices::new()));
        // The host's live OS appearance/locale is authoritative from the first frame.
        let initial_traits = {
            let mut guard = backing.lock().unwrap_or_else(|e| e.into_inner());
            guard.sync_system_env(&services);
            guard.trait_store().clone()
        };
        let reactive = ReactiveTraitStore::new(initial_traits);
        StudioStore::provide(backing, services, Some(reactive.clone()));
        reactive
    });

    rsx! {
        TraitProvider {
            store: reactive_traits,
            StudioShell {}
        }
    }
}

/// Resolves the Studio theme (OS polarity + workspace accent/density/radius) and lays out the
/// panes. Subscribes to `nav` (view, drawers, route) and `studio` (config) only.
#[component]
fn StudioShell() -> Element {
    let store = use_studio();
    let size_class = use_trait::<SizeClass>();
    let accent = use_trait::<StudioAccentTrait>();
    let os = Appearance {
        scheme: use_trait::<ColorScheme>(),
        contrast: if use_trait::<HighContrast>() { ContrastLevel::High } else { ContrastLevel::Standard },
    };
    let trait_env = use_environment();

    let config = store.studio.read().config.clone();
    let theme = studio_theme(&config, accent, os);
    let m = LayoutMetrics::from_studio(&config);
    let resolved = resolve_studio_layout(&trait_env, m);
    let (is_mobile, compact_header) = (resolved.is_mobile, resolved.compact_header);

    let nav = store.nav.read();
    let (active_view, palette_open, mobile_studio_open) = (nav.active_view, nav.palette_open, nav.mobile_studio_open);
    let route_path = nav.route_path.clone();
    drop(nav);

    let root_attrs = data_attrs([
        ("layout-engine", config.layout_engine.as_str().to_owned()),
        ("viewport-mode", if is_mobile { "mobile-portrait" } else { "desktop" }.to_owned()),
        ("size-class", size_class_label(size_class).to_owned()),
        ("active-route", route_path),
    ]);
    let root_flex_dir = if is_mobile { "column" } else { "row" };
    let center_size = if is_mobile { "width: 100%; flex: 1;" } else { "flex: 1; height: 100%;" };

    rsx! {
        ThemeProvider {
            theme,
            view {
                test_id: "workspace-root",
                accessibility_label: "Aether Studio Workspace",
                nve_table: root_attrs,
                style: "width: 100%; height: 100%; flex-direction: {root_flex_dir}; background-color: {theme.surface.canvas.hex()};",

                if is_mobile {
                    MobileChrome {}
                    if mobile_studio_open {
                        StudioPanel { mobile: true }
                    }
                } else {
                    DesktopSidebar {}
                }

                CenterWorkspace {
                    size: center_size,
                    pad: resolved.center_pad,
                    compact_header,
                    is_mobile,
                    active_view,
                    palette_open,
                }

                if !is_mobile && config.panel_open {
                    StudioPanel { mobile: false }
                }
            }
        }
    }
}

#[component]
fn CenterWorkspace(
    size: &'static str,
    pad: u32,
    compact_header: bool,
    is_mobile: bool,
    active_view: ActiveView,
    palette_open: bool,
) -> Element {
    let s = use_studio_style();
    rsx! {
        view {
            test_id: "center-workspace",
            style: "{size} flex-direction: column; padding: {pad}px; gap: {s.m.main_gap}px; background-color: {s.t.surface.canvas.hex()}; {s.panel()}",

            WorkspaceHeader { title: active_view.title(), compact_header }
            RouteNavBar { compact_header }

            if palette_open {
                view {
                    style: "width: 100%; flex: 1; flex-direction: column;",
                    CommandPalette {}
                }
            } else {
                match active_view {
                    ActiveView::Board => rsx! { BoardView {} },
                    ActiveView::ChannelEngineering => rsx! { ChannelView { ch: ChannelKind::Engineering } },
                    ActiveView::ChannelDesign => rsx! { ChannelView { ch: ChannelKind::Design } },
                    ActiveView::Docs => rsx! { DocsView { compact: is_mobile || compact_header } },
                    ActiveView::Workbench => rsx! { WorkbenchView { is_mobile } },
                    ActiveView::Stream100k => rsx! { Stream100kPanel { is_mobile } },
                    ActiveView::Islands => rsx! { IslandsView {} },
                    ActiveView::Metrics => rsx! { MetricsView {} },
                }
            }
        }
    }
}

/// View title, Cmd+K toggle, and the frame/time-travel pill: the only shell part subscribed to
/// the `inspector` domain (which changes on every action).
#[component]
fn WorkspaceHeader(title: &'static str, compact_header: bool) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let palette_open = store.nav.read().palette_open;
    let insp = store.inspector.read();
    let status_pill_label = if compact_header {
        format!("F#{} {}/{}", insp.frame_count, insp.snapshot_index, insp.history_len)
    } else {
        format!("Frame #{} | {}/{}", insp.frame_count, insp.snapshot_index, insp.history_len)
    };
    drop(insp);

    let header_card_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 8px; padding-vertical: 6px; gap: 6px; background-color: {}; border-radius: {}px; {}",
        s.t.surface.base.hex(),
        s.m.button_radius,
        s.card()
    );
    let status_pill_style = format!(
        "min-width: {}px; height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {}; border-radius: {}px; {}",
        badge_min_w(&status_pill_label),
        s.t.status.accent.subtle.hex(),
        s.m.pill_radius,
        s.control(true)
    );
    let cmdk_btn_style = format!(
        "width: 52px; height: 22px; min-height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: {}px; {}",
        s.control_bg(palette_open),
        s.m.pill_radius,
        s.control(palette_open)
    );

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            padding: Some(6.0),
            gap: Some(6.0),
            style: Some(header_card_style),
            text { style: "color: {s.t.text.primary.hex()}; font-size: 12px;", "{title}" }
            view {
                style: "flex-direction: row; align-items: center; gap: 5px; flex-shrink: 0;",
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-command-palette".into()),
                    accessibility_label: Some("Open Cmd+K Command Palette".into()),
                    style: Some(cmdk_btn_style),
                    on_press: move |_| store.update(|st| st.toggle_palette()),
                    "Cmd+K"
                }
                Badge {
                    label: status_pill_label,
                    variant: BadgeVariant::Neutral,
                    style: Some(status_pill_style),
                }
            }
        }
    }
}
