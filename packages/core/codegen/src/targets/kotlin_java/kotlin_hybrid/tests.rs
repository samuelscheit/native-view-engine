//! Kotlin-target unit tests: unsupported-type rejection and boundary integer defaults.

use super::generate_hybrid_kotlin;
use crate::error::CodegenError;
use crate::frontend::parse_hybrid_rust_spec;

/// Every position a type can occupy, with `T` standing for the type under test.
const POSITIONS: [(&str, &str); 6] = [
    (
        "struct field",
        "#[nve_hybrid_struct]\npub struct S { pub a: T }",
    ),
    (
        "module parameter",
        "#[nve_hybrid_module(name = \"M\")]\npub trait MSpec { fn f(&self, a: T); }",
    ),
    (
        "module return",
        "#[nve_hybrid_module(name = \"M\")]\npub trait MSpec { fn f(&self) -> T; }",
    ),
    (
        "view prop",
        "#[nve_hybrid_view(name = \"V\")]\npub trait VSpec { #[prop] fn a(&self) -> T; }",
    ),
    (
        "view event",
        "#[nve_hybrid_view(name = \"V\")]\npub trait VSpec { #[event] fn on_a(&self, a: T); }",
    ),
    (
        "view method",
        "#[nve_hybrid_view(name = \"V\")]\npub trait VSpec { #[method] fn f(&self, a: T) -> T; }",
    ),
];

const ALIAS: &str = "#[nve_hybrid_alias]\npub type Cam = u64;\n";

fn kotlin(source: &str) -> Result<String, CodegenError> {
    let spec = parse_hybrid_rust_spec(source).expect("spec parses");
    spec.validate()
        .expect("the spec itself is valid: only the Kotlin target rejects it");
    generate_hybrid_kotlin(&spec).map(|files| files[0].content.clone())
}

/// Asserts `source` (a valid spec) is rejected by the Kotlin target alone.
fn assert_kotlin_rejects(source: &str, what: &str) {
    let result = kotlin(source);
    assert!(
        matches!(&result, Err(CodegenError::Validation(msg)) if msg.contains("hybrid Kotlin target")),
        "{what}: {result:?}"
    );
}

#[test]
fn unsupported_types_are_rejected_in_every_position_and_nesting() {
    for ty in ["[u8; 4]", "*const u8", "*mut u8", "Cam"] {
        for nested in [
            ty.to_string(),
            format!("Option<{ty}>"),
            format!("Vec<{ty}>"),
        ] {
            for (position, template) in POSITIONS {
                let source = format!("{ALIAS}{}", template.replace('T', &nested));
                assert_kotlin_rejects(&source, &format!("{position} `{nested}`"));
            }
        }
        let source = format!(
            "{ALIAS}#[nve_hybrid_module(name = \"M\")]\npub trait MSpec {{ fn f(&self) -> Result<{ty}, String>; }}"
        );
        assert_kotlin_rejects(&source, &format!("`Result<{ty}, String>` return"));
    }
}

#[test]
fn the_same_positions_generate_for_a_supported_type() {
    for (position, template) in POSITIONS {
        let source = format!("{ALIAS}{}", template.replace('T', "Option<Vec<u32>>"));
        assert!(kotlin(&source).is_ok(), "{position}");
    }
}

#[test]
fn boundary_integer_defaults_are_kotlin_constants() {
    let kt = kotlin(
        "#[nve_hybrid_view(name = \"V\")]
pub trait VSpec {
    #[prop(default = -9223372036854775808)]
    fn min_long(&self) -> i64;
    #[prop(default = -2147483648)]
    fn min_int(&self) -> i32;
    #[prop(default = 7)]
    fn size(&self) -> usize;
    #[prop(default = 18446744073709551615)]
    fn max_u64(&self) -> u64;
    #[prop(default = 4294967295)]
    fn max_u32(&self) -> u32;
    #[prop(default = 2147483648)]
    fn min_u32(&self) -> u32;
}",
    )
    .expect("generates");
    for line in [
        "val minLong: Long = Long.MIN_VALUE,",
        "val minInt: Int = Int.MIN_VALUE,",
        "val size: Long = 7L,",
        "val maxU64: Long = -1L,",
        "val maxU32: Int = -1,",
        "val minU32: Int = Int.MIN_VALUE",
    ] {
        assert!(kt.contains(line), "missing `{line}` in:\n{kt}");
    }
}
