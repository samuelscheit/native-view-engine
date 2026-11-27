//! CBOR schema evolution, adversarial CBOR DoS validation, bounded form decoding, and
//! `CountingAlloc` zero-allocation hot-path tests for `nve-rpc`.

use nve_rpc::{
    decode_form_urlencoded_limited, format_invalidate_header_into, validate_cbor, CacheTag,
    CborCodec, CborLimits, CodecError, FieldErrors, FormDecode, FormPairs, RpcCodec, Tag64,
    TagSink, DEFAULT_MAX_FORM_FIELDS,
};

use crate::{count_allocs, IssueV1, IssueV2};

#[test]
fn cbor_schema_evolution_handles_mobile_version_skew() {
    let codec = CborCodec::default();

    let v2 = IssueV2 {
        id: 42,
        title: "Fix scroll".into(),
        open: true,
        priority: Some(2),
        labels: Some(vec!["bug".into(), "ios".into()]),
    };
    let mut wire = Vec::new();
    codec.encode_into(&v2, &mut wire).unwrap();

    let decoded_v1: IssueV1 = codec.decode_bounded(&wire).unwrap();
    assert_eq!(
        decoded_v1,
        IssueV1 {
            id: 42,
            title: "Fix scroll".into(),
            open: true,
        }
    );

    wire.clear();
    codec.encode_into(&decoded_v1, &mut wire).unwrap();
    let decoded_v2: IssueV2 = codec.decode_bounded(&wire).unwrap();
    assert_eq!(decoded_v2.id, 42);
    assert_eq!(decoded_v2.title, "Fix scroll");
    assert!(decoded_v2.open);
    assert_eq!(decoded_v2.priority, None);
    assert_eq!(decoded_v2.labels, None);
}

#[test]
fn cbor_validator_rejects_hostile_dos_payloads() {
    let limits = CborLimits {
        max_bytes: 4096,
        max_depth: 16,
        max_collection_len: 32,
        max_items: 128,
    };

    // 1. Empty payload.
    assert!(matches!(validate_cbor(&[], limits), Err(CodecError::Decode(_))));

    // 2. Oversized raw byte length.
    let too_big = vec![0x00u8; 4097];
    assert_eq!(
        validate_cbor(&too_big, limits),
        Err(CodecError::MaxBytesExceeded(4096))
    );

    // 3. Deep array nesting bomb (64 nested 1-element arrays, > max_depth 16).
    let mut deep_arrays = vec![0x81u8; 64];
    deep_arrays.push(0x00);
    assert_eq!(
        validate_cbor(&deep_arrays, limits),
        Err(CodecError::MaxDepthExceeded(16))
    );

    // 4. Empty-container depth boundary: 16 nested arrays wrapping empty array (depth 17 > 16) MUST fail,
    //    while 15 nested arrays wrapping empty array (depth 16 == max_depth) MUST succeed.
    let mut empty_depth_17 = vec![0x81u8; 16];
    empty_depth_17.push(0x80);
    assert_eq!(
        validate_cbor(&empty_depth_17, limits),
        Err(CodecError::MaxDepthExceeded(16))
    );
    let mut empty_depth_16 = vec![0x81u8; 15];
    empty_depth_16.push(0x80);
    assert_eq!(validate_cbor(&empty_depth_16, limits), Ok(()));

    // 5. Deep CBOR tag chain bomb (32 nested tag(0) wrapping 0).
    let mut deep_tags = vec![0xc0u8; 32];
    deep_tags.push(0x00);
    assert_eq!(
        validate_cbor(&deep_tags, limits),
        Err(CodecError::MaxDepthExceeded(16))
    );

    // 6. Declared OOM array bomb: array(0xFFFF_FFFF) in 5 bytes.
    let oom_array = [0x9a, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(
        validate_cbor(&oom_array, limits),
        Err(CodecError::MaxCollectionLenExceeded(32))
    );

    // 7. Declared OOM map bomb: map(1_000_000).
    let oom_map = [0xba, 0x00, 0x0f, 0x42, 0x40];
    assert_eq!(
        validate_cbor(&oom_map, limits),
        Err(CodecError::MaxCollectionLenExceeded(32))
    );

    // 8. Zero-size / tiny-item CPU bomb: array of 32 arrays of 8 nulls (256 items > 128).
    let mut many_items = vec![0x98, 32];
    for _ in 0..32 {
        many_items.push(0x88);
        many_items.extend_from_slice(&[0xf6; 8]);
    }
    assert_eq!(
        validate_cbor(&many_items, limits),
        Err(CodecError::MaxItemsExceeded(128))
    );

    // 9. Unpaired indefinite map key: 0xbf 0x00 0xff.
    assert!(matches!(
        validate_cbor(&[0xbf, 0x00, 0xff], limits),
        Err(CodecError::Decode(_))
    ));

    // 10. Unclosed indefinite containers MUST return `CodecError::UnclosedIndefinite`.
    assert_eq!(
        validate_cbor(&[0x9f, 0x00], limits),
        Err(CodecError::UnclosedIndefinite)
    );
    assert_eq!(
        validate_cbor(&[0xbf, 0x00, 0x01], limits),
        Err(CodecError::UnclosedIndefinite)
    );
    assert_eq!(
        validate_cbor(&[0x5f, 0x41, b'a'], limits),
        Err(CodecError::UnclosedIndefinite)
    );
    assert_eq!(
        validate_cbor(&[0x7f, 0x61, b'a'], limits),
        Err(CodecError::UnclosedIndefinite)
    );

    // 11. Stray break byte.
    assert_eq!(
        validate_cbor(&[0xff], limits),
        Err(CodecError::UnexpectedBreak)
    );

    // 12. Trailing garbage bytes after a valid root value.
    assert_eq!(
        validate_cbor(&[0x01, 0x02, 0x03], limits),
        Err(CodecError::TrailingBytes(2))
    );

    // 13. Invalid UTF-8 inside definite and indefinite CBOR text strings.
    assert!(matches!(
        validate_cbor(&[0x62, 0xff, 0xfe], limits),
        Err(CodecError::Decode(_))
    ));
    assert!(matches!(
        validate_cbor(&[0x7f, 0x62, 0xff, 0xfe, 0xff], limits),
        Err(CodecError::Decode(_))
    ));

    // 14. Valid indefinite-length map & text string succeed.
    assert_eq!(
        validate_cbor(&[0xbf, 0x00, 0x01, 0x02, 0xf5, 0xff], limits),
        Ok(())
    );
    assert_eq!(
        validate_cbor(&[0x7f, 0x62, b'h', b'i', 0x61, b'!', 0xff], limits),
        Ok(())
    );

    // 15. Indefinite-length array exceeding max_collection_len (33 > 32, < max_items 128).
    let mut big_indef_arr = vec![0x9f];
    big_indef_arr.extend(core::iter::repeat_n(0x00u8, 33));
    big_indef_arr.push(0xff);
    assert_eq!(
        validate_cbor(&big_indef_arr, limits),
        Err(CodecError::MaxCollectionLenExceeded(32))
    );

    // 16. Indefinite-length map exceeding max_collection_len (33 pairs > 32, 66 items < 128).
    let mut big_indef_map = vec![0xbf];
    for _ in 0..33 {
        big_indef_map.push(0x00);
        big_indef_map.push(0x01);
    }
    big_indef_map.push(0xff);
    assert_eq!(
        validate_cbor(&big_indef_map, limits),
        Err(CodecError::MaxCollectionLenExceeded(32))
    );

    // 17. RFC 8949 §3.3 ill-formed 2-byte simple values `0xf8 0x00..=0x1f` MUST be rejected,
    //     while `0xf8 0x20` (`simple(32)`) is valid.
    assert!(matches!(
        validate_cbor(&[0xf8, 0x00], limits),
        Err(CodecError::Decode(_))
    ));
    assert!(matches!(
        validate_cbor(&[0xf8, 0x1f], limits),
        Err(CodecError::Decode(_))
    ));
    assert_eq!(validate_cbor(&[0xf8, 0x20], limits), Ok(()));
}

#[test]
fn form_urlencoded_and_form_decode_handle_edge_cases_and_limits() {
    struct FormInput {
        title: String,
        urgent: bool,
        note: Option<String>,
    }

    impl FormDecode for FormInput {
        fn from_form(form: &FormPairs) -> Result<Self, FieldErrors> {
            let mut errs = FieldErrors::new();
            let title = form.get("title").unwrap_or("").trim().to_owned();
            if title.is_empty() {
                errs.push("title", "title is required");
            }
            let urgent = form.get_bool("urgent");
            let note = form.get_optional_non_empty("note").map(str::to_owned);
            errs.into_result(Self {
                title,
                urgent,
                note,
            })
        }
    }

    let form = FormPairs::from_urlencoded_limited(
        "title=Fix+UTF-8+%E2%9C%93&urgent=on&note=%20%20",
        DEFAULT_MAX_FORM_FIELDS,
    )
    .unwrap();
    let decoded = FormInput::from_form(&form).unwrap();
    assert_eq!(decoded.title, "Fix UTF-8 ✓");
    assert!(decoded.urgent);
    assert_eq!(decoded.note, None);

    // Edge cases: trailing `%`, invalid `%zz`, `a=b=c`, `=v`, invalid UTF-8 `%FF`.
    let edge = decode_form_urlencoded_limited(
        "a=b=c&=val&trail=x%&bad=%zz&utf8=%ff",
        DEFAULT_MAX_FORM_FIELDS,
    )
    .unwrap();
    assert_eq!(edge[0], ("a".into(), "b=c".into()));
    assert_eq!(edge[1], ("".into(), "val".into()));
    assert_eq!(edge[2], ("trail".into(), "x%".into()));
    assert_eq!(edge[3], ("bad".into(), "%zz".into()));
    assert_eq!(edge[4], ("utf8".into(), "\u{FFFD}".into()));

    // Bounded pair cap rejects floods.
    let flood = "a=1&".repeat(20);
    assert!(decode_form_urlencoded_limited(&flood, 10).is_err());
    assert_eq!(decode_form_urlencoded_limited(&flood, 20).unwrap().len(), 20);
}

#[test]
fn hot_paths_perform_zero_heap_allocations() {
    #[derive(Clone, Copy)]
    enum IssueTag {
        List,
        One(u64),
    }
    impl CacheTag for IssueTag {
        fn tag64(&self) -> Tag64 {
            match *self {
                Self::List => Tag64::of("issues.list"),
                Self::One(id) => Tag64::keyed("issue", id),
            }
        }
    }

    let (sink_len, allocs) = count_allocs(|| {
        let mut sink = TagSink::new();
        sink.push(IssueTag::List);
        for id in 1..=7u64 {
            sink.push(IssueTag::One(id));
        }
        sink.len()
    });
    assert_eq!(sink_len, 8);
    assert_eq!(
        allocs, 0,
        "Tag64::keyed + inline TagSink must allocate 0 times"
    );

    let codec = CborCodec::default();
    let sample = IssueV1 {
        id: 7,
        title: "Zero-alloc encode".into(),
        open: true,
    };
    let mut buf = Vec::with_capacity(256);
    let (enc_res, enc_allocs) = count_allocs(|| {
        buf.clear();
        codec.encode_into(&sample, &mut buf)
    });
    enc_res.unwrap();
    assert_eq!(
        enc_allocs, 0,
        "CborCodec::encode_into must allocate 0 times"
    );

    let (val_res, val_allocs) = count_allocs(|| validate_cbor(&buf, CborLimits::DEFAULT));
    val_res.unwrap();
    assert_eq!(val_allocs, 0, "validate_cbor must allocate 0 times");

    let tags = [Tag64::of("a"), Tag64::keyed("b", 99)];
    let mut hdr = String::with_capacity(64);
    let (_, hdr_allocs) = count_allocs(|| format_invalidate_header_into(&tags, &mut hdr));
    assert_eq!(hdr.len(), 33);
    assert_eq!(
        hdr_allocs, 0,
        "format_invalidate_header_into must allocate 0 times"
    );
}
