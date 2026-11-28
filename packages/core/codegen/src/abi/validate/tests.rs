//! ABI and table validation unit tests.

use nve_abi::InterfaceId;
use nve_abi::schema::{
    AbiAliasDescriptor, AbiConstDescriptor, AbiEnumDescriptor, AbiFieldDescriptor,
    AbiFileDescriptor, AbiMethodDescriptor, AbiModuleDescriptor, AbiParamDescriptor,
    AbiRecordDescriptor, AbiStructDescriptor,
};
use nve_abi::table::{FieldDesc, FieldKind, NO_VAL_WORD, OptEncoding, Storage, TableDesc};

use super::super::layout::layout_records;

const fn file(
    consts: &'static [AbiConstDescriptor],
    enums: &'static [AbiEnumDescriptor],
    records: &'static [AbiRecordDescriptor],
    modules: &'static [AbiModuleDescriptor],
) -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: None,
        consts,
        aliases: &[],
        enums,
        records,
        modules,
    }
}

/// A one-method interface module: signature checks run over every module method.
const fn module(methods: &'static [AbiMethodDescriptor]) -> AbiModuleDescriptor {
    AbiModuleDescriptor {
        name: "M",
        trait_name: "MSpec",
        interface_id: InterfaceId(1),
        c_name: "MApi",
        doc: None,
        methods,
    }
}

const fn record(
    name: &'static str,
    fields: &'static [AbiFieldDescriptor],
    size: usize,
    explicit_align: Option<usize>,
) -> AbiRecordDescriptor {
    AbiRecordDescriptor::Struct(AbiStructDescriptor {
        name,
        doc: None,
        size,
        // What `rustc` reports: an explicit alignment wins over the fields' natural 8.
        align: match explicit_align {
            Some(a) => a,
            None => 8,
        },
        explicit_align,
        expected_size: None,
        expected_size_64: None,
        fields,
    })
}

const fn field(name: &'static str, ty: &'static str, offset: usize) -> AbiFieldDescriptor {
    AbiFieldDescriptor {
        name,
        ty,
        c_type: None,
        doc: None,
        offset,
        expected_offset: None,
        expected_offset_64: None,
    }
}

fn error(f: &'static AbiFileDescriptor) -> String {
    match layout_records(&[f]).and_then(|r| super::abi(&[f], &r)) {
        Err(e) => e.to_string(),
        Ok(()) => String::from("<accepted>"),
    }
}

#[test]
fn consts_signatures_and_arrays_are_validated() {
    static BIG: AbiFileDescriptor = file(
        &[AbiConstDescriptor {
            name: "X",
            ty: "u32",
            value: 1 << 32,
            hex: true,
            doc: None,
        }],
        &[],
        &[],
        &[],
    );
    assert!(
        error(&BIG).contains("must be a u32 value"),
        "{}",
        error(&BIG)
    );

    static UNKNOWN: AbiFileDescriptor = file(&[], &[], &[], &[module(&[AbiMethodDescriptor { name: "f", params: &[AbiParamDescriptor { name: "a", ty: "*const Missing", }], return_type: "()", doc: None, has_self: false, optional: false, }])]);
    assert!(
        error(&UNKNOWN).contains("uses `Missing`"),
        "{}",
        error(&UNKNOWN)
    );

    static BY_VALUE_ARRAY: AbiFileDescriptor = file(
        &[],
        &[],
        &[],
        &[module(&[AbiMethodDescriptor {
            name: "f",
            params: &[],
            return_type: "[u8; 4]",
            doc: None,
            has_self: false,
            optional: false,
        }])],
    );
    assert!(
        error(&BY_VALUE_ARRAY).contains("arrays cannot cross"),
        "{}",
        error(&BY_VALUE_ARRAY)
    );

    static ZERO_LEN: AbiFileDescriptor = file(
        &[],
        &[],
        &[record("Z", &[field("a", "[u8; 0]", 0)], 0, None)],
        &[],
    );
    assert!(
        error(&ZERO_LEN).contains("zero-length arrays"),
        "{}",
        error(&ZERO_LEN)
    );

    static HUGE: AbiFileDescriptor = file(
        &[],
        &[],
        &[record(
            "H",
            &[field("a", "[u64; 18446744073709551615]", 0)],
            0,
            None,
        )],
        &[],
    );
    assert!(error(&HUGE).contains("overflows usize"), "{}", error(&HUGE));

    static ENUM_FIELD: AbiFileDescriptor = file(
        &[],
        &[AbiEnumDescriptor {
            name: "E",
            repr: "u8",
            doc: None,
            c_prefix: None,
            dart_fallback: None,
            variants: &[],
        }],
        &[record("R", &[field("e", "E", 0)], 1, None)],
        &[],
    );
    assert!(
        error(&ENUM_FIELD).contains("stores enum `E` by value"),
        "{}",
        error(&ENUM_FIELD)
    );

    // A Rust-only slab alignment (C keeps natural alignment) cannot cross a signature by value.
    static SLAB: AbiFileDescriptor = file(
        &[],
        &[],
        &[record(
            "Slab",
            &[field("words", "[u64; 8]", 0)],
            64,
            Some(64),
        )],
        &[module(&[AbiMethodDescriptor {
            name: "f",
            params: &[AbiParamDescriptor {
                name: "s",
                ty: "Slab",
            }],
            return_type: "*const Slab",
            doc: None,
            has_self: false,
            optional: false,
        }])],
    );
    assert!(
        error(&SLAB).contains("passes `Slab` by value"),
        "{}",
        error(&SLAB)
    );

    // `align(16)` on a 4-byte record changes its size everywhere: C can, Dart `ffi.Struct` cannot.
    static OVER: AbiFileDescriptor = file(
        &[],
        &[],
        &[record("O", &[field("a", "u32", 0)], 16, Some(16))],
        &[],
    );
    assert!(
        error(&OVER).contains("Dart `ffi.Struct` cannot express"),
        "{}",
        error(&OVER)
    );
}

const E_U8: AbiEnumDescriptor = AbiEnumDescriptor {
    name: "E",
    repr: "u8",
    doc: None,
    c_prefix: None,
    dart_fallback: None,
    variants: &[],
};

const fn function(
    params: &'static [AbiParamDescriptor],
    return_type: &'static str,
) -> AbiMethodDescriptor {
    AbiMethodDescriptor {
        name: "f",
        params,
        return_type,
        doc: None,
        has_self: false,
        optional: false,
    }
}

const fn param(ty: &'static str) -> AbiParamDescriptor {
    AbiParamDescriptor { name: "p", ty }
}

fn rejects(f: &'static AbiFileDescriptor, why: &str) {
    let message = error(f);
    assert!(message.contains(why), "expected `{why}`, got: {message}");
}

#[test]
fn enums_void_and_alignment_are_rejected_wherever_they_hide() {
    static ENUM_ARRAY: AbiFileDescriptor = file(
        &[],
        &[E_U8],
        &[record("R", &[field("e", "[E; 4]", 0)], 4, None)],
        &[],
    );
    rejects(&ENUM_ARRAY, "stores enum `E` by value");
    static ENUM_ALIAS: AbiFileDescriptor = AbiFileDescriptor {
        aliases: &[AbiAliasDescriptor {
            name: "A",
            target: "[E; 2]",
            doc: None,
        }],
        ..file(&[], &[E_U8], &[], &[])
    };
    rejects(&ENUM_ALIAS, "`A` stores enum `E` by value");
    // Behind a pointer an enum is fine (its size does not matter).
    static ENUM_PTR: AbiFileDescriptor = file(
        &[],
        &[E_U8],
        &[record("P", &[field("e", "*const E", 0)], 8, None)],
        &[],
    );
    assert_eq!(error(&ENUM_PTR), "<accepted>");

    static UNIT_PARAM: AbiFileDescriptor = file(&[], &[], &[], &[module(&[function(&[param("()")], "()")])]);
    rejects(&UNIT_PARAM, "`M::f`: `()` is only valid as a return type");
    static VOID_RETURN: AbiFileDescriptor = file(&[], &[], &[], &[module(&[function(&[], "c_void")])]);
    rejects(&VOID_RETURN, "`c_void` is only valid behind a pointer");
    static UNIT_POINTEE: AbiFileDescriptor = file(&[], &[], &[], &[module(&[function(&[param("*const ()")], "()")])]);
    rejects(&UNIT_POINTEE, "`()` is only valid as a return type");
    static VOID_PTR: AbiFileDescriptor = file(&[], &[], &[], &[module(&[function(&[param("*mut c_void")], "*const c_void")])]);
    assert_eq!(error(&VOID_PTR), "<accepted>");

    // Enums never cross a C signature: by value or behind a pointer.
    static ENUM_PARAM: AbiFileDescriptor = file(&[], &[E_U8], &[], &[module(&[function(&[param("E")], "()")])]);
    rejects(&ENUM_PARAM, "uses enum `E`");
    static ENUM_PTR_RET: AbiFileDescriptor = file(&[], &[E_U8], &[], &[module(&[function(&[], "*const E")])]);
    rejects(&ENUM_PTR_RET, "uses enum `E`");

    static ALIGN_0: AbiFileDescriptor = file(
        &[],
        &[],
        &[record("Z", &[field("a", "u32", 0)], 4, Some(0))],
        &[],
    );
    rejects(&ALIGN_0, "non-zero power of two");
    static ALIGN_3: AbiFileDescriptor = file(
        &[],
        &[],
        &[record("T", &[field("a", "u32", 0)], 4, Some(3))],
        &[],
    );
    rejects(&ALIGN_3, "non-zero power of two");
    // Past 16 C cannot over-align portably, so only a size- and layout-neutral slab alignment passes.
    static ALIGN_32: AbiFileDescriptor = file(
        &[],
        &[],
        &[record("W", &[field("a", "u32", 0)], 32, Some(32))],
        &[],
    );
    rejects(&ALIGN_32, "cannot over-align past 16");
    static OVERFLOW: AbiFileDescriptor = file(
        &[],
        &[],
        &[record(
            "V",
            &[
                field("a", "u8", 0),
                field("b", "[u64; 2305843009213693951]", 8),
            ],
            0,
            None,
        )],
        &[],
    );
    rejects(&OVERFLOW, "overflows usize");
}

const fn table(name: &'static str, fields: &'static [FieldDesc]) -> TableDesc {
    TableDesc {
        key: nve_hash::Key64::of(name),
        name,
        schema_hash: 0,
        storage: Storage::Sparse,
        flags: 0,
        defaults: &[0],
        word_effects: &[0],
        str_words: 0,
        resource_words: 0,
        events_word: NO_VAL_WORD,
        events: None,
        inherited: false,
        fields,
        resource_kinds: &[],
    }
}

const fn u32_field(name: &'static str) -> FieldDesc {
    FieldDesc {
        name,
        kind: FieldKind::U32,
        opt_encoding: OptEncoding::Required,
        word: 0,
        shift: 0,
        bits: 32,
        effects: 0,
        val_word: NO_VAL_WORD,
        no_inherit: false,
        values: None,
    }
}

#[test]
fn table_names_and_fields_are_valid_in_every_target() {
    static OK_FIELDS: [FieldDesc; 2] = [u32_field("set"), u32_field("get_value")];
    assert!(super::table(&table("nve.ok_2", &OK_FIELDS)).is_ok());
    for bad in [
        "nve.$x",
        "nve.Text",
        "",
        "1nve",
        "nve.te'xt",
        "nve.",
        "nve.1x",
        "nve..x",
        ".nve",
    ] {
        let t = table(bad, &[]);
        assert!(super::table(&t).is_err(), "{bad:?} must be rejected");
    }
    for (field, why) in [
        ("default", "reserved word `default`"),
        ("init", "reserved word `init`"),
        ("Bad", "must match"),
        ("where", "reserved word `where`"),
        ("words", "collides with a generated table member"),
        ("schema_hash", "collides with a generated table member"),
        ("hash_code", "collides with a generated table member"),
        ("to_string", "collides with a generated table member"),
        (
            "key_",
            "collides with a generated table member (`key`) as `key`",
        ),
        ("schema__hash", "collides with a generated table member"),
    ] {
        let fields = Box::leak(Box::new([u32_field(field)]));
        let message = super::table(&table("nve.t", fields))
            .map_or_else(|e| e.to_string(), |()| String::new());
        assert!(message.contains(why), "{field}: {message}");
    }
    static TWICE: [FieldDesc; 2] = [u32_field("x"), u32_field("x")];
    // Distinct field names whose target spellings collide.
    for (a, b, spelling) in [("foo_bar", "foo__bar", "fooBar"), ("foo_1", "foo1", "foo1")] {
        let fields = Box::leak(Box::new([u32_field(a), u32_field(b)]));
        let message = super::table(&table("nve.t", fields))
            .map_or_else(|e| e.to_string(), |()| String::new());
        assert!(
            message.contains(&format!(
                "collides with another field (`{a}`) as `{spelling}`"
            )),
            "{message}"
        );
    }
    let segment =
        super::table(&table("nve.1x", &[])).map_or_else(|e| e.to_string(), |()| String::new());
    assert!(segment.contains("name segment `1x`"), "{segment}");
    let twice =
        super::table(&table("nve.t", &TWICE)).map_or_else(|e| e.to_string(), |()| String::new());
    assert!(twice.contains("declared twice"), "{twice}");

    // Constant `GENERATED_TABLE_MEMBERS` table and allocation-free `push_snake_camel` stay exact.
    for &(snake, camel) in super::GENERATED_TABLE_MEMBERS {
        assert!(super::is_snake_ident(snake), "{snake}");
        assert_eq!(super::super::spell::camel(snake), camel);
    }
    for sample in [
        "a",
        "key_",
        "schema__hash",
        "foo_1",
        "foo_1_a",
        "foo_1a",
        "a___b___",
    ] {
        let mut buf = String::new();
        super::push_snake_camel(sample, &mut buf);
        assert_eq!(buf, super::super::spell::camel(sample), "{sample}");
    }
}
