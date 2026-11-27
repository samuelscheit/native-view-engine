//! `ActiveView::Islands` view built on shared `nve-ui` components (`Card`, `Button`, `Badge`,
//! `Separator`), separated from `ui_islands.rs` runtime logic. Subscribes to the `islands` domain.

use nve_dioxus::prelude::*;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, Separator};

use crate::theme::badge_min_w;
use crate::ui::store::use_studio;
use crate::ui::style::use_studio_style;
use crate::ui_islands::GestureSimMode;

#[component]
pub(crate) fn IslandsView() -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let islands = store.islands.read().snapshot.clone();

    let summary_line = format!(
        "Surfaces: {} | Bus: {} tx ({} deliv) | Gesture: {}",
        islands.active_surface_count,
        islands.bus_broadcast_count,
        islands.bus_total_deliveries,
        islands.last_gesture_disposition
    );
    let winner = islands.last_gesture_winner.map_or_else(|| "None".to_owned(), |id| format!("Surface #{id}"));
    let gesture_detail = format!(
        "{} | winner: {winner} | cancelled: {} | {}",
        islands.last_gesture_disposition, islands.last_gesture_cancelled_count, islands.last_gesture_reason
    );
    let bus_btn = s.compact_control("flex: 1; height: 26px; min-height: 26px;", 4, s.m.button_radius, Some(true));
    let row = s.row();
    let card = s.card();

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("islands-summary".into()),
            padding: Some(8.0),
            gap: Some(3.0),
            style: Some(format!(
                "width: 100%; flex-shrink: 0; padding: 8px; gap: 3px; background-color: {}; border-radius: {}px; {card}",
                t.surface.base.hex(),
                s.m.button_radius
            )),
            text { style: "color: {t.text.link.hex()}; font-size: 10px;", "{summary_line}" }
            text { style: "color: {t.text.secondary.hex()}; font-size: 10px;", "{islands.last_bus_summary}" }
            Separator {}
            text {
                test_id: "islands-gesture-readout",
                style: "color: {t.text.tertiary.hex()}; font-size: 10px;",
                "{gesture_detail}"
            }
        }

        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; {row}",
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("island-bus-theme".into()),
                style: Some(bus_btn.clone()),
                on_press: move |_| store.update(|st| st.broadcast_island_theme()),
                "Bus: Theme"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("island-bus-sync".into()),
                style: Some(bus_btn),
                on_press: move |_| store.update(|st| st.broadcast_island_sync()),
                "Bus: Sync Bin"
            }
        }

        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; {row}",
            for (mode, tid, lbl) in [
                (GestureSimMode::Claim, "island-gesture-claim", "Claim"),
                (GestureSimMode::PassThrough, "island-gesture-passthrough", "PassThrough"),
                (GestureSimMode::DeferToParent, "island-gesture-defer", "Defer"),
            ] {
                Button {
                    key: "{tid}",
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(tid.into()),
                    style: Some(s.compact_control(
                        "flex: 1; height: 24px; min-height: 24px;",
                        2,
                        s.m.pill_radius,
                        Some(islands.last_gesture_disposition == mode.label()),
                    )),
                    on_press: move |_| store.update(|st| st.simulate_island_gesture(mode)),
                    "{lbl}"
                }
            }
        }

        scrollview {
            test_id: "islands-list",
            style: "width: 100%; flex: 1; flex-direction: column; gap: {s.m.list_gap}px; {row}",
            for island in islands.cards {
                {
                    let is_resumed = island.lifecycle == "Resumed";
                    let meta_lbl = format!(
                        "Surface #{} | {} | {} | Bus rx: {}",
                        island.surface_id, island.mount_policy, island.lifecycle, island.received_messages
                    );
                    let toggle_style = if is_resumed {
                        format!(
                            "{} background-color: {}; border-width: 1px; border-color: {};",
                            s.compact_control("width: 64px; height: 22px; min-height: 22px;", 2, s.m.pill_radius, None),
                            t.status.success.subtle.hex(),
                            t.status.success.border.hex()
                        )
                    } else {
                        s.compact_control("width: 64px; height: 22px; min-height: 22px;", 2, s.m.pill_radius, Some(false))
                    };
                    let slug = island.slug.clone();
                    rsx! {
                        Card {
                            key: "island-{island.slug}",
                            variant: CardVariant::Elevated,
                            test_id: Some(format!("island-card-{}", island.slug)),
                            padding: Some(s.m.card_pad as f32),
                            gap: Some(s.m.card_gap as f32),
                            style: Some(format!(
                                "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {}; border-radius: {}px; {card}",
                                s.m.card_pad,
                                s.m.card_gap,
                                t.surface.raised.hex(),
                                s.m.card_radius
                            )),
                            view {
                                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center;",
                                view {
                                    style: "flex-direction: row; align-items: center; gap: 6px;",
                                    Badge {
                                        label: island.host.label().to_string(),
                                        variant: BadgeVariant::Neutral,
                                        style: Some(s.compact_control(
                                            &format!("min-width: {}px; height: 20px;", badge_min_w(island.host.label())),
                                            4,
                                            s.m.pill_radius,
                                            None,
                                        )),
                                    }
                                    text { style: "color: {t.text.primary.hex()}; font-size: 11px;", "{island.component_name}" }
                                }
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(format!("island-toggle-{}", island.slug)),
                                    style: Some(toggle_style),
                                    on_press: move |_| store.update(|st| st.toggle_island_lifecycle(&slug)),
                                    if is_resumed { "Pause" } else { "Resume" }
                                }
                            }
                            text { style: "color: {t.text.tertiary.hex()}; font-size: 10px;", "{meta_lbl}" }
                        }
                    }
                }
            }
        }
    }
}
