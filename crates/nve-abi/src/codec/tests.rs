use crate::NodeId;

use super::*;

#[test]
fn roundtrip_all_command_variants() {
    let mut b = CommandBufferBuilder::new(7, 42);
    b.create_node(1, ComponentKind::Root, None);
    b.create_node(2, ComponentKind::View, None);
    b.create_node(3, ComponentKind::Text, None);
    b.create_node(4, ComponentKind::Custom, Some("RNSVGPath"));
    b.set_children(1, &[2, 4]);
    b.set_children(2, &[3]);
    b.set_text(3, "Hello Fabric + Skia + Web! 🚀");
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(320.0),
            height: Dimension::Percent(100.0),
            flex_direction: FlexDirection::Row,
            justify_content: FlexJustify::SpaceBetween,
            align_items: FlexAlign::Center,
            flex_grow: 1.0,
            gap: 12.0,
            padding: EdgeInsets::symmetric(8.0, 16.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgba(15, 23, 42, 240),
            text_color: ColorRgba::WHITE,
            border_color: ColorRgba::rgb(56, 189, 248),
            border_width: 2.0,
            border_radius: 12.0,
            opacity: 0.95,
            font_size: 18.0,
            z_index: 5,
        },
    );
    b.set_raw_prop(4, "d", &RawPropValue::String("M0 0 L10 10".into()));
    b.set_raw_prop(4, "fillOpacity", &RawPropValue::F64(0.75));
    b.set_raw_prop(4, "visible", &RawPropValue::Bool(true));
    b.set_raw_prop(4, "seed", &RawPropValue::I64(-99));
    b.set_raw_prop(4, "blob", &RawPropValue::Bytes(vec![1, 2, 3, 4]));
    b.set_raw_prop(4, "empty", &RawPropValue::Null);
    b.set_event_mask(2, crate::EventMask::PRESS | crate::EventMask::LAYOUT);
    b.dispatch_command(2, "scrollTo", &[0, 1, 2]);
    b.remove_node(4);
    b.commit(true);

    let bytes = b.into_bytes();
    let (header, cmds) = CommandDecoder::new(&bytes).unwrap().decode_all().unwrap();
    assert_eq!(header.surface_id, 7);
    assert_eq!(header.frame_id, 42);
    assert_eq!(header.op_count as usize, cmds.len());
    assert_eq!(cmds.len(), 19);
}

#[test]
fn rejects_corrupt_buffers() {
    assert_eq!(
        CommandDecoder::new(&[0u8; 4]).err(),
        Some(DecodeError::Truncated)
    );
    let mut bad_magic = CommandBufferBuilder::new(1, 1).into_bytes();
    bad_magic[0] ^= 0xFF;
    assert!(matches!(
        CommandDecoder::new(&bad_magic),
        Err(DecodeError::InvalidMagic(_))
    ));

    let mut b = CommandBufferBuilder::new(1, 1);
    b.remove_node(5);
    let mut truncated = b.into_bytes();
    truncated.pop();
    assert_eq!(
        CommandDecoder::new(&truncated).unwrap().decode_all().err(),
        Some(DecodeError::Truncated)
    );
}

#[test]
fn set_raw_prop_str_matches_owned_encoding() {
    let mut b1 = CommandBufferBuilder::new(1, 1);
    b1.set_raw_prop(7, "label", &RawPropValue::String("hello world".into()));
    let mut b2 = CommandBufferBuilder::new(1, 1);
    b2.set_raw_prop_str(7, "label", "hello world");
    assert_eq!(b1.into_bytes(), b2.into_bytes());
}

#[test]
fn for_each_matches_decode_all_on_valid_truncated_and_corrupt_buffers() {
    fn decode_via_for_each(
        bytes: &[u8],
    ) -> Result<(CommandBufferHeader, Vec<Command>), DecodeError> {
        let mut out = Vec::new();
        let hdr = CommandDecoder::new(bytes)?.for_each(|cmd| {
            if let CommandRef::SetChildren { children, .. } = cmd {
                assert_eq!(children.len(), children.iter().count());
                assert_eq!(children.is_empty(), children.iter().next().is_none());
                let rev: Vec<NodeId> = children.iter().rev().collect();
                let mut fwd = Vec::new();
                children.copy_into(&mut fwd);
                assert_eq!(fwd, children.to_vec());
                let mut rev_expected = fwd.clone();
                rev_expected.reverse();
                assert_eq!(rev, rev_expected);
                for (idx, &cid) in fwd.iter().enumerate() {
                    assert_eq!(children.get(idx), Some(cid));
                }
                assert_eq!(children.get(fwd.len()), None);
            }
            out.push(cmd.to_owned());
            Ok::<(), DecodeError>(())
        })?;
        Ok((hdr, out))
    }

    // Build a rich buffer covering every opcode and RawPropValue variant.
    let mut b = CommandBufferBuilder::new(3, 99);
    b.create_node(1, ComponentKind::Root, None);
    b.create_node(2, ComponentKind::Custom, Some("MyView"));
    b.set_children(1, &[2, 3, 4, 5]);
    b.set_children(2, &[]);
    b.set_text(3, "Streaming zero-copy 🚀");
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(120.5),
            height: Dimension::Percent(50.0),
            min_width: Dimension::Points(10.0),
            max_height: Dimension::Percent(95.0),
            flex_direction: FlexDirection::RowReverse,
            justify_content: FlexJustify::SpaceEvenly,
            align_items: FlexAlign::Baseline,
            flex_grow: 2.0,
            flex_shrink: 0.5,
            gap: 8.0,
            padding: EdgeInsets::symmetric(4.0, 12.0),
            margin: EdgeInsets::all(6.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgba(1, 2, 3, 250),
            text_color: ColorRgba::WHITE,
            border_color: ColorRgba::rgb(9, 8, 7),
            border_width: 1.5,
            border_radius: 10.0,
            opacity: 0.85,
            font_size: 18.0,
            z_index: -3,
        },
    );
    b.set_raw_prop_str(2, "strKey", "zero-alloc");
    b.set_raw_prop_ref(2, "bytesKey", RawPropValueRef::Bytes(&[0xDE, 0xAD, 0xBE, 0xEF]));
    b.set_raw_prop(2, "nullKey", &RawPropValue::Null);
    b.set_raw_prop(2, "boolKey", &RawPropValue::Bool(true));
    b.set_raw_prop(2, "i64Key", &RawPropValue::I64(-123456789));
    b.set_raw_prop(2, "f64Key", &RawPropValue::F64(3.25));
    b.set_event_mask(2, 0x00FF_00AA_5511_2233);
    b.dispatch_command(2, "focus", &[1, 2, 3]);
    b.remove_node(5);
    b.commit(false);
    let full = b.into_bytes();

    // 1. Exact equivalence on valid buffer.
    assert_eq!(
        decode_via_for_each(&full),
        CommandDecoder::new(&full).and_then(CommandDecoder::decode_all)
    );

    // 2. Exact equivalence on every truncation prefix.
    for len in 0..full.len() {
        let slice = &full[..len];
        let streamed = decode_via_for_each(slice);
        let owned = CommandDecoder::new(slice).and_then(CommandDecoder::decode_all);
        assert_eq!(streamed, owned, "mismatch at truncated len {len}");
        assert!(streamed.is_err());
    }

    // 3. Trailing bytes rejected identically.
    let mut trailing = full.clone();
    trailing.extend_from_slice(&[0xAA, 0xBB]);
    assert_eq!(
        decode_via_for_each(&trailing),
        Err(DecodeError::TrailingBytes(2))
    );
    assert_eq!(
        decode_via_for_each(&trailing),
        CommandDecoder::new(&trailing).and_then(CommandDecoder::decode_all)
    );

    // 4. Deterministic property/fuzz sweep over bit-flips, splices, and random noise.
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next_u64 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for _ in 0..300 {
        let mut mutated = full.clone();
        let flips = (next_u64() as usize % 5) + 1;
        for _ in 0..flips {
            let idx = next_u64() as usize % mutated.len();
            mutated[idx] ^= 1u8 << (next_u64() as usize % 8);
        }
        if next_u64() & 1 == 0 {
            let trunc = next_u64() as usize % (mutated.len() + 1);
            mutated.truncate(trunc);
        }
        assert_eq!(
            decode_via_for_each(&mutated),
            CommandDecoder::new(&mutated).and_then(CommandDecoder::decode_all)
        );
    }

    for _ in 0..100 {
        let len = next_u64() as usize % 128;
        let noise: Vec<u8> = (0..len).map(|_| next_u64() as u8).collect();
        assert_eq!(
            decode_via_for_each(&noise),
            CommandDecoder::new(&noise).and_then(CommandDecoder::decode_all)
        );
    }
}
