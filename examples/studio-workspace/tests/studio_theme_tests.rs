//! Studio rendered-theme guards: every text run of every view stays readable in all four OS
//! polarities and for every workspace accent, with X-Ray on and off.

use std::sync::{Arc, Mutex};

use nve_clipboard::MemoryClipboard;
use nve_host_services::MemoryHostServices;
use nve_studio_workspace::{
    create_surface_with_services, AccentTheme, ActiveView, StudioHostServices, StudioSystemEnv,
    WorkspaceState,
};
use nve_system_env::{ColorScheme, ContrastLevel, MemorySystemEnv, ThemeSnapshot};
use nve_abi::ColorRgba;
use nve_components::tables::{TextContent, Visual};
use nve_studio_workspace::theme::studio_theme;
use nve_ui::audit::text_contrast_violations;
use nve_ui::Appearance;

const VIEWS: [ActiveView; 8] = [
    ActiveView::Board,
    ActiveView::ChannelEngineering,
    ActiveView::ChannelDesign,
    ActiveView::Docs,
    ActiveView::Workbench,
    ActiveView::Stream100k,
    ActiveView::Islands,
    ActiveView::Metrics,
];

const POLARITIES: [(ColorScheme, ContrastLevel); 4] = [
    (ColorScheme::Light, ContrastLevel::Standard),
    (ColorScheme::Dark, ContrastLevel::Standard),
    (ColorScheme::Light, ContrastLevel::High),
    (ColorScheme::Dark, ContrastLevel::High),
];

/// Services whose OS reports `scheme` / `contrast`.
fn services(scheme: ColorScheme, contrast: ContrastLevel) -> Arc<StudioHostServices> {
    let env = StudioSystemEnv::default();
    let mem = Arc::new(MemoryHostServices::new());
    mem.register(MemoryClipboard::new());
    mem.register(MemorySystemEnv::with_initial(
        ThemeSnapshot { color_scheme: Some(scheme), contrast, ..env.theme },
        env.locale,
    ));
    Arc::new(StudioHostServices::with_memory(mem))
}

/// Mounts `view` and returns `(text runs checked, violations as readable lines)`.
fn audit(view: ActiveView, accent: AccentTheme, xray: bool, os: (ColorScheme, ContrastLevel)) -> (usize, Vec<String>) {
    let mut state = WorkspaceState::default();
    state.set_accent(accent);
    if xray {
        state.toggle_xray();
    }
    state.select_view(view);
    if view == ActiveView::Stream100k {
        state.toggle_stream_row_expanded(0);
    }
    let mut surface = create_surface_with_services(1, Arc::new(Mutex::new(state)), services(os.0, os.1));
    surface.reconcile(1);
    let min = if os.1 == ContrastLevel::High { 7.0 } else { 4.5 };
    let store = surface.store();
    let runs = store
        .live_nodes()
        .filter(|&id| store.try_row::<TextContent>(id).is_some())
        .count();
    let violations = text_contrast_violations(store, min)
        .into_iter()
        .map(|v| format!("{view:?}/{accent:?}/xray={xray}/{os:?}: {:?} {:.2}:1 ({:?} on {:?})", v.text, v.ratio, v.fg, v.bg))
        .collect();
    (runs, violations)
}

#[test]
fn every_view_text_run_is_readable_in_all_four_polarities() {
    let mut runs = 0;
    let mut violations = Vec::new();
    for os in POLARITIES {
        for view in VIEWS {
            let (n, v) = audit(view, AccentTheme::Cyan, false, os);
            runs += n;
            violations.extend(v);
        }
    }
    assert!(runs > 1_000, "the walk must cover the mounted views, saw only {runs} text runs");
    assert!(violations.is_empty(), "{} unreadable text runs:\n{}", violations.len(), violations.join("\n"));
}

#[test]
fn mounted_surface_paints_the_os_polarity_from_its_first_frame() {
    for (scheme, contrast) in POLARITIES {
        let config = WorkspaceState::default().current.studio;
        let expected = studio_theme(&config, config.accent, Appearance { scheme, contrast }).surface.canvas;
        let mut surface = create_surface_with_services(1, Arc::new(Mutex::new(WorkspaceState::default())), services(scheme, contrast));
        surface.reconcile(1);
        let store = surface.store();
        let canvases: Vec<ColorRgba> = store
            .live_nodes()
            .filter_map(|id| store.try_row::<Visual>(id).map(|v| ColorRgba::from_u32(v.background().0)))
            .filter(|&c| c == expected)
            .collect();
        assert!(!canvases.is_empty(), "{scheme:?}/{contrast:?}: no node paints the expected canvas {expected:?}");
    }
}

#[test]
fn every_accent_and_xray_overlay_keeps_text_readable() {
    let mut violations = Vec::new();
    for os in POLARITIES {
        for accent in [AccentTheme::Cyan, AccentTheme::Violet, AccentTheme::Green, AccentTheme::Amber] {
            for xray in [false, true] {
                violations.extend(audit(ActiveView::Board, accent, xray, os).1);
            }
        }
    }
    assert!(violations.is_empty(), "{} unreadable text runs:\n{}", violations.len(), violations.join("\n"));
}
