//! Deep tests proving zero-serialization in-process `DrawList` and `NativeView` paths through
//! `DioxusSurface`, plus cross-language FFI binary `NVE_DRAW1` / `NVNP` mode preservation.

use nve_abi::{ColorRgba, Command, CommandDecoder, RawPropValue};
use nve_components::canvas_ir::{
    CanvasOp, DrawList, DrawListDecoder, DrawListRecorder, NVE_DRAW_MAGIC,
};
use nve_components::native_view::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, NATIVE_PROPS_MAGIC, NATIVE_VIEW_PROPS_PROP_KEY,
    NativeViewSpec, VideoPlayer, VideoPlayerCommand, VideoPlayerProps, WebView, WebViewCommand,
    WebViewProps,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;

fn sample_draw_list() -> DrawList {
    let mut rec = DrawListRecorder::with_capacity(3);
    rec.clear(ColorRgba::rgb(15, 23, 42))
        .fill_rounded_rect(4.0, 6.0, 120.0, 48.0, 8.0, ColorRgba::rgb(56, 189, 248))
        .fill_circle(64.0, 30.0, 10.0, ColorRgba::WHITE);
    rec.finish()
}

#[test]
fn inprocess_canvas_draw_list_skips_command_buffer_serialization_and_preserves_arc_identity() {
    thread_local! {
        static SHARED_DL: DrawList = sample_draw_list();
    }

    fn canvas_app() -> Element {
        let dl = SHARED_DL.with(Clone::clone);
        rsx! {
            view {
                canvas {
                    style: "width: 160px; height: 64px;",
                    draw_list: CanvasDrawList(dl),
                }
            }
        }
    }

    let expected = SHARED_DL.with(Clone::clone);
    let mut inproc_surface = DioxusSurface::new(1, canvas_app).with_in_process_mode(true);
    let frame = inproc_surface.render_frame_slice(1);
    let (_, cmds) = CommandDecoder::new(frame)
        .expect("valid header")
        .decode_all()
        .expect("valid commands");

    // In-process mode MUST NOT serialize any `drawList` bytes or base64 strings into CommandBuffer.
    let draw_list_props: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SetRawProp { key, .. } if key == "drawList"))
        .collect();
    assert!(
        draw_list_props.is_empty(),
        "in-process mode must emit 0 serialized drawList props, got {draw_list_props:?}"
    );

    // Direct zero-copy DrawList is exposed with exact Arc pointer identity.
    let synced: Vec<_> = inproc_surface.iter_draw_lists().collect();
    assert_eq!(synced.len(), 1);
    assert!(synced[0].1.ptr_eq(&expected));
    assert_eq!(&synced[0].1[..], &expected[..]);

    // FFI mode (default `in_process_mode == false`) emits raw NVE_DRAW1 bytes (still ZERO base64!).
    let mut ffi_surface = DioxusSurface::new(1, canvas_app);
    let ffi_frame = ffi_surface.render_frame_slice(1);
    let (_, ffi_cmds) = CommandDecoder::new(ffi_frame)
        .expect("valid ffi header")
        .decode_all()
        .expect("valid ffi commands");
    let raw_bytes = ffi_cmds
        .iter()
        .find_map(|c| match c {
            Command::SetRawProp {
                key,
                value: RawPropValue::Bytes(b),
                ..
            } if key == "drawList" => Some(b.as_slice()),
            _ => None,
        })
        .expect("FFI mode emits binary NVE_DRAW1 RawPropValue::Bytes (no base64)");
    assert!(raw_bytes.starts_with(&NVE_DRAW_MAGIC));
    assert_eq!(DrawListDecoder::decode(raw_bytes).unwrap(), &expected[..]);
}

#[test]
fn inprocess_native_view_skips_nvnp_and_queues_zero_serialization_commands() {
    fn demo_app() -> Element {
        use_provide_native_view_registry(|| {
            let mut reg = NativeViewRegistry::new();
            reg.register(BlurSurface::KIND)
                .register(WebView::KIND)
                .register(VideoPlayer::KIND);
            reg
        });
        let web_handle = use_native_view_handle::<WebView>();
        let video_handle = use_native_view_handle::<VideoPlayer>();
        let _ = web_handle.dispatch(WebViewCommand::Navigate("https://nve.dev/live".into()));
        let _ = video_handle.dispatch(VideoPlayerCommand::SetVolume(0.5));
        rsx! {
            view {
                NativeView::<BlurSurface> {
                    props: BlurSurfaceProps {
                        material: BlurMaterial::Chrome,
                        intensity: 0.85,
                        tint: Some(0x0F172ACC),
                    },
                }
                NativeView::<WebView> {
                    props: WebViewProps::new("https://nve.dev"),
                    handle: web_handle,
                }
                NativeView::<VideoPlayer> {
                    props: VideoPlayerProps {
                        source_uri: "https://cdn.nve.dev/clip.mp4".into(),
                        autoplay: false,
                        muted: true,
                        looping: false,
                        controls: true,
                    },
                    handle: video_handle,
                }
            }
        }
    }

    // 1. In-process mode: 0 NVNP bytes in CommandBuffer, direct typed props via iter_native_views().
    let mut inproc = DioxusSurface::new(1, demo_app).with_in_process_mode(true);
    let frame = inproc.render_frame_slice(1);
    let (_, cmds) = CommandDecoder::new(frame).unwrap().decode_all().unwrap();
    let nvnp_count = cmds
        .iter()
        .filter(|c| {
            matches!(c, Command::SetRawProp { key, .. } if key == NATIVE_VIEW_PROPS_PROP_KEY)
        })
        .count();
    assert_eq!(nvnp_count, 0, "in-process mode must emit 0 NVNP binary props");

    let typed_views: Vec<_> = inproc.iter_native_views().collect();
    assert_eq!(typed_views.len(), 3);
    assert!(typed_views.iter().any(|(_, p)| {
        p.kind() == BlurSurface::KIND
            && p.downcast_ref::<BlurSurfaceProps>().map(|b| b.material)
                == Some(BlurMaterial::Chrome)
    }));
    assert!(typed_views.iter().any(|(_, p)| {
        p.kind() == WebView::KIND
            && p.downcast_ref::<WebViewProps>().map(|w| w.url.as_str()) == Some("https://nve.dev")
    }));
    assert!(typed_views.iter().any(|(_, p)| {
        p.kind() == VideoPlayer::KIND
            && p.downcast_ref::<VideoPlayerProps>().map(|v| v.controls) == Some(true)
    }));

    // 2. FFI mode: emits valid canonical NVNP binary bytes for cross-language hosts.
    let mut ffi = DioxusSurface::new(1, demo_app);
    let ffi_frame = ffi.render_frame_slice(1);
    let (_, ffi_cmds) = CommandDecoder::new(ffi_frame).unwrap().decode_all().unwrap();
    let nvnp_payloads: Vec<&[u8]> = ffi_cmds
        .iter()
        .filter_map(|c| match c {
            Command::SetRawProp {
                key,
                value: RawPropValue::Bytes(b),
                ..
            } if key == NATIVE_VIEW_PROPS_PROP_KEY => Some(b.as_slice()),
            _ => None,
        })
        .collect();
    assert_eq!(nvnp_payloads.len(), 3);
    for bytes in nvnp_payloads {
        assert!(bytes.starts_with(&NATIVE_PROPS_MAGIC));
    }

    let mut empty_rec = DrawListRecorder::new();
    empty_rec.clear(ColorRgba::BLACK);
    let dl = empty_rec.finish();
    assert_eq!(dl[0], CanvasOp::Clear { color: ColorRgba::BLACK });
}
