//! Seeded random mutation oracle tests proving exact pixel-for-pixel equivalence between
//! incremental [`CompositorScene`] replay and a fresh [`SkiaBackend::rebuild_display_list`] on
//! both pure-Rust software rasterization and CPU Skia (`--features skia`).

use nve_abi::{ColorRgba, SurfaceId};
use nve_engine::{LayoutRect, StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::paint::{BackgroundFill, GradientStop};
use nve_tables::tables::{
    BackgroundFillRes, BoxShadow, BoxShadows, Display, FlexDirection, Layout, Overflow, Position,
    Scroll, TextContent, TextInput, Transform, Typography, Visibility, Visual,
};

use super::SceneDirtyKind;
use crate::raster::{rasterize_display_list_with_plugins, PixelBuffer};
use crate::SkiaBackend;

struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn range(&mut self, bound: usize) -> usize {
        (self.next() as usize) % bound.max(1)
    }

    fn f32_01(&mut self) -> f32 {
        ((self.next() & 0xFFFF) as f32) / 65535.0
    }
}

pub(super) fn physical_dims(logical_w: u32, logical_h: u32, scale: f32) -> (u32, u32) {
    let s = crate::text_cache::sanitize_scale(scale);
    (
        ((logical_w as f32) * s).ceil().max(1.0) as u32,
        ((logical_h as f32) * s).ceil().max(1.0) as u32,
    )
}

pub(super) fn slice_rect_pixels(buf: &PixelBuffer, rect: LayoutRect, scale: f32) -> Vec<u32> {
    let s = crate::text_cache::sanitize_scale(scale);
    let x0 = ((rect.x * s).floor().max(0.0) as u32).min(buf.width());
    let y0 = ((rect.y * s).floor().max(0.0) as u32).min(buf.height());
    let x1 = (((rect.x + rect.width) * s).ceil().max(0.0) as u32).min(buf.width());
    let y1 = (((rect.y + rect.height) * s).ceil().max(0.0) as u32).min(buf.height());
    let mut out = Vec::new();
    let stride = buf.width() as usize;
    let pixels = buf.pixels();
    for y in y0..y1 {
        let row = (y as usize) * stride;
        out.extend_from_slice(&pixels[row + (x0 as usize)..row + (x1 as usize)]);
    }
    out
}

/// Asserts exact pixel-for-pixel equivalence between the live incremental [`CompositorScene`]
/// and a freshly compiled ground-truth `DisplayList` on BOTH pure-Rust software rasterization
/// and CPU Skia (`--features skia`), sized at full physical resolution `ceil(w * scale) x ceil(h * scale)`.
pub(super) fn assert_scene_matches_fresh_display_list(
    backend: &mut SkiaBackend,
    store: &NodeStore,
    logical_w: u32,
    logical_h: u32,
    step_label: &str,
) {
    let clear = ColorRgba::rgb(11, 17, 32);
    let scale = backend.viewport().scale_factor;
    let (pw, ph) = physical_dims(logical_w, logical_h, scale);

    let mut scene_buf = PixelBuffer::new(pw, ph);
    backend.rasterize_with_store(store, &mut scene_buf, clear);

    let oracle_dl = backend.rebuild_display_list(store).clone();
    let mut oracle_buf = PixelBuffer::new(pw, ph);
    let text_engine = backend.text_engine().clone();
    let (_, registry, events) = backend.scene_paint_parts_mut();
    rasterize_display_list_with_plugins(
        &oracle_dl,
        &mut oracle_buf,
        scale,
        clear,
        text_engine.as_ref(),
        store,
        registry,
        events,
    );

    assert_eq!(
        scene_buf.pixels(),
        oracle_buf.pixels(),
        "software pixel mismatch against fresh DisplayList oracle at {step_label} ({pw}x{ph} @{scale}x)"
    );

    #[cfg(feature = "skia")]
    {
        use crate::gpu::{CpuSkiaSurface, GpuSurfaceBackend};
        let mut surf_scene =
            CpuSkiaSurface::new_with_scale(pw, ph, scale).expect("cpu skia scene");
        let mut surf_oracle =
            CpuSkiaSurface::new_with_scale(pw, ph, scale).expect("cpu skia oracle");

        {
            let (scene, reg, ev) = backend.scene_paint_parts_mut();
            surf_scene
                .render_scene_cached_with_plugins(scene, scale, clear, Some((store, reg, ev)))
                .expect("render scene");
        }
        {
            let (_, reg, ev) = backend.scene_paint_parts_mut();
            surf_oracle
                .render_cached_with_plugins(&oracle_dl, scale, clear, Some((store, reg, ev)))
                .expect("render oracle");
        }

        let mut skia_scene_buf = PixelBuffer::new(pw, ph);
        let mut skia_oracle_buf = PixelBuffer::new(pw, ph);
        surf_scene
            .copy_to_pixel_buffer(&mut skia_scene_buf)
            .expect("readback scene");
        surf_oracle
            .copy_to_pixel_buffer(&mut skia_oracle_buf)
            .expect("readback oracle");

        assert_eq!(
            skia_scene_buf.pixels(),
            skia_oracle_buf.pixels(),
            "CPU Skia pixel mismatch against fresh DisplayList oracle at {step_label} ({pw}x{ph} @{scale}x)"
        );
    }
}

#[test]
fn seeded_random_mutations_match_fresh_display_list_pixel_for_pixel() {
    let mut rng = XorShift64(0xDEAD_BEEF_CAFE_BABEu64);
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    let (w, h) = (260u32, 200u32);
    let scale = 1.5f32;
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: w as f32,
            height: h as f32,
            scale_factor: scale,
        },
    );

    // Rich tree at 1.5x HiDPI (390x300 physical): scroll container (with sticky header + cards +
    // scrolled TextInput) + nested typography parent + bottom TextInput.
    let (scroll_id, scrolled_input_id, input_id, typo_parent, mut cards) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(w as f32))
            .height(Length::points(h as f32))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Visual>(NodeId::ROOT)
            .background(Rgba::rgb(15, 23, 42));

        let scroll = tx.create();
        tx.edit::<Layout>(scroll)
            .width(Length::points(w as f32))
            .height(Length::points(140.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(scroll).virtual_total_height(Some(460.0));
        tx.edit::<Visual>(scroll)
            .overflow_y(Overflow::Scroll)
            .border_radius(8.0);

        let sticky = tx.create();
        tx.edit::<Layout>(sticky)
            .position(Position::Sticky)
            .inset_top(Length::points(0.0))
            .width(Length::points(240.0))
            .height(Length::points(22.0));
        tx.edit::<Visual>(sticky)
            .background(Rgba::rgb(30, 58, 138))
            .z_index(10);
        tx.edit::<TextContent>(sticky).text("Sticky Header");
        tx.edit::<Typography>(sticky)
            .color(Rgba::WHITE)
            .font_size(12.0);

        let scrolled_input = tx.create();
        tx.edit::<Layout>(scrolled_input)
            .width(Length::points(220.0))
            .height(Length::points(26.0))
            .margin_top(Length::points(4.0));
        tx.edit::<Visual>(scrolled_input)
            .background(Rgba::rgb(30, 41, 59))
            .border_radius(4.0);
        tx.edit::<TextInput>(scrolled_input)
            .value("Inside Scroll")
            .placeholder("Type here");
        tx.edit::<Typography>(scrolled_input)
            .color(Rgba::WHITE)
            .font_size(12.0);

        let mut scroll_children = vec![sticky, scrolled_input];
        let mut cards = Vec::new();
        for i in 0..8usize {
            let card = tx.create();
            tx.edit::<Layout>(card)
                .width(Length::points(220.0))
                .height(Length::points(36.0))
                .margin_top(Length::points(4.0));
            tx.edit::<Visual>(card)
                .background(Rgba::rgb(30 + (i as u8) * 15, 60, 110))
                .border_radius(6.0)
                .border_width(1.0)
                .border_color(Rgba::rgb(148, 163, 184));
            if i == 1 {
                tx.edit::<Visual>(card)
                    .background_fill(Some(BackgroundFillRes(BackgroundFill::linear(
                        90.0,
                        vec![
                            GradientStop::new(0.0, ColorRgba::rgb(59, 130, 246)),
                            GradientStop::new(1.0, ColorRgba::rgb(16, 185, 129)),
                        ],
                    ))));
            }
            if i == 2 {
                tx.edit::<Visual>(card).box_shadows(Some(BoxShadows::new(vec![
                    BoxShadow::new(2.0, 3.0, 4.0, Rgba::new(0, 0, 0, 140)),
                ])));
            }
            if i == 3 {
                tx.edit::<Visual>(card).box_shadows(Some(BoxShadows::new(vec![
                    BoxShadow::new(1.0, 2.0, 3.0, Rgba::new(0, 0, 0, 150)).inset(),
                ])));
            }
            tx.edit::<TextContent>(card).text(&format!("Card #{i}"));
            scroll_children.push(card);
            cards.push(card);
        }
        tx.set_children(scroll, &scroll_children);

        let typo_parent = tx.create();
        tx.edit::<Layout>(typo_parent)
            .width(Length::points(240.0))
            .height(Length::points(26.0))
            .flex_direction(FlexDirection::Row);
        tx.edit::<Typography>(typo_parent)
            .color(Rgba::rgb(250, 204, 21))
            .font_size(13.0);

        let typo_inherit_child = tx.create();
        tx.edit::<Layout>(typo_inherit_child)
            .width(Length::points(110.0))
            .height(Length::points(24.0));
        tx.edit::<TextContent>(typo_inherit_child).text("Inherited");

        let typo_override_child = tx.create();
        tx.edit::<Layout>(typo_override_child)
            .width(Length::points(110.0))
            .height(Length::points(24.0));
        tx.edit::<Typography>(typo_override_child)
            .color(Rgba::rgb(244, 63, 94))
            .font_size(11.0);
        tx.edit::<TextContent>(typo_override_child).text("Override");
        tx.set_children(typo_parent, &[typo_inherit_child, typo_override_child]);

        let input = tx.create();
        tx.edit::<Layout>(input)
            .width(Length::points(220.0))
            .height(Length::points(28.0));
        tx.edit::<Visual>(input)
            .background(Rgba::rgb(30, 41, 59))
            .border_radius(4.0);
        tx.edit::<TextInput>(input).value("Hello NVE");
        tx.edit::<Typography>(input)
            .color(Rgba::WHITE)
            .font_size(13.0);

        tx.set_children(NodeId::ROOT, &[scroll, typo_parent, input]);
        tx.commit_into(&mut changes);
        (scroll, scrolled_input, input, typo_parent, cards)
    };

    backend.commit(&store, &changes).expect("initial commit");
    assert_scene_matches_fresh_display_list(&mut backend, &store, w, h, "initial");

    // Liveness probes: prove `input`, `scrolled_input`, and `typo_parent` are actually inside the
    // physical 390x300 buffer and change pixels on focus, edit, scroll, and inherited color update!
    let (pw, ph) = physical_dims(w, h, scale);
    let clear = ColorRgba::rgb(11, 17, 32);
    let input_rect = backend.layout_of(input_id).expect("input laid out");
    let scrolled_rect = backend
        .layout_of(scrolled_input_id)
        .expect("scrolled input laid out");
    let typo_rect = backend
        .layout_of(typo_parent)
        .expect("typo_parent laid out");
    assert!(
        ((input_rect.y + input_rect.height) * scale) <= ph as f32 + 1.0,
        "bottom TextInput must be fully inside physical buffer ({ph}px)"
    );

    let mut buf_a = PixelBuffer::new(pw, ph);
    let mut buf_b = PixelBuffer::new(pw, ph);
    backend.rasterize_with_store(&store, &mut buf_a, clear);
    backend.set_focused_node(&store, Some(input_id));
    backend.rasterize_with_store(&store, &mut buf_b, clear);
    assert_ne!(
        slice_rect_pixels(&buf_a, input_rect, scale),
        slice_rect_pixels(&buf_b, input_rect, scale),
        "bottom TextInput pixels must change between unfocused and focused at 1.5x"
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, w, h, "liveness focus bottom");

    // Verify TextInput value edit uses `PaintOnly` (not `LayoutOrTree`) and changes pixels.
    {
        let mut tx = store.tx();
        tx.edit::<TextInput>(input_id).value("Edited 123");
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("text input edit");
    assert_eq!(
        backend.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::PaintOnly),
        "TextInput value-only edit must use PaintOnly fast path"
    );
    backend.rasterize_with_store(&store, &mut buf_a, clear);
    assert_ne!(
        slice_rect_pixels(&buf_b, input_rect, scale),
        slice_rect_pixels(&buf_a, input_rect, scale),
        "bottom TextInput pixels must change on value edit"
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, w, h, "liveness edit bottom");

    // Verify scrolled TextInput focus + scroll shifts pixels cleanly.
    backend.set_focused_node(&store, Some(scrolled_input_id));
    backend.rasterize_with_store(&store, &mut buf_a, clear);
    backend.set_scroll_offset(&store, scroll_id, 0.0, 12.5);
    backend.rasterize_with_store(&store, &mut buf_b, clear);
    assert_ne!(
        slice_rect_pixels(&buf_a, scrolled_rect, scale),
        slice_rect_pixels(&buf_b, scrolled_rect, scale),
        "scrolled TextInput pixels must change on scroll"
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, w, h, "liveness scrolled input");

    // Verify typo_parent pixels are visible and change on inherited color edit.
    backend.rasterize_with_store(&store, &mut buf_a, clear);
    {
        let mut tx = store.tx();
        tx.edit::<Typography>(typo_parent)
            .color(Rgba::rgb(34, 211, 238));
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("liveness typo");
    backend.rasterize_with_store(&store, &mut buf_b, clear);
    assert_ne!(
        slice_rect_pixels(&buf_a, typo_rect, scale),
        slice_rect_pixels(&buf_b, typo_rect, scale),
        "typo_parent pixels must be visible inside physical buffer and update on color change"
    );

    // Run 48 seeded random mutations spanning EVERY dirty tier.
    for step in 0..48usize {
        match rng.range(11) {
            0 => {
                let sy = rng.f32_01() * 240.0;
                backend.set_scroll_offset(&store, scroll_id, 0.0, sy);
            }
            1 => {
                let target = cards[rng.range(cards.len())];
                let op = 0.2 + rng.f32_01() * 0.8;
                let mut tx = store.tx();
                tx.edit::<Visual>(target).opacity(op);
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("opacity commit");
                if !changes.is_empty() {
                    assert_eq!(
                        backend.scene().stats().last_dirty_kind,
                        Some(SceneDirtyKind::CompositeOnly),
                        "step {step} opacity-only edit must use CompositeOnly fast path"
                    );
                }
            }
            2 => {
                let target = cards[rng.range(cards.len())];
                let angle = (rng.f32_01() - 0.5) * 16.0;
                let tx_x = (rng.f32_01() - 0.5) * 10.0;
                let mut tx = store.tx();
                tx.edit::<Transform>(target)
                    .translate_x(tx_x)
                    .rotate_deg(angle);
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("transform commit");
                if !changes.is_empty() {
                    assert_eq!(
                        backend.scene().stats().last_dirty_kind,
                        Some(SceneDirtyKind::CompositeOnly),
                        "step {step} transform-only edit must use CompositeOnly fast path"
                    );
                }
            }
            3 => {
                let target = cards[rng.range(cards.len())];
                let cur_z = store.try_row::<Visual>(target).map_or(0, |v| v.z_index());
                let mut z = (rng.range(7) as i32) - 3;
                if z == cur_z {
                    z = if cur_z == 3 { -3 } else { cur_z + 1 };
                }
                let mut tx = store.tx();
                tx.edit::<Visual>(target).z_index(z);
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("z_index commit");
                assert_eq!(
                    backend.scene().stats().last_dirty_kind,
                    Some(SceneDirtyKind::CompositeOnly)
                );
            }
            4 => {
                let target = cards[rng.range(cards.len())];
                let r = (rng.next() & 0xFF) as u8;
                let g = ((rng.next() >> 8) & 0xFF) as u8;
                let b = ((rng.next() >> 16) & 0xFF) as u8;
                let mut tx = store.tx();
                tx.edit::<Visual>(target)
                    .background(Rgba::rgb(r, g, b))
                    .border_radius((rng.range(10)) as f32);
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("paint commit");
                if !changes.is_empty() {
                    assert_eq!(
                        backend.scene().stats().last_dirty_kind,
                        Some(SceneDirtyKind::PaintOnly)
                    );
                }
            }
            5 => {
                let c = Rgba::rgb(
                    100 + (rng.range(155) as u8),
                    100 + (rng.range(155) as u8),
                    220,
                );
                let mut tx = store.tx();
                tx.edit::<Typography>(typo_parent).color(c);
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("typo commit");
            }
            6 => {
                let vis = match rng.range(3) {
                    0 => Visibility::Hidden,
                    1 => Visibility::Visible,
                    _ => Visibility::Inherit,
                };
                let target = if rng.range(2) == 0 {
                    typo_parent
                } else {
                    cards[rng.range(cards.len())]
                };
                let mut tx = store.tx();
                tx.edit::<Visual>(target).visibility(vis);
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("vis commit");
            }
            7 => {
                let target_input = if rng.range(2) == 0 {
                    input_id
                } else {
                    scrolled_input_id
                };
                match rng.range(4) {
                    0 => backend.set_focused_node(&store, Some(target_input)),
                    1 => backend.set_caret_and_selection(&store, 2, 6),
                    2 => backend.set_ime_preedit(&store, Some("あ".into()), Some((0, 3))),
                    _ => backend.set_focused_node(&store, None),
                }
            }
            8 => {
                let target = cards[rng.range(cards.len())];
                let disp = if rng.range(2) == 0 {
                    Display::None
                } else {
                    Display::Flex
                };
                let mut tx = store.tx();
                tx.edit::<Layout>(target).display(disp);
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("display commit");
            }
            9 => {
                let mut tx = store.tx();
                tx.edit::<TextInput>(scrolled_input_id)
                    .value(&format!("Step {step}"));
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("scrolled input edit");
                assert_eq!(
                    backend.scene().stats().last_dirty_kind,
                    Some(SceneDirtyKind::PaintOnly)
                );
            }
            _ => {
                if cards.len() < 12 && rng.range(2) == 0 {
                    let mut kids = store.children(scroll_id).to_vec();
                    let mut tx = store.tx();
                    let new_card = tx.create();
                    tx.edit::<Layout>(new_card)
                        .width(Length::points(220.0))
                        .height(Length::points(28.0));
                    tx.edit::<Visual>(new_card).background(Rgba::rgb(99, 102, 241));
                    tx.edit::<TextContent>(new_card).text("Dynamic");
                    kids.push(new_card);
                    tx.set_children(scroll_id, &kids);
                    tx.commit_into(&mut changes);
                    cards.push(new_card);
                    backend.commit(&store, &changes).expect("add card");
                } else if cards.len() > 4 {
                    let rem = cards.pop().expect("cards > 4");
                    let mut kids = store.children(scroll_id).to_vec();
                    kids.retain(|&c| c != rem);
                    let mut tx = store.tx();
                    tx.set_children(scroll_id, &kids);
                    tx.remove(rem);
                    tx.commit_into(&mut changes);
                    backend.commit(&store, &changes).expect("rem card");
                }
            }
        }
        assert_scene_matches_fresh_display_list(
            &mut backend,
            &store,
            w,
            h,
            &format!("random step {step}"),
        );
    }
}
