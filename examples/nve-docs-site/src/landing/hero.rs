//! Split 2-column Hero section with Live Interactive NVE Preview Card (`RSX Code` | `Live UI` | `Binary IR`).

use nve_abi::{FlexAlign, FlexJustify};
use nve_docs::syntax_color;
use nve_ui::accent_indicator;
use nve_dioxus::prelude::*;
use nve_docs::{DocsRoute, navigate_to, use_docs_state};
use nve_mdx::highlight_code;

use crate::landing::{PillBadge, resolve_example_app_href};

const CLI_DEV_CMD: &str = "cargo run -p nve-cli -- dev";

const HERO_RSX_SAMPLE: &str = r#"#[component]
pub fn TelemetryCounter() -> Element {
    let mut count = use_signal(|| 3i32);
    let mut host = use_signal(|| "macOS AppKit");

    rsx! {
        view { nve_style: TypedStyle::new().gap(12.0).padding(16.0),
            text { "Count: {count} · {host}" }
            pressable {
                onpress: move |_| count += 1,
                text { "+ Dispatch Frame" }
            }
        }
    }
}
// Exported via single C symbol: nve_get_interface(1..=5)"#;

/// Split 2-column Hero on desktop, stacked on mobile (`< 960pt`).
#[component]
pub fn LandingHero() -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let config = snap.config.clone();
    let cli_copied = snap.is_code_copied("landing-cli");
    drop(snap);

    let docs_href = config.resolve_href("/docs/getting-started/quickstart");
    let studio_href = resolve_example_app_href(&config.base_path, "/examples/studio");
    let examples_href = config.resolve_href("/examples");

    let hero_style = TypedStyle::new()
        .width_pct(100.0).max_width_px(1180.0).flex_row()
        .align_items(FlexAlign::Center).justify_content(FlexJustify::SpaceBetween)
        .gap(36.0).padding_y(56.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col().padding_xy(0.0, 28.0));
    let h1_style = TypedStyle::new()
        .font_size(44.0).font_weight(800).color(theme.text.primary)
        .at(MediaQuery::max_width(959), RowPatch::new().font_size(32.0));
    let sub_style = TypedStyle::new()
        .font_size(17.0).color(theme.text.secondary).line_height_ratio(1.55)
        .at(MediaQuery::max_width(959), RowPatch::new().font_size(15.0));
    let cta_style = TypedStyle::new()
        .flex_row().align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(10.0).padding_top(4.0)
        .at(MediaQuery::max_width(959), RowPatch::new().width_pct(100.0).flex_col().align_items(FlexAlign::Stretch));
    let copy_label = if cli_copied { "Copied ✓" } else { "Copy" };
    let copy_fg = if cli_copied { theme.status.success.text } else { theme.text.secondary };
    let copy_border = if cli_copied { theme.status.success.text } else { theme.border.default };

    rsx! {
        view {
            test_id: "landing-hero",
            nve_style: hero_style,

            // Left column: Status pill, bold H1, concise subtitle, CTAs, copyable CLI bar
            view {
                test_id: "landing-hero-copy",
                nve_style: TypedStyle::new().width_pct(100.0).flex_grow(1.0).flex_shrink(1.0).min_width_px(0.0).max_width_px(560.0).flex_col().gap(20.0),

                // Status badge pill
                view {
                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center),
                    view {
                        test_id: "landing-status-badge",
                        nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0).padding_xy(12.0, 5.0).radius(999.0).bg(theme.surface.base).border(1.0, theme.border.default),
                        view {
                            nve_style: TypedStyle::new().width_px(7.0).height_px(7.0).radius(999.0).bg(theme.status.success.text),
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(theme.text.primary),
                            "v0.1"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                            "· 10 Native & Web Hosts · Direct Binary ABI"
                        }
                    }
                }

                // Bold Headline (semantic <h1>)
                view {
                    test_id: "landing-hero-h1",
                    accessibility_role: "heading",
                    "aria-level": "1",
                    nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(4.0),
                    text {
                        nve_style: h1_style,
                        "Native-speed UI in Rust. One binary ABI, 10 hosts."
                    }
                }

                // Concise subtitle
                text {
                    test_id: "landing-hero-subtitle",
                    nve_style: sub_style,
                    "Write declarative UI once with Rust rsx! and fine-grained signals. Every frame crosses the host boundary as one flat binary CommandBuffer and one zero-copy NveSceneView of 32-byte NveNodeRow rows — zero JS VM, direct binary bridge, sub-second hot reload."
                }

                // Primary + Secondary CTAs (row on desktop, stacked on mobile for zero overflow)
                view {
                    test_id: "landing-cta-row",
                    nve_style: cta_style,

                    pressable {
                        test_id: "landing-cta-docs",
                        accessibility_role: "link",
                        accessibility_label: "Read the Docs Quickstart",
                        "href": "{docs_href}",
                        nve_style: TypedStyle::new().flex_shrink(0.0).justify_content(FlexJustify::Center).align_items(FlexAlign::Center).padding_xy(18.0, 11.0).radius(9.0).bg(theme.status.accent.solid).border(1.0, theme.status.accent.solid),
                        onpress: move |_| {
                            navigate_to(state, DocsRoute::new("/docs/getting-started/quickstart"));
                        },
                        text {
                            nve_style: TypedStyle::new().font_size(14.0).font_weight(700).color(theme.text.on_accent),
                            "Read the Docs →"
                        }
                    }

                    pressable {
                        test_id: "landing-cta-studio",
                        accessibility_role: "link",
                        accessibility_label: "Launch Live NVE Studio WASM app",
                        "href": "{studio_href}",
                        nve_table: nve_dioxus::table_attr_fn(|tx, node| {
                            tx.edit::<nve_web_html::HtmlDataAttrs>(node).entries(Some(
                                nve_web_html::DataAttrList::from_pairs(vec![(
                                    "data-nve-hard-nav".to_owned(),
                                    "true".to_owned(),
                                )]),
                            ));
                        }),
                        nve_style: TypedStyle::new().flex_shrink(0.0).justify_content(FlexJustify::Center).align_items(FlexAlign::Center).padding_xy(18.0, 11.0).radius(9.0).bg(theme.surface.base).border(1.0, theme.border.default),
                        text {
                            nve_style: TypedStyle::new().font_size(14.0).font_weight(600).color(theme.text.primary),
                            "Launch Live Studio →"
                        }
                    }

                    pressable {
                        test_id: "landing-cta-examples",
                        accessibility_role: "link",
                        accessibility_label: "Explore Examples Gallery",
                        "href": "{examples_href}",
                        nve_style: TypedStyle::new().flex_shrink(0.0).justify_content(FlexJustify::Center).align_items(FlexAlign::Center).padding_xy(14.0, 11.0).radius(9.0).bg(ColorRgba::TRANSPARENT).border(1.0, theme.border.default),
                        onpress: move |_| {
                            navigate_to(state, DocsRoute::new("/examples"));
                        },
                        text {
                            nve_style: TypedStyle::new().font_size(14.0).font_weight(500).color(theme.text.secondary),
                            "Examples Gallery"
                        }
                    }
                }

                // One-click copyable CLI command bar
                view {
                    test_id: "landing-cli-bar",
                    nve_style: TypedStyle::new().width_pct(100.0).max_width_px(440.0).flex_row().align_items(FlexAlign::Center).justify_content(FlexJustify::SpaceBetween).gap(10.0).padding_xy(14.0, 9.0).radius(10.0).bg(theme.surface.sunken).border(1.0, theme.border.default),
                    view {
                        nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0).min_width_px(0.0),
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).font_family("monospace").font_weight(700).color(theme.text.link),
                            "$"
                        }
                        text {
                            test_id: "landing-cli-text",
                            nve_style: TypedStyle::new().font_size(13.0).font_family("monospace").color(theme.text.primary),
                            "{CLI_DEV_CMD}"
                        }
                    }
                    pressable {
                        test_id: "landing-cli-copy",
                        accessibility_role: "button",
                        accessibility_label: "Copy cargo dev command",
                        nve_style: TypedStyle::new().flex_shrink(0.0).padding_xy(10.0, 4.0).radius(6.0).bg(theme.surface.subtle).border(1.0, copy_border),
                        onpress: move |_| {
                            let _ = state.write().copy_code("landing-cli", CLI_DEV_CMD);
                        },
                        text {
                            nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(copy_fg),
                            "{copy_label}"
                        }
                    }
                }
            }

            // Right column: Interactive NVE Preview Card (RSX Code | Live UI | Binary IR)
            HeroInteractiveCard {}
        }
    }
}

/// Interactive Hero Preview Card with live tabs (`RSX Code` | `Live UI` | `Binary IR`).
#[component]
fn HeroInteractiveCard() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    drop(snap);

    let mut active_tab = use_signal(|| "live");
    let mut counter = use_signal(|| 3i32);
    let mut frame_seq = use_signal(|| 1u32);
    let mut target_host = use_signal(|| "macOS AppKit");

    let tab = active_tab();
    let count_val = counter();
    let seq_val = frame_seq();
    let host_label = target_host();

    let card_style = TypedStyle::new()
        .width_px(540.0).max_width_pct(100.0).flex_shrink(0.0).flex_col()
        .radius(14.0).bg(theme.surface.base).border(1.0, theme.border.default).overflow_mode(Overflow::Hidden)
        .at(MediaQuery::max_width(959), RowPatch::new().width_pct(100.0));
    let card_header_style = TypedStyle::new()
        .width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center)
        .gap(8.0).padding_xy(14.0, 10.0).bg(theme.surface.subtle).border(1.0, theme.border.default)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col().align_items(FlexAlign::FlexStart).gap(10.0));
    let tablist_style = TypedStyle::new()
        .flex_row().align_items(FlexAlign::Center).gap(4.0).padding(3.0)
        .radius(8.0).bg(theme.surface.canvas).border(1.0, theme.border.default)
        .at(MediaQuery::max_width(959), RowPatch::new().width_pct(100.0).justify_content(FlexJustify::SpaceBetween));
    let host_row_style = TypedStyle::new()
        .width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(8.0)
        .at(MediaQuery::max_width(959), RowPatch::new().flex_col().align_items(FlexAlign::FlexStart).gap(6.0));
    let highlighted_rsx = highlight_code(Some("rust"), HERO_RSX_SAMPLE, &[]);

    let payload_text = format!("Count: {count_val} · {host_label}");
    let frame_bytes = 64 + payload_text.len() + 28;
    let count_hex = format!("{:02X}", (count_val & 0xff) as u8);
    let seq_hex = format!("{:04X}", seq_val);
    let mono_12 = |col| TypedStyle::new().font_size(12.0).font_family("monospace").color(col);

    let tabs: [(&str, &str, &str); 3] = [
        ("live", "Live UI", "hero-tab-live"),
        ("code", "RSX Code", "hero-tab-code"),
        ("ir", "Binary IR", "hero-tab-ir"),
    ];

    let hosts: [&str; 3] = ["macOS AppKit", "Skia Metal", "Web WASM"];

    rsx! {
        view {
            test_id: "landing-hero-preview",
            nve_style: card_style,

            // Card chrome header: window dots + switchable mode tabs (stacked on mobile)
            view {
                nve_style: card_header_style,

                view {
                    nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(6.0),
                    view { nve_style: TypedStyle::new().width_px(9.0).height_px(9.0).radius(999.0).bg(theme.status.danger.solid) }
                    view { nve_style: TypedStyle::new().width_px(9.0).height_px(9.0).radius(999.0).bg(theme.status.warning.solid) }
                    view { nve_style: TypedStyle::new().width_px(9.0).height_px(9.0).radius(999.0).bg(theme.status.success.solid) }
                    text {
                        nve_style: mono_12(theme.text.secondary).padding_x(6.0),
                        "telemetry_counter.rs"
                    }
                }

                // Tablist
                view {
                    accessibility_role: "tablist",
                    nve_style: tablist_style,
                    for (id, label, tid) in tabs {
                        {
                            let selected = tab == id;
                            let bg = if selected { theme.status.accent.subtle } else { ColorRgba::TRANSPARENT };
                            let fg = if selected { theme.text.link } else { theme.text.secondary };
                            let bc = if selected { accent_indicator(&theme) } else { ColorRgba::TRANSPARENT };
                            let sel_attr = if selected { "true" } else { "false" };
                            rsx! {
                                pressable {
                                    key: "hero-tab-{id}",
                                    test_id: "{tid}",
                                    accessibility_role: "tab",
                                    aria_selected: "{sel_attr}",
                                    nve_style: TypedStyle::new().padding_xy(10.0, 4.0).radius(6.0).bg(bg).border(1.0, bc),
                                    onpress: move |_| {
                                        active_tab.set(id);
                                    },
                                    text {
                                        nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(fg),
                                        "{label}"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Tab panel body
            view {
                test_id: "hero-preview-body",
                accessibility_role: "tabpanel",
                nve_style: TypedStyle::new()
                    .width_pct(100.0).min_height_px(236.0).flex_col()
                    .justify_content(FlexJustify::SpaceBetween).padding(18.0).gap(16.0).bg(theme.surface.sunken),

                if tab == "code" {
                    view {
                        test_id: "hero-panel-code",
                        nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(3.0),
                        for line in highlighted_rsx {
                            view {
                                key: "hero-rsx-ln-{line.line_no}",
                                nve_style: TypedStyle::new().width_pct(100.0).flex_row().align_items(FlexAlign::Center).gap(10.0),
                                text {
                                    nve_style: TypedStyle::new().min_width_px(18.0).font_size(11.0).font_family("monospace").color(theme.text.secondary).text_align(TextAlign::Right),
                                    "{line.line_no}"
                                }
                                view {
                                    test_id: "docs-code-tokens",
                                    nve_style: TypedStyle::new().flex_row().flex_grow(1.0),
                                    for (ti, tok) in line.tokens.into_iter().enumerate() {
                                        {
                                            let c = syntax_color(&theme.syntax, tok.kind);
                                            rsx! {
                                                text {
                                                    key: "hero-rsx-{line.line_no}-{ti}",
                                                    nve_style: mono_12(c),
                                                    "{tok.text}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if tab == "ir" {
                    view {
                        test_id: "hero-panel-ir",
                        nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(10.0),
                        view {
                            nve_style: TypedStyle::new().flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(8.0),
                            PillBadge { label: format!("NVE2 Frame #{seq_val}"), accent: true }
                            text {
                                nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").color(theme.status.success.text),
                                "{frame_bytes} B · 0 JSON bytes"
                            }
                        }
                        view {
                            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(6.0).padding(12.0).radius(10.0).bg(theme.surface.base).border(1.0, theme.border.default),
                            text { nve_style: mono_12(theme.text.link), "HDR   4E 56 45 32 00 02 {seq_hex}   ; magic='NVE2' ver=2" }
                            text { nve_style: mono_12(theme.syntax.keyword), "OP_01 CREATE_NODE   id=1 kind=View  style=flex-col" }
                            text { nve_style: mono_12(theme.syntax.string), "OP_02 SET_TEXT      id=2 utf8=\"{payload_text}\"" }
                            text { nve_style: mono_12(theme.syntax.number), "OP_03 SET_PROP_I32  id=3 key=count  val=0x{count_hex}" }
                            text { nve_style: mono_12(theme.status.info.text), "OP_04 SCENE_ROW     size=32B dirty=1" }
                            text { nve_style: mono_12(theme.text.secondary), "OP_05 COMMIT_FRAME  encode=2.02us decode=1.81us" }
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                            "Live state reflects the Live UI tab — click + or switch host target and inspect the updated binary opcodes."
                        }
                    }
                } else {
                    // Live interactive NVE widget right inside the Hero card!
                    view {
                        test_id: "hero-panel-live",
                        nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(14.0),

                        // Host target selector pills
                        view {
                            nve_style: host_row_style,
                            text {
                                nve_style: TypedStyle::new().font_size(11.0).font_weight(700).color(theme.text.secondary),
                                "RENDER TARGET"
                            }
                            view {
                                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(5.0).flex_wrap(FlexWrap::Wrap),
                                for h in hosts {
                                    {
                                        let is_sel = host_label == h;
                                        let bg = if is_sel { theme.status.accent.subtle } else { theme.surface.base };
                                        let fg = if is_sel { theme.text.link } else { theme.text.secondary };
                                        let bc = if is_sel { accent_indicator(&theme) } else { theme.border.default };
                                        let slug: String = h
                                            .chars()
                                            .map(|c| if c == ' ' { '-' } else { c.to_ascii_lowercase() })
                                            .collect();
                                        let host_tid = format!("hero-host-{slug}");
                                        rsx! {
                                            pressable {
                                                key: "hero-host-{h}",
                                                test_id: "{host_tid}",
                                                accessibility_role: "button",
                                                nve_style: TypedStyle::new().flex_shrink(0.0).padding_xy(8.0, 3.0).radius(6.0).bg(bg).border(1.0, bc),
                                                onpress: move |_| {
                                                    target_host.set(h);
                                                    frame_seq += 1;
                                                },
                                                text {
                                                    nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(fg),
                                                    "{h}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Live counter interactive surface
                        view {
                            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(14.0).padding(18.0).radius(12.0).bg(theme.surface.base).border(1.0, theme.border.default),

                            view {
                                nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center),
                                view {
                                    nve_style: TypedStyle::new().flex_col().gap(3.0),
                                    text {
                                        nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(theme.text.secondary),
                                        "LIVE NVE SIGNAL WIDGET"
                                    }
                                    text {
                                        test_id: "hero-live-count",
                                        nve_style: TypedStyle::new().font_size(26.0).font_weight(800).color(theme.text.primary),
                                        "Count: {count_val}"
                                    }
                                }
                                PillBadge { label: host_label.to_string(), accent: true }
                            }

                            view {
                                nve_style: TypedStyle::new().flex_row().align_items(FlexAlign::Center).gap(8.0).flex_wrap(FlexWrap::Wrap),
                                pressable {
                                    test_id: "hero-live-inc",
                                    accessibility_role: "button",
                                    accessibility_label: "Increment hero counter",
                                    nve_style: TypedStyle::new().padding_xy(14.0, 8.0).radius(8.0).bg(theme.status.accent.solid).border(1.0, theme.status.accent.solid),
                                    onpress: move |_| {
                                        counter += 1;
                                        frame_seq += 1;
                                    },
                                    text {
                                        nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(theme.text.on_accent),
                                        "+ Increment"
                                    }
                                }
                                pressable {
                                    test_id: "hero-live-dec",
                                    accessibility_role: "button",
                                    accessibility_label: "Decrement hero counter",
                                    nve_style: TypedStyle::new().padding_xy(12.0, 8.0).radius(8.0).bg(theme.surface.subtle).border(1.0, theme.border.default),
                                    onpress: move |_| {
                                        counter -= 1;
                                        frame_seq += 1;
                                    },
                                    text {
                                        nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(theme.text.primary),
                                        "- 1"
                                    }
                                }
                                pressable {
                                    test_id: "hero-live-reset",
                                    accessibility_role: "button",
                                    accessibility_label: "Reset hero counter",
                                    nve_style: TypedStyle::new().padding_xy(12.0, 8.0).radius(8.0).bg(ColorRgba::TRANSPARENT).border(1.0, theme.border.default),
                                    onpress: move |_| {
                                        counter.set(0);
                                        frame_seq += 1;
                                    },
                                    text {
                                        nve_style: TypedStyle::new().font_size(13.0).font_weight(500).color(theme.text.secondary),
                                        "Reset"
                                    }
                                }
                            }
                        }

                        // Zero-copy live telemetry bar
                        view {
                            nve_style: TypedStyle::new().width_pct(100.0).flex_row().justify_content(FlexJustify::SpaceBetween).align_items(FlexAlign::Center).flex_wrap(FlexWrap::Wrap).gap(6.0).padding_xy(10.0, 7.0).radius(8.0).bg(theme.surface.base).border(1.0, theme.border.default),
                            text {
                                test_id: "hero-live-frame",
                                nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").color(theme.text.secondary),
                                "Frame #{seq_val} · {frame_bytes} B NVE2 · 32B NveNodeRow"
                            }
                            text {
                                nve_style: TypedStyle::new().font_size(11.0).font_family("monospace").font_weight(700).color(theme.status.success.text),
                                "0.98 ns ABI"
                            }
                        }
                    }
                }
            }
        }
    }
}
