use std::mem::size_of;

use crate::{DecodeError, NveBufferSlice};

use super::*;

#[test]
fn rejects_null_non_empty_misaligned_and_overflowing_raw_slices() {
    let valid_u8 = [1u8, 2, 3, 4];
    let valid_val = NveValue::i64(42);
    let valid_entry = NveMapEntry::new("ok", NveValue::bool(true));
    let valid_ev = NveDirectEvent::press(1, 2, 3, 4.0, 5.0);

    // 1. NveStrSlice: EMPTY (null, 0) succeeds; (null, >0) and (>isize::MAX) rejected.
    assert_eq!(unsafe { NveStrSlice::EMPTY.try_as_bytes() }, Some(&[][..]));
    assert_eq!(unsafe { NveStrSlice::EMPTY.as_str() }, Ok(""));

    let null_str = NveStrSlice {
        ptr: std::ptr::null(),
        len: 4,
    };
    assert_eq!(unsafe { null_str.try_as_bytes() }, None);
    assert_eq!(unsafe { null_str.as_bytes() }, &[][..]);
    assert_eq!(unsafe { null_str.as_str() }, Err(DecodeError::Truncated));

    let huge_str = NveStrSlice {
        ptr: valid_u8.as_ptr(),
        len: usize::MAX,
    };
    assert_eq!(unsafe { huge_str.try_as_bytes() }, None);
    assert_eq!(unsafe { huge_str.as_bytes() }, &[][..]);
    assert_eq!(unsafe { huge_str.as_str() }, Err(DecodeError::Truncated));

    let just_over_isize_str = NveStrSlice {
        ptr: valid_u8.as_ptr(),
        len: (isize::MAX as usize) + 1,
    };
    assert_eq!(unsafe { just_over_isize_str.try_as_bytes() }, None);

    let v_bad_str = NveValue::str(null_str);
    assert_eq!(unsafe { v_bad_str.as_ref() }.as_str(), None);
    assert_eq!(
        encode_bin_value_to_vec(unsafe { v_bad_str.as_ref() }),
        Err(DecodeError::Truncated)
    );

    // 2. NveValue::Bytes: (null, 0) valid empty; (null, >0) and usize::MAX rejected.
    let v_empty_bytes = NveValue::bytes(NveBufferSlice::EMPTY);
    assert_eq!(unsafe { v_empty_bytes.as_ref() }.as_bytes(), Some(&[][..]));

    let v_null_bytes = NveValue::bytes(NveBufferSlice {
        ptr: std::ptr::null(),
        len: 8,
    });
    assert_eq!(unsafe { v_null_bytes.as_ref() }.as_bytes(), None);
    assert_eq!(
        encode_bin_value_to_vec(unsafe { v_null_bytes.as_ref() }),
        Err(DecodeError::Truncated)
    );

    let v_huge_bytes = NveValue::bytes(NveBufferSlice {
        ptr: valid_u8.as_ptr(),
        len: usize::MAX,
    });
    assert_eq!(unsafe { v_huge_bytes.as_ref() }.as_bytes(), None);
    assert_eq!(
        encode_bin_value_to_vec(unsafe { v_huge_bytes.as_ref() }),
        Err(DecodeError::Truncated)
    );

    // 3. NveValue::Array: null+len>0, misaligned (0x1), and overflowing len rejected.
    let v_empty_arr = NveValue {
        tag: NveValueTag::Array as u32,
        data: NveValueData {
            array_val: NveValueArray {
                ptr: std::ptr::null(),
                len: 0,
            },
        },
    };
    assert_eq!(
        unsafe { v_empty_arr.as_ref() }.as_array().map(|a| a.len()),
        Some(0)
    );

    let hostile_arrays = [
        NveValueArray {
            ptr: std::ptr::null(),
            len: 1,
        },
        NveValueArray {
            ptr: std::ptr::without_provenance(1),
            len: 1,
        },
        NveValueArray {
            ptr: std::ptr::without_provenance(1),
            len: 0,
        },
        NveValueArray {
            ptr: &valid_val as *const NveValue,
            len: usize::MAX,
        },
        NveValueArray {
            ptr: &valid_val as *const NveValue,
            len: (isize::MAX as usize) / size_of::<NveValue>() + 1,
        },
    ];
    for arr in hostile_arrays {
        let v = NveValue {
            tag: NveValueTag::Array as u32,
            data: NveValueData { array_val: arr },
        };
        assert!(unsafe { v.as_ref() }.as_array().is_none());
        assert_eq!(
            encode_bin_value_to_vec(unsafe { v.as_ref() }),
            Err(DecodeError::Truncated)
        );
    }

    // 4. NveValue::Map and NveMapRef::from_raw_parts: null+len>0, misaligned (0x1), overflow.
    let hostile_maps = [
        NveValueMap {
            ptr: std::ptr::null(),
            len: 1,
        },
        NveValueMap {
            ptr: std::ptr::without_provenance(1),
            len: 1,
        },
        NveValueMap {
            ptr: std::ptr::without_provenance(1),
            len: 0,
        },
        NveValueMap {
            ptr: &valid_entry as *const NveMapEntry,
            len: usize::MAX,
        },
        NveValueMap {
            ptr: &valid_entry as *const NveMapEntry,
            len: (isize::MAX as usize) / size_of::<NveMapEntry>() + 1,
        },
    ];
    for map in hostile_maps {
        let v = NveValue {
            tag: NveValueTag::Map as u32,
            data: NveValueData { map_val: map },
        };
        assert!(unsafe { v.as_ref() }.as_map().is_none());
        assert_eq!(
            encode_bin_value_to_vec(unsafe { v.as_ref() }),
            Err(DecodeError::Truncated)
        );
    }

    assert!(unsafe { NveMapRef::try_from_raw_parts(std::ptr::null(), 0) }.is_some());
    assert!(unsafe { NveMapRef::try_from_raw_parts(std::ptr::null(), 1) }.is_none());
    assert!(unsafe { NveMapRef::try_from_raw_parts(std::ptr::without_provenance(1), 1) }.is_none());
    assert!(unsafe { NveMapRef::from_raw_parts(std::ptr::without_provenance(1), 1) }.is_empty());

    // Corrupt map entry key (null ptr with len > 0) must not match empty key "".
    let bad_key_entries = [NveMapEntry {
        key: null_str,
        value: NveValue::i64(99),
    }];
    let map_ref = unsafe { NveMapRef::from_slice(&bad_key_entries) };
    assert!(map_ref.get("").is_none());
    assert_eq!(map_ref.iter().count(), 0);

    // 5. NveEventBatch / NveEventBatchRef: null+len>0 and misaligned (0x1) rejected cleanly.
    assert_eq!(unsafe { NveEventBatch::EMPTY.try_as_slice() }.map(|s| s.len()), Some(0));
    let null_batch = NveEventBatch {
        ptr: std::ptr::null(),
        len: 2,
    };
    assert!(unsafe { null_batch.try_as_slice() }.is_none());
    assert!(unsafe { null_batch.as_slice() }.is_empty());
    assert!(unsafe { NveEventBatchRef::try_from_raw(null_batch) }.is_none());
    assert!(unsafe { NveEventBatchRef::from_raw(null_batch) }.is_empty());

    let misaligned_batch = NveEventBatch {
        ptr: std::ptr::without_provenance(1),
        len: 1,
    };
    assert!(unsafe { misaligned_batch.try_as_slice() }.is_none());
    assert!(unsafe { misaligned_batch.as_slice() }.is_empty());
    assert!(unsafe { NveEventBatchRef::try_from_raw(misaligned_batch) }.is_none());
    assert!(unsafe { NveEventBatchRef::from_raw(misaligned_batch) }.is_empty());

    // Sanity: valid aligned event batch still works zero-copy.
    let valid_batch = NveEventBatch {
        ptr: &valid_ev as *const NveDirectEvent,
        len: 1,
    };
    let batch_ref = unsafe { NveEventBatchRef::try_from_raw(valid_batch) }.expect("valid batch");
    assert_eq!(batch_ref.len(), 1);
    assert_eq!(batch_ref.get(0).unwrap().node_id(), 2);
}
