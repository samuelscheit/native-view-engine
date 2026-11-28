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
use nve_dioxus::traits::{TraitProvider, use_environment, use_trait};
use nve_interop::ContrastLevel;
use nve_interop::traits::{ColorScheme, HighContrast, SizeClass};
use nve_router::Routable;
use nve_ui::{
    Appearance, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant,
    ThemeProvider,
};

use crate::sdk::StudioHostServices;
use crate::state::{ActiveView, ChannelKind, WorkspaceState};
use crate::theme::{LayoutMetrics, badge_min_w, studio_theme};
use crate::traits::{StudioAccentTrait, resolve_studio_layout, size_class_label};
use crate::ui::style::{
    StudioStyleExt, data_attrs, fixed_dims, row_center, text_style, use_studio_style,
};
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
use store::{StudioStore, use_studio};

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
    nve_dioxus::render_probe::record_render("StudioWorkspaceApp");
    let store = dioxus_core::use_hook(|| {
        let backing = PENDING_STORE
            .with(|slot| slot.borrow_mut().take())
            .unwrap_or_else(|| Arc::new(Mutex::new(WorkspaceState::default())));
        let services = PENDING_HOST_SERVICES
            .with(|slot| slot.borrow_mut().take())
            .unwrap_or_else(|| Arc::new(StudioHostServices::new()));
        StudioStore::mount(backing, services)
    });

    rsx! {
        TraitProvider {
            store: store.traits(),
            StudioShell {}
        }
    }
}

/// Resolves the Studio theme (OS polarity + workspace accent/density/radius) and lays out the
/// panes. Subscribes to `nav`, `route`, `style`, and `chrome` only.
#[component]
fn StudioShell() -> Element {
    nve_dioxus::render_probe::record_render("StudioShell");
    let store = use_studio();
    let size_class = use_trait::<SizeClass>();
    let accent = use_trait::<StudioAccentTrait>();
    let os = Appearance {
        scheme: use_trait::<ColorScheme>(),
        contrast: if use_trait::<HighContrast>() {
            ContrastLevel::High
        } else {
            ContrastLevel::Standard
        },
    };
    let trait_env = use_environment();

    let config = store.style.read().config;
    let panel_open = store.chrome.read().panel_open;
    let theme = studio_theme(&config, accent, os);
    let m = LayoutMetrics::from_studio(&config);
    let resolved = resolve_studio_layout(&trait_env, m);
    let (is_mobile, compact_header) = (resolved.is_mobile, resolved.compact_header);

    let nav = store.nav.read();
    let (active_view, palette_open, mobile_studio_open) =
        (nav.active_view, nav.palette_open, nav.mobile_studio_open);
    drop(nav);
    let route_path = store.route.read().route.to_path();

    let root_attrs = data_attrs([
        ("layout-engine", config.layout_engine.as_str().to_owned()),
        (
            "viewport-mode",
            if is_mobile {
                "mobile-portrait"
            } else {
                "desktop"
            }
            .to_owned(),
        ),
        ("size-class", size_class_label(size_class).to_owned()),
        ("active-route", route_path),
    ]);
    let root_style = TypedStyle::new()
        .width_pct(100.0)
        .height_pct(100.0)
        .flex_direction(if is_mobile {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        })
        .bg(theme.surface.canvas);

    rsx! {
        ThemeProvider {
            theme,
            view {
                test_id: "workspace-root",
                accessibility_label: "Aether Studio Workspace",
                nve_table: root_attrs,
                nve_style: root_style,

                if is_mobile {
                    MobileChrome {}
                    if mobile_studio_open {
                        StudioPanel { mobile: true }
                    }
                } else {
                    DesktopSidebar {}
                }

                CenterWorkspace {
                    pad: resolved.center_pad,
                    compact_header,
                    is_mobile,
                    active_view,
                    palette_open,
                }

                if !is_mobile && panel_open {
                    StudioPanel { mobile: false }
                }
            }
        }
    }
}

#[component]
fn CenterWorkspace(
    pad: u32,
    compact_header: bool,
    is_mobile: bool,
    active_view: ActiveView,
    palette_open: bool,
) -> Element {
    nve_dioxus::render_probe::record_render("CenterWorkspace");
    let s = use_studio_style();
    let base = if is_mobile {
        TypedStyle::new().width_pct(100.0).flex_1()
    } else {
        TypedStyle::new().flex_1().height_pct(100.0)
    };
    let center_style = base
        .flex_col()
        .padding(pad as f32)
        .gap(s.m.main_gap as f32)
        .bg(s.t.surface.canvas)
        .outline(s.panel());

    rsx! {
        view {
            test_id: "center-workspace",
            nve_style: center_style,

            WorkspaceHeader { title: active_view.title(), compact_header }
            RouteNavBar { compact_header }

            if palette_open {
                view {
                    nve_style: TypedStyle::new().width_pct(100.0).flex_1().flex_col(),
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
    nve_dioxus::render_probe::record_render("WorkspaceHeader");
    let store = use_studio();
    let s = use_studio_style();
    let palette_open = store.nav.read().palette_open;
    let insp = store.inspector.read();
    let status_pill_label = if compact_header {
        format!(
            "F#{} {}/{}",
            insp.frame_count, insp.snapshot_index, insp.history_len
        )
    } else {
        format!(
            "Frame #{} | {}/{}",
            insp.frame_count, insp.snapshot_index, insp.history_len
        )
    };
    drop(insp);

    let header_card_style = TypedStyle::new()
        .width_pct(100.0)
        .flex_shrink(0.0)
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .padding_xy(8.0, 6.0)
        .gap(6.0)
        .bg(s.t.surface.base)
        .radius(s.m.button_radius as f32)
        .outline(s.card());
    let status_pill_style = TypedStyle::new()
        .min_width_px(badge_min_w(&status_pill_label) as f32)
        .height_px(22.0)
        .flex_shrink(0.0)
        .stretch_col()
        .padding_xy(4.0, 0.0)
        .bg(s.t.status.accent.subtle)
        .radius(s.m.pill_radius as f32)
        .outline(s.control(true));
    let cmdk_btn_style = s.compact_control(
        fixed_dims(52.0, 22.0),
        2,
        s.m.pill_radius,
        Some(palette_open),
    );

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            padding: Some(6.0),
            gap: Some(6.0),
            nve_style: Some(header_card_style),
            text { nve_style: text_style(s.t.text.primary, 12.0), "{title}" }
            view {
                nve_style: row_center(5.0).flex_shrink(0.0),
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-command-palette".into()),
                    accessibility_label: Some("Open Cmd+K Command Palette".into()),
                    nve_style: Some(cmdk_btn_style),
                    on_press: move |_| store.update(|st| st.toggle_palette()),
                    "Cmd+K"
                }
                Badge {
                    label: status_pill_label,
                    variant: BadgeVariant::Neutral,
                    nve_style: Some(status_pill_style),
                }
            }
        }
    }
}
