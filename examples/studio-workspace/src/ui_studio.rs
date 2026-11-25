//! Docked / drawer **NVE Studio** live self-editing meta-workbench panel.
//!
//! Exposes Accent swatches, Corner Radius, Density, Live Taffy <-> Yoga Layout Engine switcher
//! + inspector readout, X-Ray HUD, and deterministic Time-Travel Scrubber.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;

use crate::state::{
    AccentTheme, CornerRadiusMode, DensityMode, LayoutEngineMode, WorkspaceSnapshot, WorkspaceState,
};
use crate::theme::{
    card_border, control_border, panel_border, row_border, LayoutMetrics, CARD_ALT_BG, CONTROL_BG,
    PANEL_BG, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::ui::mutate_state;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_studio_panel(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    snap_idx: usize,
    hist_len: usize,
    can_undo: bool,
    can_redo: bool,
    m: LayoutMetrics,
    is_mobile: bool,
) -> Element {
    let studio = &current.studio;
    let accent = studio.accent;
    let accent_hex = accent.hex();
    let accent_tint = accent.tint_bg();
    let xray = studio.xray;
    let layout_engine = studio.layout_engine;

    let (panel_bw, panel_bc) = panel_border(xray);
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let action_label = current.action_label.as_str();

    let undo_opacity = if can_undo { "1.0" } else { "0.4" };
    let redo_opacity = if can_redo { "1.0" } else { "0.4" };

    let studio_panel_style = if is_mobile {
        format!(
            "width: 100%; flex-shrink: 0; flex-direction: column; padding: 8px; gap: 6px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};"
        )
    } else {
        format!(
            "width: 210px; height: 100%; flex-shrink: 0; flex-direction: column; padding: 8px; gap: 6px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};"
        )
    };

    let layout_engine_str = layout_engine.as_str();
    let layout_readout_lbl = layout_engine.readout_label();

    let xray_bg = if xray { "#0c2d42" } else { CONTROL_BG };
    let xray_fg = if xray { "#38bdf8" } else { TEXT_PRIMARY };
    let xray_bc = if xray { "#38bdf8" } else { "#334155" };
    let xray_lbl = if xray { "X-Ray HUD: ON" } else { "X-Ray HUD: OFF" };

    let s_xray = Arc::clone(store);
    let s_tt_first = Arc::clone(store);
    let s_tt_undo = Arc::clone(store);
    let s_tt_redo = Arc::clone(store);
    let s_tt_live = Arc::clone(store);

    rsx! {
        view {
            test_id: "studio-panel",
            "layout-engine": "{layout_engine_str}",
            style: "{studio_panel_style}",

            // Studio Header
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                text {
                    style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                    "NVE STUDIO"
                }
                view {
                    style: "height: 18px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 6px; background-color: {accent_tint}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {accent_hex};",
                    text {
                        style: "color: {accent_hex}; font-size: 9px;",
                        "LIVE SELF-EDIT"
                    }
                }
            }

            // Accent Swatches (2x2)
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px; padding: 6px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "ACCENT" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 5px;",
                    for &(theme, tid) in [
                        (AccentTheme::Cyan, "accent-cyan"),
                        (AccentTheme::Violet, "accent-violet"),
                    ].iter() {
                        {
                            let sel = accent == theme;
                            let swatch = theme.hex();
                            let bg = if sel { swatch } else { CONTROL_BG };
                            let fg = if sel { theme.on_accent_fg() } else { swatch };
                            let (bw, bc) = control_border(xray, sel, swatch);
                            let s_acc = Arc::clone(store);
                            rsx! {
                                pressable {
                                    key: "{tid}",
                                    test_id: "{tid}",
                                    style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                    onpress: move |_| mutate_state(&s_acc, version, |st| st.set_accent(theme)),
                                    text { style: "color: {fg}; font-size: 10px;", "{theme.label()}" }
                                }
                            }
                        }
                    }
                }
                view {
                    style: "width: 100%; flex-direction: row; gap: 5px;",
                    for &(theme, tid) in [
                        (AccentTheme::Green, "accent-green"),
                        (AccentTheme::Amber, "accent-amber"),
                    ].iter() {
                        {
                            let sel = accent == theme;
                            let swatch = theme.hex();
                            let bg = if sel { swatch } else { CONTROL_BG };
                            let fg = if sel { theme.on_accent_fg() } else { swatch };
                            let (bw, bc) = control_border(xray, sel, swatch);
                            let s_acc = Arc::clone(store);
                            rsx! {
                                pressable {
                                    key: "{tid}",
                                    test_id: "{tid}",
                                    style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                    onpress: move |_| mutate_state(&s_acc, version, |st| st.set_accent(theme)),
                                    text { style: "color: {fg}; font-size: 10px;", "{theme.label()}" }
                                }
                            }
                        }
                    }
                }
            }

            // Corner Radius + Density Card
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px; padding: 6px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "RADIUS (0 / 12 / 18)" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 4px;",
                    for &(rmode, tid, short_lbl) in [
                        (CornerRadiusMode::Sharp, "radius-sharp", "Sharp"),
                        (CornerRadiusMode::Soft, "radius-soft", "Soft"),
                        (CornerRadiusMode::Pill, "radius-pill", "Pill"),
                    ].iter() {
                        {
                            let sel = studio.radius == rmode;
                            let bg = if sel { accent_tint } else { CONTROL_BG };
                            let fg = if sel { accent_hex } else { TEXT_SECONDARY };
                            let (bw, bc) = control_border(xray, sel, accent_hex);
                            let s_rad = Arc::clone(store);
                            rsx! {
                                pressable {
                                    key: "{tid}",
                                    test_id: "{tid}",
                                    style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                    onpress: move |_| mutate_state(&s_rad, version, |st| st.set_radius(rmode)),
                                    text { style: "color: {fg}; font-size: 10px;", "{short_lbl}" }
                                }
                            }
                        }
                    }
                }

                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "DENSITY" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 5px;",
                    for &(dmode, tid) in [
                        (DensityMode::Normal, "density-normal"),
                        (DensityMode::Compact, "density-compact"),
                    ].iter() {
                        {
                            let sel = studio.density == dmode;
                            let bg = if sel { accent_tint } else { CONTROL_BG };
                            let fg = if sel { accent_hex } else { TEXT_SECONDARY };
                            let (bw, bc) = control_border(xray, sel, accent_hex);
                            let s_den = Arc::clone(store);
                            rsx! {
                                pressable {
                                    key: "{tid}",
                                    test_id: "{tid}",
                                    style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                    onpress: move |_| mutate_state(&s_den, version, |st| st.set_density(dmode)),
                                    text { style: "color: {fg}; font-size: 10px;", "{dmode.label()}" }
                                }
                            }
                        }
                    }
                }
            }

            // Live Taffy <-> Yoga Layout Engine Switcher + Inspector Readout
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px; padding: 6px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "LAYOUT ENGINE" }
                view {
                    style: "width: 100%; flex-direction: row; gap: 5px;",
                    for &(emode, tid) in [
                        (LayoutEngineMode::Taffy, "layout-taffy"),
                        (LayoutEngineMode::Yoga, "layout-yoga"),
                    ].iter() {
                        {
                            let sel = layout_engine == emode;
                            let bg = if sel { accent_tint } else { CONTROL_BG };
                            let fg = if sel { accent_hex } else { TEXT_SECONDARY };
                            let (bw, bc) = control_border(xray, sel, accent_hex);
                            let s_eng = Arc::clone(store);
                            rsx! {
                                pressable {
                                    key: "{tid}",
                                    test_id: "{tid}",
                                    style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                    onpress: move |_| mutate_state(&s_eng, version, |st| st.set_layout_engine(emode)),
                                    text { style: "color: {fg}; font-size: 10px;", "{emode.label()}" }
                                }
                            }
                        }
                    }
                }
                view {
                    test_id: "layout-engine-readout",
                    "layout-engine": "{layout_engine_str}",
                    style: "width: 100%; flex-shrink: 0;",
                    text {
                        style: "color: {accent_hex}; font-size: 10px;",
                        "{layout_readout_lbl}"
                    }
                }
            }

            // X-Ray HUD Toggle
            pressable {
                test_id: "toggle-xray",
                style: "width: 100%; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {xray_bg}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {xray_bc};",
                onpress: move |_| mutate_state(&s_xray, version, |st| st.toggle_xray()),
                text {
                    style: "color: {xray_fg}; font-size: 11px;",
                    "{xray_lbl}"
                }
            }

            // Deterministic Time-Travel Scrubber
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px; padding: 6px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "TIME TRAVEL" }
                text {
                    style: "color: {TEXT_PRIMARY}; font-size: 11px;",
                    "Snapshot {snap_idx} / {hist_len}"
                }
                text {
                    style: "color: {accent_hex}; font-size: 10px;",
                    "{action_label}"
                }
                view {
                    style: "width: 100%; flex-direction: row; gap: 5px;",
                    pressable {
                        test_id: "tt-first",
                        style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {undo_opacity};",
                        onpress: move |_| mutate_state(&s_tt_first, version, |st| st.scrub_first()),
                        text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "<< #1" }
                    }
                    pressable {
                        test_id: "tt-undo",
                        style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {undo_opacity};",
                        onpress: move |_| mutate_state(&s_tt_undo, version, |st| st.scrub_undo()),
                        text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "< Undo" }
                    }
                }
                view {
                    style: "width: 100%; flex-direction: row; gap: 5px;",
                    pressable {
                        test_id: "tt-redo",
                        style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {redo_opacity};",
                        onpress: move |_| mutate_state(&s_tt_redo, version, |st| st.scrub_redo()),
                        text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "Redo >" }
                    }
                    pressable {
                        test_id: "tt-live",
                        style: "flex: 1; height: 22px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {redo_opacity};",
                        onpress: move |_| mutate_state(&s_tt_live, version, |st| st.scrub_live()),
                        text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "Live >>" }
                    }
                }
            }
        }
    }
}
