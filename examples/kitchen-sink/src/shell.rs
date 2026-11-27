//! Top-level responsive `KitchenSinkApp` shell: toolbar, search filter, sidebar/mobile nav,
//! `ThemeProvider`, `FocusScopeProvider`, `ToastProvider`, and synced `KitchenSinkState` store.

use dioxus_core::EventHandler;
use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_ui::{
    use_theme, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, FocusScopeProvider, Input,
    Separator, ThemeProvider, ToastProvider,
};

use crate::i18n_copy::copy;
use crate::sections;
use crate::state::{
    take_or_default_store, DensityPreset, GallerySection, KitchenSinkState, LocalePreset,
    SharedKitchenSinkStore, ThemeMode,
};

/// Shared reactive state handle passed to section components so every mutation updates both
/// the live Dioxus signal and the `KitchenSinkModule` persistence store synchronously.
#[derive(Clone, Copy)]
pub struct AppHandle {
    pub state: Signal<KitchenSinkState>,
    store: Signal<SharedKitchenSinkStore>,
}

impl PartialEq for AppHandle {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state && self.store == other.store
    }
}

impl AppHandle {
    pub fn read_state(self) -> KitchenSinkState {
        self.state.read().clone()
    }

    pub fn update(mut self, f: impl FnOnce(&mut KitchenSinkState)) {
        let next = {
            let mut st = self.state.write();
            f(&mut st);
            st.clone()
        };
        if let Ok(mut guard) = self.store.read().lock() {
            *guard = next;
        }
    }
}

/// Root interactive Kitchen Sink gallery component.
#[component]
pub fn KitchenSinkApp() -> Element {
    let shared_store = dioxus_core::use_hook(take_or_default_store);
    let initial_state = shared_store
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let state = use_signal(|| initial_state);
    let store_sig = use_signal(|| shared_store);
    let handle = AppHandle {
        state,
        store: store_sig,
    };

    let snap = state.read().clone();
    let theme = snap.build_theme();

    rsx! {
        FocusScopeProvider {
            ThemeProvider {
                theme: theme,
                ToastProvider {
                    max_visible: 5,
                    render_viewport: true,
                    KitchenSinkShell { handle }
                }
            }
        }
    }
}

#[component]
fn KitchenSinkShell(handle: AppHandle) -> Element {
    let theme = use_theme();
    let p = &theme.palette;
    let snap = handle.read_state();
    let pseudo = snap.pseudo_locale;
    let mut search_query = use_signal(String::new);
    let query_owned = search_query.read().clone();
    let query = query_owned.trim();
    let contains_ci = |haystack: &str, needle: &str| -> bool {
        haystack
            .as_bytes()
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
    };

    let matching_sections: Vec<GallerySection> = if query.is_empty() {
        vec![snap.active_section]
    } else {
        GallerySection::ALL
            .into_iter()
            .filter(|sec| contains_ci(sec.label(), query) || contains_ci(sec.keywords(), query))
            .collect()
    };

    let body_layout_style = TypedStyle::new()
        .flex_row()
        .width_pct(100.0)
        .flex_grow(1.0)
        .min_height_px(0.0)
        .at(MediaQuery::max_width(899), RowPatch::new().flex_col());

    let main_pad_style = TypedStyle::new()
        .width_pct(100.0)
        .max_width_px(1080.0)
        .flex_col()
        .padding(20.0)
        .gap(18.0)
        .at(
            MediaQuery::max_width(639),
            RowPatch::new().padding(12.0).gap(14.0),
        );

    let search_wrap_style = TypedStyle::new()
        .width_px(210.0)
        .at(MediaQuery::max_width(639), RowPatch::new().width_pct(100.0));

    let toolbar_controls_style = TypedStyle::new()
        .flex_row()
        .align_items(nve_abi::FlexAlign::Center)
        .justify_content(nve_abi::FlexJustify::FlexEnd)
        .gap(8.0)
        .flex_grow(1.0)
        .min_width_px(0.0)
        .at(
            MediaQuery::max_width(899),
            RowPatch::new().justify_content(nve_abi::FlexJustify::FlexStart),
        );

    let title_text = copy("NVE Kitchen Sink", pseudo);
    let subtitle_text = copy("Component & Engine Gallery", pseudo);
    let search_placeholder = copy("Filter components...", pseudo);
    let theme_btn_label = copy(
        match snap.theme_mode {
            ThemeMode::Dark => "Theme: Dark",
            ThemeMode::Light => "Theme: Light",
        },
        pseudo,
    );
    let pseudo_btn_label = if pseudo { "Pseudo: ON" } else { "Pseudo" };
    let motion_btn_label = if snap.reduced_motion {
        copy("Reduced Motion: ON", pseudo)
    } else {
        copy("Reduced Motion: OFF", pseudo)
    };

    let bg_surface = p.surface.as_ref();
    let bg_elevated = p.surface_elevated.as_ref();
    let border_col = p.border.as_ref();
    let text_primary = p.text_primary.as_ref();
    let text_muted = p.text_muted.as_ref();
    let accent_col = p.accent.as_ref();

    rsx! {
        view {
            test_id: "kitchen-sink-root",
            style: "width: 100%; height: 100%; flex-direction: column; background-color: {bg_surface}; overflow: hidden;",

            // Top live controls toolbar
            view {
                test_id: "ks-toolbar",
                style: "width: 100%; flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px; padding-horizontal: 16px; padding-vertical: 10px; background-color: {bg_elevated}; flex-shrink: 0;",

                // Brand & title
                view {
                    style: "flex-direction: row; align-items: center; gap: 10px;",
                    view {
                        style: "padding-horizontal: 8px; padding-vertical: 4px; border-radius: 6px; background-color: {accent_col};",
                        text {
                            style: "font-size: 11px; font-weight: 700; color: #ffffff;",
                            "NVE"
                        }
                    }
                    view {
                        style: "flex-direction: column; gap: 1px;",
                        text {
                            test_id: "ks-brand-title",
                            style: "font-size: 16px; font-weight: 700; color: {text_primary};",
                            "{title_text}"
                        }
                        text {
                            nve_show_on: ShowOn(MediaQuery::min_width(640)),
                            style: "font-size: 11px; color: {text_muted};",
                            "{subtitle_text}"
                        }
                    }
                }

                // Live search + Theme / Density / Locale / Pseudo / Reduced Motion controls
                view {
                    nve_style: toolbar_controls_style,
                    style: "flex-wrap: wrap;",

                    view {
                        nve_style: search_wrap_style,
                        Input {
                            test_id: Some("ks-search-input".to_owned()),
                            value: search_query(),
                            placeholder: Some(search_placeholder),
                            on_change: move |v: String| search_query.set(v),
                        }
                    }

                    // Theme toggle
                    Button {
                        test_id: Some("ks-theme-toggle".to_owned()),
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            handle.update(|st| st.theme_mode = st.theme_mode.toggle());
                        },
                        text { "{theme_btn_label}" }
                    }

                    // Density selector (Compact / Comfortable / Spacious)
                    view {
                        test_id: "ks-density-group",
                        style: "flex-direction: row; align-items: center; gap: 4px; flex-wrap: wrap;",
                        for preset in DensityPreset::ALL {
                            Button {
                                key: "{preset.id()}",
                                test_id: Some(format!("ks-density-{}", preset.id())),
                                variant: if snap.density == preset {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Outline
                                },
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    handle.update(|st| st.density = preset);
                                },
                                text { "{copy(preset.label(), pseudo)}" }
                            }
                        }
                    }

                    // Locale switcher (en-US / de-DE / ar-EG / ja-JP) + PseudoLocalizer
                    view {
                        test_id: "ks-locale-group",
                        style: "flex-direction: row; align-items: center; gap: 4px; flex-wrap: wrap;",
                        for loc in LocalePreset::ALL {
                            Button {
                                key: "{loc.tag()}",
                                test_id: Some(format!("ks-locale-{}", loc.tag())),
                                variant: if snap.locale == loc {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Ghost
                                },
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    handle.update(|st| st.locale = loc);
                                },
                                text { "{loc.tag()}" }
                            }
                        }
                        Button {
                            test_id: Some("ks-pseudo-toggle".to_owned()),
                            variant: if pseudo {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Outline
                            },
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                handle.update(|st| st.pseudo_locale = !st.pseudo_locale);
                            },
                            text { "{pseudo_btn_label}" }
                        }
                    }

                    // Reduced Motion toggle
                    Button {
                        test_id: Some("ks-reduced-motion-toggle".to_owned()),
                        variant: if snap.reduced_motion {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Outline
                        },
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            handle.update(|st| st.reduced_motion = !st.reduced_motion);
                        },
                        text { "{motion_btn_label}" }
                    }
                }
            }

            Separator { margin: 0.0 }

            // Mobile compact section pill strip (< 900px), outside the desktop flex row so it
            // never consumes horizontal space on backends that keep responsive nodes mounted.
            view {
                test_id: "ks-mobile-nav",
                nve_show_on: ShowOn(MediaQuery::max_width(899)),
                style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 6px; padding-horizontal: 12px; padding-vertical: 10px; background-color: {bg_elevated}; flex-shrink: 0;",
                for sec in GallerySection::ALL {
                    Button {
                        key: "{sec.id()}",
                        test_id: Some(format!("ks-mobile-tab-{}", sec.id())),
                        variant: if query.is_empty() && snap.active_section == sec {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        },
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            search_query.set(String::new());
                            handle.update(|st| st.active_section = sec);
                        },
                        text { "{copy(sec.short_label(), pseudo)}" }
                    }
                }
            }

            // Responsive Body: Desktop Sidebar + Scrollable Gallery Content
            view {
                nve_style: body_layout_style,

                // Desktop left navigation sidebar (>= 900px)
                view {
                    test_id: "ks-desktop-sidebar",
                    nve_show_on: ShowOn(MediaQuery::min_width(900)),
                    style: "width: 248px; flex-shrink: 0; flex-direction: column; justify-content: space-between; padding: 14px; gap: 12px; background-color: {bg_elevated}; border-width: 1px; border-color: {border_col}; overflow: auto;",
                    view {
                        style: "flex-direction: column; gap: 6px; width: 100%;",
                        text {
                            style: "font-size: 11px; font-weight: 700; color: {text_muted}; padding-horizontal: 8px; padding-vertical: 4px;",
                            "{copy(\"SECTIONS\", pseudo)}"
                        }
                        for sec in GallerySection::ALL {
                            SidebarNavItem {
                                key: "{sec.id()}",
                                section: sec,
                                active: query.is_empty() && snap.active_section == sec,
                                pseudo,
                                on_select: move |_| {
                                    search_query.set(String::new());
                                    handle.update(|st| st.active_section = sec);
                                },
                            }
                        }
                    }

                    // Live State Snapshot Summary
                    view {
                        test_id: "ks-sidebar-summary",
                        style: "flex-direction: column; gap: 6px; padding: 12px; border-radius: 10px; background-color: {bg_surface}; border-width: 1px; border-color: {border_col};",
                        text {
                            style: "font-size: 11px; font-weight: 700; color: {text_muted};",
                            "{copy(\"LIVE SESSION STATE\", pseudo)}"
                        }
                        text {
                            style: "font-size: 12px; color: {text_primary};",
                            "Theme: {snap.theme_mode.label()} · {snap.density.label()}"
                        }
                        text {
                            style: "font-size: 12px; color: {text_muted};",
                            "Locale: {snap.locale.tag()} · Count: {snap.counter}"
                        }
                        Badge {
                            label: "Binary Snapshot Ready".to_owned(),
                            variant: BadgeVariant::Success,
                        }
                    }
                }

                // Main scrollable showcase surface
                scrollview {
                    test_id: "ks-main-scroll",
                    style: "flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0; min-height: 0;",
                    view {
                        nve_style: main_pad_style,

                        if !query.is_empty() {
                            view {
                                test_id: "ks-search-banner",
                                style: "width: 100%; flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; padding: 12px; border-radius: 10px; background-color: {bg_elevated}; border-width: 1px; border-color: {border_col};",
                                text {
                                    style: "font-size: 13px; font-weight: 600; color: {text_primary};",
                                    "Filter: \"{search_query}\" ({matching_sections.len()} matching section(s))"
                                }
                                Button {
                                    test_id: Some("ks-search-clear".to_owned()),
                                    variant: ButtonVariant::Ghost,
                                    size: ButtonSize::Sm,
                                    on_press: move |_| search_query.set(String::new()),
                                    text { "{copy(\"Clear Filter\", pseudo)}" }
                                }
                            }
                        }

                        if matching_sections.is_empty() {
                            view {
                                test_id: "ks-search-empty",
                                style: "width: 100%; padding: 28px; border-radius: 12px; background-color: {bg_elevated}; border-width: 1px; border-color: {border_col}; align-items: center; gap: 8px;",
                                text {
                                    style: "font-size: 16px; font-weight: 700; color: {text_primary};",
                                    "{copy(\"No matching components\", pseudo)}"
                                }
                                text {
                                    style: "font-size: 13px; color: {text_muted};",
                                    "{copy(\"Try searching for button, combobox, datepicker, dialog, virtuallist, canvas, or errorboundary.\", pseudo)}"
                                }
                            }
                        }

                        for sec in matching_sections {
                            match sec {
                                GallerySection::Overview => rsx! { sections::overview::OverviewSection { handle } },
                                GallerySection::Forms => rsx! { sections::forms::FormsSection { handle } },
                                GallerySection::Selection => rsx! { sections::selection::SelectionSection { handle } },
                                GallerySection::Overlays => rsx! { sections::overlays::OverlaysSection { handle } },
                                GallerySection::Layout => rsx! { sections::layout::LayoutSection { handle } },
                                GallerySection::Motion => rsx! { sections::motion::MotionSection { handle } },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SidebarNavItem(
    section: GallerySection,
    active: bool,
    pseudo: bool,
    on_select: EventHandler<()>,
) -> Element {
    let theme = use_theme();
    let colors = theme.palette_rgba();
    let bg = if active {
        colors.accent
    } else {
        ColorRgba::TRANSPARENT
    };
    let hover_bg = if active {
        colors.accent
    } else {
        colors.surface_subtle
    };
    let fg = if active {
        theme.palette.accent_fg.as_ref()
    } else {
        theme.palette.text_primary.as_ref()
    };

    let nav_style = TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .align_items(nve_abi::FlexAlign::Center)
        .justify_content(nve_abi::FlexJustify::SpaceBetween)
        .padding_xy(10.0, 9.0)
        .radius(8.0)
        .bg(bg)
        .cursor(CursorKind::Pointer)
        .on_hover_with(|patch| patch.bg(hover_bg))
        .on_focus_visible_with(|patch| patch.border(2.0, colors.border_focus));

    rsx! {
        pressable {
            test_id: format!("ks-nav-{}", section.id()),
            accessibility_role: "tab",
            aria_selected: if active { "true" } else { "false" },
            nve_style: nav_style,
            onpress: move |_| on_select.call(()),
            text {
                style: "font-size: 13px; font-weight: 600; color: {fg}; text-align: left; flex-shrink: 1; min-width: 0;",
                "{copy(section.label(), pseudo)}"
            }
            Badge {
                label: section.badge_count().to_owned(),
                variant: if active { BadgeVariant::Neutral } else { BadgeVariant::Accent },
            }
        }
    }
}
