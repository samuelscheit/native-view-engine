use crate::DecodeError;

use super::*;

/// `{"a": [1, "x"], "b": null}` spelled out byte by byte from the wire layout in `bin_codec.rs`.
fn expected_sample_bytes() -> Vec<u8> {
    let mut array = vec![2u8, 0, 0, 0]; // count
    array.push(NveValueTag::I64 as u8);
    array.extend_from_slice(&1i64.to_le_bytes());
    array.extend_from_slice(&[NveValueTag::Str as u8, 1, 0, 0, 0, b'x']);

    let mut map = vec![2u8, 0, 0, 0]; // count
    map.extend_from_slice(&[1, 0, 0, 0, b'a', NveValueTag::Array as u8]);
    map.extend_from_slice(&(array.len() as u32).to_le_bytes());
    map.extend_from_slice(&array);
    map.extend_from_slice(&[1, 0, 0, 0, b'b', NveValueTag::Null as u8]);

    let mut out = vec![NveValueTag::Map as u8];
    out.extend_from_slice(&(map.len() as u32).to_le_bytes());
    out.extend_from_slice(&map);
    out
}

fn write_sample(w: &mut BinWriter) {
    w.map(|m| {
        m.key("a").array(|a| {
            a.item().i64(1);
            a.item().str("x");
        });
        m.key("b").null();
    });
}

/// Nests `levels` single-element arrays around `null`.
fn nested_arrays(w: &mut BinWriter, levels: usize) {
    if levels == 0 {
        w.null();
    } else {
        w.array(|a| nested_arrays(a.item(), levels - 1));
    }
}

#[test]
fn writer_matches_the_wire_layout_and_patches_counts() {
    let mut w = BinWriter::new();
    write_sample(&mut w);
    let bytes = w.finish().expect("sample encodes");
    assert_eq!(bytes, expected_sample_bytes());

    let view = BinValueView::parse(&bytes).expect("writer output validates");
    assert_eq!(view.map_len(), Some(2));
    assert_eq!(view.get("a").and_then(|a| a.array_len()), Some(2));
}

#[test]
fn writer_appends_after_existing_bytes_and_encodes_every_scalar() {
    let mut w = BinWriter::from_vec(vec![0xEE]);
    w.array(|a| {
        a.item().bool(true);
        a.item().f64(-0.5);
        a.item().bytes(&[]);
        a.item().str("ü");
        a.item().map(|_| {});
        a.item().array(|_| {});
    });
    let bytes = w.finish().expect("encodes");
    assert_eq!(bytes[0], 0xEE);
    let view = BinValueView::parse(&bytes[1..]).expect("validates");
    let items: Vec<_> = view.as_array().expect("array").collect();
    assert_eq!(items[0].as_bool(), Some(true));
    assert_eq!(items[1].as_f64(), Some(-0.5));
    assert_eq!(items[2].as_bytes(), Some(&[][..]));
    assert_eq!(items[3].as_str(), Some("ü"));
    assert_eq!(items[4].map_len(), Some(0));
    assert_eq!(items[5].array_len(), Some(0));
}

#[test]
fn writer_enforces_the_decoder_depth_limit_and_keeps_the_first_error() {
    let mut ok = BinWriter::new();
    nested_arrays(&mut ok, NVE_BIN_VALUE_MAX_DEPTH);
    let deepest = ok.finish().expect("64 nested containers are allowed");
    BinValueView::parse(&deepest).expect("and the decoder accepts them");

    let mut too_deep = BinWriter::new();
    nested_arrays(&mut too_deep, NVE_BIN_VALUE_MAX_DEPTH + 1);
    assert!(!too_deep.is_ok());
    too_deep.fail(DecodeError::TooLarge);
    assert_eq!(too_deep.finish(), Err(DecodeError::MaxDepthExceeded));

    // `value` counts the embedded view's own nesting against the current depth.
    let view = BinValueView::parse(&deepest).expect("validates");
    let mut at_root = BinWriter::new();
    at_root.value(view);
    assert_eq!(at_root.finish().as_deref(), Ok(&deepest[..]));
    let mut nested = BinWriter::new();
    nested.array(|a| a.item().value(view));
    assert_eq!(nested.finish(), Err(DecodeError::MaxDepthExceeded));
}

#[test]
fn encode_bin_value_of_c_trees_uses_the_writer_layout() {
    let items = [NveValue::i64(1), NveValue::from_str("x")];
    let entries = [
        NveMapEntry::new("a", NveValue::array(&items)),
        NveMapEntry::new("b", NveValue::null()),
    ];
    let root = NveValue::map(&entries);
    // SAFETY: Every pointer in `root` borrows the locals above.
    let encoded = encode_bin_value_to_vec(unsafe { root.as_ref() }).expect("encodes");
    assert_eq!(encoded, expected_sample_bytes());

    let bad_tag = NveValue {
        tag: 99,
        data: NveValue::null().data,
    };
    // SAFETY: A scalar with no nested pointers.
    let err = encode_bin_value_to_vec(unsafe { bad_tag.as_ref() });
    assert_eq!(err, Err(DecodeError::InvalidEnum("NveValueTag", 99)));
}

fn build_tree(bytes: &[u8]) -> NveValueTree<'_> {
    NveValueTree::new(BinValueView::parse(bytes).expect("validates"))
}

#[test]
fn value_tree_round_trips_nested_containers_losslessly() {
    let mut w = BinWriter::new();
    w.map(|m| {
        write_sample(m.key("sample"));
        m.key("blob").bytes(&[0, 255, 7]);
        m.key("grid").array(|rows| {
            for r in 0..3i64 {
                rows.item()
                    .array(|row| (0..r).for_each(|c| row.item().i64(r * 10 + c)));
            }
        });
        m.key("empty_map").map(|_| {});
        m.key("flag").bool(false);
        m.key("ratio").f64(f64::MAX);
    });
    let bytes = w.finish().expect("encodes");

    // The tree is returned from a function: moving it must keep every arena pointer valid.
    let tree = build_tree(&bytes);
    let again = encode_bin_value_to_vec(tree.as_value_ref()).expect("tree re-encodes");
    assert_eq!(again, bytes);

    let root = tree.as_value_ref();
    let grid = root.get("grid").and_then(|g| g.as_array()).expect("grid");
    assert_eq!(grid.len(), 3);
    assert_eq!(
        grid.get(2)
            .and_then(|row| row.as_array())
            .and_then(|row| row.get(1)?.as_i64()),
        Some(21)
    );
    assert_eq!(
        root.get("blob").and_then(|b| b.as_bytes()),
        Some(&[0, 255, 7][..])
    );
}

#[test]
fn value_tree_of_scalars_and_wide_fan_out() {
    let mut w = BinWriter::new();
    w.str("solo");
    let solo = w.finish().expect("encodes");
    assert_eq!(build_tree(&solo).as_value_ref().as_str(), Some("solo"));

    // Breadth-first arena layout across many sibling containers of both kinds.
    let mut w = BinWriter::new();
    w.array(|a| {
        for i in 0..200i64 {
            a.item().map(|m| {
                m.key("i").i64(i);
                m.key("xs")
                    .array(|xs| (0..i % 5).for_each(|x| xs.item().i64(x)));
            });
        }
    });
    let wide = w.finish().expect("encodes");
    let tree = build_tree(&wide);
    assert_eq!(
        encode_bin_value_to_vec(tree.as_value_ref()).as_deref(),
        Ok(&wide[..])
    );
}

#[test]
fn a_failed_writer_stops_encoding() {
    let mut w = BinWriter::new();
    w.i64(5);
    let written = w.len();
    w.fail(DecodeError::TooLarge);
    let mut ran = false;
    w.array(|_| ran = true);
    w.map(|_| ran = true);
    w.str("ignored");
    w.bytes(&[1, 2]);
    w.i64(1);
    w.f64(1.0);
    w.bool(true);
    w.null();
    let mut v = BinWriter::new();
    v.str("copied");
    let copied = v.finish().expect("encodes");
    w.value(BinValueView::parse(&copied).expect("validates"));
    assert!(
        !ran,
        "container bodies are skipped once an error is recorded"
    );
    assert_eq!(w.len(), written, "nothing is written after the error");
    assert_eq!(w.finish(), Err(DecodeError::TooLarge));
}

#[test]
fn empty_containers_own_no_arena_block() {
    let mut w = BinWriter::new();
    w.array(|a| {
        a.item().array(|_| {});
        a.item().map(|_| {});
    });
    let bytes = w.finish().expect("encodes");
    let tree = build_tree(&bytes);
    let items = tree.as_value_ref().as_array().expect("array");
    for item in items.iter() {
        // SAFETY: Reads the union field selected by each item's tag.
        let ptr = unsafe {
            match item.tag() {
                Some(NveValueTag::Array) => item.raw().data.array_val.ptr.cast::<u8>(),
                _ => item.raw().data.map_val.ptr.cast::<u8>(),
            }
        };
        assert!(ptr.is_null());
    }
    assert_eq!(
        encode_bin_value_to_vec(tree.as_value_ref()).as_deref(),
        Ok(&bytes[..])
    );
}
