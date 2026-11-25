//! RSX renderer for `ActiveView::Islands` built on shared `nve-ui` components
//! (`Card`, `Button`, `Badge`, `Separator`), separated from `ui_islands.rs` runtime logic.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator};

use crate::state::WorkspaceState;
use crate::theme::{
    badge_min_w, card_border, control_border, row_border, LayoutMetrics, CARD_ALT_BG, CARD_BG,
    CONTROL_BG, PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::ui::mutate_state;
use crate::ui_islands::{GestureSimMode, IslandsSnapshot};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_islands_view(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    islands: &IslandsSnapshot,
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let summary_line = format!(
        "Surfaces: {} | Bus: {} tx ({} deliv) | Gesture: {}",
        islands.active_surface_count,
        islands.bus_broadcast_count,
        islands.bus_total_deliveries,
        islands.last_gesture_disposition
    );
    let winner_str = islands
        .last_gesture_winner
        .map(|id| format!("Surface #{id}"))
        .unwrap_or_else(|| "None".into());
    let gesture_detail = format!(
        "{} | winner: {} | cancelled: {} | {}",
        islands.last_gesture_disposition,
        winner_str,
        islands.last_gesture_cancelled_count,
        islands.last_gesture_reason
    );

    let s_bus_theme = Arc::clone(store);
    let s_bus_sync = Arc::clone(store);
    let s_g_claim = Arc::clone(store);
    let s_g_pass = Arc::clone(store);
    let s_g_defer = Arc::clone(store);

    let summary_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 3px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let bus_btn_style = format!(
        "flex: 1; height: 26px; min-height: 26px; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.button_radius
    );

    rsx! {
        // Orchestrator + Bus + Gesture Summary Telemetry (`nve-ui` Card)
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("islands-summary".into()),
            padding: Some(8.0),
            gap: Some(3.0),
            style: Some(summary_card_style),
            text { style: "color: {accent_hex}; font-size: 10px;", "{summary_line}" }
            text { style: "color: {TEXT_SECONDARY}; font-size: 10px;", "{islands.last_bus_summary}" }
            Separator {}
            text {
                test_id: "islands-gesture-readout",
                style: "color: {TEXT_MUTED}; font-size: 10px;",
                "{gesture_detail}"
            }
        }

        // Row 1: NveChannelBus live broadcasts (`nve-ui` Button)
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("island-bus-theme".into()),
                style: Some(bus_btn_style.clone()),
                on_press: move |_| mutate_state(&s_bus_theme, version, |st| st.broadcast_island_theme()),
                "Bus: Theme"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("island-bus-sync".into()),
                style: Some(bus_btn_style),
                on_press: move |_| mutate_state(&s_bus_sync, version, |st| st.broadcast_island_sync()),
                "Bus: Sync Bin"
            }
        }

        // Row 2: GestureArbitrator simulation triggers (`nve-ui` Button)
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; border-width: {row_bw}px; border-color: {row_bc};",
            for &(mode, tid, lbl) in [
                (GestureSimMode::Claim, "island-gesture-claim", "Claim"),
                (GestureSimMode::PassThrough, "island-gesture-passthrough", "PassThrough"),
                (GestureSimMode::DeferToParent, "island-gesture-defer", "Defer"),
            ].iter() {
                {
                    let is_active = islands.last_gesture_disposition == mode.label();
                    let (bw, bc) = control_border(xray, is_active, accent_hex);
                    let bg = if is_active { accent_tint } else { CONTROL_BG };
                    let bstyle = format!(
                        "flex: 1; height: 24px; min-height: 24px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                        m.pill_radius
                    );
                    let s_g = match mode {
                        GestureSimMode::Claim => Arc::clone(&s_g_claim),
                        GestureSimMode::PassThrough => Arc::clone(&s_g_pass),
                        GestureSimMode::DeferToParent => Arc::clone(&s_g_defer),
                    };
                    rsx! {
                        Button {
                            key: "{tid}",
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            test_id: Some(tid.into()),
                            style: Some(bstyle),
                            on_press: move |_| mutate_state(&s_g, version, |st| st.simulate_island_gesture(mode)),
                            "{lbl}"
                        }
                    }
                }
            }
        }

        // Interactive Multi-Host Island Cards (`nve-ui` Card + Badge + Button)
        scrollview {
            test_id: "islands-list",
            style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
            for card in islands.cards.iter() {
                {
                    let slug = card.slug.clone();
                    let toggle_tid = format!("island-toggle-{}", card.slug);
                    let card_tid = format!("island-card-{}", card.slug);
                    let is_resumed = card.lifecycle == "Resumed";
                    let toggle_lbl = if is_resumed { "Pause" } else { "Resume" };
                    let life_bg = if is_resumed { "#052e16" } else { CARD_ALT_BG };
                    let (tbw, tbc) = control_border(xray, is_resumed, accent_hex);
                    let meta_lbl = format!(
                        "Surface #{} | {} | {} | Bus rx: {}",
                        card.surface_id, card.mount_policy, card.lifecycle, card.received_messages
                    );
                    let icard_style = format!(
                        "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {CARD_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
                        m.card_pad, m.card_gap, m.card_radius
                    );
                    let host_badge_style = format!(
                        "min-width: {}px; height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {}; border-radius: {}px; border-width: 0px; border-color: transparent;",
                        badge_min_w(card.host.label()), card.host.badge_bg(), m.pill_radius
                    );
                    let tog_btn_style = format!(
                        "width: 64px; height: 22px; min-height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {life_bg}; border-radius: {}px; border-width: {tbw}px; border-color: {tbc};",
                        m.pill_radius
                    );
                    let s_tog = Arc::clone(store);
                    rsx! {
                        Card {
                            key: "island-{card.slug}",
                            variant: CardVariant::Elevated,
                            test_id: Some(card_tid),
                            padding: Some(m.card_pad as f32),
                            gap: Some(m.card_gap as f32),
                            style: Some(icard_style),
                            view {
                                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center;",
                                view {
                                    style: "flex-direction: row; align-items: center; gap: 6px;",
                                    Badge {
                                        label: card.host.label().to_string(),
                                        variant: BadgeVariant::Neutral,
                                        style: Some(host_badge_style),
                                    }
                                    text { style: "color: {TEXT_PRIMARY}; font-size: 11px;", "{card.component_name}" }
                                }
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(toggle_tid),
                                    style: Some(tog_btn_style),
                                    on_press: move |_| {
                                        let sl = slug.clone();
                                        mutate_state(&s_tog, version, |st| st.toggle_island_lifecycle(&sl));
                                    },
                                    "{toggle_lbl}"
                                }
                            }
                            text { style: "color: {TEXT_MUTED}; font-size: 10px;", "{meta_lbl}" }
                        }
                    }
                }
            }
        }
    }
}
