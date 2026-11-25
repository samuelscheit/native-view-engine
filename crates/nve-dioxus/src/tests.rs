use dioxus_core::{Attribute, AttributeValue, Element, VNode};
use nve_abi::events::encode_keydown_bytes;
use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, EventPayload, NativeEvent, NativeEventKind,
    NodeId, NveBufferSlice, NveDirectEvent, NveEventBatch, NveEventBatchRef, NveStrSlice, NveValue,
    RawPropValue,
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
    let direct_scroll = [NveDirectEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 60,
        kind: NativeEventKind::Scroll as u8,
        payload_tag: 2,
        name: NveStrSlice::EMPTY,
        x: 5.0,
        y: 99.0,
        width: 320.0,
        height: 1200.0,
        text: NveStrSlice::EMPTY,
        value: NveValue::null(),
    }];
    let batch_scroll = unsafe {
        NveEventBatchRef::from_raw(NveEventBatch {
            ptr: direct_scroll.as_ptr(),
            len: 1,
        })
    };
    assert!(surface.dispatch_events_direct(batch_scroll));
    let (_, _, f7) = surface.render_frame(7);
    assert!(extract_target_and_text(&f7)
        .1
        .contains(&"scroll:5,99,320,1200".to_owned()));

    let direct_focus = [NveDirectEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 70,
        kind: NativeEventKind::Focus as u8,
        payload_tag: 0,
        ..Default::default()
    }];
    let batch_focus = unsafe {
        NveEventBatchRef::from_raw(NveEventBatch {
            ptr: direct_focus.as_ptr(),
            len: 1,
        })
    };
    assert!(surface.dispatch_events_direct(batch_focus));
    let (_, _, f8) = surface.render_frame(8);
    assert!(extract_target_and_text(&f8).1.contains(&"focus".to_owned()));

    let direct_blur = [NveDirectEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 80,
        kind: NativeEventKind::Blur as u8,
        payload_tag: 0,
        ..Default::default()
    }];
    let batch_blur = unsafe {
        NveEventBatchRef::from_raw(NveEventBatch {
            ptr: direct_blur.as_ptr(),
            len: 1,
        })
    };
    assert!(surface.dispatch_events_direct(batch_blur));
    let (_, _, f9) = surface.render_frame(9);
    assert!(extract_target_and_text(&f9).1.contains(&"blur".to_owned()));

    let direct_lp = [NveDirectEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 90,
        kind: NativeEventKind::LongPress as u8,
        payload_tag: 1,
        x: 11.0,
        y: 22.0,
        ..Default::default()
    }];
    let batch_lp = unsafe {
        NveEventBatchRef::from_raw(NveEventBatch {
            ptr: direct_lp.as_ptr(),
            len: 1,
        })
    };
    assert!(surface.dispatch_events_direct(batch_lp));
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
    let batch_custom = unsafe {
        NveEventBatchRef::from_raw(NveEventBatch {
            ptr: direct_custom.as_ptr(),
            len: 1,
        })
    };
    assert!(surface.dispatch_events_direct(batch_custom));
    let (_, _, f11) = surface.render_frame(11);
    assert!(extract_target_and_text(&f11)
        .1
        .contains(&"custom:metrics:3".to_owned()));

    // 7. Typed Submit & KeyDown (both EventRing & direct FFI; binary NK1 & UTF-8 shorthand)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 110,
        kind: NativeEventKind::Custom,
        payload: EventPayload::Custom {
            name: "submit".into(),
            data: b"ship v2".to_vec(),
        },
    }]));
    let (_, _, f12) = surface.render_frame(12);
    assert!(extract_target_and_text(&f12)
        .1
        .contains(&"submit:ship v2".to_owned()));

    let nk1_keydown = encode_keydown_bytes("k", true, false, true, false);
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: target,
        timestamp_us: 120,
        kind: NativeEventKind::Custom,
        payload: EventPayload::Custom {
            name: "keydown".into(),
            data: nk1_keydown,
        },
    }]));
    let f13 = surface.render_frame_slice(13).to_vec();
    assert!(extract_target_and_text(&f13)
        .1
        .contains(&"keydown:k:true:false:true:false".to_owned()));

    let direct_submit = [NveDirectEvent::custom(
        1,
        target,
        130,
        "onsubmit",
        NveValue::str(NveStrSlice::from_str("direct-msg")),
    )];
    let batch_submit = unsafe {
        NveEventBatchRef::from_raw(NveEventBatch {
            ptr: direct_submit.as_ptr(),
            len: 1,
        })
    };
    assert!(surface.dispatch_events_direct(batch_submit));
    let (_, _, f14) = surface.render_frame(14);
    assert!(extract_target_and_text(&f14)
        .1
        .contains(&"submit:direct-msg".to_owned()));

    let direct_keydown = [NveDirectEvent::custom(
        1,
        target,
        140,
        "keydown",
        NveValue::str(NveStrSlice::from_str("Ctrl+Alt+Enter")),
    )];
    let batch_keydown = unsafe {
        NveEventBatchRef::from_raw(NveEventBatch {
            ptr: direct_keydown.as_ptr(),
            len: 1,
        })
    };
    assert!(surface.dispatch_events_direct(batch_keydown));
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
