//! Section 1: Overview & Primitives — interactive Counter widget, `Button` variants/sizes,
//! `:hover`/`:active`/`:focus-visible` + `CursorKind` showcase, `IconButton`, `Badge`, `Card`,
//! `Separator`, and `Row`/`Col`/`Stack`.

use nve_abi::{ColorRgba, FlexAlign};
use nve_dioxus::prelude::*;
use nve_motion::{write_transitions, Motion, TransitionSpec};
use nve_ui::{
    use_theme, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Col,
    IconButton, Orientation, Row, Separator, Stack,
};

use crate::i18n_copy::copy;
use crate::sections::{responsive_two_col, responsive_two_col_cell, SectionHeader};
use crate::shell::AppHandle;

const CURSOR_DEMOS: [(&str, CursorKind); 6] = [
    ("Pointer", CursorKind::Pointer),
    ("Grab", CursorKind::Grab),
    ("Text", CursorKind::Text),
    ("ColResize", CursorKind::ColResize),
    ("NotAllowed", CursorKind::NotAllowed),
    ("Crosshair", CursorKind::Crosshair),
];

#[component]
pub fn OverviewSection(handle: AppHandle) -> Element {
    let theme = use_theme();
    let snap = handle.read_state();
    let pseudo = snap.pseudo_locale;
    let mut last_action = use_signal(|| "Ready — click any control".to_owned());
    let mut loading_btn = use_signal(|| false);

    let text_primary = theme.text.primary;
    let text_secondary = theme.text.secondary;
    let text_muted = theme.text.tertiary;
    let accent_text = theme.status.accent.text;
    let surface_subtle = theme.surface.subtle;
    let border = theme.border.default;

    let hover_card_style = TypedStyle::new()
        .padding_xy(14.0, 12.0)
        .radius(10.0)
        .bg(theme.surface.subtle)
        .border(1.0, theme.border.default)
        .cursor(CursorKind::Pointer)
        .on_hover_with(|patch| {
            patch
                .bg(theme.status.accent.solid)
                .border(1.0, theme.status.accent.text)
        })
        .on_active_with(|patch| patch.opacity(0.85))
        .on_focus_visible_with(|patch| patch.border(2.0, theme.border.focus));

    let hover_demo_label = copy(
        "Hover, press, or focus me (:hover / :active / :focus-visible)",
        pseudo,
    );

    rsx! {
        view {
            test_id: "section-overview",
            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(16.0),

            SectionHeader {
                title: copy("Overview & Primitives", pseudo),
                subtitle: copy("Core nve-ui building blocks: fine-grained signals, Button/IconButton matrices, pseudo-states + CursorKind, Badges, Cards, Separators, and Flex Stack/Row/Col.", pseudo),
                tag: "8 Primitives".to_owned(),
            }

            // Interactive Counter + Quick Telemetry Row
            view {
                nve_style: responsive_two_col(),

                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-counter-card".to_owned()),
                        variant: CardVariant::Elevated,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        Row {
                            justify: Some("space-between".to_owned()),
                            align: Some("center".to_owned()),
                            text {
                                nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(accent_text),
                                "{copy(\"INTERACTIVE COUNTER\", pseudo)}"
                            }
                            Badge {
                                label: "Persisted Snapshot".to_owned(),
                                variant: BadgeVariant::Success,
                            }
                        }
                        text {
                            test_id: "ks-counter-value",
                            nve_style: TypedStyle::new().font_size(28.0).font_weight(700).color(text_primary),
                            "Count: {snap.counter}"
                        }
                        view {
                            nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(8.0),
                            Button {
                                test_id: Some("ks-counter-inc".to_owned()),
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Md,
                                on_press: move |_| {
                                    handle.update(|st| st.counter += 1);
                                    last_action.set("Incremented counter".to_owned());
                                },
                                text { "{copy(\"Increment +\", pseudo)}" }
                            }
                            Button {
                                test_id: Some("ks-counter-dec".to_owned()),
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Md,
                                on_press: move |_| {
                                    handle.update(|st| st.counter -= 1);
                                    last_action.set("Decremented counter".to_owned());
                                },
                                text { "{copy(\"Decrement -\", pseudo)}" }
                            }
                            Button {
                                test_id: Some("ks-counter-reset".to_owned()),
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Md,
                                on_press: move |_| {
                                    handle.update(|st| st.counter = 0);
                                    last_action.set("Reset counter to 0".to_owned());
                                },
                                text { "{copy(\"Reset\", pseudo)}" }
                            }
                        }
                    }
                }

                // Interaction States (:hover / :active / :focus-visible) & CursorKind
                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-cursor-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        text {
                            nve_style: TypedStyle::new().font_size(14.0).font_weight(700).color(text_primary),
                            "{copy(\"Pseudo-States & CursorKind\", pseudo)}"
                        }
                        pressable {
                            test_id: "ks-hover-demo",
                            nve_style: hover_card_style,
                            nve_table: nve_dioxus::table_attr::<Motion, _>(
                                [TransitionSpec::all(150)],
                                |tx, node, specs| write_transitions(tx, node, specs),
                            ),
                            onpress: move |_| {
                                last_action.set("Activated interactive pseudo-state tile".to_owned());
                            },
                            text {
                                nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(text_primary),
                                "{hover_demo_label}"
                            }
                        }
                        view {
                            nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                            for (label, cursor_kind) in CURSOR_DEMOS {
                                CursorChip {
                                    key: "{label}",
                                    label: label.to_owned(),
                                    cursor_kind,
                                    bg: theme.surface.subtle,
                                    border: theme.border.default,
                                    text_color: text_secondary,
                                }
                            }
                        }
                    }
                }
            }

            // Button Variants, Sizes, States & IconButton Matrix
            Card {
                test_id: Some("ks-buttons-card".to_owned()),
                variant: CardVariant::Outlined,
                padding: Some(16.0),
                gap: Some(12.0),
                Row {
                    justify: Some("space-between".to_owned()),
                    align: Some("center".to_owned()),
                    text {
                        nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(text_primary),
                        "{copy(\"Button & IconButton Matrix\", pseudo)}"
                    }
                    text {
                        test_id: "ks-last-action",
                        nve_style: TypedStyle::new().font_size(12.0).color(text_muted),
                        "{last_action}"
                    }
                }

                // All 5 ButtonVariants
                view {
                    nve_style: TypedStyle::new()
                        .flex_row()
                        .flex_wrap(FlexWrap::Wrap)
                        .gap(8.0)
                        .align_items(FlexAlign::Center),
                    Button {
                        variant: ButtonVariant::Primary,
                        on_press: move |_| last_action.set("Pressed Primary".to_owned()),
                        text { "{copy(\"Primary\", pseudo)}" }
                    }
                    Button {
                        variant: ButtonVariant::Secondary,
                        on_press: move |_| last_action.set("Pressed Secondary".to_owned()),
                        text { "{copy(\"Secondary\", pseudo)}" }
                    }
                    Button {
                        variant: ButtonVariant::Outline,
                        on_press: move |_| last_action.set("Pressed Outline".to_owned()),
                        text { "{copy(\"Outline\", pseudo)}" }
                    }
                    Button {
                        variant: ButtonVariant::Ghost,
                        on_press: move |_| last_action.set("Pressed Ghost".to_owned()),
                        text { "{copy(\"Ghost\", pseudo)}" }
                    }
                    Button {
                        variant: ButtonVariant::Destructive,
                        on_press: move |_| last_action.set("Pressed Destructive".to_owned()),
                        text { "{copy(\"Destructive\", pseudo)}" }
                    }
                    Button {
                        variant: ButtonVariant::Primary,
                        disabled: true,
                        text { "{copy(\"Disabled\", pseudo)}" }
                    }
                    Button {
                        test_id: Some("ks-loading-btn".to_owned()),
                        variant: ButtonVariant::Secondary,
                        loading: loading_btn(),
                        on_press: move |_| {
                            let next = !loading_btn();
                            loading_btn.set(next);
                            last_action.set(if next {
                                "Loading state ON".to_owned()
                            } else {
                                "Loading state OFF".to_owned()
                            });
                        },
                        text { "{copy(\"Toggle Loading\", pseudo)}" }
                    }
                }

                Separator {}

                // Sizes + IconButtons
                view {
                    nve_style: TypedStyle::new()
                        .flex_row()
                        .flex_wrap(FlexWrap::Wrap)
                        .gap(10.0)
                        .align_items(FlexAlign::Center),
                    Button {
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        on_press: move |_| last_action.set("Size Sm".to_owned()),
                        text { "Small (Sm)" }
                    }
                    Button {
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Md,
                        on_press: move |_| last_action.set("Size Md".to_owned()),
                        text { "Medium (Md)" }
                    }
                    Button {
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Lg,
                        on_press: move |_| last_action.set("Size Lg".to_owned()),
                        text { "Large (Lg)" }
                    }
                    Separator { orientation: Orientation::Vertical, margin: 4.0 }
                    IconButton {
                        accessibility_label: "Add item".to_owned(),
                        icon: "+".to_owned(),
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        on_press: move |_| last_action.set("IconButton +".to_owned()),
                    }
                    IconButton {
                        accessibility_label: "Refresh view".to_owned(),
                        icon: "↻".to_owned(),
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Md,
                        on_press: move |_| last_action.set("IconButton ↻".to_owned()),
                    }
                    IconButton {
                        accessibility_label: "Delete item".to_owned(),
                        icon: "×".to_owned(),
                        variant: ButtonVariant::Destructive,
                        size: ButtonSize::Md,
                        on_press: move |_| last_action.set("IconButton ×".to_owned()),
                    }
                    IconButton {
                        accessibility_label: "Settings".to_owned(),
                        icon: "⚙".to_owned(),
                        variant: ButtonVariant::Outline,
                        size: ButtonSize::Lg,
                        on_press: move |_| last_action.set("IconButton ⚙".to_owned()),
                    }
                }
            }

            // Badges + Card Variants + Stack/Row/Col
            Card {
                test_id: Some("ks-badges-cards".to_owned()),
                variant: CardVariant::Subtle,
                padding: Some(16.0),
                gap: Some(12.0),
                text {
                    nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(text_primary),
                    "{copy(\"Badges, Cards & Stack / Row / Col\", pseudo)}"
                }
                view {
                    nve_style: TypedStyle::new().flex_row().flex_wrap(FlexWrap::Wrap).gap(8.0),
                    Badge { label: "Neutral".to_owned(), variant: BadgeVariant::Neutral }
                    Badge { label: "Accent".to_owned(), variant: BadgeVariant::Accent }
                    Badge { label: "Success".to_owned(), variant: BadgeVariant::Success }
                    Badge { label: "Warning".to_owned(), variant: BadgeVariant::Warning }
                    Badge { label: "Danger".to_owned(), variant: BadgeVariant::Danger }
                }
                Separator {}
                view {
                    nve_style: responsive_two_col(),
                    view {
                        nve_style: responsive_two_col_cell(),
                        Card {
                            variant: CardVariant::Elevated,
                            padding: Some(12.0),
                            gap: Some(6.0),
                            text { nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(text_primary), "CardVariant::Elevated" }
                            text { nve_style: TypedStyle::new().font_size(12.0).color(text_muted), "Elevated surface with theme-aware shadow." }
                        }
                    }
                    view {
                        nve_style: responsive_two_col_cell(),
                        Card {
                            variant: CardVariant::Outlined,
                            padding: Some(12.0),
                            gap: Some(6.0),
                            text { nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(text_primary), "CardVariant::Outlined" }
                            text { nve_style: TypedStyle::new().font_size(12.0).color(text_muted), "Crisp 1px semantic border container." }
                        }
                    }
                    view {
                        nve_style: responsive_two_col_cell(),
                        Card {
                            variant: CardVariant::Subtle,
                            padding: Some(12.0),
                            gap: Some(6.0),
                            text { nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(text_primary), "CardVariant::Subtle" }
                            text { nve_style: TypedStyle::new().font_size(12.0).color(text_muted), "Low-contrast recessed surface." }
                        }
                    }
                }
                Stack {
                    direction: Orientation::Vertical,
                    gap: Some(8.0),
                    Col {
                        gap: Some(6.0),
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(text_secondary),
                            "Row / Col / Stack Flex Primitives"
                        }
                        view {
                            nve_style: TypedStyle::new()
                                .flex_row()
                                .flex_wrap(FlexWrap::Wrap)
                                .gap(8.0)
                                .width_pct(100.0),
                            for idx in 1..=3 {
                                view {
                                    key: "{idx}",
                                    nve_style: TypedStyle::new()
                                        .padding_xy(12.0, 8.0)
                                        .radius(6.0)
                                        .bg(surface_subtle)
                                        .border(1.0, border),
                                    text {
                                        nve_style: TypedStyle::new().font_size(12.0).color(text_primary),
                                        "Stack Item #{idx}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CursorChip(
    label: String,
    cursor_kind: CursorKind,
    bg: ColorRgba,
    border: ColorRgba,
    text_color: ColorRgba,
) -> Element {
    let chip_style = TypedStyle::new()
        .padding_xy(10.0, 6.0)
        .radius(6.0)
        .bg(bg)
        .border(1.0, border)
        .cursor(cursor_kind);

    rsx! {
        view {
            nve_style: chip_style,
            text {
                nve_style: TypedStyle::new().font_size(12.0).color(text_color),
                "{label}"
            }
        }
    }
}
