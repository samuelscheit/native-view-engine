use super::derive_tokens;
use crate::tok::lex;

#[test]
fn emits_fallible_by_default_even_with_default_attr() {
    let out = derive_tokens(&lex(r#"
        #[repr(u8)]
        pub enum Contrast {
            #[default]
            Standard = 0,
            High = 1,
        }
    "#))
    .expect("valid Contrast");
    assert!(out.contains("pub const COUNT: usize = 2;"));
    assert!(out.contains("pub const ALL: [Self; 2] = [Self::Standard, Self::High];"));
    assert!(out.contains("pub const fn as_u8(self) -> u8"));
    assert!(out.contains("pub const fn try_from_u8(__nve_raw: u8) -> ::core::option::Option<Self>"));
    assert!(!out.contains("pub const fn from_u8"), "must NOT emit total converter from #[default]");
}

#[test]
fn emits_total_fallback_signed_min_and_raw_idents() {
    let out = derive_tokens(&lex(r#"
        #[repr(i32)]
        #[repr_enum(fallback = r#Match)]
        pub enum SignedEdge {
            Min = -2147483648,
            Zero = 0,
            r#Match = 7,
        }
    "#))
    .expect("valid SignedEdge");
    assert!(out.contains("-2147483648_i32 => ::core::option::Option::Some(Self::Min)"));
    assert!(out.contains("7_i32 => ::core::option::Option::Some(Self::r#Match)"));
    assert!(out.contains("pub const fn from_i32(__nve_raw: i32) -> Self"));
    assert!(out.contains("::core::option::Option::None => Self::r#Match"));

    let out_i8 = derive_tokens(&lex(r#"
        #[repr(i8)]
        pub enum Tiny {
            Min = -128,
            Max = 127,
        }
    "#))
    .expect("valid i8 MIN/MAX");
    assert!(out_i8.contains("-128_i8 => ::core::option::Option::Some(Self::Min)"));
    assert!(out_i8.contains("127_i8 => ::core::option::Option::Some(Self::Max)"));
}

#[test]
fn rejects_invalid_repr_enum_inputs() {
    let cases = [
        ("enum NoRepr { A = 0 }", "requires `#[repr("),
        ("#[repr(C)] enum ReprC { A = 0 }", "found `#[repr(C)]`"),
        ("#[repr(usize)] enum ReprUsize { A = 0 }", "found `#[repr(usize)]`"),
        ("#[repr(u8)] enum Empty {}", "at least one variant"),
        ("#[repr(u8)] enum Tuple { A(u8) }", "unit-only"),
        ("#[repr(u8)] enum Named { A { x: u8 } }", "unit-only"),
        ("#[repr(u8)] enum Gen<T> { A }", "generic parameters"),
        ("#[repr(u8)] enum Dup { A = 1, B, C = 2 }", "duplicate discriminant 2"),
        ("#[repr(u8)] enum Big { A = 256 }", "out of range"),
        ("#[repr(u8)] enum Neg { A = -1 }", "out of range"),
        ("#[repr(u8)] enum Overflow { A = 255, B }", "overflows `u8`"),
        ("#[repr(u8)] #[repr_enum(fallback = Missing)] enum BadFb { A = 0 }", "does not match any variant"),
        ("#[repr(u8)] enum ExprDisc { A = 1 << 0 }", "integer literal discriminant"),
    ];
    for (src, needle) in cases {
        let err = derive_tokens(&lex(src)).expect_err(src);
        assert!(err.contains(needle), "for `{src}`, got `{err}`");
    }
}
