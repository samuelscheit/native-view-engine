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
