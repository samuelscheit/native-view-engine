use super::derive_tokens;
use crate::tok::lex;

#[test]
fn derive_nve_table_rejects_invalid_bits_and_effects_cleanly() {
    let cases = [
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[bits(8)] x: u8 }",
            "redundant or wider",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[bits(16)] x: u8 }",
            "redundant or wider",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[bits(1)] x: bool }",
            "narrowing integer primitives",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[bits(4)] x: Option<u8> }",
            "narrowing integer primitives",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[bits(8)] x: f32 }",
            "narrowing integer primitives",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[bits(8)] x: u64 }",
            "narrowing integer primitives",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect()] x: u8 }",
            "at least one effect",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint layout)] x: u8 }",
            "expected `,` between effect names",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default()] x: u8 }",
            "requires a constant expression",
        ),
    ];
    for (src, expected) in cases {
        let err = derive_tokens(&lex(src)).expect_err(&format!("expected error for: {src}"));
        assert!(
            err.contains(expected),
            "for `{src}`, expected `{expected}`, got `{err}`"
        );
    }
}

#[test]
fn derive_nve_table_parses_option_defaults_on_tokens_and_emits_opt_to_word() {
    let src = r#"
        #[nve_table(name = "t.opts", storage = "sparse")]
        pub struct Opts {
            #[effect(paint)] #[default(Some(1.5))] pub wf: Option<f32>,
            #[effect(paint)] #[default(::core::option::Option::None)] pub wn: Option<u32>,
            #[effect(paint)] #[default(Some(true))] pub ob: Option<bool>,
            #[effect(paint)] #[default(::core::option::Option::Some(Dir::Row))] pub oe: Option<Dir>,
            #[effect(paint)] #[bits(4)] #[default(-3i8)] pub narrow_i: i8,
        }
    "#;
    let out = derive_tokens(&lex(src)).expect("valid Opts table");
    assert!(out.contains("default_lo: f32::to_bits(1.5)"));
    assert!(out.contains("default_present: true"));
    assert!(out.contains("opt_bool_to_word(::core::option::Option::Some(true))"));
    assert!(out.contains("<Dir>::opt_to_word(::core::option::Option::Some(Dir :: Row))"));
    assert!(out.contains("bits: 4u8"));
}
