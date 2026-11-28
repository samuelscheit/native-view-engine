use std::mem::size_of;

use crate::{DecodeError, NveBufferSlice};

use super::*;

#[test]
fn rejects_null_non_empty_misaligned_and_overflowing_raw_slices() {
    let valid_u8 = [1u8, 2, 3, 4];
    let valid_val = NveValue::i64(42);
    let valid_entry = NveMapEntry::new("ok", NveValue::bool(true));
    let valid_ev = NveDirectEvent::press(1, 2, 3, 4.0, 5.0);

    // SAFETY: Adversarial boundary test — every FFI view method validates null, alignment, and
    // `isize::MAX` overflow before constructing any Rust slice, and valid stack buffers outlive all refs.
    unsafe {
        // 1. NveStrSlice: EMPTY (null, 0) succeeds; (null, >0) and (>isize::MAX) rejected.
        assert_eq!(NveStrSlice::EMPTY.try_as_bytes(), Some(&[][..]));
        assert_eq!(NveStrSlice::EMPTY.as_str(), Ok(""));

        let null_str = NveStrSlice {
            ptr: std::ptr::null(),
            len: 4,
        };
        assert_eq!(null_str.try_as_bytes(), None);
        assert_eq!(null_str.as_bytes(), &[][..]);
        assert_eq!(null_str.as_str(), Err(DecodeError::Truncated));

        let huge_str = NveStrSlice {
            ptr: valid_u8.as_ptr(),
            len: usize::MAX,
        };
        assert_eq!(huge_str.try_as_bytes(), None);
        assert_eq!(huge_str.as_bytes(), &[][..]);
        assert_eq!(huge_str.as_str(), Err(DecodeError::Truncated));

        let just_over_isize_str = NveStrSlice {
            ptr: valid_u8.as_ptr(),
            len: (isize::MAX as usize) + 1,
        };
        assert_eq!(just_over_isize_str.try_as_bytes(), None);

        let v_bad_str = NveValue::str(null_str);
        assert_eq!(v_bad_str.as_ref().as_str(), None);
        #[cfg(feature = "wire")]
        assert_eq!(
            encode_bin_value_to_vec(v_bad_str.as_ref()),
            Err(DecodeError::Truncated)
        );

        // 2. NveValue::Bytes: (null, 0) valid empty; (null, >0) and usize::MAX rejected.
        let v_empty_bytes = NveValue::bytes(NveBufferSlice::EMPTY);
        assert_eq!(v_empty_bytes.as_ref().as_bytes(), Some(&[][..]));

        let v_null_bytes = NveValue::bytes(NveBufferSlice {
            ptr: std::ptr::null(),
            len: 8,
        });
        assert_eq!(v_null_bytes.as_ref().as_bytes(), None);
        #[cfg(feature = "wire")]
        assert_eq!(
            encode_bin_value_to_vec(v_null_bytes.as_ref()),
            Err(DecodeError::Truncated)
        );

        let v_huge_bytes = NveValue::bytes(NveBufferSlice {
            ptr: valid_u8.as_ptr(),
            len: usize::MAX,
        });
        assert_eq!(v_huge_bytes.as_ref().as_bytes(), None);
        #[cfg(feature = "wire")]
        assert_eq!(
            encode_bin_value_to_vec(v_huge_bytes.as_ref()),
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
        assert_eq!(v_empty_arr.as_ref().as_array().map(|a| a.len()), Some(0));

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
            assert!(v.as_ref().as_array().is_none());
            #[cfg(feature = "wire")]
            assert_eq!(
                encode_bin_value_to_vec(v.as_ref()),
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
            assert!(v.as_ref().as_map().is_none());
            #[cfg(feature = "wire")]
            assert_eq!(
                encode_bin_value_to_vec(v.as_ref()),
                Err(DecodeError::Truncated)
            );
        }

        assert!(NveMapRef::try_from_raw_parts(std::ptr::null(), 0).is_some());
        assert!(NveMapRef::try_from_raw_parts(std::ptr::null(), 1).is_none());
        assert!(NveMapRef::try_from_raw_parts(std::ptr::without_provenance(1), 1).is_none());
        assert!(NveMapRef::from_raw_parts(std::ptr::without_provenance(1), 1).is_empty());

        // Corrupt map entry key (null ptr with len > 0) must not match empty key "".
        let bad_key_entries = [NveMapEntry {
            key: null_str,
            value: NveValue::i64(99),
        }];
        let map_ref = NveMapRef::from_slice(&bad_key_entries);
        assert!(map_ref.get("").is_none());
        assert_eq!(map_ref.iter().count(), 0);

        // 5. NveEventBatch / NveEventBatchRef: null+len>0 and misaligned (0x1) rejected cleanly.
        assert_eq!(
            NveEventBatch::EMPTY
                .try_as_slice()
                .map(<[NveDirectEvent]>::len),
            Some(0)
        );
        let null_batch = NveEventBatch {
            ptr: std::ptr::null(),
            len: 2,
            _pad: 0,
        };
        assert!(null_batch.try_as_slice().is_none());
        assert!(null_batch.as_slice().is_empty());
        assert!(NveEventBatchRef::try_from_raw(null_batch).is_none());
        assert!(NveEventBatchRef::from_raw(null_batch).is_empty());

        let misaligned_batch = NveEventBatch {
            ptr: std::ptr::without_provenance(1),
            len: 1,
            _pad: 0,
        };
        assert!(misaligned_batch.try_as_slice().is_none());
        assert!(misaligned_batch.as_slice().is_empty());
        assert!(NveEventBatchRef::try_from_raw(misaligned_batch).is_none());
        assert!(NveEventBatchRef::from_raw(misaligned_batch).is_empty());

        // Sanity: valid aligned event batch still works zero-copy.
        let valid_batch = NveEventBatch {
            ptr: &valid_ev as *const NveDirectEvent,
            len: 1,
            _pad: 0,
        };
        let batch_ref = NveEventBatchRef::try_from_raw(valid_batch).expect("valid batch");
        assert_eq!(batch_ref.len(), 1);
        assert_eq!(batch_ref.get(0).unwrap().node_id(), 2);
    }
}

#[test]
fn prop_slice_and_command_slice_zero_copy_roundtrip_and_guards() {
    // 1. NvePropSlice / NvePropSliceRef zero-copy roundtrip & O(n) field_id lookup.
    let label = "hello-zero-copy";
    let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
    let prop_entries = [
        NvePropEntry::new(1, NveValue::bool(true)),
        NvePropEntry::new(7, NveValue::i64(-12345)),
        NvePropEntry::new(42, NveValue::from_str(label)),
        NvePropEntry::new(99, NveValue::from_bytes(&payload)),
    ];
    let raw_props = NvePropSlice::from_slice(&prop_entries);
    assert_eq!(raw_props.len, 4);
    assert_eq!(raw_props._pad, 0);

    // SAFETY: Stack backing slices outlive the borrowed refs, and hostile raw structs are rejected by pointer/alignment validation before dereferencing.
    unsafe {
        let props_ref = NvePropSliceRef::try_from_raw(raw_props).expect("valid prop slice");
        assert_eq!(props_ref.len(), 4);
        assert!(!props_ref.is_empty());
        assert_eq!(props_ref.get(1).and_then(|v| v.as_bool()), Some(true));
        assert_eq!(props_ref.get(7).and_then(|v| v.as_i64()), Some(-12345));
        let borrowed_str = props_ref.get(42).and_then(|v| v.as_str()).unwrap();
        assert_eq!(borrowed_str, label);
        assert_eq!(borrowed_str.as_ptr(), label.as_ptr(), "must be zero-copy");
        let borrowed_bytes = props_ref.get(99).and_then(|v| v.as_bytes()).unwrap();
        assert_eq!(borrowed_bytes, &payload);
        assert_eq!(borrowed_bytes.as_ptr(), payload.as_ptr(), "must be zero-copy");
        assert!(props_ref.get(404).is_none());
        assert_eq!(props_ref.entry(2).map(|(id, _)| id), Some(42));
        assert!(props_ref.entry(4).is_none());
        let ids: Vec<u16> = props_ref.iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![1, 7, 42, 99]);

        // Empty / null / misaligned guards for NvePropSlice.
        let empty_props = NvePropSliceRef::try_from_raw(NvePropSlice::EMPTY).expect("empty");
        assert!(empty_props.is_empty());
        assert!(empty_props.get(1).is_none());

        let null_non_empty = NvePropSlice {
            entries: std::ptr::null(),
            len: 3,
            _pad: 0,
        };
        assert!(null_non_empty.try_as_slice().is_none());
        assert!(null_non_empty.as_slice().is_empty());
        assert!(NvePropSliceRef::try_from_raw(null_non_empty).is_none());
        assert!(NvePropSliceRef::from_raw(null_non_empty).is_empty());

        let misaligned_props = NvePropSlice {
            entries: std::ptr::without_provenance(1),
            len: 1,
            _pad: 0,
        };
        assert!(NvePropSliceRef::try_from_raw(misaligned_props).is_none());
        assert!(NvePropSliceRef::from_raw(misaligned_props).is_empty());

        // 2. NveCommandSlice / NveCommandSliceRef zero-copy roundtrip & guards.
        let cmd_target = "scrollTo";
        let cmd_args = [
            NveValue::from_str(cmd_target),
            NveValue::f64(120.5),
            NveValue::bool(true),
        ];
        let raw_cmd = NveCommandSlice::from_args(0x0009, &cmd_args);
        assert_eq!(raw_cmd.opcode, 9);
        assert_eq!(raw_cmd.arg_count, 3);
        assert_eq!(raw_cmd._pad, 0);

        let cmd_ref = NveCommandSliceRef::try_from_raw(raw_cmd).expect("valid command");
        assert_eq!(cmd_ref.opcode(), 9);
        assert_eq!(cmd_ref.len(), 3);
        assert!(!cmd_ref.is_empty());
        let arg0 = cmd_ref.arg(0).and_then(|v| v.as_str()).unwrap();
        assert_eq!(arg0, cmd_target);
        assert_eq!(arg0.as_ptr(), cmd_target.as_ptr(), "must be zero-copy");
        assert_eq!(cmd_ref.arg(1).and_then(|v| v.as_f64()), Some(120.5));
        assert_eq!(cmd_ref.arg(2).and_then(|v| v.as_bool()), Some(true));
        assert!(cmd_ref.arg(3).is_none());
        assert_eq!(cmd_ref.args().len(), 3);
        assert_eq!(cmd_ref.iter().count(), 3);

        // Empty / null / misaligned guards for NveCommandSlice.
        let empty_cmd =
            NveCommandSliceRef::try_from_raw(NveCommandSlice::empty(5)).expect("empty command");
        assert_eq!(empty_cmd.opcode(), 5);
        assert!(empty_cmd.is_empty());
        assert!(empty_cmd.arg(0).is_none());

        let null_cmd = NveCommandSlice {
            opcode: 7,
            arg_count: 2,
            _pad: 0,
            args: std::ptr::null(),
        };
        assert!(null_cmd.try_args_slice().is_none());
        assert!(null_cmd.args_slice().is_empty());
        assert!(NveCommandSliceRef::try_from_raw(null_cmd).is_none());
        let fallback_cmd = NveCommandSliceRef::from_raw(null_cmd);
        assert_eq!(fallback_cmd.opcode(), 7);
        assert!(fallback_cmd.is_empty());

        let misaligned_cmd = NveCommandSlice {
            opcode: 8,
            arg_count: 1,
            _pad: 0,
            args: std::ptr::without_provenance(1),
        };
        assert!(NveCommandSliceRef::try_from_raw(misaligned_cmd).is_none());
    }
}

#[test]
fn debug_nve_value_ref_caps_recursion_depth_and_byte_preview() {
    use std::mem::MaybeUninit;

    // 1. Byte preview: <= 64 bytes print in full; > 64 bytes truncate to 64 + total count.
    let small: Vec<u8> = (0..64u8).collect();
    let large: Vec<u8> = (0..200u8).collect();
    let v_small = NveValue::from_bytes(&small);
    let v_large = NveValue::from_bytes(&large);
    // SAFETY: Stack buffers outlive the borrowed refs.
    unsafe {
        let s_small = format!("{:?}", v_small.as_ref());
        assert!(!s_small.contains(".. ("), "64-byte slice prints in full: {s_small}");
        assert!(s_small.contains("63]"), "includes last byte: {s_small}");

        let s_large = format!("{:?}", v_large.as_ref());
        assert!(s_large.contains(".. (200 bytes)"), "truncates with total length: {s_large}");
        assert!(s_large.contains("63].. (200 bytes)"), "preview ends at 64th byte: {s_large}");
        assert!(!s_large.contains("199"), "does not dump tail bytes: {s_large}");
    }

    // 1b. String preview: <= 64 bytes print in full; > 64 bytes truncate on a UTF-8 char boundary.
    let small_str = "a".repeat(64);
    let mut utf8_boundary = "a".repeat(62);
    utf8_boundary.push('🚀'); // 4-byte UTF-8 sequence straddling byte 64 (bytes 62..66)
    utf8_boundary.push_str("tail-marker");
    let v_small_str = NveValue::from_str(&small_str);
    let v_utf8_str = NveValue::from_str(&utf8_boundary);
    // SAFETY: Stack strings outlive the borrowed refs.
    unsafe {
        let f_small = format!("{:?}", v_small_str.as_ref());
        assert!(!f_small.contains(".. ("), "64-byte string prints in full: {f_small}");
        let f_utf8 = format!("{:?}", v_utf8_str.as_ref());
        assert!(
            f_utf8.contains(&format!(".. ({} bytes)", utf8_boundary.len())),
            "large string truncates with byte count: {f_utf8}"
        );
        assert!(f_utf8.contains(&"a".repeat(62)), "keeps complete prefix before boundary: {f_utf8}");
        assert!(!f_utf8.contains('🚀'), "backs off multi-byte char crossing cap: {f_utf8}");
        assert!(!f_utf8.contains("tail-marker"), "omits tail past cap: {f_utf8}");
    }

    // 2. Deeply nested Array (20 levels) caps at depth 16 and prints `[...]`.
    // Construct through raw pointers derived from `MaybeUninit::as_mut_ptr` (no `&` before writes finish).
    let mut arr_nodes = MaybeUninit::<[NveValue; 21]>::uninit();
    let arr_base = arr_nodes.as_mut_ptr().cast::<NveValue>();
    // SAFETY: Every pointer stays within `arr_nodes` (stationary on the stack during formatting)
    // and all elements are initialized before any shared reference is formed.
    let s_arr = unsafe {
        arr_base.add(20).write(NveValue::i64(999));
        for i in (0..20).rev() {
            arr_base.add(i).write(NveValue {
                tag: NveValueTag::Array as u32,
                data: NveValueData {
                    array_val: NveValueArray {
                        ptr: arr_base.add(i + 1).cast_const(),
                        len: 1,
                    },
                },
            });
        }
        format!("{:?}", NveValueRef::from_raw(&*arr_base))
    };
    assert!(s_arr.contains("[...]"), "array depth capped at 16: {s_arr}");
    assert!(!s_arr.contains("999"), "leaf beyond depth 16 is omitted: {s_arr}");

    // 3. Deeply nested Map (20 levels) caps at depth 16 and prints `{...}`.
    let mut map_entries = MaybeUninit::<[NveMapEntry; 20]>::uninit();
    let map_base = map_entries.as_mut_ptr().cast::<NveMapEntry>();
    // SAFETY: Every pointer stays within `map_entries` (stationary on the stack during formatting)
    // and all entries are initialized before any shared reference is formed.
    let s_map = unsafe {
        map_base.add(19).write(NveMapEntry::new("k", NveValue::i64(777)));
        for i in (0..19).rev() {
            map_base.add(i).write(NveMapEntry::new(
                "k",
                NveValue {
                    tag: NveValueTag::Map as u32,
                    data: NveValueData {
                        map_val: NveValueMap {
                            ptr: map_base.add(i + 1).cast_const(),
                            len: 1,
                        },
                    },
                },
            ));
        }
        let root_map = NveValue {
            tag: NveValueTag::Map as u32,
            data: NveValueData {
                map_val: NveValueMap {
                    ptr: map_base.cast_const(),
                    len: 1,
                },
            },
        };
        format!("{:?}", root_map.as_ref())
    };
    assert!(s_map.contains("{...}"), "map depth capped at 16: {s_map}");
    assert!(!s_map.contains("777"), "leaf beyond depth 16 is omitted: {s_map}");

    // 4. Cyclic self-referential Array terminates cleanly without stack overflow or Stacked Borrows UB.
    let mut cyclic = MaybeUninit::<NveValue>::uninit();
    let cyclic_ptr = cyclic.as_mut_ptr();
    // SAFETY: Self-pointer uses the raw `as_mut_ptr()` provenance; shared reference is created only after write.
    let s_cyc = unsafe {
        cyclic_ptr.write(NveValue {
            tag: NveValueTag::Array as u32,
            data: NveValueData {
                array_val: NveValueArray {
                    ptr: cyclic_ptr.cast_const(),
                    len: 1,
                },
            },
        });
        format!("{:?}", NveValueRef::from_raw(&*cyclic_ptr))
    };
    assert!(s_cyc.contains("[...]"), "cyclic value terminates at depth cap: {s_cyc}");

    // 5. 4-element branching cycle `[&c, &c, &c, &c]` (4^16 = 4.29B without a global item budget)
    //    finishes instantaneously with strictly bounded output.
    let mut branch_cycle = MaybeUninit::<[NveValue; 4]>::uninit();
    let branch_base = branch_cycle.as_mut_ptr().cast::<NveValue>();
    // SAFETY: All 4 elements point to the stationary `branch_cycle` slice via raw pointer provenance.
    let s_branch = unsafe {
        for i in 0..4 {
            branch_base.add(i).write(NveValue {
                tag: NveValueTag::Array as u32,
                data: NveValueData {
                    array_val: NveValueArray {
                        ptr: branch_base.cast_const(),
                        len: 4,
                    },
                },
            });
        }
        format!("{:?}", NveValueRef::from_raw(&*branch_base))
    };
    assert!(s_branch.contains("(+") && s_branch.contains("more)"), "budget truncates branching cycle: {s_branch}");
    assert!(s_branch.len() < 8192, "branching cycle output stays bounded (got {} bytes)", s_branch.len());

    // 6. Wide Array and wide Map cap printed entries per container with `.. (+N more)`.
    let wide_elems: Vec<NveValue> = (0..50i64).map(NveValue::i64).collect();
    let wide_arr = NveValue {
        tag: NveValueTag::Array as u32,
        data: NveValueData {
            array_val: NveValueArray {
                ptr: wide_elems.as_ptr(),
                len: wide_elems.len(),
            },
        },
    };
    let map_keys: Vec<String> = (0..50).map(|i| format!("k{i}")).collect();
    let wide_entries: Vec<NveMapEntry> = map_keys
        .iter()
        .enumerate()
        .map(|(i, k)| NveMapEntry::new(k, NveValue::i64(i as i64)))
        .collect();
    let wide_map = NveValue {
        tag: NveValueTag::Map as u32,
        data: NveValueData {
            map_val: NveValueMap {
                ptr: wide_entries.as_ptr(),
                len: wide_entries.len(),
            },
        },
    };
    // SAFETY: Backing vectors outlive formatting.
    unsafe {
        let f_wide_arr = format!("{:?}", wide_arr.as_ref());
        assert!(f_wide_arr.contains(".. (+18 more)"), "wide array shows omitted count: {f_wide_arr}");
        assert!(f_wide_arr.contains("I64(Some(31))") && !f_wide_arr.contains("I64(Some(32))"));

        let f_wide_map = format!("{:?}", wide_map.as_ref());
        assert!(f_wide_map.contains(".. (+18 more)"), "wide map shows omitted count: {f_wide_map}");
        assert!(f_wide_map.contains("\"k31\"") && !f_wide_map.contains("\"k32\""));
    }
}
