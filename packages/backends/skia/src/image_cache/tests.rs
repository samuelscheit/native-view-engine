use std::sync::Arc;

use nve_abi::ColorRgba;
use nve_components::tables::ImageFit;
use nve_engine::LayoutRect;

use super::png::{decode_pass, PngHeader};
use super::png_rows::{paeth_predictor, unfilter_scanline};
use super::*;

const AVATAR_MAYA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";
const AVATAR_LIAM: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAg9gVSsGEXB8u5fKHggFOB1JD4MAGcqfXUz+ORAAAAAAElFTkSuQmCC";
const AVATAR_SARA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAhWPTGsGEWB0tE4FDwQCvA6Eh8GAE5RZ/09pCRNAAAAAElFTkSuQmCC";
const AVATAR_DEV: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAHElEQVQY02OQ07T+jw8zEK1gyuIdKHggFJDtCwAdM3cR3wnOlQAAAABJRU5ErkJggg==";
const AVATAR_ARIA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAhXKWasGEXB13ncKHggFOB1JD4MAEs7b4ERxIxeAAAAAElFTkSuQmCC";
const AVATAR_NOAH: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEVB7eU/KHggFOB1JD4MAL/Ggglv5kNyAAAAAElFTkSuQmCC";
const AVATAR_YOU: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAh+cS2sGEXBj19/UPBAKMDrSHwYAFnZgxk5UYNxAAAAAElFTkSuQmCC";

#[test]
fn decodes_all_studio_workspace_png_avatars_to_8x8_rgba() {
    let cache = ImageCache::default();
    for uri in [
        AVATAR_MAYA,
        AVATAR_LIAM,
        AVATAR_SARA,
        AVATAR_DEV,
        AVATAR_ARIA,
        AVATAR_NOAH,
        AVATAR_YOU,
    ] {
        let img = cache.get_or_decode(uri).expect("valid Studio avatar PNG");
        assert_eq!(img.width, 8);
        assert_eq!(img.height, 8);
        assert_eq!(img.rgba.len(), 8 * 8 * 4);
        let opaque_non_black = img
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0))
            .count();
        assert!(
            opaque_non_black >= 16,
            "expected rich avatar pixels, got {opaque_non_black}"
        );
        let hit = cache.get_or_decode(uri).expect("cached avatar");
        assert!(Arc::ptr_eq(&img, &hit));
    }
    assert_eq!(cache.len(), 7);
}

#[test]
fn decodes_file_uri_and_raw_rgba_and_evicts_lru() {
    let raw_png = decode_base64(
        AVATAR_MAYA
            .strip_prefix("data:image/png;base64,")
            .unwrap(),
    )
    .unwrap();
    let tmp_path = std::env::temp_dir().join(format!(
        "nve_image_cache_test_{}.png",
        std::process::id()
    ));
    std::fs::write(&tmp_path, &raw_png).unwrap();
    let file_uri = format!("file://{}", tmp_path.display());

    let cache = ImageCache::new(2, 1024);
    let from_file = cache.get_or_decode(&file_uri).expect("decode file:// PNG");
    assert_eq!((from_file.width, from_file.height), (8, 8));

    let raw = cache
        .insert_raw_rgba(
            "raw://2x2",
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        )
        .expect("insert raw RGBA");
    assert_eq!(raw.pixel_clamped(0, 0), ColorRgba::rgb(255, 0, 0));
    assert_eq!(raw.pixel_clamped(1, 0), ColorRgba::rgb(0, 255, 0));

    // Touch file_uri so raw://2x2 becomes least-recently-used.
    assert!(cache.get(&file_uri).is_some());
    let _third = cache.get_or_decode(AVATAR_SARA).expect("third image");
    assert_eq!(cache.len(), 2);
    assert!(cache.get(&file_uri).is_some());
    assert!(cache.get(AVATAR_SARA).is_some());
    assert!(cache.get("raw://2x2").is_none());

    let _ = std::fs::remove_file(&tmp_path);
}

#[test]
fn lru_eviction_order_and_byte_budget_bounds_hold_under_churn() {
    // 4-entry / 64-byte budget; each 2x2 RGBA image is 16 bytes.
    let cache = ImageCache::new(4, 64);
    for i in 0..4u32 {
        cache
            .insert_raw_rgba(format!("k{i}"), 2, 2, vec![i as u8; 16])
            .unwrap();
    }
    assert_eq!(cache.len(), 4);
    assert_eq!(cache.total_bytes(), 64);

    // Touch k0 and k1 so eviction order becomes k2, k3, k0, k1.
    assert!(cache.get("k0").is_some());
    assert!(cache.get("k1").is_some());

    // Overwrite k1 with a 4x2 (32-byte) image: removes old k1 (16B), leaving 48B; evicts k2 (16B)
    // so k3 (16B) + k0 (16B) + k1 (32B) = 64B fits exact byte budget.
    cache.insert_raw_rgba("k1", 4, 2, vec![99; 32]).unwrap();
    assert!(cache.get("k2").is_none(), "oldest untouched entry k2 evicted");
    assert_eq!(cache.total_bytes(), 64);
    assert!(cache.get("k0").is_some());
    assert!(cache.get("k1").is_some());

    // Inserting another 16B entry must now evict k3 (the oldest remaining untouched entry).
    cache.insert_raw_rgba("k4", 2, 2, vec![4; 16]).unwrap();
    assert!(cache.get("k3").is_none(), "next oldest k3 evicted for byte budget");
    assert!(cache.get("k0").is_some());
    assert!(cache.get("k1").is_some());
    assert!(cache.get("k4").is_some());
    assert!(cache.total_bytes() <= 64);

    // Oversized single entry (> max_bytes) succeeds for caller without corrupting cache bounds.
    let huge = cache.insert_raw_rgba("huge", 8, 8, vec![1; 256]).unwrap();
    assert_eq!(huge.byte_len(), 256);
    assert!(cache.get("huge").is_none());
    assert!(cache.total_bytes() <= 64);

    // High-churn O(log n) stress test: 1000 inserts into a 32-entry cache.
    let big = ImageCache::new(32, 32 * 16);
    for i in 0..1000u32 {
        big.insert_raw_rgba(format!("item-{i}"), 2, 2, vec![0; 16])
            .unwrap();
        assert!(big.len() <= 32);
        assert!(big.total_bytes() <= 32 * 16);
    }
    assert_eq!(big.len(), 32);
    for i in 0..968u32 {
        assert!(big.get(&format!("item-{i}")).is_none());
    }
    for i in 968..1000u32 {
        assert!(big.get(&format!("item-{i}")).is_some());
    }
}

#[test]
fn png_unfilter_per_filter_row_loops_match_rfc2083_reference_across_all_bpps() {
    fn reference_unfilter(filter: u8, src: &[u8], prev: &[u8], bpp: usize) -> Vec<u8> {
        let mut dst = vec![0u8; src.len()];
        for i in 0..src.len() {
            let a = if i >= bpp { dst[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let pred = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) >> 1) as u8,
                4 => paeth_predictor(a, b, c),
                _ => unreachable!(),
            };
            dst[i] = src[i].wrapping_add(pred);
        }
        dst
    }

    for bpp in [1usize, 2, 3, 4, 6, 8] {
        for len in [0usize, 1, bpp.saturating_sub(1), bpp, bpp * 7 + 3] {
            let src: Vec<u8> = (0..len).map(|i| ((i * 37 + 13) & 0xFF) as u8).collect();
            let prev: Vec<u8> = (0..len).map(|i| ((i * 53 + 7) & 0xFF) as u8).collect();
            for filter in 0u8..=4 {
                let expected = reference_unfilter(filter, &src, &prev, bpp);
                let mut actual = vec![0u8; len];
                unfilter_scanline(filter, &src, &prev, &mut actual, bpp).unwrap();
                assert_eq!(
                    actual, expected,
                    "mismatch for filter={filter} bpp={bpp} len={len}"
                );
            }
            let mut dst = vec![0u8; len];
            assert!(unfilter_scanline(5, &src, &prev, &mut dst, bpp).is_err());
        }
    }
}

#[test]
fn rejects_corrupt_and_truncated_pngs_without_panicking() {
    let valid_png = decode_base64(
        AVATAR_MAYA
            .strip_prefix("data:image/png;base64,")
            .unwrap(),
    )
    .unwrap();
    for cut in 0..valid_png.len() {
        assert!(decode_png(&valid_png[..cut]).is_err());
    }
    let mut flipped = valid_png.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0xFF;
    assert!(decode_png(&flipped).is_err());

    assert!(decode_image_uri("").is_err());
    assert!(decode_image_uri("data:image/png;base64,!!!not-base64!!!").is_err());
    assert!(decode_image_uri("https://example.com/nope.png").is_err());
    assert!(DecodedImage::from_rgba(0, 10, vec![]).is_err());
    assert!(DecodedImage::from_rgba(2, 2, vec![0; 4]).is_err());
}

#[test]
fn computes_exact_placement_for_all_four_resize_modes() {
    let dst = LayoutRect {
        x: 10.0,
        y: 20.0,
        width: 100.0,
        height: 50.0,
    };
    let stretch = compute_image_placement(dst, 40, 40, ImageFit::Fill).unwrap();
    assert_eq!(
        (stretch.dst_x, stretch.dst_y, stretch.dst_w, stretch.dst_h),
        (10.0, 20.0, 100.0, 50.0)
    );
    assert_eq!(
        (stretch.src_x, stretch.src_y, stretch.src_w, stretch.src_h),
        (0.0, 0.0, 40.0, 40.0)
    );

    let contain = compute_image_placement(dst, 40, 40, ImageFit::Contain).unwrap();
    assert_eq!(
        (contain.dst_x, contain.dst_y, contain.dst_w, contain.dst_h),
        (35.0, 20.0, 50.0, 50.0)
    );

    let cover = compute_image_placement(dst, 40, 40, ImageFit::Cover).unwrap();
    assert_eq!(
        (cover.dst_x, cover.dst_y, cover.dst_w, cover.dst_h),
        (10.0, 20.0, 100.0, 50.0)
    );
    assert!((cover.src_y - 10.0).abs() < 1e-4 && (cover.src_h - 20.0).abs() < 1e-4);

    let center = compute_image_placement(dst, 40, 40, ImageFit::None).unwrap();
    assert_eq!(
        (center.dst_x, center.dst_y, center.dst_w, center.dst_h),
        (40.0, 25.0, 40.0, 40.0)
    );

    let center_crop = compute_image_placement(dst, 200, 100, ImageFit::None).unwrap();
    assert_eq!(
        (
            center_crop.dst_x,
            center_crop.dst_y,
            center_crop.dst_w,
            center_crop.dst_h
        ),
        (10.0, 20.0, 100.0, 50.0)
    );
    assert_eq!(
        (
            center_crop.src_x,
            center_crop.src_y,
            center_crop.src_w,
            center_crop.src_h
        ),
        (50.0, 25.0, 100.0, 50.0)
    );
}

#[test]
fn adam7_and_pass_indexing_reject_overflow_and_out_of_bounds_cleanly() {
    let hdr = PngHeader {
        width: 4,
        height: 4,
        bit_depth: 8,
        color_type: 6,
        interlace: 1,
    };
    let mut rgba = vec![0u8; 4 * 4 * 4];
    let one_row = vec![0u8; 1 + 4];

    assert!(decode_pass(hdr, &one_row, None, None, 0, u32::MAX, 1, 1, 1, 1, &mut rgba).is_err());
    assert!(decode_pass(hdr, &[0u8; 10], None, None, 0, 0, 1, u32::MAX, 1, 2, &mut rgba).is_err());
    assert!(decode_pass(hdr, &one_row, None, None, 0, 4, 1, 1, 1, 1, &mut rgba).is_err());
    assert!(decode_pass(hdr, &one_row, None, None, u32::MAX, 0, 1, 1, 1, 1, &mut rgba).is_err());
    assert!(decode_pass(hdr, &one_row, None, None, 4, 0, 1, 1, 1, 1, &mut rgba).is_err());
    assert!(decode_pass(hdr, &[], None, None, 0, 0, 1, 1, 1, 1, &mut rgba).is_err());
    let mut tiny = vec![0u8; 2];
    assert!(decode_pass(hdr, &one_row, None, None, 0, 0, 1, 1, 1, 1, &mut tiny).is_err());
}
