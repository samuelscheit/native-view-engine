//! Messaging channel views (`# engineering` & `# design-sync`) built on shared `nve-ui`
//! components (`Card`, `Button`, `Input`, `Separator`) with reaction buttons, real `TextInput`
//! composer, and quick-reply chips.
//!
//! [`ChannelView`] subscribes to the `channels` domain; each [`MessageCard`] is memoized on its
//! message so a reaction re-renders one card.

use nve_dioxus::prelude::*;
use nve_ui::{Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, Input, Separator};

use crate::assets::avatar_for_author;
use crate::state::{ChannelKind, ChatMessage};
use crate::theme::{author_tone, badge_min_w, STRETCH_COL};
use crate::ui::store::use_studio;
use crate::ui::style::use_studio_style;

#[component]
pub(crate) fn ChannelView(ch: ChannelKind) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let channels = store.channels.read();
    let (messages, draft) = match ch {
        ChannelKind::Engineering => (channels.eng_messages.clone(), channels.eng_draft.clone()),
        ChannelKind::Design => (channels.design_messages.clone(), channels.design_draft.clone()),
    };
    drop(channels);

    let composer_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 6px; background-color: {}; border-radius: {}px; {}",
        s.t.surface.base.hex(),
        s.m.button_radius,
        s.card()
    );
    let send_btn_style = format!(
        "width: 54px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px;",
        s.m.button_radius
    );
    let chip_style = format!(
        "flex: 1; height: 26px; min-height: 26px; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: {}px; {}",
        s.t.surface.raised.hex(),
        s.m.button_radius,
        s.control(false)
    );

    rsx! {
        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: {s.m.list_gap}px; {s.row()}",
            for msg in messages {
                MessageCard { key: "msg-{msg.id}", ch, msg }
            }
        }

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
                        placeholder: Some(format!("Message {}...", ch.tag())),
                        value: draft,
                        accessibility_label: Some("Channel message input".into()),
                        on_change: move |txt: String| store.update(|st| st.set_channel_draft(ch, txt)),
                        on_submit: move |_| store.update(|st| {
                            let _ = st.submit_channel_draft(ch);
                        }),
                    }
                }
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("chat-composer-send".into()),
                    style: Some(send_btn_style),
                    on_press: move |_| store.update(|st| {
                        let _ = st.submit_channel_draft(ch);
                    }),
                    "Send"
                }
            }
            Separator {}
            text { style: "color: {s.t.text.tertiary.hex()}; font-size: 10px;", "QUICK REPLY" }
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px;",
                for (tid, chip_label, reply_body) in [
                    ("reply-ship", "Ship It", "Ship It - all checks green!"),
                    ("reply-bench", "Run Benches", "Run Benches - checking p99."),
                    ("reply-lgtm", "LGTM", "LGTM - clean single-ABI PR."),
                ] {
                    Button {
                        key: "{tid}",
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some(tid.into()),
                        style: Some(chip_style.clone()),
                        on_press: move |_| store.update(|st| st.send_quick_reply(ch, reply_body)),
                        "{chip_label}"
                    }
                }
            }
        }
    }
}

/// One chat message; the author chip keeps its synced per-author color pair.
#[component]
fn MessageCard(ch: ChannelKind, msg: ChatMessage) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let mid = msg.id;
    let avatar_src = avatar_for_author(&msg.author);
    let author = author_tone(&s.t, &msg.badge_fg);
    let card_style = format!(
        "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {}; border-radius: {}px; {}",
        s.m.card_pad,
        s.m.card_gap,
        s.t.surface.raised.hex(),
        s.m.card_radius,
        s.card()
    );
    let react_btn_style = format!(
        "width: 60px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: {}px; {}",
        s.t.status.accent.subtle.hex(),
        s.m.pill_radius,
        s.control(true)
    );

    rsx! {
        Card {
            variant: CardVariant::Elevated,
            padding: Some(s.m.card_pad as f32),
            gap: Some(s.m.card_gap as f32),
            style: Some(card_style),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; {s.row()}",
                view {
                    style: "flex-direction: row; align-items: center; gap: 6px;",
                    image {
                        test_id: "msg-avatar-{mid}",
                        src: "{avatar_src}",
                        alt: "{msg.author}",
                        resize_mode: "cover",
                        accessibility_label: "Author avatar {msg.author}",
                        style: "width: 18px; height: 18px; flex-shrink: 0; border-radius: 9px; border-width: 1px; border-color: {s.t.border.default.hex()};",
                    }
                    view {
                        style: "min-width: {badge_min_w(&msg.author)}px; height: 20px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 6px; background-color: {author.subtle.hex()}; border-radius: {s.m.pill_radius}px; border-width: 1px; border-color: {author.border.hex()};",
                        text { style: "color: {author.text.hex()}; font-size: 10px; font-weight: 600;", "{msg.author}" }
                    }
                    text { style: "color: {s.t.text.tertiary.hex()}; font-size: 10px;", "{msg.timestamp}" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(format!("msg-react-{mid}")),
                    style: Some(react_btn_style),
                    on_press: move |_| store.update(|st| st.react_message(ch, mid)),
                    "+1  {msg.reactions}"
                }
            }
            text { style: "color: {s.t.text.primary.hex()}; font-size: 11px;", "{msg.body}" }
        }
    }
}
