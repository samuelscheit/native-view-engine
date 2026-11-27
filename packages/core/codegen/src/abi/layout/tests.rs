//! Layout and `rustc` cross-check unit tests.

use nve_abi::schema::{
    AbiFieldDescriptor, AbiFileDescriptor, AbiRecordDescriptor, AbiStructDescriptor,
};

use super::{Model, Ty, layout_records};

const fn field(
    name: &'static str,
    ty: &'static str,
    offset: usize,
    expected: Option<usize>,
) -> AbiFieldDescriptor {
    AbiFieldDescriptor {
        name,
        ty,
        c_type: None,
        doc: None,
        offset,
        expected_offset: expected,
        expected_offset_64: None,
    }
}

const fn file(records: &'static [AbiRecordDescriptor]) -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: None,
        consts: &[],
        aliases: &[],
        enums: &[],
        records,
        modules: &[],
        c_functions: &[],
    }
}

const fn record(
    fields: &'static [AbiFieldDescriptor],
    size: usize,
    expected_size: Option<usize>,
) -> AbiRecordDescriptor {
    AbiRecordDescriptor::Struct(AbiStructDescriptor {
        name: "S",
        doc: None,
        size,
        align: 8,
        explicit_align: None,
        expected_size,
        expected_size_64: None,
        fields,
    })
}

// `{ u8 tag; u64 value; const void* ptr; }` as `rustc` lays it out on this 64-bit host.
static OK: [AbiRecordDescriptor; 1] = [record(
    &[
        field("tag", "u8", 0, Some(0)),
        field("value", "u64", 8, None),
        field("ptr", "*const c_void", 16, None),
    ],
    24,
    None,
)];

#[test]
fn three_data_models_from_one_descriptor() {
    assert_eq!(
        Model::host(),
        Model::P64,
        "the codegen tests run on a 64-bit host"
    );
    let ok = file(&OK);
    let r = layout_records(&[&ok]).expect("valid layout");
    let [tag, value, ptr] = r[0].fields.as_slice() else {
        panic!("three fields")
    };
    assert_eq!(tag.offset.fixed(), Some(0));
    assert_eq!(
        value.offset.0,
        [8, 8, 4],
        "i386 System V aligns u64 to 4 inside structs"
    );
    assert_eq!(ptr.offset.0, [16, 16, 12]);
    assert_eq!(r[0].size.0, [24, 24, 16]);
    assert!(!r[0].is_fixed());
}

#[test]
fn drift_from_rustc_or_a_contract_is_an_error() {
    static WRONG_OFFSET: [AbiRecordDescriptor; 1] = [record(
        &[field("tag", "u8", 0, None), field("value", "u64", 4, None)],
        16,
        None,
    )];
    let err = layout_records(&[&file(&WRONG_OFFSET)])
        .unwrap_err()
        .to_string();
    assert!(err.contains("`value` offset 8 != rustc 4"), "{err}");

    static BROKEN_CONTRACT: [AbiRecordDescriptor; 1] = [record(
        &[
            field("tag", "u8", 0, None),
            field("value", "u64", 8, Some(8)),
        ],
        16,
        None,
    )];
    let err = layout_records(&[&file(&BROKEN_CONTRACT)])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("breaks `offset = Some(8)`"),
        "differs on i386: {err}"
    );

    static WRONG_SIZE: [AbiRecordDescriptor; 1] =
        [record(&[field("a", "u64", 0, None)], 8, Some(16))];
    assert!(
        layout_records(&[&file(&WRONG_SIZE)])
            .unwrap_err()
            .to_string()
            .contains("`size =")
    );

    static UNDECLARED: [AbiRecordDescriptor; 1] =
        [record(&[field("a", "Later", 0, None)], 4, None)];
    assert!(
        layout_records(&[&file(&UNDECLARED)])
            .unwrap_err()
            .to_string()
            .contains("before it is declared")
    );
}

#[test]
fn type_spellings() {
    assert_eq!(
        Ty::parse("*mut NveFrameStats").ok(),
        Some(Ty::Ptr {
            mutable: true,
            pointee: "NveFrameStats"
        })
    );
    assert_eq!(
        Ty::parse("[u32; 5]").ok(),
        Some(Ty::Array {
            elem: "u32",
            len: 5
        })
    );
    assert_eq!(Ty::parse("()").ok(), Some(Ty::Unit));
    assert!(Ty::parse("Option<fn()>").is_err());
    assert!(Ty::parse("[u8; x]").is_err());
}

/// A slab-aligned record (`align(64)`, size-neutral) embedded by value, also only as an array
/// element and with no aliases at all, changes the containing C layout: it is an error.
#[test]
fn slab_alignment_embedded_through_an_array_is_rejected() {
    static SLAB_IN_ARRAY: [AbiRecordDescriptor; 2] = [
        AbiRecordDescriptor::Struct(AbiStructDescriptor {
            name: "Slab",
            doc: None,
            size: 64,
            align: 64,
            explicit_align: Some(64),
            expected_size: None,
            expected_size_64: None,
            fields: &[field("words", "[u64; 8]", 0, None)],
        }),
        AbiRecordDescriptor::Struct(AbiStructDescriptor {
            name: "Holder",
            doc: None,
            size: 128,
            align: 64,
            explicit_align: None,
            expected_size: None,
            expected_size_64: None,
            fields: &[field("slabs", "[Slab; 2]", 0, None)],
        }),
    ];
    static F: AbiFileDescriptor = file(&SLAB_IN_ARRAY);
    let message = layout_records(&[&F])
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        message.contains("`align(64)` changes the C layout"),
        "{message}"
    );
}
