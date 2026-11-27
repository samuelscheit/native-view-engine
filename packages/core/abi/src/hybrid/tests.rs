use std::fmt::Debug;

use crate::DecodeError;
use crate::value::{BinMapWriter, BinValueView, BinWriter, NveMapEntry, NveValue, NveValueTag};

use super::*;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Idle = 0,
    Busy = 7,
}

impl Mode {
    const fn to_u32(self) -> u32 {
        self as u32
    }
    const fn from_u32(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Idle),
            7 => Some(Self::Busy),
            _ => None,
        }
    }
}
crate::hybrid_enum_codec!(Mode);

#[derive(Debug, Clone, PartialEq, Default)]
struct Record {
    id: u32,
    mode: Mode,
    label: Option<String>,
    tags: Vec<Option<Vec<u8>>>,
    meta: NveHybridValue,
}

impl HybridStruct for Record {
    fn encode_fields(&self, m: &mut BinMapWriter<'_>) {
        self.id.encode_bin(m.key("id"));
        self.mode.encode_bin(m.key("mode"));
        self.label.encode_bin(m.key("label"));
        self.tags.encode_bin(m.key("tags"));
        self.meta.encode_bin(m.key("meta"));
    }
    fn decode_fields(fields: &impl HybridFields) -> Result<Self, DecodeError> {
        Ok(Self {
            id: fields.field("id")?,
            mode: fields.field("mode")?,
            label: fields.field("label")?,
            tags: fields.field("tags")?,
            meta: fields.field("meta")?,
        })
    }
}
crate::hybrid_struct_codec!(Record);

fn sample_record() -> Record {
    Record {
        id: 42,
        mode: Mode::Busy,
        label: Some("héllo".to_owned()),
        tags: vec![Some(vec![1, 2]), None, Some(vec![])],
        meta: NveHybridValue::encode(&vec![vec![1i32, -2], vec![]]).expect("encodes"),
    }
}

fn round_trip<T: HybridCodec + PartialEq + Debug>(value: T) {
    let bytes = encode_to_vec(&value).expect("encodes");
    assert_eq!(decode_bin_arg::<T>(&bytes), Ok(value), "bytes {bytes:?}");
}

fn single(write: impl FnOnce(&mut BinWriter)) -> Vec<u8> {
    let mut w = BinWriter::new();
    write(&mut w);
    w.finish().expect("encodes")
}

#[test]
fn scalars_round_trip_at_their_boundaries() {
    round_trip(true);
    round_trip(u8::MAX);
    round_trip(u16::MAX);
    round_trip(u32::MAX);
    round_trip(i32::MIN);
    round_trip(i64::MIN);
    round_trip(i64::MAX);
    round_trip(u64::MAX);
    round_trip(usize::MAX >> 1);
    round_trip(f32::MIN_POSITIVE);
    round_trip(f64::NEG_INFINITY);
    round_trip(String::new());
    round_trip("ünïcødé 😀".to_owned());
    round_trip(Vec::<u8>::new());
    round_trip(Mode::Busy);
    let nan = decode_bin_arg::<f64>(&encode_to_vec(&f64::NAN).expect("encodes")).expect("decodes");
    assert!(nan.is_nan());
}

#[test]
fn containers_round_trip_and_bytes_stay_distinct_from_arrays() {
    round_trip(Some(5u16));
    round_trip(None::<String>);
    round_trip(vec![Some(1i32), None, Some(-1)]);
    round_trip(vec![vec![1u8, 2], vec![]]);
    round_trip(vec![vec![vec![3u64]]]);
    round_trip([1.5f32, -0.0, 3.0]);
    round_trip(["a".to_owned(), "ü".to_owned()]);
    round_trip([vec![1i32], vec![]]);
    round_trip([7u8; 64]);
    round_trip([[Some(Mode::Busy), None]; 40]);
    round_trip(vec![Mode::Idle, Mode::Busy]);
    round_trip(sample_record());
    round_trip(vec![sample_record(), Record::default()]);

    let bytes = encode_to_vec(&vec![1u8, 2]).expect("encodes");
    assert_eq!(
        BinValueView::parse(&bytes).map(|v| v.tag()),
        Ok(NveValueTag::Bytes)
    );
    let array = encode_to_vec(&vec![1u16, 2]).expect("encodes");
    assert_eq!(
        BinValueView::parse(&array).map(|v| v.tag()),
        Ok(NveValueTag::Array)
    );
}

#[test]
fn decoding_rejects_out_of_range_mismatched_and_malformed_input() {
    let big = single(|w| w.i64(300));
    assert_eq!(
        decode_bin_arg::<u8>(&big),
        Err(DecodeError::OutOfRange("u8"))
    );
    let negative = single(|w| w.i64(-1));
    assert_eq!(
        decode_bin_arg::<u32>(&negative),
        Err(DecodeError::OutOfRange("u32"))
    );
    assert_eq!(
        decode_bin_arg::<usize>(&negative),
        Err(DecodeError::OutOfRange("usize"))
    );
    assert_eq!(
        decode_bin_arg::<Mode>(&negative),
        Err(DecodeError::OutOfRange("Mode"))
    );
    let wide = single(|w| w.i64(i64::from(u32::MAX) + 7));
    assert_eq!(
        decode_bin_arg::<Mode>(&wide),
        Err(DecodeError::OutOfRange("Mode")),
        "never truncated into a declared discriminant"
    );
    let unknown = single(|w| w.i64(3));
    assert_eq!(
        decode_bin_arg::<Mode>(&unknown),
        Err(DecodeError::InvalidEnum("Mode", 3))
    );

    assert_eq!(
        decode_bin_arg::<String>(&big),
        Err(DecodeError::UnexpectedTag {
            expected: NveValueTag::Str,
            found: NveValueTag::I64 as u32
        })
    );
    assert!(matches!(
        decode_bin_arg::<Record>(&big),
        Err(DecodeError::UnexpectedTag { .. })
    ));
    let pair = encode_to_vec(&vec![1i32, 2]).expect("encodes");
    assert_eq!(
        decode_bin_arg::<[i32; 3]>(&pair),
        Err(DecodeError::OutOfRange("fixed-size array length"))
    );
    assert_eq!(decode_bin_arg::<i64>(&[]), Err(DecodeError::Truncated));
    let mut trailing = big.clone();
    trailing.push(0);
    assert_eq!(
        decode_bin_arg::<i64>(&trailing),
        Err(DecodeError::TrailingBytes(1))
    );
}

#[test]
fn struct_fields_are_looked_up_by_key() {
    // Reordered keys plus an unknown one decode; an absent `Option` is `None`.
    let reordered = single(|w| {
        w.map(|m| {
            m.key("unknown").str("ignored");
            m.key("meta").null();
            m.key("tags").array(|_| {});
            m.key("mode").i64(0);
            m.key("id").i64(9);
        })
    });
    let decoded = decode_bin_arg::<Record>(&reordered).expect("decodes");
    assert_eq!(
        (decoded.id, decoded.label, decoded.tags.len()),
        (9, None, 0)
    );

    let missing = single(|w| w.map(|m| m.key("id").i64(1)));
    assert_eq!(
        decode_bin_arg::<Record>(&missing),
        Err(DecodeError::MissingField("mode"))
    );
}

#[test]
fn c_value_trees_decode_as_maps_arrays_or_encoded_bytes() {
    let record = sample_record();
    let encoded = NveHybridValue::encode(&record).expect("encodes");
    // A struct as a C `Map` tree (no `Bytes` wrapper) decodes losslessly…
    let tree = encoded.to_tree();
    assert_eq!(tree.root().tag, NveValueTag::Map as u32);
    assert_eq!(
        Record::decode_value_ref(tree.as_value_ref()),
        Ok(record.clone())
    );
    assert_eq!(
        NveHybridValue::decode_value_ref(tree.as_value_ref()),
        Ok(encoded.clone())
    );
    // …and so does its `NveBinValue` encoding wrapped in `Bytes`.
    let wrapped = NveValue::from_bytes(encoded.as_bytes());
    // SAFETY: `wrapped` borrows `encoded`, which outlives every use.
    let wrapped_ref = unsafe { wrapped.as_ref() };
    assert_eq!(Record::decode_value_ref(wrapped_ref), Ok(record));

    let items = [NveValue::i64(4), NveValue::null()];
    let array = NveValue::array(&items);
    // SAFETY: `array` borrows `items`.
    let decoded = Vec::<Option<u8>>::decode_value_ref(unsafe { array.as_ref() });
    assert_eq!(decoded, Ok(vec![Some(4), None]));
    // A `Value` prop keeps `Bytes` as `Bytes` instead of unwrapping it.
    let dynamic = NveHybridValue::decode_value_ref(wrapped_ref).expect("encodes");
    assert_eq!(dynamic.as_view().tag(), NveValueTag::Bytes);

    // A fixed-size array prop also arrives as `Bytes`-wrapped encoding.
    let array = encode_to_vec(&["x".to_owned(), "y".to_owned()]).expect("encodes");
    let array_bytes = NveValue::from_bytes(&array);
    // SAFETY: `array_bytes` borrows `array`.
    let decoded = <[String; 2]>::decode_value_ref(unsafe { array_bytes.as_ref() });
    assert_eq!(decoded, Ok(["x".to_owned(), "y".to_owned()]));

    // The right tag over invalid contents is reported as such, not as a tag mismatch.
    let bad_utf8 = [0xffu8, 0xfe];
    let bad_str = NveValue {
        tag: NveValueTag::Str as u32,
        data: crate::value::NveValueData {
            str_val: crate::NveStrSlice {
                ptr: bad_utf8.as_ptr(),
                len: bad_utf8.len(),
            },
        },
    };
    // SAFETY: `bad_str` borrows `bad_utf8`.
    let bad_str_ref = unsafe { bad_str.as_ref() };
    assert_eq!(
        String::decode_value_ref(bad_str_ref),
        Err(DecodeError::InvalidUtf8)
    );
    let dangling = NveValue {
        tag: NveValueTag::Bytes as u32,
        data: crate::value::NveValueData {
            bytes_val: crate::NveBufferSlice {
                ptr: std::ptr::null(),
                len: 9,
            },
        },
    };
    // SAFETY: A null pointer is rejected before any read.
    let dangling_ref = unsafe { dangling.as_ref() };
    assert_eq!(
        Vec::<u8>::decode_value_ref(dangling_ref),
        Err(DecodeError::Truncated)
    );

    let entries = [NveMapEntry::new("id", NveValue::f64(1.0))];
    let map = NveValue::map(&entries);
    // SAFETY: `map` borrows `entries`.
    let wrong = Record::decode_value_ref(unsafe { map.as_ref() });
    assert!(matches!(
        wrong,
        Err(DecodeError::UnexpectedTag {
            expected: NveValueTag::I64,
            ..
        })
    ));
}

#[test]
fn hybrid_values_validate_and_respect_the_depth_limit() {
    assert_eq!(
        NveHybridValue::default().as_bytes(),
        &[NveValueTag::Null as u8]
    );
    assert_eq!(
        NveHybridValue::from_bin(vec![]),
        Err(DecodeError::Truncated)
    );
    assert_eq!(
        NveHybridValue::from_bin(vec![0, 0]),
        Err(DecodeError::TrailingBytes(1))
    );
    assert_eq!(
        NveHybridValue::from_bin(vec![1, 2]),
        Err(DecodeError::InvalidEnum("NveBinBool", 2))
    );

    fn nest(w: &mut BinWriter, levels: usize) {
        if levels == 0 {
            w.null()
        } else {
            w.array(|a| nest(a.item(), levels - 1))
        }
    }
    let mut w = BinWriter::new();
    nest(&mut w, crate::NVE_BIN_VALUE_MAX_DEPTH);
    let deepest = NveHybridValue::from_bin(w.finish().expect("encodes")).expect("validates");
    round_trip(deepest.clone());
    // Wrapping it in one more container would exceed what any decoder accepts.
    assert_eq!(
        encode_to_vec(&vec![deepest]),
        Err(DecodeError::MaxDepthExceeded)
    );
}

#[test]
fn owned_bytes_and_trampoline_helpers_follow_the_c_contract() {
    let bytes = NveHybridOwnedBytes::from_vec(vec![1, 2, 3]);
    // SAFETY: Built by `from_vec` and released exactly once below.
    unsafe {
        assert_eq!(bytes.as_slice(), &[1, 2, 3]);
        assert_eq!(bytes.into_vec(), vec![1, 2, 3]);
    }
    for empty in [
        NveHybridOwnedBytes::from_vec(Vec::new()),
        NveHybridOwnedBytes::from_vec(Vec::with_capacity(64)),
        NveHybridOwnedBytes::from_string(String::with_capacity(8)),
        nve_hybrid_alloc_bytes(0),
        nve_hybrid_alloc_bytes(usize::MAX),
        nve_hybrid_alloc_bytes(isize::MAX as usize + 1),
    ] {
        assert!(empty.ptr.is_null() && empty.len == 0 && empty.cap == 0);
        // SAFETY: `EMPTY` is always releasable.
        unsafe { nve_hybrid_free_bytes(empty) };
    }

    let zeroed = nve_hybrid_alloc_bytes(5);
    // SAFETY: Allocated by `nve_hybrid_alloc_bytes`, released once.
    unsafe {
        assert_eq!(zeroed.as_slice(), &[0; 5]);
        nve_hybrid_free_bytes(zeroed);
    }

    let mut out_err = NveHybridOwnedBytes::EMPTY;
    let mut out_ok = 0u32;
    // SAFETY: Valid out-pointers or null.
    unsafe {
        assert_eq!(
            fail(
                &mut out_err,
                NveHybridStatus::InvalidArg,
                "bad 😀".to_owned()
            ),
            2
        );
        assert_eq!(std::str::from_utf8(out_err.as_slice()), Ok("bad 😀"));
        nve_hybrid_free_bytes(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY));
        store(&mut out_ok, 7);
        store(std::ptr::null_mut(), 8u32);
        assert_eq!(succeed(&mut out_err), NveHybridStatus::Ok as i32);
        assert!(out_err.ptr.is_null());
        assert_eq!(succeed(std::ptr::null_mut()), 0);
        let mut out_bytes = NveHybridOwnedBytes::EMPTY;
        store_bytes(&mut out_bytes, NveHybridOwnedBytes::from_vec(vec![4, 5]));
        assert_eq!(out_bytes.as_slice(), &[4, 5]);
        nve_hybrid_free_bytes(out_bytes);
        store_bytes(std::ptr::null_mut(), NveHybridOwnedBytes::from_vec(vec![6]));
        assert_eq!(
            fail(std::ptr::null_mut(), NveHybridStatus::Err, String::new()),
            1
        );
    }
    assert_eq!(out_ok, 7);

    let encoded = NveHybridOwnedBytes::encode(&sample_record()).expect("encodes");
    // SAFETY: Built by `encode`, released once.
    let decoded = unsafe { decode_bin_arg::<Record>(encoded.as_slice()) };
    assert_eq!(decoded, Ok(sample_record()));
    // SAFETY: Same buffer, released exactly once.
    unsafe { nve_hybrid_free_bytes(encoded) };
}

#[test]
fn status_list_matches_the_discriminants() {
    for (index, status) in NveHybridStatus::ALL.into_iter().enumerate() {
        assert_eq!(usize::try_from(status as i32), Ok(index));
        assert_eq!(format!("{status:?}"), status.name());
    }
}

#[test]
fn raw_carriers_validate_before_becoming_rust_values() {
    assert_eq!(decode_bool(0), Ok(false));
    assert_eq!(decode_bool(1), Ok(true));
    assert_eq!(decode_bool(2), Err(DecodeError::OutOfRange("bool")));

    let mode = |raw: u32| decode_enum(i64::from(raw), "Mode", Mode::from_u32);
    // A `None` may carry an undeclared discriminant (zeroed `std::nullopt`): never decoded.
    assert_eq!(decode_opt(0, 99u32, mode), Ok(None));
    assert_eq!(decode_opt(1, 7u32, mode), Ok(Some(Mode::Busy)));
    assert_eq!(
        decode_opt(1, 99u32, mode),
        Err(DecodeError::InvalidEnum("Mode", 99))
    );
    assert_eq!(
        decode_opt(2, 7u32, mode),
        Err(DecodeError::OutOfRange("bool"))
    );

    assert_eq!(decode_array([0u32, 7], mode), Ok([Mode::Idle, Mode::Busy]));
    assert_eq!(
        decode_array([7u32, 3, 4], mode),
        Err(DecodeError::InvalidEnum("Mode", 3)),
        "the first failing element wins"
    );

    let bytes = [1u8, 2, 3];
    // SAFETY: `bytes` outlives the borrow; the other slices are rejected before any read.
    unsafe {
        assert_eq!(
            buffer_arg(crate::NveBufferSlice::from_slice(&bytes)),
            Ok(&bytes[..])
        );
        assert_eq!(buffer_arg(crate::NveBufferSlice::EMPTY), Ok(&[][..]));
        let null = crate::NveBufferSlice {
            ptr: std::ptr::null(),
            len: 100,
        };
        assert_eq!(buffer_arg(null), Err(DecodeError::Truncated));
        let huge = crate::NveBufferSlice {
            ptr: bytes.as_ptr(),
            len: isize::MAX as usize + 1,
        };
        assert_eq!(buffer_arg(huge), Err(DecodeError::Truncated));
        assert!(null.as_slice().is_empty());
    }
}

#[test]
fn direct_props_reject_corrupt_keys_instead_of_skipping_them() {
    let bad_key = [0xffu8];
    let entries = [
        NveMapEntry::new("ok", NveValue::null()),
        NveMapEntry {
            key: crate::NveStrSlice {
                ptr: bad_key.as_ptr(),
                len: 1,
            },
            value: NveValue::null(),
        },
    ];
    // SAFETY: `entries` and `bad_key` outlive the view.
    let map = unsafe { crate::value::NveMapRef::from_slice(&entries) };
    assert_eq!(map.iter().count(), 1, "`iter` skips the corrupt key");
    let keys: Vec<_> = map.try_iter().map(|e| e.map(|(k, _)| k)).collect();
    assert_eq!(keys, [Ok("ok"), Err(DecodeError::InvalidUtf8)]);
}
