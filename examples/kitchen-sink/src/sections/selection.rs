//! Section 3: Selection & Dates — searchable single/multi `Combobox`, locale-aware `DatePicker`
//! and `DateRangePicker` bound to the topbar `nve-i18n` locale switcher, plus live ICU readout.

use nve_dioxus::prelude::*;
use nve_i18n::CalendarDate;
use nve_ui::{ColorHex, 
    use_theme, Badge, BadgeVariant, Card, CardVariant, Combobox, ComboboxOption, DatePicker,
    DateRange, DateRangePicker, Separator,
};

use crate::i18n_copy::{compute_locale_readout, copy};
use crate::sections::{responsive_two_col, responsive_two_col_cell, SectionHeader};
use crate::shell::AppHandle;

fn runtime_options() -> Vec<ComboboxOption> {
    vec![
        ComboboxOption::new("rust", "Rust (Native + WASM)"),
        ComboboxOption::new("swift", "SwiftUI / AppKit Host"),
        ComboboxOption::new("kotlin", "Jetpack Compose Host"),
        ComboboxOption::new("win32", "Windows Direct3D12 / RNW"),
        ComboboxOption::new("flutter", "Flutter Canvas Plugin"),
    ]
}

fn engine_module_options() -> Vec<ComboboxOption> {
    vec![
        ComboboxOption::new("taffy", "Taffy Flexbox Layout"),
        ComboboxOption::new("skia", "Skia GPU Backend"),
        ComboboxOption::new("web", "Web DOM + SSR Backend"),
        ComboboxOption::new("i18n", "nve-i18n Platform ICU"),
        ComboboxOption::new("router", "nve-router Deep Links"),
        ComboboxOption::new("wire", "nve-wire QUIC / WebTransport"),
    ]
}

#[component]
pub fn SelectionSection(handle: AppHandle) -> Element {
    let theme = use_theme();
    let snap = handle.read_state();
    let pseudo = snap.pseudo_locale;

    let locale_id = snap.locale.locale_id();
    let locale_tag = snap.locale.tag();
    let active_date = snap
        .selected_date
        .unwrap_or_else(|| CalendarDate::new(2026, 11, 25).expect("valid static date"));
    let readout = compute_locale_readout(snap.locale, active_date, snap.counter);

    let text_primary = theme.text.primary.hex();
    let text_muted = theme.text.tertiary.hex();
    let surface_subtle = theme.surface.subtle.hex();
    let border = theme.border.default.hex();

    let single_selected = if snap.combobox_single.is_empty() {
        Vec::new()
    } else {
        vec![snap.combobox_single.clone()]
    };

    rsx! {
        view {
            test_id: "section-selection",
            style: "width: 100%; flex-direction: column; gap: 16px;",

            SectionHeader {
                title: copy("Selection & Dates", pseudo),
                subtitle: copy("Searchable single & multi-select Combobox plus locale-aware DatePicker, DateRangePicker, and live nve-i18n ICU formatting.", pseudo),
                tag: format!("Locale: {locale_tag}"),
            }

            // Combobox Single + Multi
            view {
                nve_style: responsive_two_col(),

                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-combobox-single-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(10.0),
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                            "{copy(\"Single-Select Combobox\", pseudo)}"
                        }
                        Combobox {
                            test_id: Some("ks-combobox-single".to_owned()),
                            options: runtime_options(),
                            selected: single_selected,
                            multi: false,
                            label: Some(copy("Target Host Runtime", pseudo)),
                            placeholder: Some(copy("Search runtimes...", pseudo)),
                            on_change: move |vals: Vec<String>| {
                                let chosen = vals.into_iter().next().unwrap_or_default();
                                handle.update(|st| st.combobox_single = chosen);
                            },
                        }
                        text {
                            test_id: "ks-combobox-single-value",
                            style: "font-size: 12px; color: {text_muted};",
                            "Selected runtime: {snap.combobox_single}"
                        }
                    }
                }

                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-combobox-multi-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(10.0),
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                            "{copy(\"Multi-Select Combobox (Tags)\", pseudo)}"
                        }
                        Combobox {
                            test_id: Some("ks-combobox-multi".to_owned()),
                            options: engine_module_options(),
                            selected: snap.combobox_multi.clone(),
                            multi: true,
                            label: Some(copy("Enabled NVE Modules", pseudo)),
                            placeholder: Some(copy("Filter engine modules...", pseudo)),
                            on_change: move |vals: Vec<String>| {
                                handle.update(|st| st.combobox_multi = vals);
                            },
                        }
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            for tag in snap.combobox_multi.iter() {
                                Badge {
                                    key: "{tag}",
                                    label: tag.clone(),
                                    variant: BadgeVariant::Accent,
                                }
                            }
                        }
                    }
                }
            }

            // Locale-Aware DatePicker + DateRangePicker
            view {
                nve_style: responsive_two_col(),

                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-datepicker-card".to_owned()),
                        variant: CardVariant::Elevated,
                        padding: Some(16.0),
                        gap: Some(10.0),
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                            "{copy(\"Locale-Aware DatePicker\", pseudo)}"
                        }
                        text {
                            style: "font-size: 12px; color: {text_muted};",
                            "{snap.locale.week_note()}"
                        }
                        for loc_key in [locale_tag] {
                            DatePicker {
                                key: "dp-{loc_key}",
                                test_id: Some("ks-date-picker".to_owned()),
                                value: snap.selected_date,
                                locale: locale_id.clone(),
                                today: CalendarDate::new(2026, 11, 25).ok(),
                                label: Some(copy("Release Date", pseudo)),
                                on_change: move |next: Option<CalendarDate>| {
                                    handle.update(|st| st.selected_date = next);
                                },
                            }
                        }
                    }
                }

                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-daterange-card".to_owned()),
                        variant: CardVariant::Elevated,
                        padding: Some(16.0),
                        gap: Some(10.0),
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                            "{copy(\"Locale-Aware DateRangePicker\", pseudo)}"
                        }
                        text {
                            style: "font-size: 12px; color: {text_muted};",
                            "Automatically flips week start & direction per locale"
                        }
                        for loc_key in [locale_tag] {
                            DateRangePicker {
                                key: "drp-{loc_key}",
                                test_id: Some("ks-date-range-picker".to_owned()),
                                value: snap.selected_range,
                                locale: locale_id.clone(),
                                today: CalendarDate::new(2026, 11, 25).ok(),
                                label: Some(copy("Sprint Window", pseudo)),
                                on_change: move |next: Option<DateRange>| {
                                    handle.update(|st| st.selected_range = next);
                                },
                            }
                        }
                    }
                }
            }

            // Live nve-i18n ICU Readout
            Card {
                test_id: Some("ks-i18n-readout".to_owned()),
                variant: CardVariant::Subtle,
                padding: Some(16.0),
                gap: Some(10.0),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;",
                    text {
                        style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                        "{copy(\"Live nve-i18n ICU Readout\", pseudo)}"
                    }
                    Badge {
                        label: format!("{} · {}", readout.canonical_tag, readout.direction_label),
                        variant: BadgeVariant::Accent,
                    }
                }
                Separator {}
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 10px; width: 100%;",
                    ReadoutTile {
                        label: "First Day of Week".to_owned(),
                        value: readout.first_weekday.to_owned(),
                        surface_subtle: surface_subtle.to_owned(),
                        border: border.to_owned(),
                        text_primary: text_primary.to_owned(),
                        text_muted: text_muted.to_owned(),
                    }
                    ReadoutTile {
                        label: "Date (Short / Medium / Long)".to_owned(),
                        value: format!("{} · {} · {}", readout.date_short, readout.date_medium, readout.date_long),
                        surface_subtle: surface_subtle.to_owned(),
                        border: border.to_owned(),
                        text_primary: text_primary.to_owned(),
                        text_muted: text_muted.to_owned(),
                    }
                    ReadoutTile {
                        label: "Currency".to_owned(),
                        value: readout.currency_formatted,
                        surface_subtle: surface_subtle.to_owned(),
                        border: border.to_owned(),
                        text_primary: text_primary.to_owned(),
                        text_muted: text_muted.to_owned(),
                    }
                    ReadoutTile {
                        label: "Decimal Grouping".to_owned(),
                        value: readout.number_formatted,
                        surface_subtle: surface_subtle.to_owned(),
                        border: border.to_owned(),
                        text_primary: text_primary.to_owned(),
                        text_muted: text_muted.to_owned(),
                    }
                    ReadoutTile {
                        label: "Relative Time".to_owned(),
                        value: format!("{} / {}", readout.relative_past, readout.relative_future),
                        surface_subtle: surface_subtle.to_owned(),
                        border: border.to_owned(),
                        text_primary: text_primary.to_owned(),
                        text_muted: text_muted.to_owned(),
                    }
                    ReadoutTile {
                        label: "CLDR Plural (Counter)".to_owned(),
                        value: readout.plural_label,
                        surface_subtle: surface_subtle.to_owned(),
                        border: border.to_owned(),
                        text_primary: text_primary.to_owned(),
                        text_muted: text_muted.to_owned(),
                    }
                }
            }
        }
    }
}

#[component]
fn ReadoutTile(
    label: String,
    value: String,
    surface_subtle: String,
    border: String,
    text_primary: String,
    text_muted: String,
) -> Element {
    rsx! {
        view {
            style: "flex-grow: 1; flex-basis: 210px; min-width: 0; padding: 10px; border-radius: 8px; background-color: {surface_subtle}; border-width: 1px; border-color: {border}; gap: 4px;",
            text {
                style: "font-size: 11px; font-weight: 600; color: {text_muted};",
                "{label}"
            }
            text {
                style: "font-size: 13px; font-weight: 700; color: {text_primary};",
                "{value}"
            }
        }
    }
}
