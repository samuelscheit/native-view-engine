//! `ActiveView::Islands` view built on shared `nve-ui` components (`Card`, `Button`, `Badge`,
//! `Separator`), separated from `ui_islands.rs` runtime logic. Subscribes to the `islands` domain.

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator,
};

use crate::theme::badge_min_w;
use crate::ui::store::use_studio;
use crate::ui::style::{
    StudioStyleExt, badge_dims, fixed_dims, flex_dims, row_between, row_center, text_style,
    use_studio_style,
};
use crate::ui_islands::GestureSimMode;

#[component]
pub(crate) fn IslandsView() -> Element {
    nve_dioxus::render_probe::record_render("IslandsView");
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
    let winner = islands
        .last_gesture_winner
        .map_or_else(|| "None".to_owned(), |id| format!("Surface #{id}"));
    let gesture_detail = format!(
        "{} | winner: {winner} | cancelled: {} | {}",
        islands.last_gesture_disposition,
        islands.last_gesture_cancelled_count,
        islands.last_gesture_reason
    );
    let bus_btn = s.compact_control(flex_dims(26.0), 4, s.m.button_radius, Some(true));
    let row = s.row();

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            test_id: Some("islands-summary".into()),
            padding: Some(8.0),
            gap: Some(3.0),
            nve_style: Some(s.card_surface(8, 3, t.surface.base, s.m.button_radius)),
            text { nve_style: text_style(t.text.link, 10.0), "{summary_line}" }
            text { nve_style: text_style(t.text.secondary, 10.0), "{islands.last_bus_summary}" }
            Separator {}
            text {
                test_id: "islands-gesture-readout",
                nve_style: text_style(t.text.tertiary, 10.0),
                "{gesture_detail}"
            }
        }

        view {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_shrink(0.0)
                .flex_row()
                .gap(6.0)
                .outline(row),
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("island-bus-theme".into()),
                nve_style: Some(bus_btn.clone()),
                on_press: move |_| store.update(|st| st.broadcast_island_theme()),
                "Bus: Theme"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("island-bus-sync".into()),
                nve_style: Some(bus_btn),
                on_press: move |_| store.update(|st| st.broadcast_island_sync()),
                "Bus: Sync Bin"
            }
        }

        view {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_shrink(0.0)
                .flex_row()
                .gap(5.0)
                .outline(row),
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
                    nve_style: Some(s.compact_control(
                        flex_dims(24.0),
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
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_1()
                .flex_col()
                .gap(s.m.list_gap as f32)
                .outline(row),
            for island in islands.cards {
                {
                    let is_resumed = island.lifecycle == "Resumed";
                    let meta_lbl = format!(
                        "Surface #{} | {} | {} | Bus rx: {}",
                        island.surface_id, island.mount_policy, island.lifecycle, island.received_messages
                    );
                    let toggle_style = if is_resumed {
                        s.compact_control(fixed_dims(64.0, 22.0), 2, s.m.pill_radius, None)
                            .bg(t.status.success.subtle)
                            .border(1.0, t.status.success.border)
                    } else {
                        s.compact_control(fixed_dims(64.0, 22.0), 2, s.m.pill_radius, Some(false))
                    };
                    let slug = island.slug.clone();
                    rsx! {
                        Card {
                            key: "island-{island.slug}",
                            variant: CardVariant::Elevated,
                            test_id: Some(format!("island-card-{}", island.slug)),
                            padding: Some(s.m.card_pad as f32),
                            gap: Some(s.m.card_gap as f32),
                            nve_style: Some(s.card_surface(
                                s.m.card_pad,
                                s.m.card_gap,
                                t.surface.raised,
                                s.m.card_radius,
                            )),
                            view {
                                nve_style: row_between(),
                                view {
                                    nve_style: row_center(6.0),
                                    Badge {
                                        label: island.host.label().to_string(),
                                        variant: BadgeVariant::Neutral,
                                        nve_style: Some(s.compact_control(
                                            badge_dims(badge_min_w(island.host.label()) as f32, 20.0),
                                            4,
                                            s.m.pill_radius,
                                            None,
                                        )),
                                    }
                                    text { nve_style: text_style(t.text.primary, 11.0), "{island.component_name}" }
                                }
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(format!("island-toggle-{}", island.slug)),
                                    nve_style: Some(toggle_style),
                                    on_press: move |_| store.update(|st| st.toggle_island_lifecycle(&slug)),
                                    if is_resumed { "Pause" } else { "Resume" }
                                }
                            }
                            text { nve_style: text_style(t.text.tertiary, 10.0), "{meta_lbl}" }
                        }
                    }
                }
            }
        }
    }
}
