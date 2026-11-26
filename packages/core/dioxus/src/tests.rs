use dioxus_core::{Attribute, AttributeValue, Element, VNode};
use nve_abi::{
    pack_key_modifiers, ColorRgba, Command, CommandDecoder, ComponentKind, EventPayload,
    NativeEvent, NativeEventKind, NodeId, NveBufferSlice, NveDirectEvent, NveEventBatch,
    NveEventBatchRef, RawPropValue, NveValue,
};
use nve_components::{DrawListBuilder, DrawListDecoder, NVE_DRAW_MAGIC};

use crate::elements;
use crate::prelude::*;
use crate::DioxusSurface;

#[component]
fn AllEventsApp() -> Element {
    let mut log = use_signal(String::new);

    let on_scroll = Attribute::new(
        "onscroll",
        AttributeValue::listener(move |ev: ScrollEvent| {
            let d = ev.data();
            log.set(format!(
                "scroll:{},{},{},{}",
                d.x, d.y, d.content_width, d.content_height
            ));
        }),
        None,
        false,
    );
    let on_focus = Attribute::new(
        "onfocus",
        AttributeValue::listener(move |_ev: FocusEvent| {
            log.set("focus".into());
        }),
        None,
        false,
    );
    let on_blur = Attribute::new(
        "onblur",
        AttributeValue::listener(move |_ev: BlurEvent| {
            log.set("blur".into());
        }),
        None,
        false,
    );
    let on_longpress = Attribute::new(
        "onlongpress",
        AttributeValue::listener(move |ev: elements::PressEvent| {
            let d = ev.data();
            log.set(format!("longpress:{},{}", d.x, d.y));
        }),
        None,
        false,
    );
    let on_custom = Attribute::new(
        "onmetrics",
        AttributeValue::listener(move |ev: CustomEvent| {
            let d = ev.data();
            log.set(format!("custom:{}:{}", d.name, d.data.len()));
        }),
        None,
        false,
    );
    let on_submit = elements::events::onsubmit(move |ev: SubmitEvent| {
        log.set(format!("submit:{}", ev.data().value));
    });
    let on_keydown = elements::events::onkeydown(move |ev: KeyEvent| {
        let d = ev.data();
        log.set(format!(
            "keydown:{}:{}:{}:{}:{}",
            d.key, d.meta, d.ctrl, d.shift, d.alt
        ));
    });

    let _ = VNode::empty();
    rsx! {
        view {
            test_id: "event-target",
            ..vec![on_scroll, on_focus, on_blur, on_longpress, on_custom, on_submit, on_keydown],
            text { "{log}" }
        }
    }
}

fn extract_target_and_text(bytes: &[u8]) -> (Option<NodeId>, Vec<String>) {
    let (_, cmds) = CommandDecoder::new(bytes).unwrap().decode_all().unwrap();
    let mut target = None;
    let mut texts = Vec::new();
    for cmd in cmds {
        match cmd {
            Command::SetRawProp {
                node_id,
                key,
                value: RawPropValue::String(v),
            } if key == "testID" && v == "event-target" => {
                target = Some(node_id);
            }
            Command::SetText { text, .. } => texts.push(text),
            _ => {}
        }
    }
    (target, texts)
}

#[test]
fn dispatches_scroll_focus_blur_longpress_and_custom_via_ring_and_direct_ffi() {
    let mut surface = DioxusSurface::new(1, AllEventsApp);
    let (_, _, f1) = surface.render_frame(1);
    let (target_opt, _) = extract_target_and_text(&f1);
    let target = target_opt.expect("event-target node mounted");

    // 1. Scroll (EventRing)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 10,
        kind: NativeEventKind::Scroll,
        payload: EventPayload::Scroll {
            x: 12.0,
            y: 240.0,
            content_width: 393.0,
            content_height: 2400.0,
        },
    }]));
    let (_, _, f2) = surface.render_frame(2);
    assert!(extract_target_and_text(&f2)
        .1
        .contains(&"scroll:12,240,393,2400".to_owned()));

    // 2. Focus (EventRing)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 20,
        kind: NativeEventKind::Focus,
        payload: EventPayload::Empty,
    }]));
    let (_, _, f3) = surface.render_frame(3);
    assert!(extract_target_and_text(&f3).1.contains(&"focus".to_owned()));

    // 3. Blur (EventRing)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 30,
        kind: NativeEventKind::Blur,
        payload: EventPayload::Empty,
    }]));
    let (_, _, f4) = surface.render_frame(4);
    assert!(extract_target_and_text(&f4).1.contains(&"blur".to_owned()));

    // 4. LongPress (EventRing)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 40,
        kind: NativeEventKind::LongPress,
        payload: EventPayload::Point { x: 75.5, y: 32.25 },
    }]));
    let (_, _, f5) = surface.render_frame(5);
    assert!(extract_target_and_text(&f5)
        .1
        .contains(&"longpress:75.5,32.25".to_owned()));

    // 5. Custom (EventRing — with leading "on" prefix normalized cleanly)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 50,
        kind: NativeEventKind::Custom,
        payload: EventPayload::Custom {
            name: "onmetrics".into(),
            data: vec![1, 2, 3, 4],
        },
    }]));
    let (_, _, f6) = surface.render_frame(6);
    assert!(extract_target_and_text(&f6)
        .1
        .contains(&"custom:onmetrics:4".to_owned()));

    // 6. Direct FFI path: Scroll, Focus, Blur, LongPress, Custom
    let direct_scroll = [NveDirectEvent::scroll(1, target, 60, 5.0, 99.0, 320.0, 1200.0)];
    let batch_scroll = unsafe { NveEventBatchRef::from_raw(NveEventBatch::from_slice(&direct_scroll)) };
    assert!(surface.dispatch_direct_events(batch_scroll));
    let (_, _, f7) = surface.render_frame(7);
    assert!(extract_target_and_text(&f7)
        .1
        .contains(&"scroll:5,99,320,1200".to_owned()));

    let direct_focus = [NveDirectEvent::empty(1, target, 70, NativeEventKind::Focus)];
    let batch_focus = unsafe { NveEventBatchRef::from_raw(NveEventBatch::from_slice(&direct_focus)) };
    assert!(surface.dispatch_direct_events(batch_focus));
    let (_, _, f8) = surface.render_frame(8);
    assert!(extract_target_and_text(&f8).1.contains(&"focus".to_owned()));

    let direct_blur = [NveDirectEvent::empty(1, target, 80, NativeEventKind::Blur)];
    let batch_blur = unsafe { NveEventBatchRef::from_raw(NveEventBatch::from_slice(&direct_blur)) };
    assert!(surface.dispatch_direct_events(batch_blur));
    let (_, _, f9) = surface.render_frame(9);
    assert!(extract_target_and_text(&f9).1.contains(&"blur".to_owned()));

    let direct_lp = [NveDirectEvent::long_press(1, target, 90, 11.0, 22.0)];
    let batch_lp = unsafe { NveEventBatchRef::from_raw(NveEventBatch::from_slice(&direct_lp)) };
    assert!(surface.dispatch_direct_events(batch_lp));
    let (_, _, f10) = surface.render_frame(10);
    assert!(extract_target_and_text(&f10)
        .1
        .contains(&"longpress:11,22".to_owned()));

    let custom_bytes = [7u8, 8, 9];
    let direct_custom = [NveDirectEvent::custom(
        1,
        target,
        100,
        "metrics",
        NveValue::bytes(NveBufferSlice::from_slice(&custom_bytes)),
    )];
    let batch_custom = unsafe { NveEventBatchRef::from_raw(NveEventBatch::from_slice(&direct_custom)) };
    assert!(surface.dispatch_direct_events(batch_custom));
    let (_, _, f11) = surface.render_frame(11);
    assert!(extract_target_and_text(&f11)
        .1
        .contains(&"custom:metrics:3".to_owned()));

    // 7. Typed Submit & KeyDown (both NativeEvent & direct FFI)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 110,
        kind: NativeEventKind::Submit,
        payload: EventPayload::Text("ship v2".into()),
    }]));
    let (_, _, f12) = surface.render_frame(12);
    assert!(extract_target_and_text(&f12)
        .1
        .contains(&"submit:ship v2".to_owned()));

    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 120,
        kind: NativeEventKind::KeyDown,
        payload: EventPayload::Key {
            key: "k".into(),
            code: "KeyK".into(),
            key_code: 75,
            modifiers: pack_key_modifiers(true, false, true, false, false),
            is_composing: false,
        },
    }]));
    let f13 = surface.render_frame_slice(13).to_vec();
    assert!(extract_target_and_text(&f13)
        .1
        .contains(&"keydown:k:true:false:true:false".to_owned()));

    let direct_submit = [NveDirectEvent::submit(1, target, 130, "direct-msg")];
    let batch_submit = unsafe { NveEventBatchRef::from_raw(NveEventBatch::from_slice(&direct_submit)) };
    assert!(surface.dispatch_direct_events(batch_submit));
    let (_, _, f14) = surface.render_frame(14);
    assert!(extract_target_and_text(&f14)
        .1
        .contains(&"submit:direct-msg".to_owned()));

    let direct_keydown = [NveDirectEvent::key_down(
        1,
        target,
        140,
        "Enter",
        "Enter",
        pack_key_modifiers(false, true, false, true, false),
        13,
        false,
    )];
    let batch_keydown = unsafe { NveEventBatchRef::from_raw(NveEventBatch::from_slice(&direct_keydown)) };
    assert!(surface.dispatch_direct_events(batch_keydown));
    let f15 = surface.render_frame_slice(15).to_vec();
    assert!(extract_target_and_text(&f15)
        .1
        .contains(&"keydown:Enter:false:true:false:true".to_owned()));
}

#[component]
fn CanvasDrawListApp() -> Element {
    let mut dl = DrawListBuilder::with_capacity(128);
    dl.clear(ColorRgba::rgb(17, 24, 39))
        .fill_rect(4.0, 4.0, 64.0, 24.0, ColorRgba::rgb(56, 189, 248));
    let attr = dl.to_attr_string();

    rsx! {
        view {
            canvas {
                test_id: "direct-canvas",
                draw_list: "{attr}",
                style: "width: 120px; height: 48px;",
            }
            canvas {
                test_id: "corrupt-canvas",
                draw_list: "nve-draw1;b64,!!!invalid!!!",
                style: "width: 40px; height: 20px;",
            }
        }
    }
}

#[test]
fn writer_decodes_draw_list_attr_directly_to_bytes_and_render_frame_slice_is_zero_copy() {
    let mut s1 = DioxusSurface::new(1, CanvasDrawListApp);
    let mut s2 = DioxusSurface::new(1, CanvasDrawListApp);

    let slice_bytes = s1.render_frame_slice(1).to_vec();
    let (_, _, owned_bytes) = s2.render_frame(1);
    assert_eq!(slice_bytes, owned_bytes);

    let (_, cmds) = CommandDecoder::new(&slice_bytes)
        .unwrap()
        .decode_all()
        .unwrap();

    let mut saw_canvas = false;
    let mut saw_binary_draw_list = false;
    let mut saw_corrupt_fallback = false;
    for cmd in cmds {
        match cmd {
            Command::CreateNode {
                kind: ComponentKind::SkiaCanvas,
                ..
            } => saw_canvas = true,
            Command::SetRawProp {
                key,
                value: RawPropValue::Bytes(bytes),
                ..
            } if key == "drawList" => {
                assert!(bytes.starts_with(&NVE_DRAW_MAGIC));
                let ops = DrawListDecoder::decode(&bytes).expect("valid NVE_DRAW1 ops");
                assert_eq!(ops.len(), 2);
                saw_binary_draw_list = true;
            }
            Command::SetRawProp {
                key,
                value: RawPropValue::String(raw),
                ..
            } if key == "drawList" && raw.contains("!!!invalid!!!") => {
                saw_corrupt_fallback = true;
            }
            _ => {}
        }
    }

    assert!(saw_canvas);
    assert!(
        saw_binary_draw_list,
        "NveMutationWriter must decode DRAW_LIST_ATTR_PREFIX directly into RawPropValue::Bytes"
    );
    assert!(
        saw_corrupt_fallback,
        "corrupt base64 drawList should degrade safely without panicking"
    );
}

#[component]
fn AriaAttributesApp() -> Element {
    let mut expanded = use_signal(|| false);
    let expanded_str = if expanded() { "true" } else { "false" };
    let val_now: i64 = if expanded() { 75 } else { 25 };
    let described: Option<&str> = if expanded() {
        Some("help-2")
    } else {
        None
    };

    let manual_kebab = Attribute::new(
        "aria-live",
        AttributeValue::Text("assertive".into()),
        None,
        false,
    );

    rsx! {
        view {
            test_id: "aria-dialog",
            accessibility_role: "dialog",
            aria_modal: "true",
            aria_labelledby: "dialog-title",
            aria_describedby: described,
            aria_live: "polite",
            tab_index: -1,
            style: "background-color: #111827; box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);",
            pressable {
                test_id: "aria-trigger",
                accessibility_role: "tab",
                tab_index: 0,
                aria_checked: "mixed",
                aria_selected: "true",
                aria_expanded: "{expanded_str}",
                aria_controls: "panel-1",
                aria_valuenow: val_now,
                aria_valuemin: 0,
                aria_valuemax: 100,
                onpress: move |_| expanded.toggle(),
                text {
                    style: "font-size: 14px; font-weight: 600; text-align: center; color: #ffffff;",
                    "Toggle"
                }
            }
            view {
                test_id: "aria-kebab",
                ..vec![manual_kebab],
            }
        }
    }
}

#[test]
fn typed_aria_and_tab_index_attributes_forward_cleanly_via_set_raw_prop() {
    let mut surface = DioxusSurface::new(1, AriaAttributesApp);
    let (_, _, frame1) = surface.render_frame(1);
    let (_, cmds1) = CommandDecoder::new(&frame1)
        .expect("valid header")
        .decode_all()
        .expect("valid frame 1");

    let find_node = |cmds: &[Command], tid: &str| -> NodeId {
        cmds.iter()
            .find_map(|c| match c {
                Command::SetRawProp {
                    node_id,
                    key,
                    value: RawPropValue::String(v),
                } if key == "testID" && v == tid => Some(*node_id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing node with testID={tid}"))
    };

    let has_str_prop = |cmds: &[Command], nid: NodeId, k: &str, expected: &str| {
        cmds.iter().any(|c| {
            matches!(
                c,
                Command::SetRawProp {
                    node_id,
                    key,
                    value: RawPropValue::String(v),
                } if *node_id == nid && key == k && v == expected
            )
        })
    };

    let dialog_id = find_node(&cmds1, "aria-dialog");
    let trigger_id = find_node(&cmds1, "aria-trigger");
    let kebab_id = find_node(&cmds1, "aria-kebab");

    // Verify all 12 typed ARIA/focus attributes plus preserved Wave 12D inline style forwarding.
    assert!(has_str_prop(&cmds1, dialog_id, "accessibilityRole", "dialog"));
    assert!(has_str_prop(&cmds1, dialog_id, "ariaModal", "true"));
    assert!(has_str_prop(&cmds1, dialog_id, "ariaLabelledBy", "dialog-title"));
    assert!(has_str_prop(&cmds1, dialog_id, "ariaLive", "polite"));
    assert!(has_str_prop(&cmds1, dialog_id, "tabIndex", "-1"));
    assert!(has_str_prop(
        &cmds1,
        dialog_id,
        "box-shadow",
        "0 8px 24px rgba(0, 0, 0, 0.4)"
    ));
    assert!(cmds1.iter().any(|c| matches!(
        c,
        Command::SetRawProp {
            node_id,
            key,
            value: RawPropValue::Null,
        } if *node_id == dialog_id && key == "ariaDescribedBy"
    )));

    assert!(has_str_prop(&cmds1, trigger_id, "tabIndex", "0"));
    assert!(has_str_prop(&cmds1, trigger_id, "ariaChecked", "mixed"));
    assert!(has_str_prop(&cmds1, trigger_id, "ariaSelected", "true"));
    assert!(has_str_prop(&cmds1, trigger_id, "ariaExpanded", "false"));
    assert!(has_str_prop(&cmds1, trigger_id, "ariaControls", "panel-1"));
    assert!(has_str_prop(&cmds1, trigger_id, "ariaValueNow", "25"));
    assert!(has_str_prop(&cmds1, trigger_id, "ariaValueMin", "0"));
    assert!(has_str_prop(&cmds1, trigger_id, "ariaValueMax", "100"));

    // Manual kebab-case attribute normalizes to canonical camelCase key.
    assert!(has_str_prop(&cmds1, kebab_id, "ariaLive", "assertive"));

    // Preserved Wave 12D inline text style forwarding.
    assert!(cmds1.iter().any(|c| matches!(
        c,
        Command::SetRawProp {
            key,
            value: RawPropValue::String(v),
            ..
        } if key == "font-weight" && v == "600"
    )));
    assert!(cmds1.iter().any(|c| matches!(
        c,
        Command::SetRawProp {
            key,
            value: RawPropValue::String(v),
            ..
        } if key == "text-align" && v == "center"
    )));

    // Trigger press toggles reactive ARIA attributes and updates CommandBuffer cleanly.
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: trigger_id,
        timestamp_us: 100,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    }]));
    let (_, _, frame2) = surface.render_frame(2);
    let (_, cmds2) = CommandDecoder::new(&frame2)
        .expect("valid header")
        .decode_all()
        .expect("valid frame 2");
    assert!(has_str_prop(&cmds2, trigger_id, "ariaExpanded", "true"));
    assert!(has_str_prop(&cmds2, trigger_id, "ariaValueNow", "75"));
    assert!(has_str_prop(&cmds2, dialog_id, "ariaDescribedBy", "help-2"));
}
