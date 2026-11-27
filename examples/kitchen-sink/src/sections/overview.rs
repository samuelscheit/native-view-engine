//! Section 1: Overview & Primitives — interactive Counter widget, `Button` variants/sizes,
//! `:hover`/`:active`/`:focus-visible` + `CursorKind` showcase, `IconButton`, `Badge`, `Card`,
//! `Separator`, and `Row`/`Col`/`Stack`.

use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_ui::{ColorHex, 
    use_theme, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Col,
    IconButton, Orientation, Row, Separator, Stack,
};

use crate::i18n_copy::copy;
use crate::sections::{responsive_two_col, SectionHeader};
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

    let text_primary = theme.text.primary.hex();
    let text_secondary = theme.text.secondary.hex();
    let text_muted = theme.text.tertiary.hex();
    let accent_text = theme.status.accent.text.hex();
    let surface_subtle = theme.surface.subtle.hex();
    let border = theme.border.default.hex();

    let hover_card_style = TypedStyle::new()
        .padding_xy(14.0, 12.0)
        .radius(10.0)
        .bg(theme.surface.subtle)
        .border(1.0, theme.border.default)
        .cursor(CursorKind::Pointer)
        .transition(TransitionSpec::all(150))
        .on_hover_with(|patch| patch.bg(theme.status.accent.solid).border(1.0, theme.status.accent.solid))
        .on_active_with(|patch| patch.opacity(0.85))
        .on_focus_visible_with(|patch| patch.border(2.0, theme.border.focus));

    let hover_demo_label = copy(
        "Hover, press, or focus me (:hover / :active / :focus-visible)",
        pseudo,
    );

    rsx! {
        view {
            test_id: "section-overview",
            style: "width: 100%; flex-direction: column; gap: 16px;",

            SectionHeader {
                title: copy("Overview & Primitives", pseudo),
                subtitle: copy("Core nve-ui building blocks: fine-grained signals, Button/IconButton matrices, pseudo-states + CursorKind, Badges, Cards, Separators, and Flex Stack/Row/Col.", pseudo),
                tag: "8 Primitives".to_owned(),
            }

            // Interactive Counter + Quick Telemetry Row
            view {
                nve_style: responsive_two_col(),

                view {
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0;",
                    Card {
                        test_id: Some("ks-counter-card".to_owned()),
                        variant: CardVariant::Elevated,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        Row {
                            justify: Some("space-between".to_owned()),
                            align: Some("center".to_owned()),
                            text {
                                style: "font-size: 12px; font-weight: 700; color: {accent_text};",
                                "{copy(\"INTERACTIVE COUNTER\", pseudo)}"
                            }
                            Badge {
                                label: "Persisted Snapshot".to_owned(),
                                variant: BadgeVariant::Success,
                            }
                        }
                        text {
                            test_id: "ks-counter-value",
                            style: "font-size: 28px; font-weight: 700; color: {text_primary};",
                            "Count: {snap.counter}"
                        }
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 8px;",
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
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0;",
                    Card {
                        test_id: Some("ks-cursor-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(12.0),
                        text {
                            style: "font-size: 14px; font-weight: 700; color: {text_primary};",
                            "{copy(\"Pseudo-States & CursorKind\", pseudo)}"
                        }
                        pressable {
                            test_id: "ks-hover-demo",
                            nve_style: hover_card_style,
                            onpress: move |_| {
                                last_action.set("Activated interactive pseudo-state tile".to_owned());
                            },
                            text {
                                style: "font-size: 13px; font-weight: 600; color: {text_primary};",
                                "{hover_demo_label}"
                            }
                        }
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            for (label, cursor_kind) in CURSOR_DEMOS {
                                CursorChip {
                                    key: "{label}",
                                    label: label.to_owned(),
                                    cursor_kind,
                                    bg: theme.surface.subtle,
                                    border: theme.border.default,
                                    text_color: text_secondary.to_owned(),
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
                        style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                        "{copy(\"Button & IconButton Matrix\", pseudo)}"
                    }
                    text {
                        test_id: "ks-last-action",
                        style: "font-size: 12px; color: {text_muted};",
                        "{last_action}"
                    }
                }

                // All 5 ButtonVariants
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 8px; align-items: center;",
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
                    style: "flex-direction: row; flex-wrap: wrap; gap: 10px; align-items: center;",
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
                    style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                    "{copy(\"Badges, Cards & Stack / Row / Col\", pseudo)}"
                }
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 8px;",
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
                        style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0;",
                        Card {
                            variant: CardVariant::Elevated,
                            padding: Some(12.0),
                            gap: Some(6.0),
                            text { style: "font-size: 13px; font-weight: 700; color: {text_primary};", "CardVariant::Elevated" }
                            text { style: "font-size: 12px; color: {text_muted};", "Elevated surface with theme-aware shadow." }
                        }
                    }
                    view {
                        style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0;",
                        Card {
                            variant: CardVariant::Outlined,
                            padding: Some(12.0),
                            gap: Some(6.0),
                            text { style: "font-size: 13px; font-weight: 700; color: {text_primary};", "CardVariant::Outlined" }
                            text { style: "font-size: 12px; color: {text_muted};", "Crisp 1px semantic border container." }
                        }
                    }
                    view {
                        style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0;",
                        Card {
                            variant: CardVariant::Subtle,
                            padding: Some(12.0),
                            gap: Some(6.0),
                            text { style: "font-size: 13px; font-weight: 700; color: {text_primary};", "CardVariant::Subtle" }
                            text { style: "font-size: 12px; color: {text_muted};", "Low-contrast recessed surface." }
                        }
                    }
                }
                Stack {
                    direction: Orientation::Vertical,
                    gap: Some(8.0),
                    Col {
                        gap: Some(6.0),
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {text_secondary};",
                            "Row / Col / Stack Flex Primitives"
                        }
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 8px; width: 100%;",
                            for idx in 1..=3 {
                                view {
                                    key: "{idx}",
                                    style: "padding-horizontal: 12px; padding-vertical: 8px; border-radius: 6px; background-color: {surface_subtle}; border-width: 1px; border-color: {border};",
                                    text {
                                        style: "font-size: 12px; color: {text_primary};",
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
    text_color: String,
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
                style: "font-size: 12px; color: {text_color};",
                "{label}"
            }
        }
    }
}
