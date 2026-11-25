//! Section 4: Overlays & Feedback — `Dialog`, `Modal`, `Popover` (with `positioning.rs`
//! placement switcher), `DropdownMenu`, `Tooltip`, `ToastProvider`/`ToastViewport` triggers,
//! and multi-detent `BottomSheet` (`Peek` / `Half` / `Full`).

use nve_dioxus::prelude::*;
use nve_ui::{
    compute_overlay_position, use_theme, use_toast, Alignment, Badge, BadgeVariant, BottomSheet,
    Button, ButtonSize, ButtonVariant, Card, CardVariant, Dialog, DropdownMenu, MenuItem, Modal,
    Placement, Popover, PositionOptions, Rect, Separator, SheetDetent, Side, Size, ToastOptions,
    ToastVariant, Tooltip, Viewport,
};

use crate::i18n_copy::copy;
use crate::sections::{responsive_two_col, SectionHeader};
use crate::shell::AppHandle;

const PLACEMENTS: [(&str, Placement); 6] = [
    ("Top", Placement::from_side_align(Side::Top, Alignment::Center)),
    ("Bottom", Placement::from_side_align(Side::Bottom, Alignment::Center)),
    ("Left", Placement::from_side_align(Side::Left, Alignment::Center)),
    ("Right", Placement::from_side_align(Side::Right, Alignment::Center)),
    ("BottomStart", Placement::from_side_align(Side::Bottom, Alignment::Start)),
    ("TopEnd", Placement::from_side_align(Side::Top, Alignment::End)),
];

#[component]
pub fn OverlaysSection(handle: AppHandle) -> Element {
    let theme = use_theme();
    let p = &theme.palette;
    let snap = handle.read_state();
    let pseudo = snap.pseudo_locale;
    let mut toast = use_toast();

    let mut dialog_open = use_signal(|| false);
    let mut modal_open = use_signal(|| false);
    let mut popover_open = use_signal(|| false);
    let mut placement_idx = use_signal(|| 1usize);
    let mut menu_open = use_signal(|| false);
    let mut menu_selected = use_signal(|| "None".to_owned());
    let mut tooltip_open = use_signal(|| false);
    let mut sheet_open = use_signal(|| false);
    let mut sheet_detent_idx = use_signal(|| 1usize);
    let mut sheet_detent_label = use_signal(|| "Half (50%)".to_owned());

    let text_primary = p.text_primary.as_ref();
    let text_muted = p.text_muted.as_ref();

    let active_placement = PLACEMENTS[placement_idx() % PLACEMENTS.len()];
    let anchor = Rect {
        x: 180.0,
        y: 120.0,
        width: 140.0,
        height: 36.0,
    };
    let overlay_sz = Size {
        width: 220.0,
        height: 96.0,
    };
    let vp = Viewport {
        width: 640.0,
        height: 360.0,
    };
    let pos_opts = PositionOptions {
        placement: active_placement.1,
        offset: 8.0,
        flip: true,
        shift: true,
        viewport_padding: 8.0,
        ..Default::default()
    };
    let computed = compute_overlay_position(anchor, overlay_sz, vp, &pos_opts);

    let menu_items = vec![
        MenuItem::new("inspect", "Inspect Scene Graph"),
        MenuItem::new("snapshot", "Capture Binary Snapshot"),
        MenuItem::new("locked", "Disabled Action").disabled(true),
        MenuItem::new("clear", "Reset Session").destructive(true),
    ];

    rsx! {
        view {
            test_id: "section-overlays",
            style: "width: 100%; flex-direction: column; gap: 16px;",

            SectionHeader {
                title: copy("Overlays & Feedback", pseudo),
                subtitle: copy("Focus-trapped Dialog & Modal, positioning.rs Popover, DropdownMenu, Tooltip, live ToastProvider queue, and multi-detent BottomSheet.", pseudo),
                tag: "7 Overlays".to_owned(),
            }

            // Toast Notification Triggers
            Card {
                test_id: Some("ks-toasts-card".to_owned()),
                variant: CardVariant::Elevated,
                padding: Some(16.0),
                gap: Some(12.0),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;",
                    text {
                        style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                        "{copy(\"ToastProvider & ToastViewport\", pseudo)}"
                    }
                    Badge {
                        label: format!("Active Toasts: {}", toast.items().len()),
                        variant: BadgeVariant::Accent,
                    }
                }
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 8px;",
                    Button {
                        test_id: Some("ks-toast-info".to_owned()),
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            toast.push(
                                ToastOptions::new("Sync Ready")
                                    .with_description("Binary snapshot persisted in < 1ms.")
                                    .with_variant(ToastVariant::Info),
                            );
                        },
                        text { "{copy(\"Fire Info Toast\", pseudo)}" }
                    }
                    Button {
                        test_id: Some("ks-toast-success".to_owned()),
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            toast.push(
                                ToastOptions::new("Build Verified")
                                    .with_description("Zero Clippy warnings across native & wasm32.")
                                    .with_variant(ToastVariant::Success),
                            );
                        },
                        text { "{copy(\"Fire Success Toast\", pseudo)}" }
                    }
                    Button {
                        test_id: Some("ks-toast-warning".to_owned()),
                        variant: ButtonVariant::Outline,
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            toast.push(
                                ToastOptions::new("High Memory Budget")
                                    .with_description("Approaching 85% of viewport texture cache.")
                                    .with_variant(ToastVariant::Warning),
                            );
                        },
                        text { "{copy(\"Fire Warning Toast\", pseudo)}" }
                    }
                    Button {
                        test_id: Some("ks-toast-error".to_owned()),
                        variant: ButtonVariant::Destructive,
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            toast.push(
                                ToastOptions::new("Connection Interrupted")
                                    .with_description("Hot-reload socket reconnecting with backoff.")
                                    .with_variant(ToastVariant::Danger),
                            );
                        },
                        text { "{copy(\"Fire Error Toast\", pseudo)}" }
                    }
                    Button {
                        test_id: Some("ks-toast-clear".to_owned()),
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        on_press: move |_| toast.clear(),
                        text { "{copy(\"Clear Toasts\", pseudo)}" }
                    }
                }
            }

            // Dialog, Modal & BottomSheet Row
            view {
                nve_style: responsive_two_col(),

                view {
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0;",
                    Card {
                        test_id: Some("ks-dialog-modal-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                            "{copy(\"Dialog & Modal\", pseudo)}"
                        }
                        text {
                            style: "font-size: 12px; color: {text_muted};",
                            "Both trap keyboard focus and dismiss via Escape or backdrop click."
                        }
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 8px;",
                            Button {
                                test_id: Some("ks-open-dialog".to_owned()),
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Sm,
                                on_press: move |_| dialog_open.set(true),
                                text { "{copy(\"Open Dialog\", pseudo)}" }
                            }
                            Button {
                                test_id: Some("ks-open-modal".to_owned()),
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                on_press: move |_| modal_open.set(true),
                                text { "{copy(\"Open Modal\", pseudo)}" }
                            }
                        }
                    }
                }

                view {
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0;",
                    Card {
                        test_id: Some("ks-sheet-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 6px;",
                            text {
                                style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                                "{copy(\"Multi-Detent BottomSheet\", pseudo)}"
                            }
                            Badge {
                                label: sheet_detent_label(),
                                variant: BadgeVariant::Neutral,
                            }
                        }
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            Button {
                                test_id: Some("ks-sheet-peek".to_owned()),
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    sheet_detent_idx.set(0);
                                    sheet_detent_label.set("Peek (25%)".to_owned());
                                    sheet_open.set(true);
                                },
                                text { "Peek" }
                            }
                            Button {
                                test_id: Some("ks-sheet-half".to_owned()),
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    sheet_detent_idx.set(1);
                                    sheet_detent_label.set("Half (50%)".to_owned());
                                    sheet_open.set(true);
                                },
                                text { "Half" }
                            }
                            Button {
                                test_id: Some("ks-sheet-full".to_owned()),
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    sheet_detent_idx.set(2);
                                    sheet_detent_label.set("Full (92%)".to_owned());
                                    sheet_open.set(true);
                                },
                                text { "Full" }
                            }
                        }
                    }
                }
            }

            // Popover (with Placement switcher), DropdownMenu, Tooltip
            Card {
                test_id: Some("ks-popover-dropdown-card".to_owned()),
                variant: CardVariant::Outlined,
                padding: Some(16.0),
                gap: Some(12.0),
                text {
                    style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                    "{copy(\"Popover (positioning.rs), DropdownMenu & Tooltip\", pseudo)}"
                }
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 6px; align-items: center;",
                    for (idx, (label, _)) in PLACEMENTS.iter().enumerate() {
                        Button {
                            key: "{label}",
                            test_id: Some(format!("ks-placement-{label}")),
                            variant: if placement_idx() == idx {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Ghost
                            },
                            size: ButtonSize::Sm,
                            on_press: move |_| placement_idx.set(idx),
                            text { "{label}" }
                        }
                    }
                }
                text {
                    test_id: "ks-positioning-readout",
                    style: "font-size: 12px; color: {text_muted};",
                    "compute_overlay_position({active_placement.0}) -> x: {computed.x:.1}px, y: {computed.y:.1}px, flipped: {computed.flipped}"
                }
                Separator {}
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 12px; align-items: flex-start;",
                    Popover {
                        test_id: Some("ks-popover".to_owned()),
                        open: popover_open(),
                        trigger_label: copy("Toggle Popover", pseudo),
                        anchor_rect: Some(anchor),
                        overlay_size: Some(overlay_sz),
                        viewport: Some(vp),
                        position_options: Some(pos_opts),
                        on_open_change: move |next: bool| popover_open.set(next),
                        view {
                            style: "flex-direction: column; gap: 6px;",
                            text {
                                style: "font-size: 13px; font-weight: 700; color: {text_primary};",
                                "Anchored Popover ({active_placement.0})"
                            }
                            text {
                                style: "font-size: 12px; color: {text_muted};",
                                "Collision-aware positioning with flip & shift."
                            }
                        }
                    }

                    DropdownMenu {
                        test_id: Some("ks-dropdown".to_owned()),
                        open: menu_open(),
                        trigger_label: format!("Actions ({})", menu_selected()),
                        items: menu_items,
                        on_open_change: move |next: bool| menu_open.set(next),
                        on_select: move |id: String| {
                            menu_selected.set(id.clone());
                            menu_open.set(false);
                            toast.push(
                                ToastOptions::new(format!("Menu selected: {id}"))
                                    .with_variant(ToastVariant::Info),
                            );
                        },
                    }

                    Tooltip {
                        test_id: Some("ks-tooltip".to_owned()),
                        content: "Keyboard-accessible tooltip overlay".to_owned(),
                        open: Some(tooltip_open()),
                        on_open_change: move |next: bool| tooltip_open.set(next),
                        Button {
                            test_id: Some("ks-tooltip-trigger".to_owned()),
                            variant: ButtonVariant::Outline,
                            size: ButtonSize::Sm,
                            on_press: move |_| tooltip_open.set(!tooltip_open()),
                            text { "{copy(\"Toggle Tooltip\", pseudo)}" }
                        }
                    }
                }
            }

            // Active Overlays
            Dialog {
                test_id: Some("ks-dialog".to_owned()),
                open: dialog_open(),
                title: Some(copy("Confirm Deployment", pseudo)),
                description: Some(copy("Dialog isolates keyboard focus and restores focus on close.", pseudo)),
                on_close: move |_| dialog_open.set(false),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 8px; padding-top: 8px;",
                    Button {
                        test_id: Some("ks-dialog-close".to_owned()),
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        on_press: move |_| dialog_open.set(false),
                        text { "{copy(\"Close Dialog\", pseudo)}" }
                    }
                }
            }

            Modal {
                test_id: Some("ks-modal".to_owned()),
                open: modal_open(),
                title: Some(copy("Inspect NVE Surface", pseudo)),
                description: Some(copy("Modal alias shares the same focus-trapped Dialog surface.", pseudo)),
                on_close: move |_| modal_open.set(false),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 8px; padding-top: 8px;",
                    Button {
                        test_id: Some("ks-modal-close".to_owned()),
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        on_press: move |_| modal_open.set(false),
                        text { "{copy(\"Dismiss Modal\", pseudo)}" }
                    }
                }
            }

            for detent_idx in [sheet_detent_idx()] {
                BottomSheet {
                    key: "sheet-{detent_idx}",
                    test_id: Some("ks-bottom-sheet".to_owned()),
                    open: sheet_open(),
                    detents: vec![SheetDetent::Peek, SheetDetent::Half, SheetDetent::Full],
                    initial_detent: detent_idx,
                    viewport_height: 760.0,
                    title: Some(copy("Multi-Detent BottomSheet", pseudo)),
                    on_dismiss: move |_| sheet_open.set(false),
                    on_detent_change: move |d: SheetDetent| {
                        let lbl = match d {
                            SheetDetent::Peek => "Peek (25%)",
                            SheetDetent::Half => "Half (50%)",
                            SheetDetent::Full => "Full (92%)",
                            SheetDetent::Custom(_) => "Custom",
                        };
                        sheet_detent_label.set(lbl.to_owned());
                    },
                    view {
                        style: "flex-direction: column; gap: 10px;",
                        text {
                            style: "font-size: 13px; color: {text_primary};",
                            "Active detent: {sheet_detent_label}"
                        }
                        Button {
                            test_id: Some("ks-sheet-close".to_owned()),
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_press: move |_| sheet_open.set(false),
                            text { "{copy(\"Close Sheet\", pseudo)}" }
                        }
                    }
                }
            }
        }
    }
}
