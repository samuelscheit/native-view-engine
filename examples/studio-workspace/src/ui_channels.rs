//! Messaging channel views (`# engineering` & `# design-sync`) built on shared `nve-ui`
//! components (`Card`, `Button`, `Input`, `Separator`) with reaction buttons, real `TextInput`
//! composer, and quick-reply chips.
//!
//! [`ChannelView`] subscribes to the `channels` domain and [`ChannelComposer`] to
//! `channel_drafts`, so typing re-renders the composer only; each [`MessageCard`] is memoized on
//! its message so a reaction re-renders one card.

use nve_dioxus::prelude::*;
use nve_ui::{Button, ButtonSize, ButtonVariant, Card, CardVariant, Input, Separator};

use crate::assets::avatar_for_author;
use crate::state::{ChannelKind, ChatMessage};
use crate::theme::{author_tone, badge_min_w};
use crate::ui::store::use_studio;
use crate::ui::style::{
    StudioStyleExt, fixed_dims, flex_dims, row_between, row_center, text_style, use_studio_style,
};

#[component]
pub(crate) fn ChannelView(ch: ChannelKind) -> Element {
    nve_dioxus::render_probe::record_render("ChannelView");
    let store = use_studio();
    let s = use_studio_style();
    let channels = store.channels.read();
    let messages = match ch {
        ChannelKind::Engineering => channels.eng_messages.clone(),
        ChannelKind::Design => channels.design_messages.clone(),
    };
    drop(channels);

    rsx! {
        scrollview {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_1()
                .flex_col()
                .gap(s.m.list_gap as f32)
                .outline(s.row()),
            for msg in messages.iter().cloned() {
                MessageCard { key: "msg-{msg.id}", ch, msg }
            }
        }
        ChannelComposer { ch }
    }
}

/// Message composer and quick replies: the only channel part subscribed to `channel_drafts`.
#[component]
fn ChannelComposer(ch: ChannelKind) -> Element {
    nve_dioxus::render_probe::record_render("ChannelComposer");
    let store = use_studio();
    let s = use_studio_style();
    let drafts = store.channel_drafts.read();
    let draft = match ch {
        ChannelKind::Engineering => drafts.eng_draft.clone(),
        ChannelKind::Design => drafts.design_draft.clone(),
    };
    drop(drafts);

    let composer_card_style = s.card_surface(8, 6, s.t.surface.base, s.m.button_radius);
    let send_btn_style = s.compact_control(fixed_dims(54.0, 34.0), 2, s.m.button_radius, None);
    let chip_style = s
        .compact_control(flex_dims(26.0), 2, s.m.button_radius, None)
        .bg(s.t.surface.raised)
        .outline(s.control(false));

    rsx! {
        Card {
            variant: CardVariant::Outlined,
            padding: Some(8.0),
            gap: Some(6.0),
            nve_style: Some(composer_card_style),
            view {
                nve_style: row_center(6.0).width_pct(100.0).flex_shrink(0.0),
                view {
                    nve_style: TypedStyle::new().flex_1(),
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
                    nve_style: Some(send_btn_style),
                    on_press: move |_| store.update(|st| {
                        let _ = st.submit_channel_draft(ch);
                    }),
                    "Send"
                }
            }
            Separator {}
            text { nve_style: text_style(s.t.text.tertiary, 10.0), "QUICK REPLY" }
            view {
                nve_style: TypedStyle::new().width_pct(100.0).flex_shrink(0.0).flex_row().gap(6.0),
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
                        nve_style: Some(chip_style.clone()),
                        on_press: move |_| store.update(|st| st.send_quick_reply(ch, reply_body)),
                        "{chip_label}"
                    }
                }
            }
        }
    }
}

/// One chat message; the author chip wears the tone hashed from the author name (`author_tone`).
#[component]
fn MessageCard(ch: ChannelKind, msg: ChatMessage) -> Element {
    nve_dioxus::render_probe::record_render("MessageCard");
    let store = use_studio();
    let s = use_studio_style();
    let mid = msg.id;
    let avatar_src = avatar_for_author(&msg.author);
    let author = author_tone(&s.t, &msg.author);
    let card_style = s.card_surface(s.m.card_pad, s.m.card_gap, s.t.surface.raised, s.m.card_radius);
    let react_btn_style = s
        .compact_control(fixed_dims(60.0, 24.0), 2, s.m.pill_radius, None)
        .bg(s.t.status.accent.subtle)
        .outline(s.control(true));

    rsx! {
        Card {
            variant: CardVariant::Elevated,
            padding: Some(s.m.card_pad as f32),
            gap: Some(s.m.card_gap as f32),
            nve_style: Some(card_style),
            view {
                nve_style: row_between().outline(s.row()),
                view {
                    nve_style: row_center(6.0),
                    image {
                        test_id: "msg-avatar-{mid}",
                        src: "{avatar_src}",
                        alt: "{msg.author}",
                        resize_mode: "cover",
                        accessibility_label: "Author avatar {msg.author}",
                        nve_style: TypedStyle::new()
                            .width_px(18.0)
                            .height_px(18.0)
                            .flex_shrink(0.0)
                            .radius(9.0)
                            .border(1.0, s.t.border.default),
                    }
                    view {
                        nve_style: TypedStyle::new()
                            .min_width_px(badge_min_w(&msg.author) as f32)
                            .height_px(20.0)
                            .flex_shrink(0.0)
                            .justify_content(FlexJustify::Center)
                            .align_items(FlexAlign::Center)
                            .padding_x(6.0)
                            .bg(author.subtle)
                            .radius(s.m.pill_radius as f32)
                            .border(1.0, author.border),
                        text {
                            nve_style: text_style(author.text, 10.0).font_weight(600),
                            "{msg.author}"
                        }
                    }
                    text { nve_style: text_style(s.t.text.tertiary, 10.0), "{msg.timestamp}" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(format!("msg-react-{mid}")),
                    nve_style: Some(react_btn_style),
                    on_press: move |_| store.update(|st| st.react_message(ch, mid)),
                    "+1  {msg.reactions}"
                }
            }
            text { nve_style: text_style(s.t.text.primary, 11.0), "{msg.body}" }
        }
    }
}
