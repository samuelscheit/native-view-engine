//! Native view adapter tests (`BlurSurface`, `WebView`, `VideoPlayer`, `WebElement`, `CameraPreview`).

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    any(
        all(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
        ),
        feature = "camera",
    )
))]
use nve_abi::{CommandBufferBuilder, ComponentKind, RawPropValue, ROOT_NODE_ID};
#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    any(
        all(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
        ),
        feature = "camera",
    )
))]
use nve_engine::RenderBackend;

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    any(
        all(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
        ),
        feature = "camera",
    )
))]
use crate::WebBackend;

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    any(
        all(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
        ),
        feature = "camera",
    )
))]
fn nvnp_encode(fields: &[(u16, crate::NativeValueRef<'_>)]) -> Vec<u8> {
    let mut enc = nve_components::native_view::NativePropEncoder::new();
    for &(id, val) in fields {
        enc.field(id, val).unwrap();
    }
    enc.finish()
}

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    any(
        all(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
        ),
        feature = "camera",
    )
))]
fn nvnc_encode(opcode: u16, args: &[crate::NativeValueRef<'_>]) -> Vec<u8> {
    let mut enc = nve_components::native_view::NativeCommandEncoder::new(opcode);
    for &val in args {
        enc.arg(val).unwrap();
    }
    enc.finish()
}

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
))]
fn pack_web_attrs(attrs: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(k, v) in attrs {
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v.as_bytes());
    }
    out
}

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
))]
#[test]
fn native_view_web_element_blur_webview_and_video_ssr_hydration_and_commands() {
    use crate::native_view::{
        is_safe_web_attr, is_safe_web_url, validate_web_element_tag, NATIVE_VIEW_KIND_PROP_KEY,
        NATIVE_VIEW_PROPS_PROP_KEY,
    };
    use crate::{BlurMaterial, NativeValueRef};

    // 1. Security allowlist & sanitization checks.
    for ok_tag in [
        "video", "audio", "svg", "iframe", "canvas", "input", "select", "textarea", "progress",
        "meter", "details", "summary", "my-chart", "x-video-hud",
    ] {
        assert!(validate_web_element_tag(ok_tag).is_some(), "expected {ok_tag} allowed");
    }
    for bad_tag in ["script", "style", "object", "embed", "My-Chart", "bad-", "font-face", "div"] {
        assert!(validate_web_element_tag(bad_tag).is_none(), "expected {bad_tag} rejected");
    }
    assert!(!is_safe_web_url("javascript:alert(1)"));
    assert!(!is_safe_web_url("  JaVa\tScRiPt:alert(1)"));
    assert!(!is_safe_web_url("data:text/html,<script>alert(1)</script>"));
    assert!(is_safe_web_url("https://nve.dev/docs"));
    assert!(is_safe_web_url("/assets/intro.mp4"));
    assert!(!is_safe_web_attr("onclick", "alert(1)"));
    assert!(!is_safe_web_attr("ONERROR", "alert(1)"));
    assert!(!is_safe_web_attr("srcdoc", "<script>alert(1)</script>"));
    assert!(!is_safe_web_attr("src", "javascript:alert(1)"));
    assert!(is_safe_web_attr("viewBox", "0 0 24 24"));

    // 2. Build a single frame containing BlurSurface, WebView, VideoPlayer, allowlisted WebElement,
    //    custom Web Component, and an adversarial <script> attempt.
    let blur_bytes = nvnp_encode(&[
        (1, NativeValueRef::U32(BlurMaterial::Thick as u32)),
        (2, NativeValueRef::F32(0.5)),
        (3, NativeValueRef::U32(0x10203080)),
    ]);
    let webview_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("https://nve.dev/embed")),
        (2, NativeValueRef::Bool(true)),
    ]);
    let video_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("https://cdn.nve.dev/hero.mp4")),
        (2, NativeValueRef::Bool(false)),
        (3, NativeValueRef::Bool(true)),
        (4, NativeValueRef::Bool(true)),
        (5, NativeValueRef::Bool(true)),
    ]);
    let svg_attrs = pack_web_attrs(&[
        ("viewBox", "0 0 32 32"),
        ("onclick", "alert(1)"),
        ("aria-hidden", "true"),
    ]);
    let svg_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("svg")),
        (2, NativeValueRef::U32(3)),
        (3, NativeValueRef::Bytes(&svg_attrs)),
    ]);
    let custom_attrs = pack_web_attrs(&[("data-mode", "pro"), ("srcdoc", "<b>bad</b>")]);
    let custom_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("my-chart")),
        (2, NativeValueRef::U32(2)),
        (3, NativeValueRef::Bytes(&custom_attrs)),
    ]);
    let bad_script_attrs = pack_web_attrs(&[("src", "https://evil.test/x.js")]);
    let bad_script_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("script")),
        (2, NativeValueRef::U32(1)),
        (3, NativeValueRef::Bytes(&bad_script_attrs)),
    ]);

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Custom, Some("nve.blur_surface"));
    b.set_raw_prop(2, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("nve.blur_surface".into()));
    b.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(blur_bytes));

    b.create_node(3, ComponentKind::Custom, Some("nve.web_view"));
    b.set_raw_prop(3, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("nve.web_view".into()));
    b.set_raw_prop(3, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(webview_bytes));

    b.create_node(4, ComponentKind::Custom, Some("nve.video_player"));
    b.set_raw_prop(4, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("nve.video_player".into()));
    b.set_raw_prop(4, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(video_bytes));

    b.create_node(5, ComponentKind::Custom, Some("web.svg"));
    b.set_raw_prop(5, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("web.svg".into()));
    b.set_raw_prop(5, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(svg_bytes));

    b.create_node(6, ComponentKind::Custom, Some("web.my-chart"));
    b.set_raw_prop(6, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("web.my-chart".into()));
    b.set_raw_prop(6, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(custom_bytes));

    b.create_node(7, ComponentKind::Custom, Some("web.script"));
    b.set_raw_prop(7, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("web.script".into()));
    b.set_raw_prop(7, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(bad_script_bytes));

    b.set_children(ROOT_NODE_ID, &[2, 3, 4, 5, 6, 7]);
    b.commit(true);
    let frame0 = b.finish().to_vec();

    let mut backend = WebBackend::new();
    backend.submit_frame(&frame0).unwrap();
    let ssr = backend.render_ssr();
    let mirror_html = backend.render_html();

    for html in [&ssr.html, &mirror_html] {
        // BlurSurface emits backdrop-filter + -webkit-backdrop-filter + tint
        assert!(html.contains("backdrop-filter:blur(16px);-webkit-backdrop-filter:blur(16px);"));
        assert!(html.contains("background-color:rgba(16,32,48,0.502);"));
        // WebView emits sandboxed <iframe>
        assert!(html.contains("<iframe data-nve-id=\"3\""));
        assert!(html.contains("sandbox=\"allow-scripts allow-same-origin allow-forms allow-popups\""));
        assert!(html.contains("src=\"https://nve.dev/embed\""));
        // VideoPlayer emits <video> with muted/loop/controls
        assert!(html.contains("<video data-nve-id=\"4\""));
        assert!(html.contains("src=\"https://cdn.nve.dev/hero.mp4\""));
        assert!(html.contains(" muted loop controls playsinline"));
        // WebElement <svg> preserves safe attrs and strips onclick
        assert!(html.contains("<svg data-nve-id=\"5\""));
        assert!(html.contains("viewBox=\"0 0 32 32\""));
        assert!(html.contains("aria-hidden=\"true\""));
        assert!(!html.contains("onclick"));
        // Custom element <my-chart> mounted and srcdoc stripped
        assert!(html.contains("<my-chart data-nve-id=\"6\""));
        assert!(html.contains("data-mode=\"pro\""));
        assert!(!html.contains("srcdoc"));
        // <script> rejected and downgraded to <div> without src
        assert!(!html.contains("<script"));
        assert!(html.contains("<div data-nve-id=\"7\""));
        assert!(!html.contains("evil.test"));
    }

    // 3. Zero-flicker SSR hydration adopts built-in native views (blur <div>, <iframe>, <video>, <svg>).
    let mut b_hyd = CommandBufferBuilder::new(1, 1);
    b_hyd.create_node(2, ComponentKind::Custom, Some("nve.blur_surface"));
    b_hyd.create_node(3, ComponentKind::Custom, Some("nve.web_view"));
    b_hyd.create_node(4, ComponentKind::Custom, Some("nve.video_player"));
    b_hyd.create_node(5, ComponentKind::Custom, Some("web.svg"));
    b_hyd.set_children(ROOT_NODE_ID, &[2, 3, 4, 5]);
    b_hyd.commit(true);
    let hyd_frame = b_hyd.finish().to_vec();
    let mut srv_hyd = WebBackend::new();
    srv_hyd.submit_frame(&hyd_frame).unwrap();
    let ssr_hyd = srv_hyd.render_ssr();
    let mut cli_hyd = WebBackend::hydrate_from_ssr(&ssr_hyd);
    cli_hyd.submit_frame(&hyd_frame).unwrap();
    assert!(cli_hyd.hydration_report().mismatches.is_empty());
    assert_eq!(cli_hyd.hydration_report().adopted_nodes, 5);

    // 4. Imperative NVNC commands via Command::DispatchCommand (WebView + VideoPlayer).
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.dispatch_command(3, "navigate", &nvnc_encode(1, &[NativeValueRef::Str("javascript:alert(1)")]));
    b2.dispatch_command(3, "navigate", &nvnc_encode(1, &[NativeValueRef::Str("https://nve.dev/next")]));
    b2.dispatch_command(3, "reload", &nvnc_encode(2, &[]));
    b2.dispatch_command(3, "go_back", &nvnc_encode(3, &[]));
    b2.dispatch_command(3, "go_forward", &nvnc_encode(4, &[]));
    b2.dispatch_command(4, "play", &nvnc_encode(1, &[]));
    b2.dispatch_command(4, "seek_ms", &nvnc_encode(3, &[NativeValueRef::U64(12_345)]));
    b2.dispatch_command(4, "set_volume", &nvnc_encode(4, &[NativeValueRef::F32(0.42)]));
    b2.dispatch_command(4, "pause", &nvnc_encode(2, &[]));
    b2.commit(false);
    backend.submit_frame(b2.finish()).unwrap();

    let wv = backend.native_view(3).unwrap();
    assert_eq!(wv.current_url.as_deref(), Some("https://nve.dev/next"));
    assert_eq!(wv.reload_count, 1);
    assert_eq!(wv.go_back_count, 1);
    assert_eq!(wv.go_forward_count, 1);

    let vp = backend.native_view(4).unwrap();
    assert!(!vp.playing);
    assert_eq!(vp.position_ms, 12_345);
    assert!((vp.volume - 0.42).abs() < 1e-6);

    // 5. Shorthand aliases ("nve.blur", "nve.webview", "nve.video", "nve.native.kind") + corrupt buffer resilience.
    let mut b3 = CommandBufferBuilder::new(1, 3);
    b3.create_node(10, ComponentKind::Custom, None);
    b3.set_raw_prop(10, "nve.native.kind", &RawPropValue::String("nve.blur".into()));
    b3.set_raw_prop(10, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(vec![0xFF, 0x00]));
    b3.dispatch_command(3, "navigate", &[0x00, 0x01, 0x02]);
    b3.commit(true);
    backend.submit_frame(b3.finish()).unwrap();
    assert_eq!(backend.native_view(10).unwrap().resolved_tag, "div");
    assert!(backend.native_view(10).unwrap().extra_css.contains("backdrop-filter:blur("));
    assert_eq!(backend.native_view(3).unwrap().current_url.as_deref(), Some("https://nve.dev/next"));
}

#[cfg(all(feature = "ssr", feature = "hydrate", feature = "camera"))]
#[test]
fn native_view_camera_preview_ssr_hydration_commands_and_corrupt_resilience() {
    use crate::native_view::{
        decode_camera_preview_command, decode_camera_preview_props, CameraFacing,
        CameraPreviewCommand, NativeViewCodecError, StabilizationMode, TorchMode,
        KIND_CAMERA_ALIAS, KIND_CAMERA_PREVIEW, NATIVE_VIEW_KIND_PROP_KEY,
        NATIVE_VIEW_PROPS_PROP_KEY,
    };
    use crate::NativeValueRef;

    let encode_cam_props = |dev: Option<&str>,
                            facing: u32,
                            torch: u32,
                            zoom: f32,
                            fps: u32,
                            hdr: bool,
                            stab: u32,
                            fp: bool,
                            audio: bool,
                            active: bool| {
        nvnp_encode(&[
            (1, dev.map_or(NativeValueRef::Null, NativeValueRef::Str)),
            (2, NativeValueRef::U32(facing)),
            (3, NativeValueRef::U32(torch)),
            (4, NativeValueRef::F32(zoom)),
            (5, NativeValueRef::U32(fps)),
            (6, NativeValueRef::Bool(hdr)),
            (7, NativeValueRef::U32(stab)),
            (8, NativeValueRef::Bool(fp)),
            (9, NativeValueRef::Bool(audio)),
            (10, NativeValueRef::Bool(active)),
        ])
    };

    // 1. SSR + mirror HTML emission for `"nve.camera.preview"` with full NVNP fields.
    let props1 = encode_cam_props(Some("usb-cam-1"), 2, 1, 2.5, 60, true, 2, true, true, true);
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Custom, Some(KIND_CAMERA_PREVIEW));
    b.set_raw_prop(2, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String(KIND_CAMERA_PREVIEW.into()));
    b.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(props1));
    b.create_node(3, ComponentKind::Custom, Some(KIND_CAMERA_ALIAS));
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    let frame0 = b.finish().to_vec();

    let mut backend = WebBackend::new();
    backend.submit_frame(&frame0).unwrap();
    let ssr = backend.render_ssr();
    let mirror = backend.render_html();

    for html in [&ssr.html, &mirror] {
        assert!(html.contains("<video data-nve-id=\"2\""));
        assert!(html.contains("data-nve-native-kind=\"nve.camera.preview\""));
        assert!(html.contains(" autoplay playsinline muted data-nve-camera=\"1\""));
        assert!(html.contains("data-nve-camera-facing=\"front\""));
        assert!(html.contains("data-nve-camera-torch=\"on\""));
        assert!(html.contains("data-nve-camera-zoom=\"2.5\""));
        assert!(html.contains("data-nve-camera-fps=\"60\""));
        assert!(html.contains("data-nve-camera-active=\"1\""));
        assert!(html.contains("data-nve-camera-audio=\"1\""));
        assert!(html.contains("data-nve-camera-device=\"usb-cam-1\""));
        // Default alias node without NVNP still renders `<video autoplay playsinline muted data-nve-camera="1">`.
        assert!(html.contains("<video data-nve-id=\"3\""));
        assert!(html.contains("data-nve-camera-facing=\"back\""));
    }

    // 2. Zero-flicker SSR hydration adopts camera `<video>` nodes with 0 mismatches & exact DOM identity.
    let mut cli = WebBackend::hydrate_from_ssr(&ssr);
    let cam_id_before = cli.native_dom_instance_id(2).unwrap();
    let alias_id_before = cli.native_dom_instance_id(3).unwrap();
    cli.submit_frame(&frame0).unwrap();
    assert!(cli.hydration_report().mismatches.is_empty());
    assert_eq!(cli.hydration_report().adopted_nodes, 3);
    assert_eq!(cli.native_dom_instance_id(2), Some(cam_id_before));
    assert_eq!(cli.native_dom_instance_id(3), Some(alias_id_before));

    // 3. Imperative NVNC commands (opcodes 1..=7) update state & reflected HTML attributes.
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.dispatch_command(2, "switch_facing", &nvnc_encode(1, &[NativeValueRef::U32(3)]));
    b2.dispatch_command(2, "set_torch", &nvnc_encode(2, &[NativeValueRef::U32(2)]));
    b2.dispatch_command(2, "set_zoom", &nvnc_encode(3, &[NativeValueRef::F32(3.25)]));
    b2.dispatch_command(2, "focus_point", &nvnc_encode(4, &[NativeValueRef::F32(0.25), NativeValueRef::F32(0.75)]));
    b2.dispatch_command(2, "capture_photo", &nvnc_encode(5, &[NativeValueRef::U32(88), NativeValueRef::U32(2)]));
    b2.dispatch_command(2, "start_recording", &nvnc_encode(6, &[NativeValueRef::Str("/tmp/web.webm")]));
    b2.commit(false);
    backend.submit_frame(b2.finish()).unwrap();

    let cam = &backend.native_view(2).unwrap().camera;
    assert_eq!(cam.facing, CameraFacing::External);
    assert_eq!(cam.torch, TorchMode::Auto);
    assert!((cam.zoom - 3.25).abs() < 1e-6);
    assert_eq!(cam.last_focus_point, Some((0.25, 0.75)));
    assert_eq!(cam.photo_capture_count, 1);
    assert_eq!(cam.last_photo_quality, Some(88));
    assert_eq!(cam.last_photo_flash, Some(2));
    assert!(cam.recording);
    assert_eq!(cam.last_recording_path.as_deref(), Some("/tmp/web.webm"));
    let ssr_rec = backend.render_ssr();
    assert!(ssr_rec.html.contains("data-nve-camera-facing=\"external\""));
    assert!(ssr_rec.html.contains("data-nve-camera-torch=\"auto\""));
    assert!(ssr_rec.html.contains("data-nve-camera-zoom=\"3.25\""));
    assert!(ssr_rec.html.contains("data-nve-camera-recording=\"1\""));

    // StopRecording + deactivate via NVNP prop update.
    let props_inactive = encode_cam_props(None, 1, 0, 1.0, 30, false, 0, false, false, false);
    let mut b3 = CommandBufferBuilder::new(1, 3);
    b3.dispatch_command(2, "stop_recording", &nvnc_encode(7, &[]));
    b3.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(props_inactive));
    b3.commit(true);
    backend.submit_frame(b3.finish()).unwrap();
    let cam_off = &backend.native_view(2).unwrap().camera;
    assert!(!cam_off.recording);
    assert!(!cam_off.active);
    assert_eq!(cam_off.device_id, None);
    assert_eq!(cam_off.stabilization, StabilizationMode::Off);
    let ssr_off = backend.render_ssr();
    assert!(ssr_off.html.contains("data-nve-camera-active=\"0\""));
    assert!(!ssr_off.html.contains("data-nve-camera-recording="));
    assert!(!ssr_off.html.contains("data-nve-camera-device="));

    // 4. Corrupt buffer, out-of-range enum, and invalid numeric argument resilience.
    let bad_facing = encode_cam_props(None, 0, 0, 1.0, 30, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&bad_facing),
        Err(NativeViewCodecError::InvalidEnum("CameraFacing", 0))
    ));
    let bad_torch = encode_cam_props(None, 1, 3, 1.0, 30, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&bad_torch),
        Err(NativeViewCodecError::InvalidEnum("TorchMode", 3))
    ));
    let bad_zoom = encode_cam_props(None, 1, 0, f32::NAN, 30, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&bad_zoom),
        Err(NativeViewCodecError::TypeMismatch { .. })
    ));
    let bad_fps = encode_cam_props(None, 1, 0, 1.0, 241, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&bad_fps),
        Err(NativeViewCodecError::InvalidEnum("CameraFps", 241))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(3, &[NativeValueRef::F32(-1.0)])),
        Err(NativeViewCodecError::TypeMismatch { .. })
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(4, &[NativeValueRef::F32(1.5), NativeValueRef::F32(0.5)])),
        Err(NativeViewCodecError::TypeMismatch { .. })
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(5, &[NativeValueRef::U32(101), NativeValueRef::U32(0)])),
        Err(NativeViewCodecError::InvalidEnum("PhotoQuality", 101))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(5, &[NativeValueRef::U32(90), NativeValueRef::U32(3)])),
        Err(NativeViewCodecError::InvalidEnum("FlashMode", 3))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(99, &[])),
        Err(NativeViewCodecError::UnknownCommandOpcode(99))
    ));
    assert_eq!(
        decode_camera_preview_command(&nvnc_encode(7, &[])).unwrap(),
        CameraPreviewCommand::StopRecording
    );

    // Submitting corrupt NVNP / NVNC frames does not panic or overwrite valid state.
    let mut b4 = CommandBufferBuilder::new(1, 4);
    b4.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(vec![0xDE, 0xAD]));
    b4.dispatch_command(2, "set_zoom", &[0x00, 0x01, 0x02]);
    b4.dispatch_command(2, "switch_facing", &nvnc_encode(1, &[NativeValueRef::U32(99)]));
    b4.commit(false);
    backend.submit_frame(b4.finish()).unwrap();
    assert_eq!(backend.native_view(2).unwrap().camera.facing, CameraFacing::Back);
    assert!((backend.native_view(2).unwrap().camera.zoom - 1.0).abs() < 1e-6);
}

#[cfg(all(
    feature = "ssr",
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
))]
#[test]
fn inprocess_zero_serialization_native_view_props_and_commands_bypass_nvnp_nvnc() {
    use nve_components::native_view::{
        BlurMaterial, BlurSurface, BlurSurfaceProps, EncodeNativeCommand, NativeViewSpec,
        TypedNativeViewCommand, TypedNativeViewProps, VideoPlayer, VideoPlayerCommand,
        VideoPlayerProps, WebElement, WebElementProps, WebView, WebViewCommand, WebViewProps,
    };

    // Mount Custom nodes with ZERO `nve.native_props` bytes in CommandBuffer.
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(1, ComponentKind::View, None);
    b.create_node(2, ComponentKind::Custom, Some(BlurSurface::KIND));
    b.create_node(3, ComponentKind::Custom, Some(WebView::KIND));
    b.create_node(4, ComponentKind::Custom, Some(VideoPlayer::KIND));
    b.create_node(5, ComponentKind::Custom, Some("web.details"));
    b.set_children(1, &[2, 3, 4, 5]);
    b.commit(true);

    let mut backend = WebBackend::new();
    backend.submit_frame(b.finish()).unwrap();

    let blur_typed = TypedNativeViewProps::from_spec::<BlurSurface>(BlurSurfaceProps {
        material: BlurMaterial::Chrome,
        intensity: 0.75,
        tint: Some(0x11223380),
    })
    .unwrap();
    let webview_typed =
        TypedNativeViewProps::from_spec::<WebView>(WebViewProps::new("https://nve.dev/docs"))
            .unwrap();
    let video_typed = TypedNativeViewProps::from_spec::<VideoPlayer>(VideoPlayerProps {
        source_uri: "https://cdn.nve.dev/demo.mp4".into(),
        autoplay: false,
        muted: true,
        looping: true,
        controls: true,
    })
    .unwrap();
    let web_el_typed = TypedNativeViewProps::from_spec::<WebElement>(
        WebElementProps::new("details").with_attr("aria-label", "DirectTyped"),
    )
    .unwrap();

    backend.set_native_view_props(2, &blur_typed);
    backend.set_native_view_props(3, &webview_typed);
    backend.set_native_view_props(4, &video_typed);
    backend.set_native_view_props(5, &web_el_typed);

    let html = backend.render_ssr().html;
    assert!(html.contains("<iframe data-nve-id=\"3\""));
    assert!(html.contains("src=\"https://nve.dev/docs\""));
    assert!(html.contains("<video data-nve-id=\"4\""));
    assert!(html.contains("<details data-nve-id=\"5\""));
    assert!(html.contains("aria-label=\"DirectTyped\""));
    assert!(backend.native_view(2).unwrap().extra_css.contains("backdrop-filter:blur("));

    // Direct typed commands (0 NVNC encoding).
    let nav_cmd = TypedNativeViewCommand::from_spec::<WebView>(WebViewCommand::Navigate(
        "https://nve.dev/benchmarks".into(),
    ))
    .unwrap();
    assert!(backend.dispatch_typed_native_command(3, &nav_cmd));
    assert_eq!(
        backend.native_view(3).unwrap().current_url.as_deref(),
        Some("https://nve.dev/benchmarks")
    );

    let play_cmd = VideoPlayerCommand::Play.to_native_command().unwrap();
    assert!(backend.dispatch_native_command(4, &play_cmd));
    assert!(backend.native_view(4).unwrap().playing);

    let vol_cmd = TypedNativeViewCommand::from_spec::<VideoPlayer>(VideoPlayerCommand::SetVolume(
        0.4,
    ))
    .unwrap();
    assert!(backend.dispatch_typed_native_command(4, &vol_cmd));
    assert!((backend.native_view(4).unwrap().volume - 0.4).abs() < 1e-6);

    #[cfg(feature = "camera")]
    {
        use nve_camera::{CameraFacing, CameraPreview, CameraPreviewCommand, CameraPreviewProps};

        let mut b_cam = CommandBufferBuilder::new(1, 2);
        b_cam.create_node(6, ComponentKind::Custom, Some(CameraPreview::KIND));
        b_cam.set_children(1, &[2, 3, 4, 5, 6]);
        b_cam.commit(false);
        backend.submit_frame(b_cam.finish()).unwrap();

        let cam_props = TypedNativeViewProps::from_spec::<CameraPreview>(
            CameraPreviewProps::new()
                .with_facing(CameraFacing::Front)
                .with_zoom(2.0),
        )
        .unwrap();
        backend.set_native_view_props(6, &cam_props);
        assert_eq!(backend.native_view(6).unwrap().camera.facing, CameraFacing::Front);
        assert!((backend.native_view(6).unwrap().camera.zoom - 2.0).abs() < 1e-6);

        let zoom_cmd = TypedNativeViewCommand::from_spec::<CameraPreview>(
            CameraPreviewCommand::SetZoom(3.5),
        )
        .unwrap();
        assert!(backend.dispatch_typed_native_command(6, &zoom_cmd));
        assert!((backend.native_view(6).unwrap().camera.zoom - 3.5).abs() < 1e-6);
    }
}
