//! Accessible searchable single-select & multi-select [`Combobox`] RSX component backed by
//! [`ComboboxState`] (`role="combobox"`, `aria-expanded`, `aria-activedescendant`,
//! `role="listbox"`, `role="option"`).

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

pub use crate::components::combobox_state::{
    default_combobox_filter, ComboboxAction, ComboboxOption, ComboboxState,
};
use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::use_theme_rgba;

/// Accessible searchable single-select & multi-select combobox component with `role="combobox"`,
/// `aria-expanded`, `aria-activedescendant`, `role="listbox"`, and `role="option"`.
#[component]
pub fn Combobox(
    options: Vec<ComboboxOption>,
    #[props(default)] selected: Vec<String>,
    #[props(default)] multi: bool,
    #[props(default)] loading: bool,
    #[props(default)] composing: bool,
    #[props(default)] disabled: bool,
    #[props(default)] label: Option<String>,
    #[props(default)] placeholder: Option<String>,
    #[props(default = String::from("No matching options"))] empty_message: String,
    #[props(default = String::from("Loading..."))] loading_message: String,
    #[props(default)] on_change: Option<EventHandler<Vec<String>>>,
    #[props(default)] on_query_change: Option<EventHandler<String>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut focused = use_signal(|| false);
    let mut state = use_signal(|| {
        let mut s = ComboboxState::new(options.clone(), multi).with_selected(selected.clone());
        s.set_loading(loading);
        s.set_composing(composing);
        s
    });

    // Synchronize prop updates without triggering render loops.
    {
        let mut s = state.write_unchecked();
        if s.options() != options.as_slice() {
            s.set_options(options.clone());
        }
        s.set_loading(loading);
        s.set_composing(composing);
    }

    let base_id = test_id.as_deref().unwrap_or("nve-combobox");
    let input_id = format!("{base_id}-input");
    let listbox_id = format!("{base_id}-listbox");
    let label_id = label.as_ref().map(|_| format!("{base_id}-label"));

    let (is_open, query_val, active_desc, selected_vals, is_empty, filtered_opts, active_val) = {
        let s = state.read();
        (
            s.is_open(),
            s.query().to_owned(),
            s.active_option_id(base_id),
            s.selected().to_vec(),
            s.is_empty(),
            s.filtered_options().into_iter().cloned().collect::<Vec<_>>(),
            s.highlighted_option().map(|o| o.value.clone()),
        )
    };

    let wrap_style = TypedStyleBuilder::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .width_pct(100.0)
        .finish();

    let input_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .min_height_px(38.0)
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .surface(TypedSurfaceStyle {
            background: if disabled {
                colors.surface_subtle
            } else {
                colors.surface
            },
            border_width: if focused() && !disabled { 2.0 } else { 1.0 },
            border_color: if focused() && !disabled {
                colors.border_focus
            } else {
                colors.border
            },
            border_radius: theme.radii.md,
            opacity: if disabled { 0.55 } else { 1.0 },
            box_shadow: None,
        })
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.md,
            font_weight: "400",
            text_align: "left",
            opacity: if disabled { 0.55 } else { 1.0 },
        })
        .finish();

    let listbox_style = TypedStyleBuilder::new()
        .flex_col()
        .padding(theme.spacing.xs)
        .gap(2.0)
        .z_index(930)
        .surface(TypedSurfaceStyle {
            background: colors.surface_elevated,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: colors.border,
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        })
        .finish();

    let label_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.sm,
            font_weight: "600",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let muted_style = TypedStyleBuilder::new()
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .text_style(TypedTextStyle {
            color: colors.text_muted,
            font_size: theme.typography.sm,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let tags_row_style = TypedStyleBuilder::new()
        .flex_row()
        .gap(theme.spacing.xs)
        .align_items(FlexAlign::Center)
        .finish();

    let tag_style = TypedStyleBuilder::new()
        .flex_row()
        .align_items(FlexAlign::Center)
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .surface(TypedSurfaceStyle {
            background: colors.surface_subtle,
            border_width: 1.0,
            border_color: colors.border,
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: None,
        })
        .finish();

    let tag_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.xs,
            font_weight: "500",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let rendered_tags: Vec<Element> = if multi {
        selected_vals
            .iter()
            .map(|val| {
                let tag_label = options
                    .iter()
                    .find(|o| o.value == *val)
                    .map(|o| o.label.clone())
                    .unwrap_or_else(|| val.clone());
                let remove_val = val.clone();
                let tag_tid = format!("{base_id}-tag-{val}");
                rsx! {
                    pressable {
                        key: "{val}",
                        test_id: "{tag_tid}",
                        accessibility_role: "button",
                        accessibility_label: format!("Remove {tag_label}"),
                        nve_style: tag_style.clone(),
                        onpress: move |_| {
                            if !disabled {
                                let removed = state.write().remove_selected(&remove_val);
                                if removed && let Some(handler) = on_change {
                                    handler.call(state.read().selected().to_vec());
                                }
                            }
                        },
                        text {
                            nve_style: tag_text_style.clone(),
                            "{tag_label} ×"
                        }
                    }
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let rendered_options: Vec<Element> = filtered_opts
        .into_iter()
        .map(|opt| {
            let is_hl = active_val.as_deref() == Some(opt.value.as_str()) && !opt.disabled;
            let is_sel = selected_vals.iter().any(|v| v == &opt.value);
            let opt_tid = format!("{base_id}-option-{}", opt.value);
            let opt_val = opt.value.clone();
            let opt_label = opt.label.clone();
            let opt_disabled = opt.disabled;

            let opt_style = TypedStyleBuilder::new()
                .flex_row()
                .justify_content(FlexJustify::SpaceBetween)
                .align_items(FlexAlign::Center)
                .padding_xy(theme.spacing.md, theme.spacing.sm)
                .surface(TypedSurfaceStyle {
                    background: if is_hl {
                        colors.surface_subtle
                    } else {
                        ColorRgba::TRANSPARENT
                    },
                    border_width: if is_hl { 1.0 } else { 0.0 },
                    border_color: if is_hl {
                        colors.border_focus
                    } else {
                        ColorRgba::TRANSPARENT
                    },
                    border_radius: theme.radii.sm,
                    opacity: if opt_disabled { 0.45 } else { 1.0 },
                    box_shadow: None,
                })
                .finish();

            let opt_text_style = TypedStyleBuilder::new()
                .text_style(TypedTextStyle {
                    color: if is_sel {
                        colors.accent
                    } else {
                        colors.text_primary
                    },
                    font_size: theme.typography.sm,
                    font_weight: if is_sel || is_hl { "600" } else { "400" },
                    text_align: "left",
                    opacity: 1.0,
                })
                .finish();

            let aria_sel = if is_sel { "true" } else { "false" };
            rsx! {
                pressable {
                    key: "{opt_val}",
                    test_id: "{opt_tid}",
                    accessibility_role: "option",
                    aria_selected: "{aria_sel}",
                    disabled: opt_disabled,
                    nve_style: opt_style,
                    onpress: move |_| {
                        if !disabled && !opt_disabled {
                            let act = state.write().select_value(&opt_val);
                            if matches!(
                                act,
                                ComboboxAction::Selected(_) | ComboboxAction::Deselected(_)
                            ) && let Some(handler) = on_change
                            {
                                handler.call(state.read().selected().to_vec());
                            }
                        }
                    },
                    text {
                        nve_style: opt_text_style,
                        "{opt_label}"
                    }
                }
            }
        })
        .collect();

    let aria_expanded = if is_open { "true" } else { "false" };
    let aria_multi = if multi { "true" } else { "false" };

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            style: style,
            if let Some(ref lbl) = label {
                text {
                    test_id: label_id.clone(),
                    nve_style: label_style,
                    "{lbl}"
                }
            }
            if !rendered_tags.is_empty() {
                view {
                    test_id: format!("{base_id}-tags"),
                    nve_style: tags_row_style,
                    {rendered_tags.into_iter()}
                }
            }
            textinput {
                test_id: "{input_id}",
                value: "{query_val}",
                placeholder: placeholder,
                disabled: disabled,
                tab_index: if disabled { -1 } else { 0 },
                accessibility_role: "combobox",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{listbox_id}",
                aria_autocomplete: "list",
                aria_activedescendant: active_desc,
                aria_labelledby: label_id,
                nve_style: input_style,
                onfocus: move |_| {
                    focused.set(true);
                    if !disabled {
                        state.write().open();
                    }
                },
                onblur: move |_| focused.set(false),
                onchange: move |ev: TextEvent| {
                    if !disabled {
                        let next_q = ev.data().value.clone();
                        state.write().set_query(next_q.clone());
                        if let Some(handler) = on_query_change {
                            handler.call(next_q);
                        }
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    if disabled {
                        return;
                    }
                    let act = state.write().handle_key(ev.data().key.as_str());
                    if matches!(
                        act,
                        ComboboxAction::Selected(_)
                            | ComboboxAction::Deselected(_)
                            | ComboboxAction::RemovedLastTag(_)
                    ) && let Some(handler) = on_change
                    {
                        handler.call(state.read().selected().to_vec());
                    }
                },
            }
            if is_open {
                view {
                    test_id: "{listbox_id}",
                    accessibility_role: "listbox",
                    aria_multiselectable: "{aria_multi}",
                    nve_style: listbox_style,
                    if loading {
                        text {
                            test_id: format!("{base_id}-loading"),
                            aria_live: "polite",
                            nve_style: muted_style,
                            "{loading_message}"
                        }
                    } else if is_empty {
                        text {
                            test_id: format!("{base_id}-empty"),
                            aria_live: "polite",
                            nve_style: muted_style,
                            "{empty_message}"
                        }
                    } else {
                        {rendered_options.into_iter()}
                    }
                }
            }
        }
    }
}
