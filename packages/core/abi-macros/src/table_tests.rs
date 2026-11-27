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
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[bits(4294967297)] x: u8 }",
            "strictly narrowing integer primitives",
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
    assert!(
        out.contains("const __NVE_DEF_0_WF: Option < f32 > = (Some (1.5));"),
        "unexpected output:\n{out}"
    );
    assert!(out.contains("default_present: ::core::option::Option::is_some(&__NVE_DEF_0_WF)"));
    assert!(
        out.contains("const __NVE_DEF_1_WN: Option < u32 > = (:: core :: option :: Option :: None);"),
        "unexpected output:\n{out}"
    );
    assert!(out.contains("opt_bool_to_word(__NVE_DEF_2_OB)"));
    assert!(out.contains("<Dir>::opt_to_word(__NVE_DEF_3_OE)"));
    assert!(out.contains("const __NVE_DEF_4_NARROW_I: i8 = (- 3i8);"));
    assert!(out.contains("default for `narrow_i` overflows #[bits(4)] signed range"));
    assert!(out.contains("bits: 4u8"));
}

#[test]
fn derive_nve_table_rejects_magic_sentinels_split_bools_and_inherited_conflicts() {
    let cases = [
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(f32::NAN)] x: f32 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(::core::f64::INFINITY)] x: f64 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(-f32::INFINITY)] x: f32 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(f32::NEG_INFINITY)] x: f32 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default((u32::MAX))] x: u32 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(u8::MAX)] x: u8 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(i32::MIN)] x: i32 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(i64::MIN)] x: i64 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] color_enabled: bool, #[effect(paint)] color: u32 }",
            "split-bool sentinel pair",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] color_set: bool, #[effect(paint)] color: u32 }",
            "split-bool sentinel pair",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] color_present: bool, #[effect(paint)] color: u32 }",
            "split-bool sentinel pair",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] color_specified: bool, #[effect(paint)] color: u32 }",
            "split-bool sentinel pair",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] has_color: bool, #[effect(paint)] color: u32 }",
            "split-bool sentinel pair",
        ),
        (
            "#[nve_table(name = \"t.a\", inherited, events = MyEvent)] struct T { #[effect(paint)] x: u32 }",
            "cannot combine `inherited` and `events`",
        ),
        (
            "#[nve_table(name = \"t.a\", inherited = \"true\")] struct T { #[effect(paint)] x: u32 }",
            "bare flag",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(-f32::NAN)] x: f32 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[default(-f64::NEG_INFINITY)] x: f64 }",
            "magic sentinel default",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] has_foo_set: bool, #[effect(paint)] foo_set: u32 }",
            "split-bool sentinel pair",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] #[no_inherit] x: u32 }",
            "only valid on `#[nve_table(inherited)]`",
        ),
        (
            "#[nve_table(name = \"t.a\", inherited)] struct T { #[effect(paint)] explicit_mask: u32 }",
            "collides with generated `*Ref::explicit_mask()`",
        ),
        (
            "#[nve_table(name = \"t.a\", inherited)] struct T { #[effect(paint)] color: u32, #[effect(paint)] clear_color: u32 }",
            "collides with generated `clear_color()`",
        ),
        (
            "#[nve_table(name = \"t.a\", inherited)] struct T { #[effect(paint)] color: u32, #[effect(paint)] is_color_explicit: bool }",
            "collides with generated `is_color_explicit()`",
        ),
        (
            "#[nve_table(name = \"t.a\")] struct T { #[effect(paint)] get: u32 }",
            "collides with generated `*Mut::get()`",
        ),
    ];
    for (src, expected) in cases {
        let err = derive_tokens(&lex(src)).expect_err(&format!("expected error for: {src}"));
        assert!(
            err.contains(expected),
            "for `{src}`, expected `{expected}`, got `{err}`"
        );
    }

    // 33 fields on an inherited table exceeds the u32 explicit mask.
    let mut fields = String::new();
    for i in 0..33 {
        fields.push_str(&format!("#[effect(paint)] f{i}: bool, "));
    }
    let too_many = format!("#[nve_table(name = \"t.many\", inherited)] struct Many {{ {fields} }}");
    let err = derive_tokens(&lex(&too_many)).expect_err("expected >32 inherited fields error");
    assert!(err.contains("at most 32 fields"), "got: {err}");
}

#[test]
fn derive_nve_table_emits_inherited_table_explicit_helpers_and_branchless_resolve() {
    let src = r#"
        #[nve_table(name = "t.typo", storage = "sparse", flags = "publish", inherited)]
        pub struct Typo {
            #[effect(paint)] #[default(Rgba::BLACK)] pub color: Rgba,
            #[effect(measure)] pub family: Str,
            #[effect(measure)] pub max_lines: Option<u32>,
        }
    "#;
    let out = derive_tokens(&lex(src)).expect("valid inherited table");
    assert!(out.contains("inherited: true"), "missing inherited: true");
    assert!(out.contains("unsafe impl ::nve_store::InheritedTable for Typo"));
    assert!(out.contains("pub fn explicit_mask(&self) -> u32"));
    assert!(out.contains("pub fn is_color_explicit(&self) -> bool"));
    assert!(out.contains("pub fn is_family_explicit(&self) -> bool"));
    assert!(out.contains("pub fn clear_color(mut self) -> Self"));
    assert!(out.contains("pub fn clear_family(mut self) -> Self"));
    assert!(out.contains(".with_explicit(0u8, __F.effects)"));
    assert!(out.contains("self.0.set_explicit_bit(0u32, __F.effects);"));
    assert!(out.contains("self.0.clear_explicit_bit(0u32, __F.effects);"));
    assert!(out.contains("fn resolve_inherited_arrays("));
    assert!(out.contains("fn inherit_only("));
    assert!(out.contains("const INHERITED_FIELD_MASK: u32"));
}

#[test]
fn derive_nve_table_emits_no_inherit_mask_nonzero_generic_and_delim_none_paths() {
    use crate::tok::{render_expr_tokens, Delim, Tok};

    // Delim::None around `$t::CONST`, `$p::new(3)`, `Enum::$VAR` must NOT emit `(T)::CONST`.
    let path_toks = vec![
        Tok::Group(Delim::None, vec![Tok::Ident("MyType".into())]),
        Tok::Punct(':', true),
        Tok::Punct(':', false),
        Tok::Ident("DEFAULT".into()),
    ];
    assert_eq!(render_expr_tokens(&path_toks), "MyType :: DEFAULT");

    let multi_path = vec![
        Tok::Group(
            Delim::None,
            vec![
                Tok::Ident("core".into()),
                Tok::Punct(':', true),
                Tok::Punct(':', false),
                Tok::Ident("num".into()),
                Tok::Punct(':', true),
                Tok::Punct(':', false),
                Tok::Ident("NonZeroU16".into()),
            ],
        ),
        Tok::Punct(':', true),
        Tok::Punct(':', false),
        Tok::Ident("new".into()),
        Tok::Group(Delim::Paren, vec![Tok::Literal("3".into())]),
    ];
    assert_eq!(
        render_expr_tokens(&multi_path),
        "core :: num :: NonZeroU16 :: new (3)"
    );

    // Multi-token arithmetic expression inside Delim::None MUST still be parenthesized.
    let arith = vec![
        Tok::Group(
            Delim::None,
            vec![
                Tok::Literal("1".into()),
                Tok::Punct('+', false),
                Tok::Literal("2".into()),
            ],
        ),
        Tok::Punct('*', false),
        Tok::Literal("2".into()),
    ];
    assert_eq!(render_expr_tokens(&arith), "(1 + 2) * 2");

    // `$cond` before `{ ... }` in `if false && $cond { 1u32 } else { 2u32 }` MUST still be
    // parenthesized so `false && false || true` does not flip precedence.
    let if_cond = vec![
        Tok::Ident("if".into()),
        Tok::Ident("false".into()),
        Tok::Punct('&', true),
        Tok::Punct('&', false),
        Tok::Group(
            Delim::None,
            vec![
                Tok::Ident("false".into()),
                Tok::Punct('|', true),
                Tok::Punct('|', false),
                Tok::Ident("true".into()),
            ],
        ),
        Tok::Group(Delim::Brace, vec![Tok::Literal("1u32".into())]),
        Tok::Ident("else".into()),
        Tok::Group(Delim::Brace, vec![Tok::Literal("2u32".into())]),
    ];
    assert_eq!(
        render_expr_tokens(&if_cond),
        "if false && (false || true) {1u32} else {2u32}"
    );

    // Pure generic/turbofish paths inside Delim::None before `(...)` or `{...}` skip parens.
    let turbo_path = vec![
        Tok::Group(
            Delim::None,
            vec![
                Tok::Ident("Option".into()),
                Tok::Punct(':', true),
                Tok::Punct(':', false),
                Tok::Punct('<', false),
                Tok::Ident("u16".into()),
                Tok::Punct('>', false),
                Tok::Punct(':', true),
                Tok::Punct(':', false),
                Tok::Ident("Some".into()),
            ],
        ),
        Tok::Group(Delim::Paren, vec![Tok::Literal("3".into())]),
    ];
    assert_eq!(
        render_expr_tokens(&turbo_path),
        "Option :: < u16 > :: Some (3)"
    );

    // `#[no_inherit]` + `Option<core::num::NonZero<u16>>` + const finiteness check for f32.
    let src = r#"
        #[nve_table(name = "t.no_inh", storage = "sparse", inherited)]
        pub struct NoInh {
            #[effect(paint)] #[default(1.0)] pub scale: f32,
            #[effect(measure)] #[no_inherit] #[default(core::num::NonZero::new(2))] pub lines: Option<core::num::NonZero<u16>>,
        }
    "#;
    let out = derive_tokens(&lex(src)).expect("valid #[no_inherit] table");
    assert!(out.contains("INHERITED_FIELD_MASK: u32 = 0x00000001u32"), "got:\n{out}");
    assert!(out.contains("no_inherit: true"), "got:\n{out}");
    assert!(out.contains("opt_nz_u16_to_word(__NVE_DEF_1_LINES)"), "got:\n{out}");
    assert!(out.contains("0x7F80_0000"), "missing const finiteness guard:\n{out}");
}
