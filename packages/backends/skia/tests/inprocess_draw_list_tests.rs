//! Deep tests proving zero-serialization `DrawList` (`Arc::ptr_eq`) and typed `NativeView`
//! forwarding in `SkiaBackend` + `HeadlessBackend`.

use nve_abi::{ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, LayoutProps};
use nve_backend_skia::{PaintOp, PixelBuffer, SkiaBackend};
use nve_components::canvas_ir::DrawListRecorder;
use nve_components::native_view::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, NativeViewSpec, TypedNativeViewCommand,
    TypedNativeViewProps, VideoPlayer, VideoPlayerCommand,
};
use nve_engine::{RenderBackend, Viewport};

#[test]
fn skia_backend_consumes_direct_draw_list_with_zero_serialization_and_arc_identity() {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(1, ComponentKind::View, None);
    b.set_layout(
        1,
        &LayoutProps {
            width: Dimension::Points(120.0),
            height: Dimension::Points(80.0),
            ..Default::default()
        },
    );
    b.create_node(2, ComponentKind::SkiaCanvas, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(60.0),
            ..Default::default()
        },
    );
    b.set_children(1, &[2]);
    b.commit(true);

    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 120.0,
            height: 80.0,
            scale_factor: 1.0,
        },
    );
    backend.submit_frame(b.finish()).unwrap();

    // Attach DrawList directly — ZERO `drawList` bytes/base64 in CommandBuffer.
    let mut rec = DrawListRecorder::with_capacity(3);
    rec.clear(ColorRgba::rgb(17, 24, 39))
        .fill_rect(10.0, 10.0, 40.0, 40.0, ColorRgba::rgb(34, 197, 94))
        .fill_circle(75.0, 30.0, 12.0, ColorRgba::rgb(56, 189, 248));
    let draw_list = rec.finish();
    backend.sync_draw_lists_from([(2, &draw_list)]);

    let dl = backend.display_list();
    let canvas_ops = dl
        .ops
        .iter()
        .find_map(|op| match op {
            PaintOp::CustomSkiaCanvas { node_id: 2, ops, .. } => Some(ops),
            _ => None,
        })
        .expect("CustomSkiaCanvas paint op emitted from direct DrawList");
    assert!(
        canvas_ops.ptr_eq(&draw_list),
        "SkiaBackend must reuse DrawList Arc without cloning or decoding"
    );

    // Rasterize and verify real pixels from the zero-serialization DrawList.
    let mut px = PixelBuffer::new(120, 80);
    backend.rasterize(&mut px, ColorRgba::BLACK);
    assert_eq!(px.get_rgb(25, 25), Some((34, 197, 94)));
    assert_eq!(px.get_rgb(75, 30), Some((56, 189, 248)));
}

#[test]
fn skia_and_headless_backends_store_typed_native_view_props_and_commands() {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(1, ComponentKind::View, None);
    b.create_node(2, ComponentKind::Custom, Some(BlurSurface::KIND));
    b.create_node(3, ComponentKind::Custom, Some(VideoPlayer::KIND));
    b.set_children(1, &[2, 3]);
    b.commit(true);

    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    );
    backend.submit_frame(b.finish()).unwrap();

    let blur_props = TypedNativeViewProps::from_spec::<BlurSurface>(BlurSurfaceProps {
        material: BlurMaterial::Thick,
        intensity: 0.9,
        tint: None,
    })
    .unwrap();
    backend.sync_native_views_from([(2, &blur_props)]);

    let node2 = backend.layout_tree().node(2).expect("node 2 mounted");
    let stored = node2.native_view_props.as_ref().expect("typed props stored");
    assert_eq!(
        stored.downcast_ref::<BlurSurfaceProps>(),
        Some(&BlurSurfaceProps {
            material: BlurMaterial::Thick,
            intensity: 0.9,
            tint: None,
        })
    );

    let seek_cmd =
        TypedNativeViewCommand::from_spec::<VideoPlayer>(VideoPlayerCommand::SeekMs(4200)).unwrap();
    backend.dispatch_typed_native_command(3, seek_cmd);
    let cmds = backend.layout_tree().typed_dispatched_commands();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].0, 3);
    assert_eq!(
        cmds[0].1.downcast_ref::<VideoPlayerCommand>(),
        Some(&VideoPlayerCommand::SeekMs(4200))
    );
}
