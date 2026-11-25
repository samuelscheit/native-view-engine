use std::cell::Cell;
use std::rc::Rc;

use nve_abi::events::{decode_keydown_bytes, DecodedKeyDown, KEYDOWN_WIRE_MAGIC};
use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask, EventPayload,
    FlexAlign, FlexDirection, FlexJustify, LayoutProps, NativeEvent, NativeEventKind, RawPropValue,
    StyleProps, ROOT_NODE_ID,
};
use nve_components::canvas_ir::{CanvasOp, DrawListBuilder, ImageResizeMode};
use nve_engine::RenderBackend;

use crate::events::IME_PROCESS_KEY_CODE;
use crate::{
    encode_keydown_payload, layout_to_css, layout_to_css_into, map_a11y_or_input_attr,
    style_to_css, style_to_css_into, ImeCompositionGuard, MappedHtmlAttr, WebBackend,
};

#[test]
fn renders_flexbox_html_from_command_buffer() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_text(2, "Click");
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();
    let html = web.render_html();
    assert!(html.contains("<button data-nve-id=\"2\""));
    assert!(html.contains("Click</button>"));

    // Reusing a scratch String via `fmt_html_into` matches `render_html` identically.
    let mut scratch = String::with_capacity(256);
    web.fmt_html_into(ROOT_NODE_ID, &mut scratch);
    assert_eq!(scratch, html);
}

#[test]
fn css_into_writers_match_wrappers_and_reuse_buffers() {
    let layout = LayoutProps {
        width: Dimension::Percent(100.0),
        height: Dimension::Points(64.0),
        min_width: Dimension::Points(120.0),
        min_height: Dimension::Points(32.0),
        max_width: Dimension::Points(480.0),
        max_height: Dimension::Percent(90.0),
        flex_direction: FlexDirection::RowReverse,
        justify_content: FlexJustify::SpaceBetween,
        align_items: FlexAlign::Baseline,
        flex_grow: 1.0,
        flex_shrink: 0.0,
        gap: 8.0,
        padding: EdgeInsets::symmetric(10.0, 14.0),
        margin: EdgeInsets::all(4.0),
    };
    let style = StyleProps {
        background: ColorRgba::rgba(17, 24, 39, 128),
        text_color: ColorRgba::WHITE,
        border_color: ColorRgba::rgb(56, 189, 248),
        border_width: 2.0,
        border_radius: 12.0,
        opacity: 0.9,
        font_size: 18.0,
        z_index: 5,
    };

    let mut buf = String::with_capacity(512);
    layout_to_css_into(&layout, &mut buf);
    assert_eq!(buf, layout_to_css(&layout));
    assert!(buf.contains("min-width:120px;"));
    assert!(buf.contains("min-height:32px;"));
    assert!(buf.contains("max-width:480px;"));
    assert!(buf.contains("max-height:90%;"));

    let cap_before = buf.capacity();
    buf.clear();
    style_to_css_into(&style, &mut buf);
    assert_eq!(buf, style_to_css(&style));
    assert_eq!(buf.capacity(), cap_before);
    assert!(buf.contains("background-color:rgba(17,24,39,0.502);"));
    assert!(buf.contains("border:2px solid rgba(56,189,248,1);"));
    assert!(buf.contains("z-index:5;"));
}

#[test]
fn updates_style_and_layout_incrementally() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Points(64.0),
            min_width: Dimension::Points(120.0),
            max_width: Dimension::Points(480.0),
            flex_direction: FlexDirection::Row,
            justify_content: FlexJustify::Center,
            align_items: FlexAlign::Center,
            gap: 12.0,
            padding: EdgeInsets::all(16.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(37, 99, 235),
            text_color: ColorRgba::WHITE,
            border_radius: 12.0,
            font_size: 18.0,
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let css1 = web.node(2).unwrap().combined_css();
    assert!(css1.contains("flex-direction:row;"));
    assert!(css1.contains("min-width:120px;"));
    assert!(css1.contains("max-width:480px;"));
    assert!(css1.contains("background-color:rgba(37,99,235,1);"));
    assert!(css1.contains("border-radius:12px;"));

    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            text_color: ColorRgba::rgb(56, 189, 248),
            border_radius: 20.0,
            opacity: 0.75,
            font_size: 24.0,
            ..Default::default()
        },
    );
    b2.commit(true);
    let stats = web.submit_frame(b2.finish()).unwrap();
    assert_eq!(stats.mutation_count, 1);

    let css2 = web.node(2).unwrap().combined_css();
    assert!(css2.contains("flex-direction:row;"));
    assert!(css2.contains("padding:16px 16px 16px 16px;"));
    assert!(css2.contains("background-color:rgba(15,23,42,1);"));
    assert!(css2.contains("border-radius:20px;"));
    assert!(css2.contains("opacity:0.75;"));
}

#[test]
fn removes_nodes_and_detaches_from_parent() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Text, None);
    b.set_text(2, "Keep");
    b.create_node(3, ComponentKind::Text, None);
    b.set_text(3, "Drop");
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();
    assert_eq!(web.node_count(), 3);
    assert!(web.render_html().contains("Drop</span>"));

    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.remove_node(3);
    b2.commit(true);
    web.submit_frame(b2.finish()).unwrap();

    assert_eq!(web.node_count(), 2);
    assert!(web.node(3).is_none());
    assert_eq!(web.node(ROOT_NODE_ID).unwrap().children, vec![2]);
    let html = web.render_html();
    assert!(html.contains("Keep</span>"));
    assert!(!html.contains("Drop"));
}

#[test]
fn tracks_event_masks_and_fires_render_callback() {
    let callback_hits = Rc::new(Cell::new(0u32));
    let hits_clone = Rc::clone(&callback_hits);

    let mut web = WebBackend::new().with_render_callback(move || {
        hits_clone.set(hits_clone.get() + 1);
    });

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_event_mask(2, EventMask::PRESS);
    b.create_node(3, ComponentKind::TextInput, None);
    b.set_event_mask(3, EventMask::TEXT_CHANGE | EventMask::FOCUS);
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    assert_eq!(web.event_mask_of(2), EventMask::PRESS);
    assert_eq!(
        web.event_mask_of(3),
        EventMask::TEXT_CHANGE | EventMask::FOCUS
    );

    web.push_event(NativeEvent {
        surface_id: 1,
        node_id: 2,
        timestamp_us: 42,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 20.0 },
    });
    web.push_event(NativeEvent {
        surface_id: 1,
        node_id: 3,
        timestamp_us: 99,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("hello".into()),
    });

    assert_eq!(callback_hits.get(), 2);
    let drained = web.drain_events();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].node_id, 2);
    assert_eq!(drained[1].payload, EventPayload::Text("hello".into()));
    assert!(web.drain_events().is_empty());

    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_event_mask(2, EventMask::NONE);
    b2.commit(true);
    web.submit_frame(b2.finish()).unwrap();
    assert_eq!(web.event_mask_of(2), EventMask::NONE);
}

#[test]
fn applies_incremental_patches_and_raw_props() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.create_node(3, ComponentKind::Text, None);
    b.set_text(3, "Count: 0");
    b.create_node(4, ComponentKind::TextInput, None);
    b.set_raw_prop(4, "placeholder", &RawPropValue::String("Type here".into()));
    b.set_children(2, &[3, 4]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    assert_eq!(
        web.node(4).unwrap().raw_props.get("placeholder"),
        Some(&RawPropValue::String("Type here".into()))
    );
    assert!(web.render_html().contains("Count: 0</span>"));

    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_text(3, "Count: 1");
    b2.commit(true);
    let stats = web.submit_frame(b2.finish()).unwrap();
    assert_eq!(stats.mutation_count, 1);
    assert!(web.render_html().contains("Count: 1</span>"));
    assert_eq!(
        web.node(4).unwrap().raw_props.get("placeholder"),
        Some(&RawPropValue::String("Type here".into()))
    );

    let patched = web.apply_template_patch(&nve_engine::HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "counter".into(),
        attr_updates: vec![("style".into(), "background-color: #111827".into())],
        text_updates: vec![(0, "Count: 99".into())],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    });
    assert!(patched >= 2);
    assert!(web.render_html().contains("Count: 99</span>"));
    assert!(web
        .node(2)
        .unwrap()
        .combined_css()
        .contains("background-color: #111827;"));
}

#[test]
fn targeted_template_patch_updates_only_matching_dom_mirror_nodes() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_raw_prop(2, "testID", &RawPropValue::String("nav-eng".into()));
    b.create_node(3, ComponentKind::Text, None);
    b.set_text(3, "# engineering");
    b.set_children(2, &[3]);

    b.create_node(4, ComponentKind::View, None);
    b.set_raw_prop(4, "testID", &RawPropValue::String("nav-design".into()));
    b.create_node(5, ComponentKind::Text, None);
    b.set_text(5, "# design-sync");
    b.set_children(4, &[5]);

    b.set_children(ROOT_NODE_ID, &[2, 4]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let mutations = web.apply_template_patch(&nve_engine::HotPatchMsg::TemplatePatch {
        generation: 2,
        template_id: "ui.rs".into(),
        attr_updates: vec![(
            "style".into(),
            "padding: 14px; background-color: #1d4ed8;".into(),
        )],
        text_updates: vec![(0, "# ergoiherg".into())],
        target_test_id: Some("nav-eng".into()),
        node_path: vec![0],
        text_replacements: vec![("# engineering".into(), "# ergoiherg".into())],
    });
    assert_eq!(mutations, 2);
    assert_eq!(web.node(3).unwrap().text.as_deref(), Some("# ergoiherg"));
    assert_eq!(web.node(5).unwrap().text.as_deref(), Some("# design-sync"));
    assert!(web
        .node(2)
        .unwrap()
        .combined_css()
        .contains("background-color: #1d4ed8;"));
    assert!(!web
        .node(4)
        .unwrap()
        .combined_css()
        .contains("background-color: #1d4ed8;"));

    let noop = web.apply_template_patch(&nve_engine::HotPatchMsg::TemplatePatch {
        generation: 3,
        template_id: "ui.rs".into(),
        attr_updates: vec![("opacity".into(), "0.2".into())],
        text_updates: vec![],
        target_test_id: Some("missing-id".into()),
        node_path: vec![],
        text_replacements: vec![],
    });
    assert_eq!(noop, 0);
    assert!(!web.node(2).unwrap().combined_css().contains("opacity:0.2;"));
    assert!(!web.node(4).unwrap().combined_css().contains("opacity:0.2;"));
}

#[test]
fn maps_image_resize_mode_alt_and_src_in_mirror_and_html() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    for (idx, (id, prop_key, prop_val)) in [
        (2u32, "resizeMode", "cover"),
        (3u32, "resize_mode", "contain"),
        (4u32, "objectFit", "stretch"),
        (5u32, "resizeMode", "center"),
    ]
    .into_iter()
    .enumerate()
    {
        b.create_node(id, ComponentKind::Image, None);
        b.set_raw_prop(
            id,
            if idx == 3 { "uri" } else { "src" },
            &RawPropValue::String(format!("https://example.com/img{idx}.png")),
        );
        b.set_raw_prop(
            id,
            "alt",
            &RawPropValue::String(format!("Avatar \"{idx}\"")),
        );
        b.set_raw_prop(id, prop_key, &RawPropValue::String(prop_val.into()));
    }
    b.set_children(ROOT_NODE_ID, &[2, 3, 4, 5]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    assert_eq!(web.node(2).unwrap().raw_style.object_fit, Some("cover"));
    assert_eq!(web.node(3).unwrap().raw_style.object_fit, Some("contain"));
    assert_eq!(web.node(4).unwrap().raw_style.object_fit, Some("fill"));
    assert_eq!(web.node(5).unwrap().raw_style.object_fit, Some("none"));

    let html = web.render_html();
    assert!(html.contains(
        "<img data-nve-id=\"2\" src=\"https://example.com/img0.png\" alt=\"Avatar &quot;0&quot;\""
    ));
    assert!(html.contains(
        "<img data-nve-id=\"5\" src=\"https://example.com/img3.png\" alt=\"Avatar &quot;3&quot;\""
    ));
    assert!(html.contains("object-fit:cover;"));
    assert!(html.contains("object-fit:contain;"));
    assert!(html.contains("object-fit:fill;"));
    assert!(html.contains("object-fit:none;"));

    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_raw_prop(
        2,
        "resizeMode",
        &RawPropValue::I64(ImageResizeMode::Contain.to_u8() as i64),
    );
    b2.set_raw_prop(3, "resizeMode", &RawPropValue::Null);
    b2.commit(true);
    web.submit_frame(b2.finish()).unwrap();
    assert_eq!(web.node(2).unwrap().raw_style.object_fit, Some("contain"));
    assert_eq!(web.node(3).unwrap().raw_style.object_fit, None);
}

#[test]
fn decodes_nve_draw1_canvas_ops_and_handles_corrupt_or_null_draw_lists() {
    let mut dl = DrawListBuilder::with_capacity(256);
    dl.clear(ColorRgba::rgb(17, 24, 39))
        .fill_rect(4.0, 4.0, 40.0, 20.0, ColorRgba::rgb(56, 189, 248))
        .fill_rounded_rect(50.0, 4.0, 40.0, 20.0, 6.0, ColorRgba::rgb(167, 139, 250))
        .fill_circle(110.0, 14.0, 8.0, ColorRgba::rgb(34, 197, 94))
        .stroke_line(0.0, 36.0, 120.0, 36.0, 2.0, ColorRgba::WHITE)
        .stroke_polyline(
            &[(10.0, 60.0), (40.0, 45.0), (80.0, 55.0)],
            2.5,
            ColorRgba::rgb(245, 158, 11),
        )
        .clip_rect(0.0, 0.0, 120.0, 72.0)
        .restore_clip();
    let raw_bytes = dl.finish();

    let mut spark = DrawListBuilder::with_capacity(256);
    spark.sparkline(
        &[120.0, 180.0, 150.0, 240.0],
        320.0,
        72.0,
        ColorRgba::rgb(56, 189, 248),
        ColorRgba::rgb(17, 24, 39),
    );
    let spark_attr = spark.to_attr_string();

    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::SkiaCanvas, None);
    b.set_raw_prop(2, "drawList", &RawPropValue::Bytes(raw_bytes));
    b.create_node(3, ComponentKind::SkiaCanvas, None);
    b.set_raw_prop(3, "draw_list", &RawPropValue::String(spark_attr));
    b.create_node(4, ComponentKind::SkiaCanvas, None);
    b.set_raw_prop(4, "drawList", &RawPropValue::Bytes(b"CORRUPT".to_vec()));
    b.set_children(ROOT_NODE_ID, &[2, 3, 4]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let n2 = web.node(2).unwrap();
    assert_eq!(n2.tag, "canvas");
    assert_eq!(n2.draw_op_count, 8);
    assert_eq!(n2.draw_ops.len(), 8);
    assert!(matches!(n2.draw_ops[0], CanvasOp::Clear { .. }));
    assert!(matches!(n2.draw_ops[7], CanvasOp::RestoreClip));

    let n3 = web.node(3).unwrap();
    assert!(n3.draw_op_count > 3);
    assert_eq!(web.node(4).unwrap().draw_op_count, 0);

    let html = web.render_html();
    assert!(html.contains("<canvas data-nve-id=\"2\" data-nve-draw-ops=\"8\""));

    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_raw_prop(2, "drawList", &RawPropValue::Null);
    b2.commit(true);
    web.submit_frame(b2.finish()).unwrap();
    assert_eq!(web.node(2).unwrap().draw_op_count, 0);
    assert!(web.node(2).unwrap().draw_ops.is_empty());
}

#[test]
fn maps_aria_accessibility_and_input_attributes_in_html_ssr_and_dom_mapper() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_raw_prop(
        2,
        "accessibilityLabel",
        &RawPropValue::String("Submit \"Order\" & Pay".into()),
    );
    b.set_raw_prop(2, "accessibilityRole", &RawPropValue::String("button".into()));
    b.set_raw_prop(
        2,
        "accessibilityHint",
        &RawPropValue::String("Charges saved card".into()),
    );
    b.set_raw_prop(2, "disabled", &RawPropValue::Bool(true));

    b.create_node(3, ComponentKind::TextInput, None);
    b.set_raw_prop(
        3,
        "accessibilityLabel",
        &RawPropValue::String("API Token".into()),
    );
    b.set_raw_prop(3, "secureTextEntry", &RawPropValue::Bool(true));
    b.set_raw_prop(3, "readOnly", &RawPropValue::Bool(true));

    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let html = web.render_html();
    assert!(html.contains("aria-label=\"Submit &quot;Order&quot; &amp; Pay\""));
    assert!(html.contains("role=\"button\""));
    assert!(html.contains("aria-description=\"Charges saved card\""));
    assert!(html.contains(" disabled aria-disabled=\"true\""));
    assert!(html.contains("<input data-nve-id=\"3\" aria-label=\"API Token\" type=\"password\" readonly"));

    // Clearing boolean props removes ARIA/HTML attributes cleanly.
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_raw_prop(2, "disabled", &RawPropValue::Bool(false));
    b2.set_raw_prop(3, "secureTextEntry", &RawPropValue::Bool(false));
    b2.set_raw_prop(3, "readOnly", &RawPropValue::Null);
    b2.commit(true);
    web.submit_frame(b2.finish()).unwrap();

    let html2 = web.render_html();
    assert!(!html2.contains("aria-disabled=\"true\""));
    assert!(!html2.contains("type=\"password\""));
    assert!(!html2.contains(" readonly"));

    // Direct DOM attribute mapper checks for wasm32 sync parity.
    assert_eq!(
        map_a11y_or_input_attr(
            ComponentKind::View,
            "accessibilityLabel",
            &RawPropValue::String("Palette".into())
        ),
        Some(MappedHtmlAttr::Set("aria-label", "Palette"))
    );
    assert_eq!(
        map_a11y_or_input_attr(
            ComponentKind::View,
            "accessibilityLabel",
            &RawPropValue::Null
        ),
        Some(MappedHtmlAttr::Remove("aria-label"))
    );
    assert_eq!(
        map_a11y_or_input_attr(
            ComponentKind::View,
            "accessibilityHint",
            &RawPropValue::String("Opens commands".into())
        ),
        Some(MappedHtmlAttr::Set("aria-description", "Opens commands"))
    );
    assert_eq!(
        map_a11y_or_input_attr(ComponentKind::TextInput, "secureTextEntry", &RawPropValue::Bool(true)),
        Some(MappedHtmlAttr::Set("type", "password"))
    );
    // A non-secure input and any non-input element carry no `type`, like render_html and SSR.
    assert_eq!(
        map_a11y_or_input_attr(ComponentKind::TextInput, "secureTextEntry", &RawPropValue::Bool(false)),
        Some(MappedHtmlAttr::Remove("type"))
    );
    assert_eq!(
        map_a11y_or_input_attr(ComponentKind::View, "secureTextEntry", &RawPropValue::Bool(true)),
        Some(MappedHtmlAttr::Remove("type"))
    );
    assert_eq!(
        map_a11y_or_input_attr(ComponentKind::TextInput, "readOnly", &RawPropValue::String("true".into())),
        Some(MappedHtmlAttr::ReadOnly(true))
    );
    assert_eq!(
        map_a11y_or_input_attr(ComponentKind::Pressable, "disabled", &RawPropValue::String("false".into())),
        Some(MappedHtmlAttr::Disabled(false))
    );
}

#[test]
fn ime_composition_guard_suppresses_preedit_input_and_commit_enter() {
    let guard = ImeCompositionGuard::new();
    assert!(!guard.is_composing());
    assert!(!guard.should_suppress_input());
    assert!(!guard.should_suppress_keydown(false, 13));

    // During CJK IME composition, pre-edit input and Enter keydown are suppressed.
    guard.on_composition_start();
    assert!(guard.is_composing());
    assert!(guard.should_suppress_input());
    assert!(guard.should_suppress_keydown(false, 13));
    assert!(guard.should_suppress_keydown(true, 13));

    // Ending composition resumes normal input and Enter submit handling.
    guard.on_composition_end();
    assert!(!guard.is_composing());
    assert!(!guard.should_suppress_input());
    assert!(!guard.should_suppress_keydown(false, 13));

    // WebKit/Safari `keyCode == 229` or `ev.is_composing()` still suppresses Enter even after flag flips.
    assert!(guard.should_suppress_keydown(true, 13));
    assert!(guard.should_suppress_keydown(false, IME_PROCESS_KEY_CODE));
}

#[test]
fn encodes_binary_nk1_keydown_payload_and_prunes_orphans_on_full_rebuild() {
    let payload = encode_keydown_payload("k", true, false, false, false);
    assert_eq!(&payload[..3], &KEYDOWN_WIRE_MAGIC);
    assert_eq!(
        decode_keydown_bytes(&payload),
        Some(DecodedKeyDown {
            key: "k".into(),
            meta: true,
            ctrl: false,
            shift: false,
            alt: false,
        })
    );
    let special = encode_keydown_payload("\"\\", false, true, true, false);
    assert_eq!(
        decode_keydown_bytes(&special),
        Some(DecodedKeyDown {
            key: "\"\\".into(),
            meta: false,
            ctrl: true,
            shift: true,
            alt: false,
        })
    );

    let mut web = WebBackend::new();
    let mut b1 = CommandBufferBuilder::new(1, 1);
    b1.create_node(2, ComponentKind::View, None);
    b1.create_node(3, ComponentKind::Text, None);
    b1.create_node(4, ComponentKind::Text, None);
    b1.set_children(2, &[3, 4]);
    b1.set_children(ROOT_NODE_ID, &[2]);
    b1.commit(true);
    web.submit_frame(b1.finish()).unwrap();
    assert_eq!(web.node_count(), 4);

    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.create_node(2, ComponentKind::View, None);
    b2.create_node(3, ComponentKind::Text, None);
    b2.set_text(3, "Mobile");
    b2.set_children(2, &[3]);
    b2.set_children(ROOT_NODE_ID, &[2]);
    b2.commit(true);
    web.submit_frame(b2.finish()).unwrap();

    assert_eq!(web.node_count(), 3);
    assert!(web.node(4).is_none());
    assert_eq!(web.node(3).unwrap().text.as_deref(), Some("Mobile"));
}
