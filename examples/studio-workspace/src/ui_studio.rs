//! Docked / drawer **NVE Studio** live self-editing meta-workbench panel.
//!
//! Built with shared `nve-ui` primitives (`Card`, `Button`, `Badge`, `Separator`), exposing
//! Accent swatches, Corner Radius, Density, Live Taffy <-> Yoga Layout Engine switcher,
//! X-Ray HUD, Live Sync & multi-peer binary `NveCrdtDelta` CRDT telemetry (with Sync Now and
//! Partition/Heal controls), and deterministic Time-Travel Scrubber.
//!
//! [`StudioPanel`] subscribes to the `studio` domain; [`SyncCard`] to `sync` and
//! [`TimeTravelCard`] to `inspector`, so telemetry ticks and history scrubs repaint only their card.

use dioxus_core::EventHandler;
use nve_dioxus::prelude::*;
use nve_dioxus::traits::TraitScope;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator,
};

use crate::state::{AccentTheme, CornerRadiusMode, DensityMode, LayoutEngineMode, WorkspaceState};
use crate::theme::badge_min_w;
use crate::traits::{StudioTraitPreviewChip, studio_compact_override};
use crate::ui::store::{StudioStore, use_studio};
use crate::ui::style::{
    StudioStyle, StudioStyleExt, badge_dims, data_attrs, flex_dims, min_w_dims, row_between,
    text_style, use_studio_style,
};

fn section_style(s: &StudioStyle) -> TypedStyle {
    s.card_surface(5, 3, s.t.surface.sunken, s.m.button_radius)
}

fn segment_style(s: &StudioStyle, selected: bool) -> TypedStyle {
    s.compact_control(flex_dims(20.0), 2, s.m.pill_radius, Some(selected))
}

/// One radio group of mutually exclusive Studio options (the checked one exposes `aria-checked`
/// and its ≥3:1 accent boundary); `apply` mutates the workspace for the pressed one.
#[component]
fn OptionRow(
    options: Vec<(&'static str, &'static str, bool)>,
    apply: EventHandler<usize>,
) -> Element {
    nve_dioxus::render_probe::record_render("OptionRow");
    let s = use_studio_style();
    rsx! {
        view {
            accessibility_role: "radiogroup",
            nve_style: TypedStyle::new().width_pct(100.0).flex_row().gap(4.0),
            for (idx, (tid, label, selected)) in options.into_iter().enumerate() {
                pressable {
                    key: "{tid}",
                    test_id: "{tid}",
                    accessibility_role: "radio",
                    aria_checked: if selected { "true" } else { "false" },
                    nve_style: segment_style(&s, selected),
                    onpress: move |_| apply.call(idx),
                    text { nve_style: text_style(s.control_fg(selected), 11.0), "{label}" }
                }
            }
        }
    }
}

/// `OptionRow` options for `choices` with the `current` one selected.
fn options<T: Copy + PartialEq>(
    choices: &[(T, &'static str, &'static str)],
    current: T,
) -> Vec<(&'static str, &'static str, bool)> {
    choices
        .iter()
        .map(|&(v, tid, label)| (tid, label, v == current))
        .collect()
}

/// Applies `choices[idx]` through `set`.
fn apply_choice<T: Copy>(
    store: StudioStore,
    choices: &[(T, &'static str, &'static str)],
    idx: usize,
    set: fn(&mut WorkspaceState, T),
) {
    if let Some(&(v, _, _)) = choices.get(idx) {
        store.update(|st| set(st, v));
    }
}

const ACCENTS: &[(AccentTheme, &str, &str)] = &[
    (AccentTheme::Cyan, "accent-cyan", "Cyan"),
    (AccentTheme::Violet, "accent-violet", "Violet"),
    (AccentTheme::Green, "accent-green", "Green"),
    (AccentTheme::Amber, "accent-amber", "Amber"),
];
const RADII: &[(CornerRadiusMode, &str, &str)] = &[
    (CornerRadiusMode::Sharp, "radius-sharp", "Sharp"),
    (CornerRadiusMode::Soft, "radius-soft", "Soft"),
    (CornerRadiusMode::Pill, "radius-pill", "Pill"),
];
const DENSITIES: &[(DensityMode, &str, &str)] = &[
    (DensityMode::Normal, "density-normal", "Normal"),
    (DensityMode::Compact, "density-compact", "Compact"),
];
const ENGINES: &[(LayoutEngineMode, &str, &str)] = &[
    (LayoutEngineMode::Taffy, "layout-taffy", "Taffy"),
    (LayoutEngineMode::Yoga, "layout-yoga", "Yoga"),
];

#[component]
pub(crate) fn StudioPanel(mobile: bool) -> Element {
    nve_dioxus::render_probe::record_render("StudioPanel");
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let config = store.style.read().config;
    let engine = config.layout_engine;
    let panel_base = if mobile {
        TypedStyle::new().width_pct(100.0)
    } else {
        TypedStyle::new().width_px(210.0).height_pct(100.0)
    };
    let panel_style = panel_base
        .flex_shrink(0.0)
        .flex_col()
        .padding(7.0)
        .gap(5.0)
        .bg(t.surface.base)
        .outline(s.panel());
    let section = section_style(&s);
    let caption = t.text.tertiary;
    let xray = config.xray;

    rsx! {
        view {
            test_id: "studio-panel",
            nve_table: data_attrs([("layout-engine", engine.as_str().to_owned())]),
            nve_style: panel_style,

            view {
                nve_style: row_between().outline(s.row()),
                text { nve_style: text_style(t.text.primary, 12.0), "NVE STUDIO" }
                Badge {
                    label: "LIVE SELF-EDIT".to_string(),
                    variant: BadgeVariant::Accent,
                    nve_style: Some(s.compact_control(
                        badge_dims(badge_min_w("LIVE SELF-EDIT") as f32, 17.0),
                        4,
                        s.m.pill_radius,
                        None,
                    )),
                }
            }

            // Live environment readout (inherits the root TraitProvider; the compact preview uses TraitScope).
            StudioTraitPreviewChip { test_id: Some("studio-trait-live".into()) }
            if !mobile {
                TraitScope {
                    overrides: studio_compact_override(true),
                    StudioTraitPreviewChip { test_id: Some("studio-trait-scoped".into()) }
                }
            }

            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                nve_style: Some(section.clone()),
                text { nve_style: text_style(caption, 9.0), "ACCENT" }
                OptionRow {
                    options: options(&ACCENTS[..2], config.accent),
                    apply: move |idx| apply_choice(store, ACCENTS, idx, WorkspaceState::set_accent),
                }
                OptionRow {
                    options: options(&ACCENTS[2..], config.accent),
                    apply: move |idx| apply_choice(store, &ACCENTS[2..], idx, WorkspaceState::set_accent),
                }
            }

            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                nve_style: Some(section.clone()),
                text { nve_style: text_style(caption, 9.0), "RADIUS (0 / 12 / 18)" }
                OptionRow {
                    options: options(RADII, config.radius),
                    apply: move |idx| apply_choice(store, RADII, idx, WorkspaceState::set_radius),
                }
                text { nve_style: text_style(caption, 9.0), "DENSITY" }
                OptionRow {
                    options: options(DENSITIES, config.density),
                    apply: move |idx| apply_choice(store, DENSITIES, idx, WorkspaceState::set_density),
                }
            }

            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                nve_style: Some(section),
                text { nve_style: text_style(caption, 9.0), "LAYOUT ENGINE" }
                OptionRow {
                    options: options(ENGINES, engine),
                    apply: move |idx| apply_choice(store, ENGINES, idx, WorkspaceState::set_layout_engine),
                }
                view {
                    test_id: "layout-engine-readout",
                    nve_table: data_attrs([("layout-engine", engine.as_str().to_owned())]),
                    nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0),
                    text { nve_style: text_style(t.text.link, 9.0), "{engine.readout_label()}" }
                }
            }

            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("toggle-xray".into()),
                nve_style: Some(s.compact_control(
                    TypedStyle::new().width_pct(100.0).height_px(24.0).min_height_px(24.0).flex_shrink(0.0),
                    4,
                    s.m.button_radius,
                    Some(xray),
                )),
                on_press: move |_| store.update(|st| st.toggle_xray()),
                if xray { "X-Ray HUD: ON" } else { "X-Ray HUD: OFF" }
            }

            SyncCard {}
            TimeTravelCard {}
        }
    }
}

/// Live sync / pairing + binary `NveCrdtDelta` CRDT mesh telemetry.
#[component]
fn SyncCard() -> Element {
    nve_dioxus::render_probe::record_render("SyncCard");
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let sync = store.sync.read().telemetry.clone();
    let peers = format!(
        "{} {}",
        sync.connected_peers,
        if sync.connected_peers == 1 {
            "peer"
        } else {
            "peers"
        }
    );
    let metrics = format!(
        "{} | {} | {}",
        sync.format_kbps(),
        sync.format_compression(),
        sync.format_rtt()
    );
    let crdt_summary = format!(
        "CRDT {}/{}d | {}B",
        sync.crdt_deltas_sent, sync.crdt_deltas_received, sync.crdt_bytes_synced
    );
    let split = sync.crdt_partitioned;
    let status_lbl = if split { "SPLIT" } else { "MESH" };
    let transport = sync.transport.label();
    let between = TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center);

    rsx! {
        Card {
            variant: CardVariant::Subtle,
            test_id: Some("studio-sync-telemetry".into()),
            padding: Some(5.0),
            gap: Some(3.0),
            nve_style: Some(section_style(&s)),
            view {
                nve_table: data_attrs([("sync-transport", sync.transport.as_str().to_owned())]),
                nve_style: between.clone(),
                text { nve_style: text_style(t.text.tertiary, 9.0), "LIVE SYNC" }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("sync-transport-cycle".into()),
                    accessibility_label: Some("Cycle live sync transport mode".into()),
                    nve_style: Some(s.compact_control(
                        min_w_dims(badge_min_w(transport) as f32, 17.0),
                        4,
                        s.m.pill_radius,
                        Some(true),
                    )),
                    on_press: move |_| store.update(|st| st.cycle_sync_transport()),
                    "{transport}"
                }
            }
            view {
                nve_style: between.clone(),
                text { nve_style: text_style(t.text.primary, 9.0), "{peers}" }
                text { nve_style: text_style(t.text.secondary, 9.0), "{metrics}" }
            }
            Separator {}
            view {
                test_id: "studio-crdt-metrics",
                nve_style: between,
                text { nve_style: text_style(t.text.link, 9.0), "{crdt_summary}" }
                Badge {
                    label: status_lbl.to_string(),
                    variant: if split { BadgeVariant::Danger } else { BadgeVariant::Success },
                    nve_style: Some(s.compact_control(
                        badge_dims(badge_min_w(status_lbl) as f32, 15.0),
                        3,
                        s.m.pill_radius,
                        None,
                    )),
                }
            }
            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_row().gap(4.0),
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("crdt-sync-now".into()),
                    accessibility_label: Some("Sync binary CRDT deltas now".into()),
                    nve_style: Some(segment_style(&s, true)),
                    on_press: move |_| store.update(|st| st.sync_crdt_now()),
                    "Sync Now"
                }
                Button {
                    variant: if split { ButtonVariant::Primary } else { ButtonVariant::Destructive },
                    size: ButtonSize::Sm,
                    test_id: Some("crdt-toggle-partition".into()),
                    accessibility_label: Some("Simulate or heal CRDT network partition".into()),
                    nve_style: Some(s.compact_control(flex_dims(20.0), 2, s.m.pill_radius, None)),
                    on_press: move |_| store.update(|st| st.toggle_crdt_partition()),
                    if split { "Heal Mesh" } else { "Partition" }
                }
            }
        }
    }
}

/// Deterministic time-travel scrubber.
#[component]
fn TimeTravelCard() -> Element {
    nve_dioxus::render_probe::record_render("TimeTravelCard");
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let i = store.inspector.read().clone();
    let step = |enabled: bool| {
        let style = segment_style(&s, false);
        if enabled { style } else { style.opacity(0.4) }
    };
    let (undo_style, redo_style) = (step(i.can_undo), step(i.can_redo));
    let btn_row = TypedStyle::new().width_pct(100.0).flex_row().gap(4.0);

    rsx! {
        Card {
            variant: CardVariant::Subtle,
            padding: Some(5.0),
            gap: Some(3.0),
            nve_style: Some(section_style(&s)),
            text { nve_style: text_style(t.text.tertiary, 9.0), "TIME TRAVEL" }
            text { nve_style: text_style(t.text.primary, 10.0), "Snapshot {i.snapshot_index} / {i.history_len}" }
            text { nve_style: text_style(t.text.link, 9.0), "{i.action_label}" }
            view {
                nve_style: btn_row.clone(),
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-first".into()),
                    nve_style: Some(undo_style.clone()),
                    on_press: move |_| store.update(|st| st.scrub_first()),
                    "<< #1"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-undo".into()),
                    nve_style: Some(undo_style),
                    on_press: move |_| store.update(|st| st.scrub_undo()),
                    "< Undo"
                }
            }
            view {
                nve_style: btn_row,
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-redo".into()),
                    nve_style: Some(redo_style.clone()),
                    on_press: move |_| store.update(|st| st.scrub_redo()),
                    "Redo >"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-live".into()),
                    nve_style: Some(redo_style),
                    on_press: move |_| store.update(|st| st.scrub_live()),
                    "Live >>"
                }
            }
        }
    }
}
