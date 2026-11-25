use nve_abi::{ColorRgba, RawPropValue};

use super::types::DrawOpcode;
use super::*;

#[test]
fn roundtrip_every_canvas_op_and_raw_prop_formats() {
    let mut builder = DrawListBuilder::new();
    builder
        .clear(ColorRgba::rgba(11, 17, 32, 255))
        .clip_rect(4.0, 4.0, 200.0, 80.0)
        .fill_rect(8.0, 12.0, 40.0, 24.0, ColorRgba::rgb(30, 41, 59))
        .fill_rounded_rect(10.0, 10.0, 180.0, 60.0, 12.0, ColorRgba::rgb(15, 23, 42))
        .stroke_line(
            12.0,
            40.0,
            188.0,
            40.0,
            1.5,
            ColorRgba::rgba(148, 163, 184, 128),
        )
        .stroke_polyline(
            &[(12.0, 50.0), (60.0, 32.0), (120.0, 42.0), (188.0, 18.0)],
            2.5,
            ColorRgba::rgb(56, 189, 248),
        )
        .fill_circle(188.0, 18.0, 4.0, ColorRgba::rgb(34, 197, 94))
        .restore_clip();

    assert_eq!(builder.op_count(), 8);
    let attr = builder.clone().to_attr_string();
    let raw_prop = builder.clone().to_raw_prop();
    let bytes = builder.finish();

    assert!(bytes.starts_with(&NVE_DRAW_MAGIC));
    let header = DrawListDecoder::decode_header(&bytes).expect("valid header");
    assert_eq!(header.version, NVE_DRAW_VERSION);
    assert_eq!(header.op_count, 8);

    let decoded = DrawListDecoder::decode(&bytes).expect("decode bytes");
    assert_eq!(decoded.len(), 8);
    assert_eq!(
        decoded[0],
        CanvasOp::Clear {
            color: ColorRgba::rgba(11, 17, 32, 255)
        }
    );
    assert_eq!(
        decoded[1],
        CanvasOp::ClipRect {
            x: 4.0,
            y: 4.0,
            w: 200.0,
            h: 80.0
        }
    );
    assert_eq!(decoded[7], CanvasOp::RestoreClip);

    // Both RawPropValue::Bytes and prefixed base64 String decode identically.
    assert_eq!(
        DrawListDecoder::decode_raw_prop(&raw_prop).unwrap(),
        decoded
    );
    assert_eq!(
        DrawListDecoder::decode_raw_prop(&RawPropValue::String(attr.clone())).unwrap(),
        decoded
    );
    assert_eq!(DrawListDecoder::decode_attr_bytes(&attr).unwrap(), bytes);
}

#[test]
fn empty_draw_list_and_image_resize_mode_roundtrip() {
    let empty = DrawListBuilder::new().finish();
    assert_eq!(empty.len(), NVE_DRAW_HEADER_SIZE);
    assert!(DrawListDecoder::decode(&empty).unwrap().is_empty());

    for mode in [
        ImageResizeMode::Cover,
        ImageResizeMode::Contain,
        ImageResizeMode::Stretch,
        ImageResizeMode::Center,
    ] {
        assert_eq!(ImageResizeMode::parse(mode.as_str()), Some(mode));
        assert_eq!(
            ImageResizeMode::parse(&mode.as_str().to_ascii_uppercase()),
            Some(mode)
        );
        assert_eq!(ImageResizeMode::from_u8(mode.to_u8()), Some(mode));
    }
    assert_eq!(ImageResizeMode::parse("invalid"), None);
    assert_eq!(ImageResizeMode::from_u8(99), None);
}

#[test]
fn sparkline_handles_normal_empty_single_flat_and_non_finite_samples() {
    let stroke = ColorRgba::rgb(56, 189, 248);
    let bg = ColorRgba::rgb(17, 24, 39);

    let cases: &[&[f32]] = &[
        &[],
        &[42.0],
        &[10.0, 10.0, 10.0, 10.0],
        &[120.0, 240.0, 180.0, 310.0, 195.0, 160.0],
        &[
            f32::NAN,
            12.0,
            f32::INFINITY,
            -5.0,
            f32::NEG_INFINITY,
            25.0,
        ],
    ];
    for samples in cases {
        let mut b = DrawListBuilder::new();
        b.sparkline(samples, 260.0, 72.0, stroke, bg);
        let encoded = b.finish();
        let ops = DrawListDecoder::decode(&encoded).expect("sparkline decodes cleanly");
        assert!(ops.len() >= 6);
        assert_eq!(ops.last(), Some(&CanvasOp::RestoreClip));
    }

    // Normal multi-point sparkline contains a StrokePolyline with matching vertex count.
    let ops = CanvasOp::sparkline(&[10.0, 20.0, 15.0, 30.0], 200.0, 60.0, stroke, bg);
    let polyline = ops.iter().find_map(|op| match op {
        CanvasOp::StrokePolyline { points, .. } => Some(points),
        _ => None,
    });
    assert_eq!(polyline.map(Vec::len), Some(4));
}

#[test]
fn decoder_rejects_every_truncation_offset_and_corrupt_headers() {
    let mut b = DrawListBuilder::new();
    b.sparkline(
        &[110.0, 165.0, 140.0, 290.0, 210.0],
        240.0,
        64.0,
        ColorRgba::rgb(167, 139, 250),
        ColorRgba::rgb(15, 23, 42),
    );
    let valid = b.finish();

    // Truncating at EVERY single byte offset before the end must return Truncated (zero panics).
    for len in 0..valid.len() {
        let err = DrawListDecoder::decode(&valid[..len]).unwrap_err();
        assert_eq!(err, DrawListError::Truncated, "failed at len={len}");
    }

    // Trailing garbage byte rejected.
    let mut trailing = valid.clone();
    trailing.push(0xAA);
    assert_eq!(
        DrawListDecoder::decode(&trailing),
        Err(DrawListError::TrailingBytes(1))
    );

    // Bad magic rejected.
    let mut bad_magic = valid.clone();
    bad_magic[0] = b'X';
    assert_eq!(
        DrawListDecoder::decode(&bad_magic),
        Err(DrawListError::BadMagic)
    );

    // Unsupported version rejected.
    let mut bad_ver = valid.clone();
    bad_ver[9..11].copy_from_slice(&99u16.to_le_bytes());
    assert_eq!(
        DrawListDecoder::decode(&bad_ver),
        Err(DrawListError::UnsupportedVersion(99))
    );

    // Huge op_count rejected without allocating.
    let mut huge_ops = valid[..NVE_DRAW_HEADER_SIZE].to_vec();
    huge_ops[11..15].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        DrawListDecoder::decode(&huge_ops),
        Err(DrawListError::OpCountExceedsLimit {
            count: u32::MAX,
            max: MAX_DRAW_OPS
        })
    );

    // Lying op_count within MAX_DRAW_OPS but larger than remaining bytes rejected before OOM.
    let mut lying_ops = valid[..NVE_DRAW_HEADER_SIZE].to_vec();
    lying_ops[11..15].copy_from_slice(&(MAX_DRAW_OPS as u32).to_le_bytes());
    assert_eq!(
        DrawListDecoder::decode(&lying_ops),
        Err(DrawListError::Truncated)
    );

    // Huge polyline point count rejected without allocating.
    let mut huge_poly = Vec::new();
    huge_poly.extend_from_slice(&NVE_DRAW_MAGIC);
    huge_poly.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
    huge_poly.extend_from_slice(&1u32.to_le_bytes());
    huge_poly.push(DrawOpcode::StrokePolyline as u8);
    huge_poly.extend_from_slice(&2.0f32.to_le_bytes());
    huge_poly.extend_from_slice(&ColorRgba::WHITE.to_u32().to_le_bytes());
    huge_poly.extend_from_slice(&((MAX_POLYLINE_POINTS as u32) + 1).to_le_bytes());
    assert_eq!(
        DrawListDecoder::decode(&huge_poly),
        Err(DrawListError::PolylinePointCountExceedsLimit {
            count: (MAX_POLYLINE_POINTS as u32) + 1,
            max: MAX_POLYLINE_POINTS
        })
    );

    // Unknown opcode rejected.
    let mut bad_op = Vec::new();
    bad_op.extend_from_slice(&NVE_DRAW_MAGIC);
    bad_op.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
    bad_op.extend_from_slice(&1u32.to_le_bytes());
    bad_op.push(0xFE);
    assert_eq!(
        DrawListDecoder::decode(&bad_op),
        Err(DrawListError::UnknownOpcode(0xFE))
    );

    // Non-finite float (NaN / Inf) rejected.
    for bad_float in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut buf = Vec::new();
        buf.extend_from_slice(&NVE_DRAW_MAGIC);
        buf.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(DrawOpcode::FillCircle as u8);
        buf.extend_from_slice(&bad_float.to_le_bytes());
        buf.extend_from_slice(&10.0f32.to_le_bytes());
        buf.extend_from_slice(&5.0f32.to_le_bytes());
        buf.extend_from_slice(&ColorRgba::WHITE.to_u32().to_le_bytes());
        assert_eq!(
            DrawListDecoder::decode(&buf),
            Err(DrawListError::NonFiniteFloat)
        );
    }

    // Invalid base64 / unsupported prop values rejected cleanly.
    assert_eq!(
        DrawListDecoder::decode_attr_bytes("nve-draw1;b64,!!!"),
        Err(DrawListError::InvalidBase64)
    );
    assert_eq!(
        DrawListDecoder::decode_raw_prop(&RawPropValue::Bool(true)),
        Err(DrawListError::UnsupportedPropValue)
    );
}

#[test]
fn fuzz_arbitrary_and_mutated_draw_list_buffers_never_panic() {
    let mut base = DrawListBuilder::new();
    base.sparkline(
        &[10.0, 40.0, 22.0, 55.0],
        180.0,
        60.0,
        ColorRgba::rgb(34, 197, 94),
        ColorRgba::rgb(17, 24, 39),
    );
    let seed_buf = base.finish();

    // Deterministic xorshift64 fuzzing across 2,000 random & bit-flipped inputs.
    let mut rng: u64 = 0x5EED_CAFE_BABE_1234;
    let next_u64 = |state: &mut u64| -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    };

    for iter in 0..2_000usize {
        let mut buf = if iter % 2 == 0 {
            // Mutate a valid header/body buffer.
            let mut m = seed_buf.clone();
            let flips = (next_u64(&mut rng) % 4 + 1) as usize;
            for _ in 0..flips {
                let pos = (next_u64(&mut rng) as usize) % m.len();
                m[pos] ^= (next_u64(&mut rng) & 0xFF) as u8;
            }
            let cut = (next_u64(&mut rng) as usize) % (m.len() + 1);
            m.truncate(cut);
            m
        } else {
            // Valid magic + version + random trailing payload.
            let len = (next_u64(&mut rng) % 96) as usize;
            let mut m = Vec::with_capacity(NVE_DRAW_HEADER_SIZE + len);
            m.extend_from_slice(&NVE_DRAW_MAGIC);
            m.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
            for _ in 0..(4 + len) {
                m.push((next_u64(&mut rng) & 0xFF) as u8);
            }
            m
        };

        let _ = DrawListDecoder::decode(&buf);
        // Also fuzz valid header with random raw bytes reversed.
        buf.reverse();
        let _ = DrawListDecoder::decode(&buf);
    }
}

#[test]
fn zero_serialization_draw_list_and_recorder_match_nve_draw1_and_sanitize_edges() {
    let mut rec = DrawListRecorder::with_capacity(8);
    rec.clear(ColorRgba::rgba(11, 17, 32, 255))
        .clip_rect(4.0, 4.0, 200.0, 80.0)
        .fill_rect(8.0, 12.0, 40.0, 24.0, ColorRgba::rgb(30, 41, 59))
        .fill_rounded_rect(10.0, 10.0, 180.0, 60.0, 12.0, ColorRgba::rgb(15, 23, 42))
        .stroke_line(
            12.0,
            40.0,
            188.0,
            40.0,
            1.5,
            ColorRgba::rgba(148, 163, 184, 128),
        )
        .stroke_polyline(
            &[(12.0, 50.0), (60.0, 32.0), (120.0, 42.0), (188.0, 18.0)],
            2.5,
            ColorRgba::rgb(56, 189, 248),
        )
        .fill_circle(188.0, 18.0, 4.0, ColorRgba::rgb(34, 197, 94))
        .restore_clip();

    let draw_list = rec.finish();
    assert_eq!(draw_list.len(), 8);
    let cloned = draw_list.clone();
    assert!(draw_list.ptr_eq(&cloned));

    // Byte-for-byte parity between DrawList::to_nve_draw1_bytes() and DrawListBuilder::finish().
    let mut wire = DrawListBuilder::new();
    wire.clear(ColorRgba::rgba(11, 17, 32, 255))
        .clip_rect(4.0, 4.0, 200.0, 80.0)
        .fill_rect(8.0, 12.0, 40.0, 24.0, ColorRgba::rgb(30, 41, 59))
        .fill_rounded_rect(10.0, 10.0, 180.0, 60.0, 12.0, ColorRgba::rgb(15, 23, 42))
        .stroke_line(
            12.0,
            40.0,
            188.0,
            40.0,
            1.5,
            ColorRgba::rgba(148, 163, 184, 128),
        )
        .stroke_polyline(
            &[(12.0, 50.0), (60.0, 32.0), (120.0, 42.0), (188.0, 18.0)],
            2.5,
            ColorRgba::rgb(56, 189, 248),
        )
        .fill_circle(188.0, 18.0, 4.0, ColorRgba::rgb(34, 197, 94))
        .restore_clip();
    let expected_bytes = wire.finish();
    assert_eq!(draw_list.to_nve_draw1_bytes(), expected_bytes);
    assert_eq!(DrawList::from_nve_draw1_bytes(&expected_bytes).unwrap(), draw_list);

    // Non-finite floats are sanitized to 0.0 and oversized polylines clamp to MAX_POLYLINE_POINTS.
    let mut edge = DrawListRecorder::new();
    let oversized: Vec<(f32, f32)> = (0..(MAX_POLYLINE_POINTS + 16))
        .map(|i| (if i == 0 { f32::NAN } else { i as f32 }, f32::INFINITY))
        .collect();
    edge.fill_circle(f32::NAN, f32::NEG_INFINITY, f32::INFINITY, ColorRgba::WHITE)
        .stroke_polyline(&oversized, f32::NAN, ColorRgba::WHITE);
    let edge_list = edge.finish();
    assert_eq!(
        edge_list[0],
        CanvasOp::FillCircle {
            cx: 0.0,
            cy: 0.0,
            radius: 0.0,
            color: ColorRgba::WHITE,
        }
    );
    match &edge_list[1] {
        CanvasOp::StrokePolyline { points, width, .. } => {
            assert_eq!(points.len(), MAX_POLYLINE_POINTS);
            assert_eq!(points[0], (0.0, 0.0));
            assert_eq!(*width, 0.0);
        }
        other => panic!("expected StrokePolyline, got {other:?}"),
    }
    // Encoded sanitized DrawList round-trips cleanly with zero NonFiniteFloat errors.
    let edge_bytes = edge_list.to_nve_draw1_bytes();
    assert_eq!(DrawList::from_nve_draw1_bytes(&edge_bytes).unwrap(), edge_list);
}
