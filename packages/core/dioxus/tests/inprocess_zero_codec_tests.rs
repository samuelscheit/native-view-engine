//! Deep tests proving zero-serialization in-process `flush_into<S: RenderSink>` for `DrawList`
//! and `NativeView`, plus canonical cross-boundary `BatchSink` binary `NVE_DRAW1` / `NVNP` output.

use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, FrameId, LayoutProps, NodeId, RawPropValue,
    RawPropValueRef, StyleProps, SurfaceId,
};
use nve_components::canvas_ir::{
    CanvasOp, DrawList, DrawListDecoder, DrawListRecorder, NVE_DRAW_MAGIC,
};
use nve_components::native_view::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, NATIVE_PROPS_MAGIC, NATIVE_VIEW_PROPS_PROP_KEY,
    NativeViewSpec, TypedNativeViewProps, VideoPlayer, VideoPlayerCommand, VideoPlayerProps,
    WebView, WebViewCommand, WebViewProps,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{BackendApplyStats, BackendError, RenderSink};

#[derive(Default)]
struct RecordingSink {
    raw_prop_keys: Vec<String>,
    draw_lists: Vec<(NodeId, DrawList)>,
    native_views: Vec<(NodeId, TypedNativeViewProps)>,
}

impl RenderSink for RecordingSink {
    fn begin_frame(&mut self, _: SurfaceId, _: FrameId) {
        self.raw_prop_keys.clear();
        self.draw_lists.clear();
        self.native_views.clear();
    }
    fn create_node(&mut self, _: NodeId, _: ComponentKind, _: Option<&str>) {}
    fn remove_node(&mut self, _: NodeId) {}
    fn set_children(&mut self, _: NodeId, _: &[NodeId]) {}
    fn set_text(&mut self, _: NodeId, _: &str) {}
    fn set_layout(&mut self, _: NodeId, _: &LayoutProps) {}
    fn set_style(&mut self, _: NodeId, _: &StyleProps) {}
    fn set_raw_prop(&mut self, _: NodeId, key: &str, _: RawPropValueRef<'_>) {
        self.raw_prop_keys.push(key.to_owned());
    }
    fn set_event_mask(&mut self, _: NodeId, _: u64) {}
    fn dispatch_command(&mut self, _: NodeId, _: &str, _: &[u8]) {}
    fn set_draw_list(&mut self, node_id: NodeId, draw_list: Option<&DrawList>) {
        if let Some(dl) = draw_list {
            self.draw_lists.push((node_id, dl.clone()));
        }
    }
    fn set_native_view_props(&mut self, node_id: NodeId, props: Option<&TypedNativeViewProps>) {
        if let Some(p) = props {
            self.native_views.push((node_id, p.clone()));
        }
    }
    fn commit_frame(&mut self, _: bool) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

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
    let mut inproc_surface = DioxusSurface::new(1, canvas_app);
    let mut sink = RecordingSink::default();
    inproc_surface.flush_into(1, &mut sink).unwrap();

    // Direct `flush_into` MUST NOT emit any serialized `drawList` raw prop.
    assert!(
        !sink.raw_prop_keys.iter().any(|k| k == "drawList"),
        "flush_into must emit 0 serialized drawList raw props"
    );
    // Direct zero-copy DrawList arrives via RenderSink with exact Arc pointer identity.
    assert_eq!(sink.draw_lists.len(), 1);
    assert!(sink.draw_lists[0].1.ptr_eq(&expected));
    assert_eq!(&sink.draw_lists[0].1[..], &expected[..]);

    // Cross-boundary `BatchSink` (`render_frame_slice`) emits raw NVE_DRAW1 bytes (ZERO base64!).
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
        .expect("BatchSink emits binary NVE_DRAW1 RawPropValue::Bytes (no base64)");
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

    // 1. Direct `flush_into`: 0 NVNP raw props, typed props pushed straight to RenderSink.
    let mut inproc = DioxusSurface::new(1, demo_app);
    let mut sink = RecordingSink::default();
    inproc.flush_into(1, &mut sink).unwrap();
    assert!(
        !sink
            .raw_prop_keys
            .iter()
            .any(|k| k == NATIVE_VIEW_PROPS_PROP_KEY),
        "flush_into must emit 0 NVNP binary raw props"
    );

    assert_eq!(sink.native_views.len(), 3);
    assert!(sink.native_views.iter().any(|(_, p)| {
        p.kind() == BlurSurface::KIND
            && p.downcast_ref::<BlurSurfaceProps>().map(|b| b.material)
                == Some(BlurMaterial::Chrome)
    }));
    assert!(sink.native_views.iter().any(|(_, p)| {
        p.kind() == WebView::KIND
            && p.downcast_ref::<WebViewProps>().map(|w| w.url.as_str()) == Some("https://nve.dev")
    }));
    assert!(sink.native_views.iter().any(|(_, p)| {
        p.kind() == VideoPlayer::KIND
            && p.downcast_ref::<VideoPlayerProps>().map(|v| v.controls) == Some(true)
    }));

    // 2. Cross-boundary `BatchSink`: emits valid canonical NVNP binary bytes for cross-language hosts.
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
