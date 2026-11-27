//! Split 2-column Hero section with Live Interactive NVE Preview Card (`RSX Code` | `Live UI` | `Binary IR`).

use nve_abi::{FlexAlign, FlexJustify};
use nve_docs::syntax_color;
use nve_ui::{ColorHex, Hex};
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
        view { style: "gap: 12px; padding: 16px;",
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

    let hero_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new().flex_col().padding_xy(0.0, 28.0),
    );
    let h1_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new().font_size(32.0),
    );
    let sub_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new().font_size(15.0),
    );
    let cta_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new()
            .width_pct(100.0)
            .flex_col()
            .align_items(FlexAlign::Stretch),
    );
    let copy_label = if cli_copied { "Copied ✓" } else { "Copy" };
    let copy_fg = if cli_copied { theme.status.success.text.hex() } else { theme.text.secondary.hex() };
    let copy_border = if cli_copied { theme.status.success.solid.hex() } else { theme.border.default.hex() };

    rsx! {
        view {
            test_id: "landing-hero",
            nve_style: hero_resp,
            style: "width: 100%; max-width: 1180px; flex-direction: row; align-items: center; justify-content: space-between; gap: 36px; padding-vertical: 56px;",

            // Left column: Status pill, bold H1, concise subtitle, CTAs, copyable CLI bar
            view {
                test_id: "landing-hero-copy",
                style: "width: 100%; flex-grow: 1; flex-shrink: 1; min-width: 0; max-width: 560px; flex-direction: column; gap: 20px;",

                // Status badge pill
                view {
                    style: "flex-direction: row; align-items: center;",
                    view {
                        test_id: "landing-status-badge",
                        style: "flex-direction: row; align-items: center; gap: 8px; padding-horizontal: 12px; padding-vertical: 5px; border-radius: 999px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                        view {
                            style: "width: 7px; height: 7px; border-radius: 999px; background-color: {theme.status.success.solid.hex()};",
                        }
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {theme.text.primary.hex()};",
                            "v0.1"
                        }
                        text {
                            style: "font-size: 12px; color: {theme.text.secondary.hex()};",
                            "· 10 Native & Web Hosts · Direct Binary ABI"
                        }
                    }
                }

                // Bold Headline (semantic <h1>)
                view {
                    test_id: "landing-hero-h1",
                    accessibility_role: "heading",
                    "aria-level": "1",
                    style: "width: 100%; flex-direction: column; gap: 4px;",
                    text {
                        nve_style: h1_resp,
                        style: "font-size: 44px; font-weight: 800; color: {theme.text.primary.hex()};",
                        "Native-speed UI in Rust. One binary ABI, 10 hosts."
                    }
                }

                // Concise subtitle
                text {
                    test_id: "landing-hero-subtitle",
                    nve_style: sub_resp,
                    style: "font-size: 17px; color: {theme.text.secondary.hex()}; line-height: 1.55;",
                    "Write declarative UI once with Rust rsx! and fine-grained signals. Every frame crosses the host boundary as one flat binary CommandBuffer and 64-byte cache-line NveSceneView — zero JS VM, direct binary bridge, sub-second hot reload."
                }

                // Primary + Secondary CTAs (row on desktop, stacked on mobile for zero overflow)
                view {
                    test_id: "landing-cta-row",
                    nve_style: cta_resp,
                    style: "flex-direction: row; align-items: center; flex-wrap: wrap; gap: 10px; padding-top: 4px;",

                    pressable {
                        test_id: "landing-cta-docs",
                        accessibility_role: "link",
                        accessibility_label: "Read the Docs Quickstart",
                        "href": "{docs_href}",
                        style: "flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 18px; padding-vertical: 11px; border-radius: 9px; background-color: {theme.status.accent.solid.hex()}; border-width: 1px; border-color: {theme.status.accent.solid.hex()};",
                        onpress: move |_| {
                            navigate_to(state, DocsRoute::new("/docs/getting-started/quickstart"));
                        },
                        text {
                            style: "font-size: 14px; font-weight: 700; color: {theme.text.on_accent.hex()};",
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
                        style: "flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 18px; padding-vertical: 11px; border-radius: 9px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                        text {
                            style: "font-size: 14px; font-weight: 600; color: {theme.text.primary.hex()};",
                            "Launch Live Studio →"
                        }
                    }

                    pressable {
                        test_id: "landing-cta-examples",
                        accessibility_role: "link",
                        accessibility_label: "Explore Examples Gallery",
                        "href": "{examples_href}",
                        style: "flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 14px; padding-vertical: 11px; border-radius: 9px; background-color: transparent; border-width: 1px; border-color: {theme.border.default.hex()};",
                        onpress: move |_| {
                            navigate_to(state, DocsRoute::new("/examples"));
                        },
                        text {
                            style: "font-size: 14px; font-weight: 500; color: {theme.text.secondary.hex()};",
                            "Examples Gallery"
                        }
                    }
                }

                // One-click copyable CLI command bar
                view {
                    test_id: "landing-cli-bar",
                    style: "width: 100%; max-width: 440px; flex-direction: row; align-items: center; justify-content: space-between; gap: 10px; padding-horizontal: 14px; padding-vertical: 9px; border-radius: 10px; background-color: {theme.surface.sunken.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                    view {
                        style: "flex-direction: row; align-items: center; gap: 8px; min-width: 0;",
                        text {
                            style: "font-size: 13px; font-family: monospace; font-weight: 700; color: {theme.text.link.hex()};",
                            "$"
                        }
                        text {
                            test_id: "landing-cli-text",
                            style: "font-size: 13px; font-family: monospace; color: {theme.text.primary.hex()};",
                            "{CLI_DEV_CMD}"
                        }
                    }
                    pressable {
                        test_id: "landing-cli-copy",
                        accessibility_role: "button",
                        accessibility_label: "Copy cargo dev command",
                        style: "flex-shrink: 0; padding-horizontal: 10px; padding-vertical: 4px; border-radius: 6px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {copy_border};",
                        onpress: move |_| {
                            let _ = state.write().copy_code("landing-cli", CLI_DEV_CMD);
                        },
                        text {
                            style: "font-size: 11px; font-weight: 600; color: {copy_fg};",
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

    let card_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new().width_pct(100.0),
    );
    let card_header_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new()
            .flex_col()
            .align_items(FlexAlign::FlexStart)
            .gap(10.0),
    );
    let tablist_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new()
            .width_pct(100.0)
            .justify_content(FlexJustify::SpaceBetween),
    );
    let host_row_resp = TypedStyle::new().at(
        MediaQuery::max_width(959),
        ResponsiveStylePatch::new()
            .flex_col()
            .align_items(FlexAlign::FlexStart)
            .gap(6.0),
    );
    let highlighted_rsx = highlight_code(Some("rust"), HERO_RSX_SAMPLE, &[]);

    let payload_text = format!("Count: {count_val} · {host_label}");
    let frame_bytes = 64 + payload_text.len() + 28;
    let count_hex = format!("{:02X}", (count_val & 0xff) as u8);
    let seq_hex = format!("{:04X}", seq_val);

    let tabs: [(&str, &str, &str); 3] = [
        ("live", "Live UI", "hero-tab-live"),
        ("code", "RSX Code", "hero-tab-code"),
        ("ir", "Binary IR", "hero-tab-ir"),
    ];

    let hosts: [&str; 3] = ["macOS AppKit", "Skia Metal", "Web WASM"];

    rsx! {
        view {
            test_id: "landing-hero-preview",
            nve_style: card_resp,
            style: "width: 540px; max-width: 100%; align-self: flex-start; flex-shrink: 0; flex-direction: column; border-radius: 14px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()}; overflow: hidden;",

            // Card chrome header: window dots + switchable mode tabs (stacked on mobile)
            view {
                nve_style: card_header_resp,
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; gap: 8px; padding-horizontal: 14px; padding-vertical: 10px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",

                view {
                    style: "flex-direction: row; align-items: center; gap: 6px;",
                    view { style: "width: 9px; height: 9px; border-radius: 999px; background-color: {theme.status.danger.solid.hex()};" }
                    view { style: "width: 9px; height: 9px; border-radius: 999px; background-color: {theme.status.warning.solid.hex()};" }
                    view { style: "width: 9px; height: 9px; border-radius: 999px; background-color: {theme.status.success.solid.hex()};" }
                    text {
                        style: "font-size: 12px; font-family: monospace; color: {theme.text.secondary.hex()}; padding-horizontal: 6px;",
                        "telemetry_counter.rs"
                    }
                }

                // Tablist
                view {
                    accessibility_role: "tablist",
                    nve_style: tablist_resp,
                    style: "flex-direction: row; align-items: center; gap: 4px; padding: 3px; border-radius: 8px; background-color: {theme.surface.canvas.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                    for (id, label, tid) in tabs {
                        {
                            let selected = tab == id;
                            let bg = if selected { theme.status.accent.subtle.hex() } else { Hex::TRANSPARENT };
                            let fg = if selected { theme.text.link.hex() } else { theme.text.secondary.hex() };
                            let bc = if selected { theme.status.accent.solid.hex() } else { Hex::TRANSPARENT };
                            let sel_attr = if selected { "true" } else { "false" };
                            rsx! {
                                pressable {
                                    key: "hero-tab-{id}",
                                    test_id: "{tid}",
                                    accessibility_role: "tab",
                                    aria_selected: "{sel_attr}",
                                    style: "padding-horizontal: 10px; padding-vertical: 4px; border-radius: 6px; background-color: {bg}; border-width: 1px; border-color: {bc};",
                                    onpress: move |_| {
                                        active_tab.set(id);
                                    },
                                    text {
                                        style: "font-size: 12px; font-weight: 600; color: {fg};",
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
                style: "width: 100%; min-height: 236px; flex-direction: column; justify-content: space-between; padding: 18px; gap: 16px; background-color: {theme.surface.sunken.hex()};",

                if tab == "code" {
                    view {
                        test_id: "hero-panel-code",
                        style: "width: 100%; flex-direction: column; gap: 3px;",
                        for line in highlighted_rsx {
                            view {
                                key: "hero-rsx-ln-{line.line_no}",
                                style: "width: 100%; flex-direction: row; align-items: center; gap: 10px;",
                                text {
                                    style: "min-width: 18px; font-size: 11px; font-family: monospace; color: {theme.text.secondary.hex()}; text-align: right;",
                                    "{line.line_no}"
                                }
                                view {
                                    test_id: "docs-code-tokens",
                                    style: "flex-direction: row; flex-grow: 1;",
                                    for (ti, tok) in line.tokens.into_iter().enumerate() {
                                        {
                                            let c = syntax_color(&theme.syntax, tok.kind).hex();
                                            rsx! {
                                                text {
                                                    key: "hero-rsx-{line.line_no}-{ti}",
                                                    style: "font-size: 12px; font-family: monospace; color: {c};",
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
                        style: "width: 100%; flex-direction: column; gap: 10px;",
                        view {
                            style: "flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;",
                            PillBadge { label: format!("NVE1 Frame #{seq_val}"), accent: true }
                            text {
                                style: "font-size: 11px; font-family: monospace; color: {theme.status.success.text.hex()};",
                                "{frame_bytes} B · 0 JSON bytes"
                            }
                        }
                        view {
                            style: "width: 100%; flex-direction: column; gap: 6px; padding: 12px; border-radius: 10px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                            text {
                                style: "font-size: 12px; font-family: monospace; color: {theme.text.link.hex()};",
                                "HDR   4E 56 45 31 00 01 {seq_hex}   ; magic='NVE1' ver=1"
                            }
                            text {
                                style: "font-size: 12px; font-family: monospace; color: {theme.syntax.keyword.hex()};",
                                "OP_01 CREATE_NODE   id=1 kind=View  style=flex-col"
                            }
                            text {
                                style: "font-size: 12px; font-family: monospace; color: {theme.syntax.string.hex()};",
                                "OP_02 SET_TEXT      id=2 utf8=\"{payload_text}\""
                            }
                            text {
                                style: "font-size: 12px; font-family: monospace; color: {theme.syntax.number.hex()};",
                                "OP_03 SET_PROP_I32  id=3 key=count  val=0x{count_hex}"
                            }
                            text {
                                style: "font-size: 12px; font-family: monospace; color: {theme.status.info.text.hex()};",
                                "OP_04 SCENE_NODE    size=64B align=64 dirty=1"
                            }
                            text {
                                style: "font-size: 12px; font-family: monospace; color: {theme.text.secondary.hex()};",
                                "OP_05 COMMIT_FRAME  encode=2.02us decode=1.81us"
                            }
                        }
                        text {
                            style: "font-size: 12px; color: {theme.text.secondary.hex()};",
                            "Live state reflects the Live UI tab — click + or switch host target and inspect the updated binary opcodes."
                        }
                    }
                } else {
                    // Live interactive NVE widget right inside the Hero card!
                    view {
                        test_id: "hero-panel-live",
                        style: "width: 100%; flex-direction: column; gap: 14px;",

                        // Host target selector pills
                        view {
                            nve_style: host_row_resp,
                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;",
                            text {
                                style: "font-size: 11px; font-weight: 700; color: {theme.text.secondary.hex()};",
                                "RENDER TARGET"
                            }
                            view {
                                style: "flex-direction: row; align-items: center; gap: 5px; flex-wrap: wrap;",
                                for h in hosts {
                                    {
                                        let is_sel = host_label == h;
                                        let bg = if is_sel { theme.status.accent.subtle.hex() } else { theme.surface.base.hex() };
                                        let fg = if is_sel { theme.text.link.hex() } else { theme.text.secondary.hex() };
                                        let bc = if is_sel { theme.status.accent.solid.hex() } else { theme.border.default.hex() };
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
                                                style: "flex-shrink: 0; padding-horizontal: 8px; padding-vertical: 3px; border-radius: 6px; background-color: {bg}; border-width: 1px; border-color: {bc};",
                                                onpress: move |_| {
                                                    target_host.set(h);
                                                    frame_seq += 1;
                                                },
                                                text {
                                                    style: "font-size: 11px; font-weight: 600; color: {fg};",
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
                            style: "width: 100%; flex-direction: column; gap: 14px; padding: 18px; border-radius: 12px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",

                            view {
                                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                                view {
                                    style: "flex-direction: column; gap: 3px;",
                                    text {
                                        style: "font-size: 11px; font-weight: 600; color: {theme.text.secondary.hex()};",
                                        "LIVE NVE SIGNAL WIDGET"
                                    }
                                    text {
                                        test_id: "hero-live-count",
                                        style: "font-size: 26px; font-weight: 800; color: {theme.text.primary.hex()};",
                                        "Count: {count_val}"
                                    }
                                }
                                PillBadge { label: host_label.to_string(), accent: true }
                            }

                            view {
                                style: "flex-direction: row; align-items: center; gap: 8px; flex-wrap: wrap;",
                                pressable {
                                    test_id: "hero-live-inc",
                                    accessibility_role: "button",
                                    accessibility_label: "Increment hero counter",
                                    style: "padding-horizontal: 14px; padding-vertical: 8px; border-radius: 8px; background-color: {theme.status.accent.solid.hex()}; border-width: 1px; border-color: {theme.status.accent.solid.hex()};",
                                    onpress: move |_| {
                                        counter += 1;
                                        frame_seq += 1;
                                    },
                                    text {
                                        style: "font-size: 13px; font-weight: 700; color: {theme.text.on_accent.hex()};",
                                        "+ Increment"
                                    }
                                }
                                pressable {
                                    test_id: "hero-live-dec",
                                    accessibility_role: "button",
                                    accessibility_label: "Decrement hero counter",
                                    style: "padding-horizontal: 12px; padding-vertical: 8px; border-radius: 8px; background-color: {theme.surface.subtle.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                                    onpress: move |_| {
                                        counter -= 1;
                                        frame_seq += 1;
                                    },
                                    text {
                                        style: "font-size: 13px; font-weight: 600; color: {theme.text.primary.hex()};",
                                        "- 1"
                                    }
                                }
                                pressable {
                                    test_id: "hero-live-reset",
                                    accessibility_role: "button",
                                    accessibility_label: "Reset hero counter",
                                    style: "padding-horizontal: 12px; padding-vertical: 8px; border-radius: 8px; background-color: transparent; border-width: 1px; border-color: {theme.border.default.hex()};",
                                    onpress: move |_| {
                                        counter.set(0);
                                        frame_seq += 1;
                                    },
                                    text {
                                        style: "font-size: 13px; font-weight: 500; color: {theme.text.secondary.hex()};",
                                        "Reset"
                                    }
                                }
                            }
                        }

                        // Zero-copy live telemetry bar
                        view {
                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 6px; padding-horizontal: 10px; padding-vertical: 7px; border-radius: 8px; background-color: {theme.surface.base.hex()}; border-width: 1px; border-color: {theme.border.default.hex()};",
                            text {
                                test_id: "hero-live-frame",
                                style: "font-size: 11px; font-family: monospace; color: {theme.text.secondary.hex()};",
                                "Frame #{seq_val} · {frame_bytes} B NVE1 · 64B NveRenderNode"
                            }
                            text {
                                style: "font-size: 11px; font-family: monospace; font-weight: 700; color: {theme.status.success.text.hex()};",
                                "0.98 ns ABI"
                            }
                        }
                    }
                }
            }
        }
    }
}
