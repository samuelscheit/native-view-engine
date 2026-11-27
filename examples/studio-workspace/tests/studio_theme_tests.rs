//! Studio rendered-theme guards: every text run and every checked/selected indicator of every
//! view, chrome state (mobile drawers, Cmd+K palette, in-place doc edit), workspace accent, and
//! X-Ray overlay stays readable in all four OS polarities; every Studio [`Tone`] keeps its
//! contrast contract.

use std::sync::{Arc, Mutex};

use nve_abi::ColorRgba;
use nve_clipboard::MemoryClipboard;
use nve_components::tables::{Accessibility, AriaTriState, TextContent, Visual};
use nve_host_services::MemoryHostServices;
use nve_studio_workspace::theme::{author_tone, studio_theme, task_status_tone};
use nve_studio_workspace::{
    AccentTheme, ActiveView, StudioHostServices, StudioSystemEnv, WorkspaceState,
    create_surface_with_services,
};
use nve_studio_workspace::{StudioConfig, TaskStatus};
use nve_system_env::{ColorScheme, ContrastLevel, MemorySystemEnv, ThemeSnapshot};
use nve_ui::audit::{indicator_contrast_violations, text_contrast_violations};
use nve_ui::{Appearance, Theme, Tone, wcag_contrast};

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
        ThemeSnapshot {
            color_scheme: Some(scheme),
            contrast,
            ..env.theme
        },
        env.locale,
    ));
    Arc::new(StudioHostServices::with_memory(mem))
}

/// Chrome state layered on a mounted view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chrome {
    /// Desktop panes with the docked Studio panel.
    Desktop,
    /// 393pt portrait with the navigation drawer and the Studio sheet open.
    MobileDrawers,
    /// The Cmd+K palette open over the view.
    Palette,
    /// A doc block open in the in-place editor (Docs view).
    DocEdit,
}

#[derive(Debug, Clone, Copy)]
struct Scenario {
    view: ActiveView,
    chrome: Chrome,
    accent: AccentTheme,
    xray: bool,
    os: (ColorScheme, ContrastLevel),
}

/// `(text runs, indicators, violations as readable lines)` of one audit.
#[derive(Default)]
struct AuditTotals {
    runs: usize,
    indicators: usize,
    violations: Vec<String>,
}

/// Mounts `sc` and audits its text runs (WCAG 1.4.3/1.4.6) and checked/selected indicators
/// (WCAG 1.4.11) against the bars of its OS contrast level.
fn audit(sc: Scenario, totals: &mut AuditTotals) {
    let mut state = WorkspaceState::default();
    state.set_accent(sc.accent);
    if sc.xray {
        state.toggle_xray();
    }
    state.select_view(sc.view);
    if sc.view == ActiveView::Stream100k {
        state.toggle_stream_row_expanded(0);
    }
    match sc.chrome {
        Chrome::Desktop => {}
        Chrome::MobileDrawers => {
            state.set_viewport_size(393.0, 852.0);
            state.toggle_mobile_nav();
            state.current.mobile_studio_open = true;
        }
        Chrome::Palette => {
            state.open_palette();
            state.set_palette_query("a");
        }
        Chrome::DocEdit => {
            let block = state.current.docs[0].blocks[0].id;
            assert!(
                state.begin_edit_doc_block(block),
                "{sc:?}: block {block} must open for editing"
            );
        }
    }
    let mut surface =
        create_surface_with_services(1, Arc::new(Mutex::new(state)), services(sc.os.0, sc.os.1));
    surface.reconcile(1);
    let (text_min, indicator_min) = if sc.os.1 == ContrastLevel::High {
        (7.0, 4.5)
    } else {
        (4.5, 3.0)
    };
    let store = surface.store();
    totals.runs += store
        .live_nodes()
        .filter(|&id| store.try_row::<TextContent>(id).is_some())
        .count();
    totals.indicators += store
        .live_nodes()
        .filter(|&id| {
            store.try_row::<Accessibility>(id).is_some_and(|a| {
                matches!(a.checked(), AriaTriState::True | AriaTriState::Mixed)
                    || a.selected() == AriaTriState::True
            })
        })
        .count();
    let line = |kind: &str, v: nve_ui::audit::ContrastViolation| {
        format!(
            "{sc:?} {kind}: {:?} {:.2}:1 ({:?} on {:?})",
            v.text, v.ratio, v.fg, v.bg
        )
    };
    totals.violations.extend(
        text_contrast_violations(store, text_min)
            .into_iter()
            .map(|v| line("text", v)),
    );
    totals.violations.extend(
        indicator_contrast_violations(store, indicator_min)
            .into_iter()
            .map(|v| line("indicator", v)),
    );
}

const ACCENTS: [AccentTheme; 4] = AccentTheme::ALL;

#[test]
fn every_view_accent_and_polarity_keeps_text_and_indicators_readable() {
    let mut totals = AuditTotals::default();
    for os in POLARITIES {
        for accent in ACCENTS {
            for view in VIEWS {
                audit(
                    Scenario {
                        view,
                        chrome: Chrome::Desktop,
                        accent,
                        xray: false,
                        os,
                    },
                    &mut totals,
                );
            }
        }
    }
    assert!(
        totals.runs > 10_000,
        "the walk must cover the mounted views, saw only {} text runs",
        totals.runs
    );
    // Every mount shows at least the selected nav tab and the checked Studio options.
    let mounts = POLARITIES.len() * ACCENTS.len() * VIEWS.len();
    assert!(
        totals.indicators >= mounts * 4,
        "the walk must cover selected tabs/options, saw only {}",
        totals.indicators
    );
    assert!(
        totals.violations.is_empty(),
        "{} violations:\n{}",
        totals.violations.len(),
        totals.violations.join("\n")
    );
}

#[test]
fn mobile_drawers_palette_and_doc_edit_chrome_stay_readable() {
    let mut totals = AuditTotals::default();
    for os in POLARITIES {
        for accent in ACCENTS {
            for (view, chrome) in [
                (ActiveView::Board, Chrome::MobileDrawers),
                (ActiveView::Docs, Chrome::MobileDrawers),
                (ActiveView::Board, Chrome::Palette),
                (ActiveView::Docs, Chrome::DocEdit),
            ] {
                audit(
                    Scenario {
                        view,
                        chrome,
                        accent,
                        xray: false,
                        os,
                    },
                    &mut totals,
                );
            }
        }
    }
    assert!(totals.runs > 1_000, "saw only {} text runs", totals.runs);
    assert!(
        totals.violations.is_empty(),
        "{} violations:\n{}",
        totals.violations.len(),
        totals.violations.join("\n")
    );
}

/// The pairings a [`Tone`] promises (see `nve_ui::Tone`): `text` on the page surfaces and on its
/// own `subtle` fill, a `border` visible against the page surfaces, and `on_solid` on `solid`
/// (`solid` is only ever a fill under `on_solid`, so it promises no boundary of its own).
fn tone_violations(theme: &Theme, name: &str, tone: Tone) -> Vec<String> {
    let high = theme.contrast() == ContrastLevel::High;
    let (text_min, non_text_min) = if high { (7.0, 4.5) } else { (4.5, 3.0) };
    let page = [
        ("base", theme.surface.base),
        ("subtle", theme.surface.subtle),
    ];
    let own_fill = [("tone.subtle", tone.subtle)];
    let checks = page
        .iter()
        .chain(&own_fill)
        .map(|&(surface, bg)| ("text", tone.text, surface, bg, text_min))
        .chain(
            page.iter()
                .map(|&(surface, bg)| ("border", tone.border, surface, bg, non_text_min)),
        );
    let mut out = Vec::new();
    for (role, fg, surface, bg, min) in checks {
        let ratio = wcag_contrast(fg, bg);
        if ratio < min {
            out.push(format!(
                "{:?} {name}.{role} on {surface}: {ratio:.2}:1 < {min}",
                theme.polarity
            ));
        }
    }
    let ratio = wcag_contrast(tone.on_solid, tone.solid);
    if ratio < text_min {
        out.push(format!(
            "{:?} {name}.on_solid on solid: {ratio:.2}:1 < {text_min}",
            theme.polarity
        ));
    }
    out
}

#[test]
fn task_status_and_author_tones_keep_their_contrast_contract_in_all_four_polarities() {
    let mut violations = Vec::new();
    for (scheme, contrast) in POLARITIES {
        for accent in ACCENTS {
            let theme = studio_theme(
                &StudioConfig::default(),
                accent,
                Appearance { scheme, contrast },
            );
            for status in [TaskStatus::Todo, TaskStatus::InProgress, TaskStatus::Done] {
                violations.extend(tone_violations(
                    &theme,
                    &format!("{status:?}"),
                    task_status_tone(&theme, status),
                ));
            }
            for author in [
                "Samuel", "Ada", "Linus", "Grace", "Ken", "Barbara", "Edsger", "Margaret",
            ] {
                violations.extend(tone_violations(&theme, author, author_tone(&theme, author)));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "{} tone violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

#[test]
fn mounted_surface_paints_the_os_polarity_from_its_first_frame() {
    for (scheme, contrast) in POLARITIES {
        let config = WorkspaceState::default().current.studio;
        let expected = studio_theme(&config, config.accent, Appearance { scheme, contrast })
            .surface
            .canvas;
        let mut surface = create_surface_with_services(
            1,
            Arc::new(Mutex::new(WorkspaceState::default())),
            services(scheme, contrast),
        );
        surface.reconcile(1);
        let store = surface.store();
        let canvases: Vec<ColorRgba> = store
            .live_nodes()
            .filter_map(|id| {
                store
                    .try_row::<Visual>(id)
                    .map(|v| ColorRgba::from_u32(v.background().0))
            })
            .filter(|&c| c == expected)
            .collect();
        assert!(
            !canvases.is_empty(),
            "{scheme:?}/{contrast:?}: no node paints the expected canvas {expected:?}"
        );
    }
}

#[test]
fn xray_overlay_keeps_text_and_indicators_readable() {
    let mut totals = AuditTotals::default();
    for os in POLARITIES {
        for accent in ACCENTS {
            for view in [ActiveView::Board, ActiveView::Docs, ActiveView::Workbench] {
                audit(
                    Scenario {
                        view,
                        chrome: Chrome::Desktop,
                        accent,
                        xray: true,
                        os,
                    },
                    &mut totals,
                );
            }
        }
    }
    assert!(
        totals.violations.is_empty(),
        "{} violations:\n{}",
        totals.violations.len(),
        totals.violations.join("\n")
    );
}
