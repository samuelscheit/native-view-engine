//! End-to-end tests proving cold `frame0.nvereloc` (`SceneRelocView`) renders pixel-for-pixel
//! identically to live `SkiaBackend` + `HeadlessBackend` rendering and hands off seamlessly when
//! the first live frame arrives.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
    FlexDirection, LayoutProps, StyleProps, ROOT_NODE_ID,
};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::raster::PixelBuffer;
use nve_backend_skia::SkiaBackend;
use nve_engine::{RenderBackend, SceneRelocView, VerifyMode, Viewport};

fn build_sample_frame(frame_id: u64, title: &str, btn_color: ColorRgba) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            padding: EdgeInsets::all(20.0),
            gap: 12.0,
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            ..Default::default()
        },
    );

    b.create_node(3, ComponentKind::Text, None);
    b.set_style(
        3,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_text(3, title);

    b.create_node(4, ComponentKind::Pressable, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(180.0),
            height: Dimension::Points(48.0),
            ..Default::default()
        },
    );
    b.set_style(
        4,
        &StyleProps {
            background: btn_color,
            border_radius: 10.0,
            border_color: ColorRgba::rgb(148, 163, 184),
            border_width: 1.0,
            ..Default::default()
        },
    );
    b.set_event_mask(4, EventMask::PRESS);

    b.set_children(2, &[3, 4]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    b.into_bytes()
}

#[test]
fn cold_frame0_nvereloc_matches_live_render_pixel_for_pixel_and_hands_off_cleanly() {
    let vp = Viewport {
        width: 320.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let clear = ColorRgba::rgb(15, 23, 42);
    let live_cmd = build_sample_frame(1, "Instant Boot Parity", ColorRgba::rgb(37, 99, 235));

    // 1. Live render + rasterize + export frame0.nvereloc.
    let mut live_skia = SkiaBackend::new();
    live_skia.set_viewport(1, vp);
    live_skia.submit_frame(&live_cmd).unwrap();
    let mut live_pixels = PixelBuffer::new(320, 200);
    live_skia.rasterize(&mut live_pixels, clear);

    let reloc_buf = live_skia
        .export_scene_reloc(1, 1, "parity-app", clear)
        .expect("export frame0.nvereloc");

    // 2. Cold SkiaBackend loads frame0.nvereloc with zero live app boot and rasterizes.
    let mut cold_skia = SkiaBackend::new();
    let count = cold_skia
        .load_reloc_scene(&reloc_buf, VerifyMode::Full)
        .expect("cold load_reloc_scene");
    assert_eq!(count, 4);

    let mut cold_pixels = PixelBuffer::new(320, 200);
    cold_skia.rasterize(&mut cold_pixels, clear);

    // Every single pixel and computed rect MUST match 100%!
    assert_eq!(live_skia.layout_of(2), cold_skia.layout_of(2));
    assert_eq!(live_skia.layout_of(3), cold_skia.layout_of(3));
    assert_eq!(live_skia.layout_of(4), cold_skia.layout_of(4));
    assert_eq!(live_pixels.pixels(), cold_pixels.pixels());

    // HeadlessBackend also hydrates identically from the same SceneRelocView.
    let scene = SceneRelocView::from_bytes(&reloc_buf, VerifyMode::Full).unwrap();
    let mut cold_headless = HeadlessBackend::new();
    assert_eq!(cold_headless.load_reloc_scene(scene).unwrap(), 4);
    assert_eq!(cold_headless.layout_of(4), live_skia.layout_of(4));
    assert!(cold_headless.render_ascii_tree().contains("Instant Boot Parity"));

    // 3. Seamless live handoff: submit updated live frame onto the pre-warmed backend.
    let next_cmd = build_sample_frame(2, "Live Handoff Complete", ColorRgba::rgb(16, 185, 129));
    cold_skia.submit_frame(&next_cmd).unwrap();
    let mut handoff_pixels = PixelBuffer::new(320, 200);
    cold_skia.rasterize(&mut handoff_pixels, clear);

    let btn = cold_skia.layout_of(4).unwrap();
    let cx = (btn.x + btn.width * 0.5) as u32;
    let cy = (btn.y + btn.height * 0.5) as u32;
    assert_eq!(live_pixels.get_rgb(cx, cy), Some((37, 99, 235)));
    assert_eq!(handoff_pixels.get_rgb(cx, cy), Some((16, 185, 129)));
}
