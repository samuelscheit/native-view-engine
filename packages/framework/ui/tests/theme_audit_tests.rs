//! Rendered contract: a showcase of every stateful `nve-ui` component, mounted in each polarity,
//! keeps every text run readable (WCAG 1.4.3) and every checked/selected indicator visible
//! (WCAG 1.4.11). Complements the pure role-pairing matrix in `theme_contrast_tests.rs`.

mod common;

use std::cell::Cell;

use common::UiHarness;
use nve_dioxus::prelude::*;
use nve_i18n::{CalendarDate, LocaleId};
use nve_ui::audit::{indicator_contrast_violations, text_contrast_violations};
use nve_ui::prelude::*;
use nve_ui::{AppBrand, Polarity};

thread_local! {
    static POLARITY: Cell<Polarity> = const { Cell::new(Polarity::Light) };
}

#[component]
fn Showcase() -> Element {
    let theme = Theme::preset(POLARITY.with(Cell::get), AppBrand::Default);
    let mut toast = use_toast();
    use_hook(|| {
        for v in [ToastVariant::Info, ToastVariant::Success, ToastVariant::Warning, ToastVariant::Danger] {
            toast.push(ToastOptions::new(format!("{v:?} toast")).with_description("Details").with_variant(v).with_duration_ms(None));
        }
    });
    let date = CalendarDate::new(2024, 2, 14).ok();
    let Ok(locale) = LocaleId::parse("en-US") else { return rsx! {} };
    let faq = vec![AccordionItem::new("q1", "Question", "Answer"), AccordionItem::new("q2", "Other", "More")];
    let options = vec![ComboboxOption::new("rust", "Rust"), ComboboxOption::new("zig", "Zig")];
    let menu = vec![MenuItem::new("edit", "Edit"), MenuItem::new("delete", "Delete").destructive(true)];

    rsx! {
        ThemeProvider {
            theme,
            Col {
                padding: 12.0,
                gap: 8.0,
                nve_style: TypedStyle::new().bg(theme.surface.canvas).color(theme.text.primary),
                Row {
                    for v in [ButtonVariant::Primary, ButtonVariant::Secondary, ButtonVariant::Ghost, ButtonVariant::Destructive, ButtonVariant::Outline] {
                        Button { variant: v, "{v:?}" }
                    }
                    IconButton { accessibility_label: "Close".to_string(), icon: "x".to_string() }
                }
                Row {
                    for v in [BadgeVariant::Neutral, BadgeVariant::Accent, BadgeVariant::Success, BadgeVariant::Warning, BadgeVariant::Danger] {
                        Badge { label: format!("{v:?}"), variant: v }
                    }
                }
                Row {
                    Checkbox { label: "Checked", checked: true }
                    Checkbox { label: "Mixed", indeterminate: true }
                    Checkbox { label: "Off" }
                    Switch { label: "On", checked: true }
                    Switch { label: "Off" }
                }
                TabsRoot {
                    default_value: "a",
                    TabList {
                        TabTrigger { value: "a", "Selected" }
                        TabTrigger { value: "b", "Other" }
                    }
                    TabPanel { value: "a", text { "Panel" } }
                }
                Input { value: "ada".to_string(), label: "Email", helper_text: "Work address", error: Some("Invalid email".into()) }
                Input { value: String::new(), label: "Notes", multiline: true }
                Combobox { label: "Languages", options, multi: true }
                Card {
                    variant: CardVariant::Elevated,
                    text { "Card body" }
                    Separator { orientation: nve_ui::Orientation::Horizontal }
                }
                Accordion { items: faq, default_open_id: Some("q1".into()) }
                Disclosure { summary: "More", default_open: true, text { "Disclosed" } }
                DatePicker { label: "Date", locale, value: date, today: date, open: Some(true) }
                DropdownMenu { open: true, trigger_label: "Menu".to_string(), items: menu }
                Tooltip { content: "Hint".to_string(), open: Some(true), Button { "Hover" } }
            }
        }
    }
}

#[component]
fn Root() -> Element {
    rsx! { ToastProvider { Showcase {} } }
}

#[test]
fn every_component_state_is_readable_and_visible_in_every_polarity() {
    let mut violations = Vec::new();
    let mut runs = 0;
    let mut indicators = 0;
    for p in Polarity::ALL {
        POLARITY.with(|c| c.set(p));
        let h = UiHarness::new(Root);
        let store = h.surface.store();
        let theme = Theme::preset(p, AppBrand::Default);
        let text_min = if theme.is_high_contrast() { 7.0 } else { 4.5 };
        runs += store.live_nodes().filter(|&id| store.try_row::<nve_tables::tables::TextContent>(id).is_some()).count();
        indicators += store
            .live_nodes()
            .filter(|&id| {
                store.try_row::<nve_tables::tables::Accessibility>(id).is_some_and(|a| {
                    use nve_tables::tables::AriaTriState::{Mixed, True};
                    matches!(a.checked(), Some(True | Mixed)) || a.selected() == Some(True)
                })
            })
            .count();
        for v in text_contrast_violations(store, text_min) {
            violations.push(format!("{p:?} text {:?} {:.2}:1 ({:?} on {:?})", v.text, v.ratio, v.fg, v.bg));
        }
        for v in indicator_contrast_violations(store, theme.non_text_min()) {
            violations.push(format!("{p:?} {} indicator #{} {:.2}:1 ({:?} on {:?})", v.text, v.node.raw(), v.ratio, v.fg, v.bg));
        }
    }
    assert!(runs > 200, "showcase must mount its components, saw {runs} text runs");
    assert!(indicators >= 4 * 5, "checked/selected states must reach the store, saw {indicators}");
    assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
}
