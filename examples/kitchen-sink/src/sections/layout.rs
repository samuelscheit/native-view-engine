//! Section 5: Layout & Virtualization — resizable `SplitPane`, `TabsRoot`/`TabList`/`TabTrigger`/
//! `TabPanel`, 1,000 variable-height rows `VirtualList` (`FenwickHeightTree`) with live jump
//! controls, and `KeyboardAvoidingView` preview.

use dioxus_core::Callback;
use nve_dioxus::prelude::*;
use nve_ui::{
    effective_bottom_inset, use_theme, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant,
    Card, CardVariant, FenwickHeightTree, KeyboardAvoidingView, KeyboardInsets, Orientation,
    RowHeights, SplitBound, SplitPane, TabList, TabPanel, TabTrigger, TabsRoot, VirtualList,
};

use crate::i18n_copy::copy;
use crate::sections::SectionHeader;
use crate::shell::AppHandle;

const VIRTUAL_ROW_COUNT: usize = 1_000;

fn build_variable_heights() -> Vec<u32> {
    (0..VIRTUAL_ROW_COUNT)
        .map(|i| match i % 4 {
            0 => 38,
            1 => 48,
            2 => 58,
            _ => 44,
        })
        .collect()
}

#[component]
pub fn LayoutSection(handle: AppHandle) -> Element {
    let theme = use_theme();
    let p = &theme.palette;
    let snap = handle.read_state();
    let pseudo = snap.pseudo_locale;

    let mut split_ratio = use_signal(|| 0.42f32);
    let mut jump_epoch = use_signal(|| 0u32);
    let mut target_offset = use_signal(|| 0.0f32);
    let mut selected_row = use_signal(|| Some(0usize));
    let mut simulated_kb_px = use_signal(|| 0.0f32);

    let heights_vec = dioxus_core::use_hook(build_variable_heights);
    let fenwick = dioxus_core::use_hook(|| FenwickHeightTree::from_heights(heights_vec.clone()));
    let row_heights = RowHeights::variable(heights_vec.clone());
    let total_height_px = fenwick.total_height();

    let text_primary = p.text_primary.as_ref();
    let text_secondary = p.text_secondary.as_ref();
    let text_muted = p.text_muted.as_ref();
    let surface_subtle = p.surface_subtle.as_ref();
    let border = p.border.as_ref();
    let accent = p.accent.as_ref();

    let kb_insets = KeyboardInsets {
        height: simulated_kb_px(),
        visible: simulated_kb_px() > 0.0,
        ..Default::default()
    };
    let effective_kb = effective_bottom_inset(0.0, kb_insets, false);

    let selected_label = selected_row().map_or_else(|| "None".to_owned(), |i| format!("#{i}"));
    let jump_targets: [(usize, f32); 4] = [0usize, 250, 500, 999]
        .map(|idx| (idx, fenwick.prefix_sum(idx) as f32));
    let row_accent = accent.to_owned();
    let row_primary = text_primary.to_owned();
    let row_muted = text_muted.to_owned();

    rsx! {
        view {
            test_id: "section-layout",
            style: "width: 100%; flex-direction: column; gap: 16px;",

            SectionHeader {
                title: copy("Layout & Virtualization", pseudo),
                subtitle: copy("Resizable SplitPane, WAI-ARIA Tabs, 1,000 variable-height row VirtualList powered by FenwickHeightTree, and KeyboardAvoidingView.", pseudo),
                tag: "4 Layout Engines".to_owned(),
            }

            // Tabs Showcase (persisted active_tab)
            Card {
                test_id: Some("ks-tabs-card".to_owned()),
                variant: CardVariant::Outlined,
                padding: Some(16.0),
                gap: Some(10.0),
                text {
                    style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                    "{copy(\"TabsRoot / TabList / TabTrigger / TabPanel\", pseudo)}"
                }
                TabsRoot {
                    value: Some(snap.active_tab.clone()),
                    id_prefix: Some("ks-tabs".to_owned()),
                    on_change: move |next: String| {
                        handle.update(|st| st.active_tab = next);
                    },
                    TabList {
                        TabTrigger {
                            value: "architecture".to_owned(),
                            text { "{copy(\"Architecture\", pseudo)}" }
                        }
                        TabTrigger {
                            value: "binary".to_owned(),
                            text { "{copy(\"Binary IR\", pseudo)}" }
                        }
                        TabTrigger {
                            value: "a11y".to_owned(),
                            text { "{copy(\"Accessibility\", pseudo)}" }
                        }
                    }
                    TabPanel {
                        value: "architecture".to_owned(),
                        text {
                            style: "font-size: 13px; color: {text_secondary};",
                            "One Dioxus rsx! tree compiles to flat binary CommandBuffers targeting Web DOM, Skia GPU, SwiftUI/AppKit, Jetpack Compose, and Win32."
                        }
                    }
                    TabPanel {
                        value: "binary".to_owned(),
                        text {
                            style: "font-size: 13px; color: {text_secondary};",
                            "Zero JSON on hot paths: direct #[repr(C)] slabs, borrowed NveSceneView slices, and compact versioned state snapshots."
                        }
                    }
                    TabPanel {
                        value: "a11y".to_owned(),
                        text {
                            style: "font-size: 13px; color: {text_secondary};",
                            "Roving tabindex, FocusTrap, ARIA roles/expanded/selected attributes, and ReducedMotion preferences are first-class."
                        }
                    }
                }
            }

            // Interactive SplitPane
            Card {
                test_id: Some("ks-split-pane-card".to_owned()),
                variant: CardVariant::Outlined,
                padding: Some(16.0),
                gap: Some(10.0),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;",
                    text {
                        style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                        "{copy(\"Resizable SplitPane\", pseudo)}"
                    }
                    Badge {
                        label: format!("Split Ratio: {:.0}% / {:.0}%", split_ratio() * 100.0, (1.0 - split_ratio()) * 100.0),
                        variant: BadgeVariant::Accent,
                    }
                }
                view {
                    style: "width: 100%; height: 150px;",
                    SplitPane {
                        test_id: Some("ks-split-pane".to_owned()),
                        orientation: Orientation::Horizontal,
                        initial_ratio: split_ratio(),
                        container_size: 720.0,
                        min_primary: SplitBound::Pixels(120.0),
                        min_secondary: SplitBound::Pixels(120.0),
                        on_change: move |r: f32| split_ratio.set(r),
                        primary: rsx! {
                            view {
                                style: "width: 100%; height: 100%; padding: 12px; gap: 6px; background-color: {surface_subtle}; border-radius: 8px;",
                                text { style: "font-size: 13px; font-weight: 700; color: {text_primary};", "Primary Pane" }
                                text { style: "font-size: 12px; color: {text_muted};", "Drag or use Left/Right arrows on the divider." }
                            }
                        },
                        secondary: rsx! {
                            view {
                                style: "width: 100%; height: 100%; padding: 12px; gap: 6px; background-color: {surface_subtle}; border-radius: 8px;",
                                text { style: "font-size: 13px; font-weight: 700; color: {text_primary};", "Secondary Pane" }
                                text { style: "font-size: 12px; color: {text_muted};", "Collapsible inspector pane with min/max bounds." }
                            }
                        },
                    }
                }
            }

            // 1,000 Variable-Height Rows VirtualList (FenwickHeightTree)
            Card {
                test_id: Some("ks-virtual-list-card".to_owned()),
                variant: CardVariant::Elevated,
                padding: Some(16.0),
                gap: Some(12.0),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;",
                    text {
                        style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                        "{copy(\"VirtualList — 1,000 Variable-Height Rows\", pseudo)}"
                    }
                    Badge {
                        label: format!("FenwickHeightTree · {total_height_px}px total"),
                        variant: BadgeVariant::Success,
                    }
                }
                view {
                    style: "flex-direction: row; flex-wrap: wrap; gap: 6px; align-items: center;",
                    for (target_idx, target_off) in jump_targets {
                        Button {
                            key: "{target_idx}",
                            test_id: Some(format!("ks-vl-jump-{target_idx}")),
                            variant: if selected_row() == Some(target_idx) {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Outline
                            },
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                target_offset.set(target_off);
                                selected_row.set(Some(target_idx));
                                jump_epoch.set(jump_epoch() + 1);
                            },
                            text { "Jump #{target_idx}" }
                        }
                    }
                    text {
                        test_id: "ks-vl-selected",
                        style: "font-size: 12px; color: {text_muted};",
                        "Selected row: {selected_label}"
                    }
                }

                for epoch in [jump_epoch()] {
                    VirtualList {
                        key: "vl-{epoch}",
                        test_id: Some("ks-virtual-list".to_owned()),
                        heights: row_heights.clone(),
                        viewport_height: 240.0,
                        overscan: 3,
                        initial_scroll_offset: target_offset(),
                        initial_selected: selected_row(),
                        on_select: move |idx: usize| selected_row.set(Some(idx)),
                        render_row: Callback::new({
                            let row_accent = row_accent.clone();
                            let row_primary = row_primary.clone();
                            let row_muted = row_muted.clone();
                            move |(idx, is_sel): (usize, bool)| {
                                let row_h = match idx % 4 {
                                    0 => 38,
                                    1 => 48,
                                    2 => 58,
                                    _ => 44,
                                };
                                let label_col = if is_sel { &row_accent } else { &row_primary };
                                rsx! {
                                    view {
                                        style: "width: 100%; flex-direction: row; align-items: center; justify-content: space-between; padding-horizontal: 12px; gap: 8px;",
                                        text {
                                            style: "font-size: 13px; font-weight: 600; color: {label_col};",
                                            "Row #{idx} — Variable Height Telemetry Entry"
                                        }
                                        text {
                                            style: "font-size: 11px; color: {row_muted};",
                                            "{row_h}px"
                                        }
                                    }
                                }
                            }
                        }),
                    }
                }
            }

            // KeyboardAvoidingView Preview
            Card {
                test_id: Some("ks-keyboard-avoiding-card".to_owned()),
                variant: CardVariant::Subtle,
                padding: Some(16.0),
                gap: Some(10.0),
                view {
                    style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;",
                    text {
                        style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                        "{copy(\"KeyboardAvoidingView Preview\", pseudo)}"
                    }
                    view {
                        style: "flex-direction: row; gap: 6px; flex-wrap: wrap;",
                        Button {
                            test_id: Some("ks-kb-toggle".to_owned()),
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                let next = if simulated_kb_px() > 0.0 { 0.0 } else { 72.0 };
                                simulated_kb_px.set(next);
                            },
                            text {
                                if simulated_kb_px() > 0.0 {
                                    "Dismiss Soft Keyboard"
                                } else {
                                    "Simulate Soft Keyboard (+72px)"
                                }
                            }
                        }
                    }
                }
                view {
                    style: "width: 100%; border-width: 1px; border-color: {border}; border-radius: 8px; padding: 10px;",
                    KeyboardAvoidingView {
                        test_id: Some("ks-keyboard-avoiding".to_owned()),
                        extra_bottom_padding: effective_kb,
                        view {
                            style: "width: 100%; padding: 10px; border-radius: 6px; background-color: {surface_subtle};",
                            text {
                                style: "font-size: 12px; color: {text_primary};",
                                "Effective bottom keyboard inset: {effective_kb:.0}px (input stays visible above virtual keyboard)"
                            }
                        }
                    }
                }
            }
        }
    }
}
