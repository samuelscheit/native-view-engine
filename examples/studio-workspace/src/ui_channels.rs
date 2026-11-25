//! Messaging channel views (`# engineering` & `# design-sync`) with reaction buttons,
//! real `TextInput` composer, and quick-reply chips.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;

use crate::assets::avatar_for_author;
use crate::state::{ChannelKind, ChatMessage, WorkspaceState};
use crate::theme::{
    card_border, control_border, row_border, LayoutMetrics, BORDER_DEFAULT, CARD_ALT_BG, CARD_BG,
    PANEL_BG, TEXT_MUTED, TEXT_PRIMARY, XRAY_ROW,
};
use crate::ui::mutate_state;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_channel_view(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    ch: ChannelKind,
    channel_messages: &[ChatMessage],
    channel_draft: &str,
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);
    let composer_placeholder = format!("Message {}...", ch.tag());
    let s_chat_input = Arc::clone(store);
    let s_chat_submit = Arc::clone(store);
    let s_chat_send = Arc::clone(store);

    rsx! {
        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
            for msg in channel_messages.iter() {
                {
                    let mid = msg.id;
                    let react_tid = format!("msg-react-{mid}");
                    let avatar_tid = format!("msg-avatar-{mid}");
                    let avatar_src = avatar_for_author(&msg.author);
                    let card_style = format!(
                        "width: 100%; flex-shrink: 0; flex-direction: column; padding: {}px; gap: {}px; background-color: {CARD_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
                        m.card_pad, m.card_gap, m.card_radius
                    );
                    let react_lbl = format!("+1  {}", msg.reactions);
                    let react_bc = if xray { XRAY_ROW } else { accent_hex };
                    let s_react = Arc::clone(store);
                    rsx! {
                        view {
                            key: "msg-{mid}",
                            style: "{card_style}",
                            view {
                                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                                view {
                                    style: "flex-direction: row; align-items: center; gap: 6px;",
                                    image {
                                        test_id: "{avatar_tid}",
                                        src: "{avatar_src}",
                                        alt: "{msg.author}",
                                        resize_mode: "cover",
                                        accessibility_label: "Author avatar {msg.author}",
                                        style: "width: 18px; height: 18px; flex-shrink: 0; border-radius: 9px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                    }
                                    view {
                                        style: "height: 20px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 7px; background-color: {msg.badge_bg}; border-radius: {m.pill_radius}px;",
                                        text {
                                            style: "color: {msg.badge_fg}; font-size: 10px;",
                                            "{msg.author}"
                                        }
                                    }
                                    text {
                                        style: "color: {TEXT_MUTED}; font-size: 10px;",
                                        "{msg.timestamp}"
                                    }
                                }
                                pressable {
                                    test_id: "{react_tid}",
                                    style: "width: 58px; height: 24px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_tint}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {react_bc};",
                                    onpress: move |_| mutate_state(&s_react, version, |st| st.react_message(ch, mid)),
                                    text {
                                        style: "color: {accent_hex}; font-size: 11px;",
                                        "{react_lbl}"
                                    }
                                }
                            }
                            text {
                                style: "color: {TEXT_PRIMARY}; font-size: 11px;",
                                "{msg.body}"
                            }
                        }
                    }
                }
            }
        }

        // Live TextInput Chat Composer + Quick-Reply Chips
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px; padding: 9px; background-color: {PANEL_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; align-items: center;",
                textinput {
                    test_id: "chat-composer-input",
                    placeholder: "{composer_placeholder}",
                    value: "{channel_draft}",
                    multiline: false,
                    accessibility_label: "Channel message input",
                    style: "flex: 1; height: 28px; padding-horizontal: 8px; background-color: {CARD_ALT_BG}; color: {TEXT_PRIMARY}; font-size: 11px; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                    onchange: move |ev| {
                        let txt = ev.data().value.clone();
                        mutate_state(&s_chat_input, version, |st| st.set_channel_draft(ch, txt));
                    },
                    onsubmit: move |_| {
                        mutate_state(&s_chat_submit, version, |st| {
                            let _ = st.submit_channel_draft(ch);
                        });
                    },
                }
                pressable {
                    test_id: "chat-composer-send",
                    style: "width: 48px; height: 28px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_hex}; border-radius: {m.button_radius}px;",
                    onpress: move |_| mutate_state(&s_chat_send, version, |st| {
                        let _ = st.submit_channel_draft(ch);
                    }),
                    text { style: "color: #0b1120; font-size: 10px;", "Send" }
                }
            }
            text {
                style: "color: {TEXT_MUTED}; font-size: 10px;",
                "QUICK REPLY"
            }
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px;",
                for &(tid, chip_label, reply_body) in [
                    ("reply-ship", "Ship It", "Ship It - all checks green!"),
                    ("reply-bench", "Run Benches", "Run Benches - checking p99."),
                    ("reply-lgtm", "LGTM", "LGTM - clean single-ABI PR."),
                ].iter() {
                    {
                        let (bw, bc) = control_border(xray, false, accent_hex);
                        let chip_style = format!(
                            "flex: 1; height: 26px; justify-content: center; align-items: center; background-color: {CARD_BG}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                            m.button_radius
                        );
                        let s_rep = Arc::clone(store);
                        rsx! {
                            pressable {
                                key: "{tid}",
                                test_id: "{tid}",
                                style: "{chip_style}",
                                onpress: move |_| mutate_state(&s_rep, version, |st| st.send_quick_reply(ch, reply_body)),
                                text {
                                    style: "color: {TEXT_PRIMARY}; font-size: 10px;",
                                    "{chip_label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
