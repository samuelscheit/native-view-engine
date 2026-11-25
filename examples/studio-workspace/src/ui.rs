//! Dioxus 0.7 `rsx!` shell for **Aether Workspace** + docked **NVE Studio** meta-workbench.
//!
//! Powered by shared `nve-ui` components, `nve-router` typed navigation, `nve-sdk` host services,
//! and the unified Environment [`TraitProvider`] + [`ReactiveTraitStore`] system adapting between
//! desktop (`SizeClass::Medium` / `SizeClass::Expanded`) and mobile portrait (`SizeClass::Compact`).

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_dioxus::traits::{
    use_environment, use_trait, use_trait_store, ReactiveTraitStore, TraitProvider,
};
use nve_interop::traits::SizeClass;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, ThemeProvider};

use crate::sdk::StudioHostServices;
use crate::state::{ActiveView, ChannelKind, WorkspaceState, HOST_SURFACES};
use crate::theme::{
    badge_min_w, card_border, control_border, panel_border, studio_ui_theme, LayoutMetrics,
    CANVAS_BG, CONTROL_BG, PANEL_BG, STRETCH_COL, TEXT_PRIMARY,
};
use crate::traits::{
    resolve_studio_layout, size_class_label, sync_reactive_trait_store, StudioAccentTrait,
};
use crate::ui_board::render_board_view;
use crate::ui_channels::render_channel_view;
use crate::ui_chrome::{
    render_desktop_sidebar, render_mobile_top_bar_and_nav, render_route_nav_bar, NavItem,
};
use crate::ui_docs::render_docs_view;
use crate::ui_islands_view::render_islands_view;
use crate::ui_metrics::render_metrics_view;
use crate::ui_palette::{filter_palette_items, render_command_palette};
use crate::ui_stream::{SharedStore, Stream100kPanel};
use crate::ui_studio::render_studio_panel;
use crate::ui_workbench::render_workbench_view;

thread_local! {
    /// Hands a module-owned `Arc<Mutex<WorkspaceState>>` into the `DioxusSurface` root hook
    /// on first mount without any cross-instance global contention.
    static PENDING_STORE: RefCell<Option<Arc<Mutex<WorkspaceState>>>> = const { RefCell::new(None) };
    /// Hands a module-owned `Arc<StudioHostServices>` into the `DioxusSurface` root hook.
    static PENDING_HOST_SERVICES: RefCell<Option<Arc<StudioHostServices>>> = const { RefCell::new(None) };
}

pub(crate) fn set_pending_store(store: Arc<Mutex<WorkspaceState>>) {
    PENDING_STORE.with(|slot| {
        *slot.borrow_mut() = Some(store);
    });
}

pub(crate) fn clear_pending_store() {
    PENDING_STORE.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn take_pending_store() -> Option<Arc<Mutex<WorkspaceState>>> {
    PENDING_STORE.with(|slot| slot.borrow_mut().take())
}

pub(crate) fn set_pending_host_services(services: Arc<StudioHostServices>) {
    PENDING_HOST_SERVICES.with(|slot| {
        *slot.borrow_mut() = Some(services);
    });
}

pub(crate) fn clear_pending_host_services() {
    PENDING_HOST_SERVICES.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn take_pending_host_services() -> Option<Arc<StudioHostServices>> {
    PENDING_HOST_SERVICES.with(|slot| slot.borrow_mut().take())
}

#[derive(Clone)]
struct ShellStoreProp(Arc<Mutex<WorkspaceState>>);

impl PartialEq for ShellStoreProp {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone)]
struct ShellHostServicesProp(Arc<StudioHostServices>);

impl PartialEq for ShellHostServicesProp {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub(crate) fn mutate_state(
    store: &Arc<Mutex<WorkspaceState>>,
    mut version: Signal<u64>,
    f: impl FnOnce(&mut WorkspaceState),
) {
    let trait_sync = if let Ok(mut guard) = store.lock() {
        f(&mut guard);
        Some((guard.current.clone(), guard.system_env.clone()))
    } else {
        None
    };
    if let (Some((snapshot, system_env)), Some(reactive_traits)) = (trait_sync, use_trait_store()) {
        let _ = sync_reactive_trait_store(&reactive_traits, &snapshot, &system_env);
    }
    let next = version().wrapping_add(1);
    version.set(next);
}

#[component]
pub fn StudioWorkspaceApp() -> Element {
    let store = dioxus_core::use_hook(|| {
        take_pending_store().unwrap_or_else(|| Arc::new(Mutex::new(WorkspaceState::default())))
    });
    let host_services = dioxus_core::use_hook(|| {
        take_pending_host_services().unwrap_or_else(|| Arc::new(StudioHostServices::new()))
    });
    let version = use_signal(|| 0u64);

    let reactive_traits = dioxus_core::use_hook(|| {
        let initial_traits = store
            .lock()
            .map(|g| g.trait_store().clone())
            .unwrap_or_default();
        ReactiveTraitStore::new(initial_traits)
    });

    rsx! {
        TraitProvider {
            store: reactive_traits,
            StudioWorkspaceShell {
                store: ShellStoreProp(Arc::clone(&store)),
                host_services: ShellHostServicesProp(Arc::clone(&host_services)),
                version,
            }
        }
    }
}

#[component]
fn StudioWorkspaceShell(
    store: ShellStoreProp,
    host_services: ShellHostServicesProp,
    version: Signal<u64>,
) -> Element {
    let store = store.0;
    let host_services = host_services.0;
    let rev = version();

    // Subscribe strictly to breakpoint-only `SizeClass`, custom `StudioAccentTrait`, and
    // effective `TraitEnvironment` from the root `TraitProvider`.
    let size_class = use_trait::<SizeClass>();
    let trait_accent = use_trait::<StudioAccentTrait>();
    let trait_env = use_environment();

    // Clone only the active snapshot + lightweight metadata (never the full 64-entry history vec!).
    let (
        current,
        palette,
        sync_telemetry,
        palette_matches,
        route_path,
        can_nav_back,
        can_nav_forward,
        focused_task_id,
        clipboard_notice,
        frame_num,
        snap_idx,
        hist_len,
        can_undo,
        can_redo,
        active_tasks,
        done_tasks,
        total_msgs,
        filtered_tasks,
    ) = {
        let guard = store.lock().unwrap_or_else(|e| e.into_inner());
        let active_view = guard.current.active_view;
        let filtered = if active_view == ActiveView::Board {
            guard.filtered_tasks()
        } else {
            Vec::new()
        };
        let pal_matches = if guard.palette.open {
            filter_palette_items(&guard)
        } else {
            Vec::new()
        };
        (
            guard.current.clone(),
            guard.palette.clone(),
            guard.sync_telemetry.clone(),
            pal_matches,
            guard.current_route_path(),
            guard.can_navigate_back(),
            guard.can_navigate_forward(),
            guard.focused_task_id,
            guard.last_clipboard_notice.clone(),
            guard.frame_count,
            guard.snapshot_index_1based(),
            guard.history_len(),
            guard.can_undo(),
            guard.can_redo(),
            guard.active_task_count(),
            guard.done_task_count(),
            guard.total_message_count(),
            filtered,
        )
    };

    let studio = &current.studio;
    let ui_theme = studio_ui_theme(studio);
    let accent = trait_accent;
    let accent_hex = accent.hex();
    let accent_tint = accent.tint_bg();
    let xray = studio.xray;
    let layout_engine_str = studio.layout_engine.as_str();
    let m = LayoutMetrics::from_studio(studio);
    let resolved = resolve_studio_layout(&trait_env, m);
    let is_mobile = resolved.is_mobile;
    let compact_header = resolved.compact_header;
    let center_pad = resolved.center_pad;
    let viewport_mode_str = if is_mobile { "mobile-portrait" } else { "desktop" };
    let size_class_str = size_class_label(size_class);

    let (panel_bw, panel_bc) = panel_border(xray);
    let (card_bw, card_bc) = card_border(xray);

    let active_view = current.active_view;
    let total_tasks = current.tasks.len();
    let nav_items: &[NavItem] = &[
        (ActiveView::Board, "nav-board", "Board", total_tasks.to_string()),
        (ActiveView::ChannelEngineering, "nav-engineering", "# engineering", current.eng_messages.len().to_string()),
        (ActiveView::ChannelDesign, "nav-design", "# design-sync", current.design_messages.len().to_string()),
        (ActiveView::Docs, "nav-docs", "Docs", current.docs.len().to_string()),
        (ActiveView::Workbench, "nav-workbench", "Workbench", HOST_SURFACES.len().to_string()),
        (ActiveView::Stream100k, "nav-stream", "100k Stream", "100k".into()),
        (ActiveView::Islands, "nav-islands", "Islands", current.islands.cards.len().to_string()),
        (ActiveView::Metrics, "nav-metrics", "Metrics", current.stress_rows.to_string()),
    ];

    let center_style = if is_mobile {
        format!(
            "width: 100%; flex: 1; flex-direction: column; padding: {center_pad}px; gap: {}px; background-color: {CANVAS_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
            m.main_gap
        )
    } else {
        format!(
            "flex: 1; height: 100%; flex-direction: column; padding: {center_pad}px; gap: {}px; background-color: {CANVAS_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
            m.main_gap
        )
    };
    let header_card_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 8px; padding-vertical: 6px; gap: 6px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let status_pill_label = if compact_header {
        format!("F#{frame_num} {snap_idx}/{hist_len}")
    } else {
        format!("Frame #{frame_num} | {snap_idx}/{hist_len}")
    };
    let status_pill_style = format!(
        "min-width: {}px; height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w(&status_pill_label),
        m.pill_radius
    );
    let (cmdk_bw, cmdk_bc) = control_border(xray, palette.open, accent_hex);
    let cmdk_bg = if palette.open { accent_tint } else { CONTROL_BG };
    let cmdk_btn_style = format!(
        "width: 52px; height: 22px; min-height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {cmdk_bg}; border-radius: {}px; border-width: {cmdk_bw}px; border-color: {cmdk_bc};",
        m.pill_radius
    );

    let active_channel = match active_view {
        ActiveView::ChannelDesign => Some(ChannelKind::Design),
        ActiveView::ChannelEngineering => Some(ChannelKind::Engineering),
        _ => None,
    };
    let channel_messages = match active_channel {
        Some(ChannelKind::Engineering) => current.eng_messages.as_slice(),
        Some(ChannelKind::Design) => current.design_messages.as_slice(),
        None => &[],
    };
    let channel_draft = match active_channel {
        Some(ChannelKind::Engineering) => current.eng_draft.as_str(),
        Some(ChannelKind::Design) => current.design_draft.as_str(),
        None => "",
    };

    let root_flex_dir = if is_mobile { "column" } else { "row" };
    let s_toggle_palette = Arc::clone(&store);

    rsx! {
        ThemeProvider {
            theme: ui_theme,
            view {
                test_id: "workspace-root",
                accessibility_label: "Aether Studio Workspace",
                "layout-engine": "{layout_engine_str}",
                "viewport-mode": "{viewport_mode_str}",
                "size-class": "{size_class_str}",
                "active-route": "{route_path}",
                style: "width: 100%; height: 100%; flex-direction: {root_flex_dir}; background-color: {CANVAS_BG};",

                // 1A. MOBILE PORTRAIT CHROME (`SizeClass::Compact`)
                if is_mobile {
                    {render_mobile_top_bar_and_nav(&store, version, &current, nav_items, accent_hex, accent_tint, xray, m)}
                    if current.mobile_studio_open {
                        {render_studio_panel(&store, version, &current, &sync_telemetry, snap_idx, hist_len, can_undo, can_redo, m, true)}
                    }
                }

                // 1B. DESKTOP LEFT SIDEBAR (`SizeClass::Medium` / `SizeClass::Expanded`)
                if !is_mobile {
                    {render_desktop_sidebar(&store, version, &current, nav_items, accent_hex, accent_tint, xray, m)}
                }

                // 2. CENTER MAIN WORKSPACE
                view {
                    test_id: "center-workspace",
                    style: "{center_style}",

                    Card {
                        variant: CardVariant::Outlined,
                        padding: Some(6.0),
                        gap: Some(6.0),
                        style: Some(header_card_style),
                        text {
                            style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                            "{active_view.title()}"
                        }
                        view {
                            style: "flex-direction: row; align-items: center; gap: 5px; flex-shrink: 0;",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                test_id: Some("btn-command-palette".into()),
                                accessibility_label: Some("Open Cmd+K Command Palette".into()),
                                style: Some(cmdk_btn_style),
                                on_press: move |_| mutate_state(&s_toggle_palette, version, |st| st.toggle_palette()),
                                "Cmd+K"
                            }
                            Badge {
                                label: status_pill_label,
                                variant: BadgeVariant::Neutral,
                                style: Some(status_pill_style),
                            }
                        }
                    }

                    {render_route_nav_bar(&store, &host_services, version, &route_path, clipboard_notice.as_deref(), can_nav_back, can_nav_forward, compact_header, accent_hex, xray, m)}

                    if palette.open {
                        view {
                            style: "width: 100%; flex: 1; flex-direction: column;",
                            {render_command_palette(&store, version, &palette, &palette_matches, accent_hex, accent_tint, xray, m)}
                        }
                    } else {
                        match active_view {
                            ActiveView::Board => {
                                render_board_view(&store, version, &current, &filtered_tasks, focused_task_id, total_tasks, active_tasks, done_tasks, accent_hex, accent_tint, xray, m)
                            }
                            ActiveView::ChannelEngineering | ActiveView::ChannelDesign => {
                                let ch = active_channel.unwrap_or(ChannelKind::Engineering);
                                render_channel_view(&store, version, ch, channel_messages, channel_draft, accent_hex, accent_tint, xray, m)
                            }
                            ActiveView::Docs => {
                                render_docs_view(&store, &host_services, version, &current, clipboard_notice.as_deref(), accent_hex, accent_tint, xray, m, is_mobile || compact_header)
                            }
                            ActiveView::Workbench => {
                                render_workbench_view(&store, version, &current, &sync_telemetry, accent_hex, accent_tint, xray, m, is_mobile)
                            }
                            ActiveView::Stream100k => {
                                rsx! {
                                    Stream100kPanel {
                                        store: SharedStore(Arc::clone(&store)),
                                        accent,
                                        density: studio.density,
                                        xray,
                                        pill_radius: m.pill_radius,
                                        button_radius: m.button_radius,
                                        is_mobile,
                                        parent_rev: rev,
                                    }
                                }
                            }
                            ActiveView::Islands => {
                                render_islands_view(&store, version, &current.islands, accent_hex, accent_tint, xray, m)
                            }
                            ActiveView::Metrics => {
                                render_metrics_view(&store, version, &current, frame_num, snap_idx, hist_len, active_tasks, total_msgs, accent_hex, accent_tint, xray, m)
                            }
                        }
                    }
                }

                // 3. RIGHT DOCKED NVE STUDIO META-WORKBENCH (`!is_mobile` when open)
                if !is_mobile && studio.panel_open {
                    {render_studio_panel(&store, version, &current, &sync_telemetry, snap_idx, hist_len, can_undo, can_redo, m, false)}
                }
            }
        }
    }
}
