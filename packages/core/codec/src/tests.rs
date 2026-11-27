use super::*;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
enum Mode {
    Fast = 1,
    Accurate = 4,
}

#[derive(Debug, PartialEq, NveCodec)]
#[nve_codec(crate_path = "crate")]
struct Wrapper(u32, Mode);

#[derive(Debug, PartialEq, NveCodec)]
#[nve_codec(crate_path = "crate")]
enum Packet {
    #[nve_codec(tag = 1)]
    Empty,
    #[nve_codec(tag = 2)]
    Scalar(i32, f32),
    #[nve_codec(tag = 3)]
    Named {
        flag: bool,
        label: String,
        opt: Option<Wrapper>,
        items: Vec<u16>,
    },
}

#[test]
fn derive_roundtrip_and_strict_validation() {
    let cases = [
        Packet::Empty,
        Packet::Scalar(-42, -0.0),
        Packet::Named {
            flag: true,
            label: "hello-東京".into(),
            opt: Some(Wrapper(99, Mode::Accurate)),
            items: vec![1, 2, 300],
        },
    ];
    for p in &cases {
        let bytes = p.encode_to_vec().unwrap();
        let decoded = Packet::decode_exact(&bytes).unwrap();
        assert_eq!(&decoded, p);
        for cut in 0..bytes.len() {
            assert!(matches!(
                Packet::decode_exact(&bytes[..cut]),
                Err(CodecError::Truncated { .. })
            ));
        }
        let mut extra = bytes.clone();
        extra.push(0xAA);
        assert_eq!(Packet::decode_exact(&extra), Err(CodecError::TrailingBytes(1)));
    }

    // Canonical -0.0 -> +0.0 and NaN -> canonical quiet NaN.
    let mut w = ByteWriter::new();
    w.write_f32(-0.0);
    w.write_f32(f32::from_bits(0x7FC1_2345));
    w.write_f64(-0.0);
    w.write_f64(f64::from_bits(0x7FF9_1234_5678_9ABC));
    let mut r = ByteReader::new(w.as_slice());
    assert_eq!(r.read_f32().unwrap().to_bits(), 0);
    assert_eq!(r.read_f32().unwrap().to_bits(), CANONICAL_F32_NAN_BITS);
    assert_eq!(r.read_f64().unwrap().to_bits(), 0);
    assert_eq!(r.read_f64().unwrap().to_bits(), CANONICAL_F64_NAN_BITS);
    r.finish().unwrap();

    // Strict bool, bad enum, bad magic, non-zero reserved, bad UTF-8, len overflow.
    assert_eq!(bool::decode_exact(&[2]), Err(CodecError::BadBool(2)));
    assert_eq!(
        Mode::decode_exact(&[9]),
        Err(CodecError::BadEnum { ty: "Mode", raw: 9 })
    );
    assert_eq!(
        Packet::decode_exact(&[99]),
        Err(CodecError::BadEnum { ty: "Packet", raw: 99 })
    );
    assert_eq!(ByteReader::new(b"NOPE").expect_magic(b"GOOD"), Err(CodecError::BadMagic));
    assert_eq!(
        ByteReader::new(&[0, 1]).expect_zero(2),
        Err(CodecError::NonZeroReserved(1))
    );
    assert_eq!(
        ByteReader::new(&[2, 0, 0, 0, 0xFF, 0xFE]).read_str(16),
        Err(CodecError::InvalidUtf8)
    );
    assert_eq!(
        ByteReader::new(&[100, 0, 0, 0]).read_bytes(16),
        Err(CodecError::LenOverflow { len: 100, max: 16 })
    );
}

#[derive(Debug, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
struct UnitStruct;

#[derive(Debug, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
struct EmptyBraced {}

#[derive(Debug, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
struct EmptyTuple();

#[derive(Debug, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
struct RectWithShadowedNames {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    r: u32,
}

#[derive(Debug, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
enum EnumWithShadowedFields {
    #[nve_codec(tag = 1)]
    Box { w: u32, r: u16 },
    #[nve_codec(tag = 2)]
    Empty,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
enum WideU16 {
    Low = 0,
    High = 0xFFFF,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
enum WideU32 {
    Zero = 0,
    Max = 0xFFFF_FFFFu32,
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveCodec)]
#[nve_codec(crate_path = "crate")]
enum SignedI32 {
    Min = -2147483648i32,
    NegOne = -1i32,
    Zero = 0,
    Max = 2147483647i32,
}

#[test]
fn derive_handles_empty_structs_shadowed_fields_and_wide_reprs() {
    assert_eq!(UnitStruct::decode_exact(&UnitStruct.encode_to_vec().unwrap()).unwrap(), UnitStruct);
    assert_eq!(EmptyBraced::decode_exact(&EmptyBraced {}.encode_to_vec().unwrap()).unwrap(), EmptyBraced {});
    assert_eq!(EmptyTuple::decode_exact(&EmptyTuple().encode_to_vec().unwrap()).unwrap(), EmptyTuple());

    let rect = RectWithShadowedNames { x: 1, y: 2, w: 300, h: 200, r: 12 };
    assert_eq!(RectWithShadowedNames::decode_exact(&rect.encode_to_vec().unwrap()).unwrap(), rect);

    let ev = EnumWithShadowedFields::Box { w: 42, r: 99 };
    assert_eq!(EnumWithShadowedFields::decode_exact(&ev.encode_to_vec().unwrap()).unwrap(), ev);

    for v in [WideU16::Low, WideU16::High] {
        let b = v.encode_to_vec().unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(WideU16::decode_exact(&b).unwrap(), v);
    }
    for v in [WideU32::Zero, WideU32::Max] {
        let b = v.encode_to_vec().unwrap();
        assert_eq!(b.len(), 4);
        assert_eq!(WideU32::decode_exact(&b).unwrap(), v);
    }
    for v in [SignedI32::Min, SignedI32::NegOne, SignedI32::Zero, SignedI32::Max] {
        let b = v.encode_to_vec().unwrap();
        assert_eq!(b.len(), 4);
        assert_eq!(SignedI32::decode_exact(&b).unwrap(), v);
    }
}

#[test]
fn slice_writer_patch_u32_and_write_zeroes_bounds_are_checked() {
    let mut buf = [0u8; 16];
    let mut sw = SliceWriter::new(&mut buf);
    sw.write_u8(1).unwrap();
    sw.write_u16(0x0203).unwrap();
    sw.write_u32(0x0405_0607).unwrap();
    sw.write_zeroes(1).unwrap();
    sw.write_u64(0x0809_0A0B_0C0D_0E0F).unwrap();
    sw.finish().unwrap();

    let mut r = ByteReader::new(&buf);
    assert_eq!(r.read_u8().unwrap(), 1);
    assert_eq!(r.read_u16().unwrap(), 0x0203);
    assert_eq!(r.read_u32().unwrap(), 0x0405_0607);
    r.expect_zero(1).unwrap();
    assert_eq!(r.read_u64().unwrap(), 0x0809_0A0B_0C0D_0E0F);
    r.finish().unwrap();

    let mut small = [0u8; 3];
    let mut sw2 = SliceWriter::new(&mut small);
    assert_eq!(sw2.write_u32(1), Err(CodecError::Truncated { needed: 4, remaining: 3 }));
    sw2.write_u16(1).unwrap();
    assert_eq!(sw2.finish(), Err(CodecError::TrailingBytes(1)));

    let mut bw = ByteWriter::new();
    bw.write_u32(0);
    assert!(bw.patch_u32(0, 0xDEAD_BEEF).is_ok());
    assert_eq!(bw.as_slice(), &0xDEAD_BEEFu32.to_le_bytes());
    assert_eq!(bw.patch_u32(2, 1), Err(CodecError::Truncated { needed: 4, remaining: 2 }));
    assert!(matches!(bw.patch_u32(usize::MAX, 1), Err(CodecError::LenOverflow { .. })));
    assert!(matches!(bw.write_zeroes(usize::MAX), Err(CodecError::LenOverflow { .. })));
}

#[test]
fn deterministic_fuzz_nested_derived_packets_never_panic() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next_u64 = || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };

    let seed_bytes = Packet::Named {
        flag: true,
        label: "fuzz-seed".into(),
        opt: Some(Wrapper(7, Mode::Fast)),
        items: vec![10, 20, 30],
    }
    .encode_to_vec()
    .unwrap();

    let mut buf = Vec::with_capacity(128);
    for round in 0..4096u32 {
        buf.clear();
        if round % 3 == 0 {
            buf.extend_from_slice(&seed_bytes);
            let idx = (next_u64() as usize) % buf.len();
            buf[idx] ^= (next_u64() as u8) | 1;
        } else {
            let len = (next_u64() as usize) % 96;
            for _ in 0..len {
                buf.push(next_u64() as u8);
            }
        }
        if let Ok(decoded) = Packet::decode_exact(&buf) {
            let reencoded = decoded.encode_to_vec().unwrap();
            assert_eq!(Packet::decode_exact(&reencoded).unwrap(), decoded);
        }
    }
}
