//! Messaging channel views (`# engineering` & `# design-sync`) built on shared `nve-ui`
//! components (`Card`, `Badge`, `Button`, `Input`, `Separator`) with reaction buttons,
//! real `TextInput` composer, and quick-reply chips.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Input, Separator,
};

use crate::assets::avatar_for_author;
use crate::state::{ChannelKind, ChatMessage, WorkspaceState};
use crate::theme::{
    badge_min_w, card_border, control_border, row_border, LayoutMetrics, BORDER_DEFAULT, CARD_BG,
    PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_PRIMARY, XRAY_ROW,
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

    let composer_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 6px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let send_btn_style = format!(
        "width: 54px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px;",
        m.button_radius
    );

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
                        "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {CARD_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
                        m.card_pad, m.card_gap, m.card_radius
                    );
                    let author_badge_style = format!(
                        "min-width: {}px; height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {}; border-radius: {}px; border-width: 0px; border-color: transparent;",
                        badge_min_w(&msg.author), msg.badge_bg, m.pill_radius
                    );
                    let react_lbl = format!("+1  {}", msg.reactions);
                    let react_bc = if xray { XRAY_ROW } else { accent_hex };
                    let react_btn_style = format!(
                        "width: 60px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {react_bc};",
                        m.pill_radius
                    );
                    let s_react = Arc::clone(store);
                    rsx! {
                        Card {
                            key: "msg-{mid}",
                            variant: CardVariant::Elevated,
                            padding: Some(m.card_pad as f32),
                            gap: Some(m.card_gap as f32),
                            style: Some(card_style),
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
                                    Badge {
                                        label: msg.author.clone(),
                                        variant: BadgeVariant::Neutral,
                                        style: Some(author_badge_style),
                                    }
                                    text {
                                        style: "color: {TEXT_MUTED}; font-size: 10px;",
                                        "{msg.timestamp}"
                                    }
                                }
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(react_tid),
                                    style: Some(react_btn_style),
                                    on_press: move |_| mutate_state(&s_react, version, |st| st.react_message(ch, mid)),
                                    "{react_lbl}"
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

        // Live TextInput Chat Composer + Quick-Reply Chips (`nve-ui` Card + Input + Button + Separator)
        Card {
            variant: CardVariant::Outlined,
            padding: Some(8.0),
            gap: Some(6.0),
            style: Some(composer_card_style),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; align-items: center;",
                view {
                    style: "flex: 1;",
                    Input {
                        test_id: Some("chat-composer-input".into()),
                        placeholder: Some(composer_placeholder),
                        value: channel_draft.to_string(),
                        accessibility_label: Some("Channel message input".into()),
                        on_change: move |txt: String| {
                            mutate_state(&s_chat_input, version, |st| st.set_channel_draft(ch, txt));
                        },
                        on_submit: move |_| {
                            mutate_state(&s_chat_submit, version, |st| {
                                let _ = st.submit_channel_draft(ch);
                            });
                        },
                    }
                }
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("chat-composer-send".into()),
                    style: Some(send_btn_style),
                    on_press: move |_| mutate_state(&s_chat_send, version, |st| {
                        let _ = st.submit_channel_draft(ch);
                    }),
                    "Send"
                }
            }
            Separator {}
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
                            "flex: 1; height: 26px; min-height: 26px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CARD_BG}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                            m.button_radius
                        );
                        let s_rep = Arc::clone(store);
                        rsx! {
                            Button {
                                key: "{tid}",
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                test_id: Some(tid.into()),
                                style: Some(chip_style),
                                on_press: move |_| mutate_state(&s_rep, version, |st| st.send_quick_reply(ch, reply_body)),
                                "{chip_label}"
                            }
                        }
                    }
                }
            }
        }
    }
}
