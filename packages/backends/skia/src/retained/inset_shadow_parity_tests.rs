//! Expanded CPU↔GPU shadow parity & edge-case regression matrix (P1-1, P1-2, P2-6):
//! - Extreme negative spread (`-f32::MAX * 0.6`, `-1e38`) and huge blur (`1e38`) on CPU & GPU
//! - Transform-aware rotation & non-uniform scale parity for BOTH outer & inset shadows (zero bleed)
//! - 2x HiDPI fractional blur (`0.4`, `0.6`, `0.8`) parity for BOTH outer & inset shadows
//! - Group `opacity < 1.0`, nested rounded clips, and `border_radius > half_extent` (pill/circle)

use nve_abi::SurfaceId;
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{BoxShadow, BoxShadows, Layout, Overflow, Position, Transform, Visual};

use super::inset_shadow_tests::{
    assert_cpu_gpu_parity_roi, build_single_box, luma, render_cpu_and_optionally_gpu, BoxSpec,
    BG_CLEAR,
};
use super::oracle_tests::assert_scene_matches_fresh_display_list;
use crate::SkiaBackend;

#[test]
fn inset_shadow_huge_negative_spread_casts_zero_shadow_and_huge_blur_stays_finite() {
    // 1. Huge negative spread `-f32::MAX * 0.6` overflows `inner_w - 2 * spread` to `+inf`:
    //    MUST cast zero shadow (NEVER solid-fill the padding box!) on BOTH CPU and GPU.
    for (tag, spread) in [
        ("neg_max_0.6", -f32::MAX * 0.6),
        ("neg_1e38", -1e38_f32),
    ] {
        let (store, mut backend, _) = build_single_box(BoxSpec {
            canvas: (100, 100),
            scale: 1.0,
            rect: (10.0, 10.0, 80.0, 80.0),
            bg: Rgba::WHITE,
            radius: 10.0,
            border: (2.0, Rgba::rgb(30, 64, 175)),
            shadows: vec![BoxShadow::new(0.0, 0.0, 8.0, Rgba::rgb(220, 38, 38))
                .spread(spread)
                .inset()],
        });
        assert_scene_matches_fresh_display_list(&mut backend, &store, 100, 100, tag);
        let (cpu, gpu) = render_cpu_and_optionally_gpu(&mut backend, &store, 100, 100);
        for (side, buf) in std::iter::once(("cpu", &cpu)).chain(gpu.as_ref().map(|g| ("gpu", g))) {
            // Interior must remain pure white (zero red shadow inversion!).
            for &(px, py) in &[(50, 50), (50, 16), (16, 50), (84, 50), (50, 84)] {
                assert_eq!(
                    buf.get_rgb(px, py),
                    Some((255, 255, 255)),
                    "{side} ({tag}): huge negative spread must cast zero shadow at ({px},{py})"
                );
            }
        }
        assert_cpu_gpu_parity_roi(&cpu, gpu.as_ref(), (9, 9, 91, 91), 2.0, tag);
    }

    // 2. Huge blur `1e38` (both outer and inset) @ 2x HiDPI: sigma clamps to `MAX_BLUR_SIGMA`
    //    (512 phys px == logical blur 512.0 at 2x), GPU `outset` never overflows to non-finite
    //    `outer_*`, and CPU never panics on `pad as i32`. Must match exact capped-sigma render!
    let make_huge_blur = |blur: f32| {
        build_single_box(BoxSpec {
            canvas: (120, 120),
            scale: 2.0,
            rect: (20.0, 20.0, 80.0, 80.0),
            bg: Rgba::WHITE,
            radius: 8.0,
            border: (0.0, Rgba::TRANSPARENT),
            shadows: vec![
                BoxShadow::new(0.0, 0.0, blur, Rgba::new(220, 38, 38, 200)),
                BoxShadow::new(0.0, 0.0, blur, Rgba::new(0, 0, 0, 220)).inset(),
            ],
        })
    };
    let (store, mut backend, _) = make_huge_blur(1e38);
    let (store_cap, mut backend_cap, _) = make_huge_blur(512.0);
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 120, "huge blur 1e38");
    let (cpu, gpu) = render_cpu_and_optionally_gpu(&mut backend, &store, 120, 120);
    let (cpu_cap, gpu_cap) = render_cpu_and_optionally_gpu(&mut backend_cap, &store_cap, 120, 120);
    assert_eq!(
        cpu.pixels(),
        cpu_cap.pixels(),
        "CPU blur=1e38 must match exact MAX_BLUR_SIGMA=512px capped output"
    );
    if let (Some(g), Some(g_cap)) = (gpu.as_ref(), gpu_cap.as_ref()) {
        assert_eq!(
            g.pixels(),
            g_cap.pixels(),
            "GPU blur=1e38 must match exact MAX_BLUR_SIGMA=512px capped output (zero outset overflow)"
        );
    }
    for (side, buf) in std::iter::once(("cpu", &cpu)).chain(gpu.as_ref().map(|g| ("gpu", g))) {
        let center_luma = luma(buf.get_rgb(120, 120).unwrap());
        assert!(
            center_luma < 245.0,
            "{side}: huge blur 1e38 must still render clamped 512px inset shadow (got luma {center_luma})"
        );
    }
}

#[test]
fn transformed_rotation_and_non_uniform_scale_shadow_cpu_gpu_parity() {
    // 1. Rotated box (30 deg) with BOTH inset and outer shadows:
    //    Software MUST rotate both shadows (zero unrotated AABB bleed outside rotated box!).
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 160.0,
            height: 160.0,
            scale_factor: 1.0,
        },
    );
    {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(160.0))
            .height(Length::points(160.0));
        let card = tx.create();
        tx.edit::<Layout>(card)
            .position(Position::Absolute)
            .inset_left(Length::points(40.0))
            .inset_top(Length::points(50.0))
            .width(Length::points(80.0))
            .height(Length::points(60.0));
        tx.edit::<Visual>(card)
            .background(Rgba::WHITE)
            .border_radius(8.0)
            .box_shadows(Some(BoxShadows::new(vec![
                BoxShadow::new(0.0, 0.0, 8.0, Rgba::new(15, 23, 180, 230))
                    .spread(8.0)
                    .inset(),
            ])));
        tx.edit::<Transform>(card).rotate_deg(30.0);
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("commit rotated inset");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 160, "rotated inset");
    let (cpu, gpu) = render_cpu_and_optionally_gpu(&mut backend, &store, 160, 160);
    for (side, buf) in std::iter::once(("cpu", &cpu)).chain(gpu.as_ref().map(|g| ("gpu", g))) {
        // Corners of the unrotated AABB around the 30-deg rotated box lie strictly OUTSIDE the
        // rotated box (center=(80,80), rotated half-extents: x-aabb ~49.6, y-aabb ~46.0; point
        // (36, 39) is inside the unrotated AABB but strictly outside the rotated box!).
        for &(ox, oy) in &[(36, 39), (124, 121), (36, 121), (124, 39)] {
            assert_eq!(
                buf.get_rgb(ox, oy),
                Some((BG_CLEAR.r, BG_CLEAR.g, BG_CLEAR.b)),
                "{side}: rotated inset shadow must NOT bleed into AABB corner ({ox},{oy})"
            );
        }
    }
    assert_cpu_gpu_parity_roi(&cpu, gpu.as_ref(), (26, 30, 134, 130), 6.5, "rotated 30deg inset");

    // 2. Rotated + outer shadow parity (offset + blur rotated with box).
    let mut store_out = NodeStore::new();
    let mut ch_out = ChangeSet::default();
    let mut backend_out = SkiaBackend::new();
    backend_out.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 160.0,
            height: 160.0,
            scale_factor: 1.0,
        },
    );
    {
        let mut tx = store_out.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(160.0))
            .height(Length::points(160.0));
        let card = tx.create();
        tx.edit::<Layout>(card)
            .position(Position::Absolute)
            .inset_left(Length::points(45.0))
            .inset_top(Length::points(55.0))
            .width(Length::points(70.0))
            .height(Length::points(50.0));
        tx.edit::<Visual>(card)
            .background(Rgba::WHITE)
            .border_radius(6.0)
            .box_shadows(Some(BoxShadows::new(vec![BoxShadow::new(
                8.0,
                10.0,
                8.0,
                Rgba::new(220, 38, 38, 220),
            )])));
        tx.edit::<Transform>(card).rotate_deg(35.0);
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut ch_out);
    }
    backend_out.commit(&store_out, &ch_out).expect("commit rotated outer");
    let (cpu_o, gpu_o) = render_cpu_and_optionally_gpu(&mut backend_out, &store_out, 160, 160);
    assert_cpu_gpu_parity_roi(&cpu_o, gpu_o.as_ref(), (20, 20, 145, 145), 6.5, "rotated 35deg outer");

    // 3. Non-uniform scale (scale_x = 1.6, scale_y = 0.65) with BOTH outer & inset shadows.
    let mut store_nu = NodeStore::new();
    let mut ch_nu = ChangeSet::default();
    let mut backend_nu = SkiaBackend::new();
    backend_nu.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 200.0,
            height: 140.0,
            scale_factor: 1.0,
        },
    );
    {
        let mut tx = store_nu.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(200.0))
            .height(Length::points(140.0));
        let card = tx.create();
        tx.edit::<Layout>(card)
            .position(Position::Absolute)
            .inset_left(Length::points(60.0))
            .inset_top(Length::points(30.0))
            .width(Length::points(80.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(card)
            .background(Rgba::WHITE)
            .border_radius(8.0)
            .box_shadows(Some(BoxShadows::new(vec![
                BoxShadow::new(0.0, 6.0, 10.0, Rgba::new(220, 38, 38, 210)),
                BoxShadow::new(0.0, 6.0, 10.0, Rgba::new(15, 23, 180, 220)).inset(),
            ])));
        tx.edit::<Transform>(card).scale_x(1.6).scale_y(0.65);
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut ch_nu);
    }
    backend_nu.commit(&store_nu, &ch_nu).expect("commit non-uniform scale");
    let (cpu_nu, gpu_nu) = render_cpu_and_optionally_gpu(&mut backend_nu, &store_nu, 200, 140);
    assert_cpu_gpu_parity_roi(&cpu_nu, gpu_nu.as_ref(), (16, 20, 184, 128), 6.5, "non-uniform scale");
}

#[test]
fn hidpi_2x_fractional_blur_outer_and_inset_cpu_gpu_parity() {
    // At 2x HiDPI:
    // - logical blur = 0.4 -> physical blur = 0.8 > 0.5 -> blurred with phys_sigma = 0.5
    // - logical blur = 0.6 -> physical blur = 1.2 > 0.5 -> blurred with phys_sigma = 0.6
    // - logical blur = 0.8 -> physical blur = 1.6 > 0.5 -> blurred with phys_sigma = 0.8
    // - logical blur = 10.0 -> physical blur = 20.0 -> phys_sigma = 10.0 (not halved on GPU!)
    for blur in [0.4_f32, 0.6, 0.8, 10.0] {
        // Outer shadow @ 2x HiDPI
        let (s_out, mut b_out, _) = build_single_box(BoxSpec {
            canvas: (120, 120),
            scale: 2.0,
            rect: (24.0, 24.0, 72.0, 72.0),
            bg: Rgba::WHITE,
            radius: 8.0,
            border: (0.0, Rgba::TRANSPARENT),
            shadows: vec![BoxShadow::new(0.0, 6.0, blur, Rgba::new(20, 30, 180, 220))],
        });
        let (cpu_o, gpu_o) = render_cpu_and_optionally_gpu(&mut b_out, &s_out, 120, 120);
        let pad = ((blur * 2.0 * 1.5).ceil() as u32).max(6);
        let roi_o = (
            48u32.saturating_sub(pad),
            48u32.saturating_sub(pad),
            (192 + pad).min(240),
            (204 + pad).min(240),
        );
        assert_cpu_gpu_parity_roi(
            &cpu_o,
            gpu_o.as_ref(),
            roi_o,
            6.5,
            &format!("2x HiDPI outer blur={blur}"),
        );

        // Inset shadow @ 2x HiDPI
        let (s_in, mut b_in, _) = build_single_box(BoxSpec {
            canvas: (120, 120),
            scale: 2.0,
            rect: (24.0, 24.0, 72.0, 72.0),
            bg: Rgba::WHITE,
            radius: 8.0,
            border: (0.0, Rgba::TRANSPARENT),
            shadows: vec![BoxShadow::new(0.0, 4.0, blur, Rgba::new(20, 30, 180, 220))
                .spread(2.0)
                .inset()],
        });
        let (cpu_i, gpu_i) = render_cpu_and_optionally_gpu(&mut b_in, &s_in, 120, 120);
        if (blur - 0.4).abs() < 1e-4 {
            // At 2x HiDPI, logical blur=0.4 -> physical blur=0.8 > 0.5 -> MUST be blurred (not crisp!).
            // One physical pixel inside the crisp hole boundary (y=48 + 2*(4+2) = 60): soft transition.
            for (side, buf) in
                std::iter::once(("cpu", &cpu_i)).chain(gpu_i.as_ref().map(|g| ("gpu", g)))
            {
                let edge_luma = luma(buf.get_rgb(120, 60).unwrap());
                assert!(
                    edge_luma > 100.0 && edge_luma < 248.0,
                    "{side}: 2x HiDPI blur=0.4 must cross physical >0.5 cutoff and soften edge (got luma {edge_luma})"
                );
            }
        }
        assert_cpu_gpu_parity_roi(
            &cpu_i,
            gpu_i.as_ref(),
            (46, 46, 194, 194),
            6.0,
            &format!("2x HiDPI inset blur={blur}"),
        );
    }
}

#[test]
fn inset_shadow_group_opacity_nested_rounded_clips_and_pill_radius_parity() {
    // 1. Group opacity < 1.0 (0.55) on card with inset shadow.
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 140.0,
            height: 140.0,
            scale_factor: 1.0,
        },
    );
    {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(140.0))
            .height(Length::points(140.0));
        let card = tx.create();
        tx.edit::<Layout>(card)
            .position(Position::Absolute)
            .inset_left(Length::points(20.0))
            .inset_top(Length::points(20.0))
            .width(Length::points(100.0))
            .height(Length::points(100.0));
        tx.edit::<Visual>(card)
            .background(Rgba::WHITE)
            .border_radius(12.0)
            .opacity(0.55)
            .box_shadows(Some(BoxShadows::new(vec![BoxShadow::new(
                0.0,
                6.0,
                10.0,
                Rgba::new(15, 23, 180, 220),
            )
            .inset()])));
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("commit opacity inset");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 140, 140, "opacity inset");
    let (cpu_op, gpu_op) = render_cpu_and_optionally_gpu(&mut backend, &store, 140, 140);
    // Non-overlapping single translucent node with inset shadow: CPU push_mul vs Skia save_layer
    // differs slightly where translucent shadow blends over translucent white background (group
    // alpha compositing vs per-op alpha), so check edge darkening + bounded ROI error.
    for (side, buf) in std::iter::once(("cpu", &cpu_op)).chain(gpu_op.as_ref().map(|g| ("gpu", g))) {
        assert!(
            luma(buf.get_rgb(70, 24).unwrap()) < luma(buf.get_rgb(70, 70).unwrap()) - 15.0,
            "{side}: opacity<1 inset shadow must still darken top edge relative to center"
        );
    }
    assert_cpu_gpu_parity_roi(&cpu_op, gpu_op.as_ref(), (19, 19, 121, 121), 14.0, "opacity<1 inset");

    // 2. Nested rounded clips clipping a child with inset shadow.
    let mut store_cl = NodeStore::new();
    let mut ch_cl = ChangeSet::default();
    let mut backend_cl = SkiaBackend::new();
    backend_cl.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 140.0,
            height: 140.0,
            scale_factor: 1.0,
        },
    );
    {
        let mut tx = store_cl.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(140.0))
            .height(Length::points(140.0));
        let outer = tx.create();
        tx.edit::<Layout>(outer)
            .position(Position::Absolute)
            .inset_left(Length::points(20.0))
            .inset_top(Length::points(20.0))
            .width(Length::points(100.0))
            .height(Length::points(100.0));
        tx.edit::<Visual>(outer)
            .border_radius(36.0)
            .overflow_x(Overflow::Hidden)
            .overflow_y(Overflow::Hidden);

        let mid = tx.create();
        tx.edit::<Layout>(mid)
            .position(Position::Absolute)
            .inset_left(Length::points(10.0))
            .inset_top(Length::points(10.0))
            .width(Length::points(80.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(mid)
            .border_radius(24.0)
            .overflow_x(Overflow::Hidden)
            .overflow_y(Overflow::Hidden);

        let leaf = tx.create();
        tx.edit::<Layout>(leaf)
            .width(Length::points(80.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(leaf)
            .background(Rgba::WHITE)
            .border_radius(10.0)
            .box_shadows(Some(BoxShadows::new(vec![BoxShadow::new(
                0.0,
                0.0,
                10.0,
                Rgba::new(0, 0, 0, 220),
            )
            .spread(6.0)
            .inset()])));

        tx.set_children(mid, &[leaf]);
        tx.set_children(outer, &[mid]);
        tx.set_children(NodeId::ROOT, &[outer]);
        tx.commit_into(&mut ch_cl);
    }
    backend_cl.commit(&store_cl, &ch_cl).expect("commit nested rounded clips");
    assert_scene_matches_fresh_display_list(&mut backend_cl, &store_cl, 140, 140, "nested clips");
    let (cpu_cl, gpu_cl) = render_cpu_and_optionally_gpu(&mut backend_cl, &store_cl, 140, 140);
    for (side, buf) in std::iter::once(("cpu", &cpu_cl)).chain(gpu_cl.as_ref().map(|g| ("gpu", g))) {
        // Outer rounded clip corner (32, 32) lies inside leaf's square bounds [30..110] but
        // outside `mid`'s radius=24 rounded clip -> must remain clear background!
        assert_eq!(
            buf.get_rgb(32, 32),
            Some((BG_CLEAR.r, BG_CLEAR.g, BG_CLEAR.b)),
            "{side}: nested rounded clip must clip inset shadow at corner (32,32)"
        );
    }
    assert_cpu_gpu_parity_roi(&cpu_cl, gpu_cl.as_ref(), (29, 29, 111, 111), 5.0, "nested rounded clips");

    // 3. Pill/circle clamping: `border_radius = 120.0 > half_extent (25.0)` on 100x50 box with border.
    let (s_pill, mut b_pill, _) = build_single_box(BoxSpec {
        canvas: (140, 90),
        scale: 1.0,
        rect: (20.0, 20.0, 100.0, 50.0),
        bg: Rgba::WHITE,
        radius: 120.0,
        border: (4.0, Rgba::rgb(16, 185, 129)),
        shadows: vec![BoxShadow::new(0.0, 4.0, 8.0, Rgba::new(15, 23, 180, 220)).inset()],
    });
    assert_scene_matches_fresh_display_list(&mut b_pill, &s_pill, 140, 90, "pill radius inset");
    let (cpu_p, gpu_p) = render_cpu_and_optionally_gpu(&mut b_pill, &s_pill, 140, 90);
    for (side, buf) in std::iter::once(("cpu", &cpu_p)).chain(gpu_p.as_ref().map(|g| ("gpu", g))) {
        // Outside rounded pill corner at (22, 22) must stay clear background.
        assert_eq!(
            buf.get_rgb(22, 22),
            Some((BG_CLEAR.r, BG_CLEAR.g, BG_CLEAR.b)),
            "{side}: pill inset shadow must not bleed past clamped pill corner"
        );
    }
    assert_cpu_gpu_parity_roi(&cpu_p, gpu_p.as_ref(), (19, 19, 121, 71), 5.5, "pill radius > half_extent");
}
