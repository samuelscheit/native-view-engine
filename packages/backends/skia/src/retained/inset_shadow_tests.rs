//! Comprehensive pixel, CSS paint-order, CPU↔GPU parity, HiDPI, and retained compositor oracle
//! tests for inset `box-shadow` rasterization on both CPU (`raster::shadow`) and GPU
//! (`gpu::skia_primitives` under `--features skia`).

use nve_abi::{ColorRgba, SurfaceId};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{
    BoxShadow, BoxShadows, Layout, Position, TextContent, Typography, Visual,
};

use super::oracle_tests::{assert_scene_matches_fresh_display_list, physical_dims};
use crate::raster::PixelBuffer;
use crate::{PaintOp, SkiaBackend};

const BG_CLEAR: ColorRgba = ColorRgba::rgb(240, 244, 248);

fn render_cpu_and_optionally_gpu(
    backend: &mut SkiaBackend,
    store: &NodeStore,
    w: u32,
    h: u32,
) -> (PixelBuffer, Option<PixelBuffer>) {
    let scale = backend.viewport().scale_factor;
    let (pw, ph) = physical_dims(w, h, scale);
    let mut cpu_buf = PixelBuffer::new(pw, ph);
    backend.rasterize_with_store(store, &mut cpu_buf, BG_CLEAR);

    #[cfg(feature = "skia")]
    let gpu_buf = {
        use crate::gpu::{CpuSkiaSurface, GpuSurfaceBackend};
        let mut surf = CpuSkiaSurface::new_with_scale(pw, ph, scale).expect("cpu skia surface");
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surf.render_scene_cached_with_plugins(scene, scale, BG_CLEAR, Some((store, reg, ev)))
            .expect("render scene");
        let mut out = PixelBuffer::new(pw, ph);
        surf.copy_to_pixel_buffer(&mut out).expect("readback");
        Some(out)
    };
    #[cfg(not(feature = "skia"))]
    let gpu_buf: Option<PixelBuffer> = None;

    (cpu_buf, gpu_buf)
}

fn luma((r, g, b): (u8, u8, u8)) -> f32 {
    0.299 * (r as f32) + 0.587 * (g as f32) + 0.114 * (b as f32)
}

fn assert_cpu_gpu_parity(
    cpu: &PixelBuffer,
    gpu: Option<&PixelBuffer>,
    max_mean_diff: f32,
    label: &str,
) {
    let Some(gpu) = gpu else {
        return;
    };
    assert_eq!((cpu.width(), cpu.height()), (gpu.width(), gpu.height()));
    let mut sum_diff = 0u64;
    let mut count = 0u64;
    for (&cp, &gp) in cpu.pixels().iter().zip(gpu.pixels().iter()) {
        let cr = ((cp >> 16) & 0xFF) as u8;
        let cg = ((cp >> 8) & 0xFF) as u8;
        let cb = (cp & 0xFF) as u8;
        let gr = ((gp >> 16) & 0xFF) as u8;
        let gg = ((gp >> 8) & 0xFF) as u8;
        let gb = (gp & 0xFF) as u8;
        sum_diff += cr.abs_diff(gr) as u64 + cg.abs_diff(gg) as u64 + cb.abs_diff(gb) as u64;
        count += 3;
    }
    let mean = (sum_diff as f32) / (count.max(1) as f32);
    assert!(
        mean <= max_mean_diff,
        "CPU vs GPU inset shadow mean channel diff {mean:.2} exceeded {max_mean_diff:.2} for {label}"
    );
}

struct BoxSpec {
    canvas: (u32, u32),
    scale: f32,
    rect: (f32, f32, f32, f32),
    bg: Rgba,
    radius: f32,
    border: (f32, Rgba),
    shadows: Vec<BoxShadow>,
}

fn build_single_box(spec: BoxSpec) -> (NodeStore, SkiaBackend, NodeId) {
    let (w, h) = spec.canvas;
    let (box_x, box_y, box_w, box_h) = spec.rect;
    let (border_w, border_color) = spec.border;
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: w as f32,
            height: h as f32,
            scale_factor: spec.scale,
        },
    );
    let card = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(w as f32))
            .height(Length::points(h as f32));
        let card = tx.create();
        tx.edit::<Layout>(card)
            .position(Position::Absolute)
            .inset_left(Length::points(box_x))
            .inset_top(Length::points(box_y))
            .width(Length::points(box_w))
            .height(Length::points(box_h));
        tx.edit::<Visual>(card)
            .background(spec.bg)
            .border_radius(spec.radius)
            .border_width(border_w)
            .border_color(border_color)
            .box_shadows(Some(BoxShadows::new(spec.shadows)));
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut changes);
        card
    };
    backend.commit(&store, &changes).expect("commit inset card");
    (store, backend, card)
}

#[test]
fn inset_shadow_basic_blur_zero_bleed_and_oracle_parity() {
    let (store, mut backend, _) = build_single_box(BoxSpec {
        canvas: (140, 140),
        scale: 1.0,
        rect: (20.0, 20.0, 100.0, 100.0),
        bg: Rgba::WHITE,
        radius: 0.0,
        border: (0.0, Rgba::TRANSPARENT),
        shadows: vec![BoxShadow::new(0.0, 0.0, 12.0, Rgba::new(0, 0, 0, 200)).inset()],
    });
    assert_scene_matches_fresh_display_list(&mut backend, &store, 140, 140, "basic inset");
    let (cpu, gpu) = render_cpu_and_optionally_gpu(&mut backend, &store, 140, 140);

    for (label, buf) in std::iter::once(("cpu", &cpu)).chain(gpu.as_ref().map(|g| ("gpu", g))) {
        // Zero bleed outside border box [20..120, 20..120].
        for &(ox, oy) in &[(18, 70), (121, 70), (70, 18), (70, 121)] {
            assert_eq!(
                buf.get_rgb(ox, oy),
                Some((BG_CLEAR.r, BG_CLEAR.g, BG_CLEAR.b)),
                "{label}: inset shadow must not bleed outside box at ({ox},{oy})"
            );
        }
        // Interior edges darkened relative to bright center (70, 70).
        let center = luma(buf.get_rgb(70, 70).unwrap());
        let top_edge = luma(buf.get_rgb(70, 22).unwrap());
        let bottom_edge = luma(buf.get_rgb(70, 117).unwrap());
        let left_edge = luma(buf.get_rgb(22, 70).unwrap());
        let right_edge = luma(buf.get_rgb(117, 70).unwrap());
        assert!(
            center > 245.0,
            "{label}: center should stay bright white, got {center}"
        );
        for (side, val) in [
            ("top", top_edge),
            ("bottom", bottom_edge),
            ("left", left_edge),
            ("right", right_edge),
        ] {
            assert!(
                val < center - 35.0,
                "{label}: {side} edge ({val}) must be significantly darker than center ({center})"
            );
        }
    }
    assert_cpu_gpu_parity(&cpu, gpu.as_ref(), 3.0, "basic inset blur");
}

#[test]
fn inset_shadow_offset_positive_and_negative_spread() {
    // 1. Positive offset (+10, +12): darkens top & left interior, leaves bottom & right clean.
    let (store, mut backend, _) = build_single_box(BoxSpec {
        canvas: (140, 140),
        scale: 1.0,
        rect: (20.0, 20.0, 100.0, 100.0),
        bg: Rgba::WHITE,
        radius: 0.0,
        border: (0.0, Rgba::TRANSPARENT),
        shadows: vec![BoxShadow::new(10.0, 12.0, 8.0, Rgba::new(0, 0, 0, 220)).inset()],
    });
    assert_scene_matches_fresh_display_list(&mut backend, &store, 140, 140, "offset inset");
    let (cpu, gpu) = render_cpu_and_optionally_gpu(&mut backend, &store, 140, 140);
    for (label, buf) in std::iter::once(("cpu", &cpu)).chain(gpu.as_ref().map(|g| ("gpu", g))) {
        let top = luma(buf.get_rgb(70, 24).unwrap());
        let left = luma(buf.get_rgb(24, 70).unwrap());
        let bottom = luma(buf.get_rgb(70, 115).unwrap());
        let right = luma(buf.get_rgb(115, 70).unwrap());
        assert!(
            top < 120.0 && left < 120.0,
            "{label}: +offset inset must strongly darken top ({top}) and left ({left})"
        );
        assert!(
            bottom > 240.0 && right > 240.0,
            "{label}: +offset inset must leave bottom ({bottom}) and right ({right}) bright"
        );
    }
    assert_cpu_gpu_parity(&cpu, gpu.as_ref(), 3.0, "offset inset");

    // 2. Positive spread shrinks hole (thicker shadow); negative spread expands hole (thinner).
    let (s_pos, mut b_pos, _) = build_single_box(BoxSpec {
        canvas: (140, 140),
        scale: 1.0,
        rect: (20.0, 20.0, 100.0, 100.0),
        bg: Rgba::WHITE,
        radius: 0.0,
        border: (0.0, Rgba::TRANSPARENT),
        shadows: vec![BoxShadow::new(0.0, 0.0, 6.0, Rgba::new(0, 0, 0, 220))
            .spread(12.0)
            .inset()],
    });
    let (s_neg, mut b_neg, _) = build_single_box(BoxSpec {
        canvas: (140, 140),
        scale: 1.0,
        rect: (20.0, 20.0, 100.0, 100.0),
        bg: Rgba::WHITE,
        radius: 0.0,
        border: (0.0, Rgba::TRANSPARENT),
        shadows: vec![BoxShadow::new(0.0, 14.0, 6.0, Rgba::new(0, 0, 0, 220))
            .spread(-6.0)
            .inset()],
    });
    let (cpu_pos, gpu_pos) = render_cpu_and_optionally_gpu(&mut b_pos, &s_pos, 140, 140);
    let (cpu_neg, gpu_neg) = render_cpu_and_optionally_gpu(&mut b_neg, &s_neg, 140, 140);

    for (label, buf) in
        std::iter::once(("cpu", &cpu_pos)).chain(gpu_pos.as_ref().map(|g| ("gpu", g)))
    {
        // At y = 28 (8px inside top edge), spread=12 puts pixel well inside the dark band.
        assert!(
            luma(buf.get_rgb(70, 28).unwrap()) < 100.0,
            "{label}: positive spread must thicken inset band at (70, 28)"
        );
        // Center stays bright.
        assert!(luma(buf.get_rgb(70, 70).unwrap()) > 245.0);
    }
    for (label, buf) in
        std::iter::once(("cpu", &cpu_neg)).chain(gpu_neg.as_ref().map(|g| ("gpu", g)))
    {
        // With offset_y=14 and spread=-6, top shadow edge is 8px deep, while left/right edges
        // expanded outward by 6px so left edge (22, 70) stays bright white!
        assert!(
            luma(buf.get_rgb(70, 23).unwrap()) < 120.0,
            "{label}: negative spread with offset_y=14 still darkens top edge"
        );
        assert!(
            luma(buf.get_rgb(22, 70).unwrap()) > 245.0,
            "{label}: negative spread must pull side shadow outside padding edge"
        );
    }
    assert_cpu_gpu_parity(&cpu_pos, gpu_pos.as_ref(), 3.0, "positive spread inset");
    assert_cpu_gpu_parity(&cpu_neg, gpu_neg.as_ref(), 3.0, "negative spread inset");
}

#[test]
fn inset_shadow_zero_blur_crisp_invisible_and_collapsed_hole() {
    // 1. Zero blur + zero offset + zero spread => invisible (no pixels modified inside box).
    let (s_zero, mut b_zero, _) = build_single_box(BoxSpec {
        canvas: (100, 100),
        scale: 1.0,
        rect: (10.0, 10.0, 80.0, 80.0),
        bg: Rgba::WHITE,
        radius: 8.0,
        border: (0.0, Rgba::TRANSPARENT),
        shadows: vec![BoxShadow::new(0.0, 0.0, 0.0, Rgba::rgb(255, 0, 0)).inset()],
    });
    let (cpu_z, gpu_z) = render_cpu_and_optionally_gpu(&mut b_zero, &s_zero, 100, 100);
    for (label, buf) in std::iter::once(("cpu", &cpu_z)).chain(gpu_z.as_ref().map(|g| ("gpu", g))) {
        assert_eq!(
            buf.get_rgb(50, 50),
            Some((255, 255, 255)),
            "{label}: 0-blur 0-offset 0-spread inset must not tint interior"
        );
        assert_eq!(
            buf.get_rgb(50, 11),
            Some((255, 255, 255)),
            "{label}: 0-blur 0-offset 0-spread inset must not tint edge"
        );
    }

    // 2. Zero blur + crisp spread=10 => sharp 10px frame inside padding box.
    let (s_crisp, mut b_crisp, _) = build_single_box(BoxSpec {
        canvas: (100, 100),
        scale: 1.0,
        rect: (10.0, 10.0, 80.0, 80.0),
        bg: Rgba::WHITE,
        radius: 0.0,
        border: (0.0, Rgba::TRANSPARENT),
        shadows: vec![BoxShadow::new(0.0, 0.0, 0.0, Rgba::rgb(20, 40, 220))
            .spread(10.0)
            .inset()],
    });
    let (cpu_c, gpu_c) = render_cpu_and_optionally_gpu(&mut b_crisp, &s_crisp, 100, 100);
    for (label, buf) in std::iter::once(("cpu", &cpu_c)).chain(gpu_c.as_ref().map(|g| ("gpu", g))) {
        // Inside 10px frame: blue.
        assert_eq!(
            buf.get_rgb(50, 15),
            Some((20, 40, 220)),
            "{label}: crisp inset band at (50, 15)"
        );
        // Inside hole [20..80, 20..80]: pure white.
        assert_eq!(
            buf.get_rgb(50, 25),
            Some((255, 255, 255)),
            "{label}: crisp inset hole at (50, 25)"
        );
    }
    assert_cpu_gpu_parity(&cpu_c, gpu_c.as_ref(), 1.5, "crisp zero-blur inset");

    // 3. Collapsed hole (spread >= inner_w / 2): fills entire padding box with shadow color.
    let (s_col, mut b_col, _) = build_single_box(BoxSpec {
        canvas: (100, 100),
        scale: 1.0,
        rect: (10.0, 10.0, 80.0, 80.0),
        bg: Rgba::WHITE,
        radius: 12.0,
        border: (4.0, Rgba::rgb(16, 185, 129)),
        shadows: vec![BoxShadow::new(0.0, 0.0, 8.0, Rgba::new(220, 38, 38, 255))
            .spread(50.0)
            .inset()],
    });
    assert_scene_matches_fresh_display_list(&mut b_col, &s_col, 100, 100, "collapsed hole");
    let (cpu_col, gpu_col) = render_cpu_and_optionally_gpu(&mut b_col, &s_col, 100, 100);
    for (label, buf) in
        std::iter::once(("cpu", &cpu_col)).chain(gpu_col.as_ref().map(|g| ("gpu", g)))
    {
        // Center of padding box completely filled by red inset shadow.
        assert_eq!(
            buf.get_rgb(50, 50),
            Some((220, 38, 38)),
            "{label}: collapsed hole must fill center with shadow color"
        );
        // Border ring at (50, 11) stays bright green (border painted AFTER inset shadow).
        let (br, bg, _bb) = buf.get_rgb(50, 11).unwrap();
        assert!(
            bg > 150 && br < 60,
            "{label}: border must paint above collapsed inset shadow, got ({br},{bg})"
        );
        // Outside border box stays clear background.
        assert_eq!(
            buf.get_rgb(5, 50),
            Some((BG_CLEAR.r, BG_CLEAR.g, BG_CLEAR.b)),
            "{label}: collapsed hole must never bleed outside border box"
        );
    }
    assert_cpu_gpu_parity(&cpu_col, gpu_col.as_ref(), 2.5, "collapsed hole inset");
}

#[test]
fn inset_shadow_rounded_corners_border_width_paint_order_and_hidpi_parity() {
    // Node has BOTH outer shadow + inset shadow + rounded corners + thick green border + child box
    // + text, at 2.0x HiDPI scale to verify CSS paint order and HiDPI parity.
    let scale = 2.0f32;
    let (w, h) = (160u32, 140u32);
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: w as f32,
            height: h as f32,
            scale_factor: scale,
        },
    );

    {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(w as f32))
            .height(Length::points(h as f32));

        let card = tx.create();
        tx.edit::<Layout>(card)
            .position(Position::Absolute)
            .inset_left(Length::points(24.0))
            .inset_top(Length::points(20.0))
            .width(Length::points(112.0))
            .height(Length::points(100.0));
        tx.edit::<Visual>(card)
            .background(Rgba::WHITE)
            .border_radius(16.0)
            .border_width(6.0)
            .border_color(Rgba::rgb(34, 197, 94))
            .box_shadows(Some(BoxShadows::new(vec![
                // Outer shadow: red-tinted drop shadow outside border box
                BoxShadow::new(0.0, 8.0, 10.0, Rgba::new(220, 38, 38, 220)),
                // Inset shadow: dark blue inner shadow clipped to padding box
                BoxShadow::new(0.0, 8.0, 10.0, Rgba::new(15, 23, 120, 230)).inset(),
            ])));

        // Child sits over the top inset shadow region (proves children paint AFTER inset shadow).
        let child = tx.create();
        tx.edit::<Layout>(child)
            .position(Position::Absolute)
            .inset_left(Length::points(40.0))
            .inset_top(Length::points(8.0))
            .width(Length::points(32.0))
            .height(Length::points(20.0));
        tx.edit::<Visual>(child).background(Rgba::rgb(250, 204, 21));
        tx.edit::<TextContent>(child).text("Hi");
        tx.edit::<Typography>(child)
            .color(Rgba::rgb(15, 23, 42))
            .font_size(12.0);

        tx.set_children(card, &[child]);
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("commit combined");
    assert_scene_matches_fresh_display_list(&mut backend, &store, w, h, "combined outer+inset @2x");

    // Verify exact CSS paint order in materialized DisplayList:
    // outer DrawShadow(inset=false) < FillRoundedRect < inset DrawShadow(inset=true) < StrokeRoundedRect < child FillRoundedRect.
    let ops = &backend.materialize_display_list().ops;
    let idx_outer = ops
        .iter()
        .position(|op| matches!(op, PaintOp::DrawShadow { inset: false, .. }))
        .expect("outer shadow op");
    let idx_bg = ops
        .iter()
        .position(|op| matches!(op, PaintOp::FillRoundedRect { color, .. } if *color == ColorRgba::WHITE))
        .expect("card bg op");
    let idx_inset = ops
        .iter()
        .position(|op| matches!(op, PaintOp::DrawShadow { inset: true, border_width, .. } if (*border_width - 6.0).abs() < 1e-3))
        .expect("inset shadow op with border_width=6");
    let idx_border = ops
        .iter()
        .position(|op| matches!(op, PaintOp::StrokeRoundedRect { .. }))
        .expect("card border op");
    let idx_child = ops
        .iter()
        .position(|op| {
            matches!(op, PaintOp::FillRoundedRect { color, .. } if *color == ColorRgba::rgb(250, 204, 21))
        })
        .expect("child bg op");
    assert!(
        idx_outer < idx_bg && idx_bg < idx_inset && idx_inset < idx_border && idx_border < idx_child,
        "CSS paint order violated: outer={idx_outer}, bg={idx_bg}, inset={idx_inset}, border={idx_border}, child={idx_child}"
    );

    let (cpu, gpu) = render_cpu_and_optionally_gpu(&mut backend, &store, w, h);
    for (label, buf) in std::iter::once(("cpu", &cpu)).chain(gpu.as_ref().map(|g| ("gpu", g))) {
        // 1. Outer shadow visible below the card at logical (80, 124) -> physical (160, 248).
        let (or, og, ob) = buf.get_rgb(160, 248).unwrap();
        assert!(
            or > og + 15 && or > ob + 15,
            "{label}: outer red shadow must show below card at (160,248), got ({or},{og},{ob})"
        );
        // 2. Green border ring at logical (80, 23) -> physical (160, 46) is unaffected by inset shadow.
        let (br, bg, bb) = buf.get_rgb(160, 46).unwrap();
        assert!(
            bg > 170 && br < 60 && bb < 120,
            "{label}: green border must paint over inset shadow at (160,46), got ({br},{bg},{bb})"
        );
        // 3. Inset blue shadow just below top border (away from child) at logical (46, 29) -> physical (92, 58).
        let (ir, ig, ib) = buf.get_rgb(92, 58).unwrap();
        assert!(
            ib > ir + 25 && luma((ir, ig, ib)) < 180.0,
            "{label}: blue inset shadow must darken padding edge at (92,58), got ({ir},{ig},{ib})"
        );
        // 4. Yellow child at logical (78, 36) -> physical (156, 72) sits cleanly ON TOP of inset shadow.
        let (cr, cg, cb) = buf.get_rgb(156, 72).unwrap();
        assert!(
            cr > 220 && cg > 180 && cb < 60,
            "{label}: yellow child must paint on top of inset shadow at (156,72), got ({cr},{cg},{cb})"
        );
    }
    assert_cpu_gpu_parity(&cpu, gpu.as_ref(), 4.0, "combined rounded+border @2x HiDPI");
}
