//! Regression tests covering Pass-2 and Pass-3 dual-review P1/P2 findings in `ast_enums`:
//! - structured `variant_head` (tuples, string literals with `(`/`{`/` if `), symmetric `shared_pats`,
//!   multi-guard `SynUnit` grouping (`rewrite_named_type`) and `matched_heads` anti-inflation gate,
//! - `FreeVarScanner` rejecting struct field/method poisoning (`dst.align` / `dst.align(1)`),
//!   closure/`for`/`if let` enclosing locals, and nested `fn` / `Item::Trait` local-scope reset,
//! - `{self}` imports for both modules and PascalCase enums, reverse-order `use Al::*; use ... as Al;`
//!   and `type Al = Align; use Al::*;`, inline `mod` scope isolation, distinct-crate local enums,
//!   unresolved prelude types, first-segment-only `EXTERNAL_SDK_ROOTS`, and aliased converters,
//! - `norm_binding_pat_inner` uppercase non-binding preservation (`None`/`MAX`/unit structs),
//!   nested `Pat::Or` `(A(x) | B(x)) | C(x)` binding sharing, 2-branch `Pat::Or` `if let` chains,
//!   tuple overflow skip, 4-arm `MirrorAtom::Bool` vs `Str`, identical `loop`/`async` bodies,
//!   `Stmt::Macro` matches, arm-block `use` ingestion, and nested `ImplItem::Const` symbols.

use super::converters::scan_duplicate_enums_and_converters;
use super::fingerprint::{
    scan_duplicate_enum_match_arms, scan_duplicate_enum_match_arms_with_crates, DuplicateKind,
};
use super::patterns::{lit_to_key_and_atom, MirrorAtom};

fn scan_sources(sources: &[(&str, &str)]) -> Vec<super::MatchDuplicate> {
    let parsed: Vec<(&str, &str, syn::File)> = sources
        .iter()
        .map(|&(p, s)| (p, s, syn::parse_file(s).expect("valid rust snippet")))
        .collect();
    let refs: Vec<(&str, &str, &syn::File)> = parsed.iter().map(|(p, s, a)| (*p, *s, a)).collect();
    scan_duplicate_enum_match_arms(&refs)
}

fn scan_with_crates(sources: &[(&str, &str, Option<&str>)]) -> Vec<super::MatchDuplicate> {
    let parsed: Vec<(&str, &str, syn::File, Option<&str>)> = sources
        .iter()
        .map(|&(p, s, c)| (p, s, syn::parse_file(s).expect("valid rust snippet"), c))
        .collect();
    let refs: Vec<(&str, &str, &syn::File, Option<&str>)> =
        parsed.iter().map(|(p, s, a, c)| (*p, *s, a, *c)).collect();
    scan_duplicate_enum_match_arms_with_crates(&refs)
}

#[test]
fn guarded_split_matches_if_else_and_tuple_or_string_variant_heads_stay_intact() {
    // 1. Reproduces `rewrite_named_type`: two guarded `Struct(name)` arms vs one `if / else if` arm.
    let guarded_a = r#"
        fn rewrite_a(ty: &mut Ty, enums: &Set, aliases: &Set) {
            match ty {
                Ty::Struct(name) if enums.contains(name) => *ty = Ty::Enum(name.clone()),
                Ty::Struct(name) if aliases.contains(name) => *ty = Ty::Handle(name.clone()),
                Ty::Optional(inner) | Ty::Vec(inner) | Ty::ConstPtr(inner) | Ty::MutPtr(inner) | Ty::FixedArray { elem: inner, .. } => {
                    rewrite_a(inner, enums, aliases);
                }
                Ty::Result { ok, err } => { rewrite_a(ok, enums, aliases); rewrite_a(err, enums, aliases); }
                _ => {}
            }
        }
    "#;
    let unguarded_b = r#"
        fn rewrite_a(ty: &mut Ty, enums: &Set, aliases: &Set) {
            match ty {
                Ty::Struct(name) => {
                    if enums.contains(name) { *ty = Ty::Enum(name.clone()); }
                    else if aliases.contains(name) { *ty = Ty::Handle(name.clone()); }
                }
                Ty::Optional(inner) | Ty::Vec(inner) | Ty::ConstPtr(inner) | Ty::MutPtr(inner) | Ty::FixedArray { elem: inner, .. } => {
                    rewrite_a(inner, enums, aliases);
                }
                Ty::Result { ok, err } => { rewrite_a(ok, enums, aliases); rewrite_a(err, enums, aliases); }
                _ => {}
            }
        }
    "#;
    assert_eq!(scan_sources(&[("a.rs", guarded_a), ("b.rs", unguarded_b)]).len(), 1);
    assert_eq!(scan_sources(&[("b.rs", unguarded_b), ("a.rs", guarded_a)]).len(), 1);

    // 2. Non-vacuous string-literal `variant_head` test: 4 matching + 2 distinct `"a (...)"` arms
    //    per side. Old `split(['(', '{'])` collapsed all 6 to `"a "` -> 4/6 = 66.7% < 70% (fail).
    let lit_a = r#"
        fn parse_a(s: &str) -> Option<Tok> {
            match s {
                "a (1)" => Some(Tok::A), "a (2)" => Some(Tok::B),
                "a {x}" => Some(Tok::C), "a if z" => Some(Tok::D),
                "a (e1)" => Some(Tok::E), "a (e2)" => Some(Tok::F), _ => None,
            }
        }
        fn parse_b(s: &str) -> Option<Tok> {
            match s {
                "a (1)" => Some(Tok::A), "a (2)" => Some(Tok::B),
                "a {x}" => Some(Tok::C), "a if z" => Some(Tok::D),
                "a (o1)" => Some(Tok::G), "a (o2)" => Some(Tok::H), _ => None,
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", lit_a)]).len(),
        1,
        "string literals containing '(', '{{', ' if ' must keep distinct structured variant_heads"
    );

    // 3. Non-vacuous tuple `variant_head` test: 4 matching + 2 distinct tuple arms per side.
    //    Old `split('(')` collapsed every tuple to `""` -> 4/6 = 66.7% < 70% (fail).
    let tup_heads = r#"
        fn tup_a(p: P, d: D) -> u8 {
            match (p, d) {
                (P::A, D::X(v)) => v + 1, (P::B, D::X(v)) => v + 2,
                (P::C, D::X(v)) => v + 3, (P::D, D::X(v)) => v + 4,
                (P::E, D::X(v)) => v + 5, (P::F, D::X(v)) => v + 6, _ => 0,
            }
        }
        fn tup_b(p: P, d: D) -> u8 {
            match (p, d) {
                (P::A, D::X(n)) => n + 1, (P::B, D::X(n)) => n + 2,
                (P::C, D::X(n)) => n + 3, (P::D, D::X(n)) => n + 4,
                (P::G, D::X(n)) => n + 7, (P::H, D::X(n)) => n + 8, _ => 0,
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", tup_heads)]).len(),
        1,
        "tuple variant_heads must stay structured per element instead of collapsing to empty string"
    );

    // 4. Multi-guard anti-inflation: 3 matching guarded arms on `A` + 1 on `B` + 2 disagreeing `C`/`D`
    //    gives `matched_heads = 2 / 4 = 50% < 70%` (must NOT inflate via `shared_arms = 4 / 4`).
    let multi_guard_inflation = r#"
        fn mg_a(k: Kind) -> u8 {
            match k {
                Kind::A(x) if x == 1 => 1, Kind::A(x) if x == 2 => 2, Kind::A(x) if x == 3 => 3,
                Kind::B => 4, Kind::C => 5, Kind::D => 6, _ => 0,
            }
        }
        fn mg_b(k: Kind) -> u8 {
            match k {
                Kind::A(x) if x == 1 => 1, Kind::A(x) if x == 2 => 2, Kind::A(x) if x == 3 => 3,
                Kind::B => 4, Kind::C => 99, Kind::D => 100, _ => 0,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", multi_guard_inflation)]).is_empty(),
        "multiple guarded rows on one head must not inflate matched_heads / shared_pats"
    );
}

#[test]
fn free_var_scanner_avoids_field_method_poisoning_and_tracks_closures_and_nested_fns() {
    // 1. Field `.align` AND method `.align(...)` matching param `align` must NOT poison `$r0`.
    let field_and_method_poison = r#"
        fn apply_a(align: Align, dst: &mut Style) {
            match align {
                Align::Start => dst.align = 1, Align::Center => dst.align = 2,
                Align::End => dst.align = 3, Align::Stretch => dst.align = 4,
            }
        }
        fn apply_b(a: Align, out: &mut Style) {
            match a {
                Align::Start => out.align = 1, Align::Center => out.align = 2,
                Align::End => out.align = 3, Align::Stretch => out.align = 4,
            }
        }
        fn call_a(align: Align, dst: &mut Style) {
            match align {
                Align::Start => dst.align(1), Align::Center => dst.align(2),
                Align::End => dst.align(3), Align::Stretch => dst.align(4),
            }
        }
        fn call_b(a: Align, out: &mut Style) {
            match a {
                Align::Start => out.align(1), Align::Center => out.align(2),
                Align::End => out.align(3), Align::Stretch => out.align(4),
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", field_and_method_poison)]).len(),
        2,
        "struct field `.align` and method `.align(...)` must not poison `$rN` pre-seeding"
    );

    // 2. Closure parameters `(a, b)` are tracked in `enclosing_locals` so swapped corners stay distinct.
    let closure_swapped = r#"
        fn map_corners(items: &[(Corner, f32, f32)]) -> (Vec<(f32, f32)>, Vec<(f32, f32)>) {
            let p = items.iter().map(|&(c, a, b)| match c {
                Corner::TopLeft => (a, b), Corner::TopRight => (b, a),
                Corner::BottomLeft => (a, a), Corner::BottomRight => (b, b),
            }).collect();
            let q = items.iter().map(|&(c, a, b)| match c {
                Corner::TopLeft => (b, a), Corner::TopRight => (a, b),
                Corner::BottomLeft => (b, b), Corner::BottomRight => (a, a),
            }).collect();
            (p, q)
        }
    "#;
    assert!(scan_sources(&[("a.rs", closure_swapped)]).is_empty());

    // 3. Non-vacuous nested `fn` reset: `outer(b, a)` reverses `(a, b)` relative to `inner_a(a, b, c)`
    //    and `top_b(a, b, c)` — without resetting `enclosing_locals`, `inner_a` inherits `b=$r0, a=$r1`.
    let nested_fn = r#"
        fn outer(b: f32, a: f32) {
            fn inner_a(a: f32, b: f32, c: Corner) -> (f32, f32) {
                match c {
                    Corner::TopLeft => (a, b), Corner::TopRight => (b, a),
                    Corner::BottomLeft => (a, a), Corner::BottomRight => (b, b),
                }
            }
            let _ = (b, a, inner_a);
        }
        fn top_b(a: f32, b: f32, c: Corner) -> (f32, f32) {
            match c {
                Corner::TopLeft => (a, b), Corner::TopRight => (b, a),
                Corner::BottomLeft => (a, a), Corner::BottomRight => (b, b),
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", nested_fn)]).len(),
        1,
        "nested fn must reset enclosing_locals so outer(b, a) does not flip inner_a(a, b)'s $rN order"
    );
}

#[test]
fn self_imports_reverse_globs_inline_mod_crate_isolation_and_sdk_path_precision() {
    // 1. Module `{self}` AND PascalCase enum `{self}` imports (`use winit::window::Theme::{self, Light};`
    //    and `use nve_abi::FlexDirection::{self, Column};`).
    let self_imp_a = r#"
        use nve_components::tables::{self, AlignItems};
        use nve_abi::FlexDirection::{self, Column};
        pub fn to_abi(d: tables::FlexDirection) -> FlexDirection {
            match d {
                tables::FlexDirection::Column => Column,
                tables::FlexDirection::Row => FlexDirection::Row,
                tables::FlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
                tables::FlexDirection::RowReverse => FlexDirection::RowReverse,
            }
        }
    "#;
    let self_imp_b = r#"
        use nve_abi::{self, FlexDirection};
        use nve_components::tables::FlexDirection as TableFlexDir;
        pub fn from_abi(d: nve_abi::FlexDirection) -> TableFlexDir {
            match d {
                FlexDirection::Column => TableFlexDir::Column,
                FlexDirection::Row => TableFlexDir::Row,
                FlexDirection::ColumnReverse => TableFlexDir::ColumnReverse,
                FlexDirection::RowReverse => TableFlexDir::RowReverse,
            }
        }
    "#;
    let dups = scan_with_crates(&[
        ("a.rs", self_imp_a, Some("nve_backend_headless")),
        ("b.rs", self_imp_b, Some("nve_dioxus")),
    ]);
    assert_eq!(dups.len(), 1, "module + PascalCase `{{self}}` imports must resolve cleanly: {dups:?}");
    assert_eq!(dups[0].kind, DuplicateKind::Mirrored);

    let sdk_self = r#"
        use winit::window::Theme::{self, Light};
        fn w_a(t: Theme) -> u8 { match t { Light => 0, Theme::Dark => 1, Theme::High => 2, Theme::Auto => 3 } }
    "#;
    let local_theme = r#"
        pub enum Theme { Light, Dark, High, Auto }
        fn l_a(t: Theme) -> u8 { match t { Theme::Light => 0, Theme::Dark => 1, Theme::High => 2, Theme::Auto => 3 } }
    "#;
    assert!(
        scan_with_crates(&[("w.rs", sdk_self, Some("nve_host")), ("l.rs", local_theme, Some("nve_ui"))]).is_empty(),
        "`use winit::window::Theme::{{self, Light}};` must resolve as external SDK and never collide with local `Theme`"
    );

    // 2. Two distinct workspace crates each defining a local `pub enum Foo` must NOT collide.
    let crate_a = "pub enum Foo { A, B, C, D }\nfn fa(x: Foo) -> u8 { match x { Foo::A => 1, Foo::B => 2, Foo::C => 3, Foo::D => 4 } }";
    let crate_b = "pub enum Foo { A, B, C, D }\nfn fb(x: Foo) -> u8 { match x { Foo::A => 1, Foo::B => 2, Foo::C => 3, Foo::D => 4 } }";
    assert!(
        scan_with_crates(&[("a.rs", crate_a, Some("crate_a")), ("b.rs", crate_b, Some("crate_b"))]).is_empty(),
        "local `pub enum Foo` in two distinct crates must NOT collide"
    );

    // 3. Reverse-order `use Al::*; use crate::Align as Al;` AND `type Al = Align; use Al::*;`
    //    must resolve `glob_enums` cleanly with ZERO stale `"Al"` entry (tested via 4/5 partial dup).
    let rev_glob = r#"
        use Al::*;
        use crate::Align as Al;
        fn f_a(a: Align) -> u8 { match a { Start => 1, Center => 2, End => 3, Stretch => 4, Extra => 99 } }
    "#;
    let type_glob = r#"
        use Al::*;
        type Al = Align;
        fn f_b(a: Align) -> u8 { match a { Start => 1, Center => 2, End => 3, Stretch => 4, Extra => 5 } }
    "#;
    let qualified = r#"
        fn f_c(a: Align) -> u8 { match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4, Align::Extra => 5 } }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", rev_glob), ("c.rs", qualified)]).len(),
        1,
        "reverse-order `use Al::*; use crate::Align as Al;` must match qualified `Align::*` on partial 4/5"
    );
    assert_eq!(
        scan_sources(&[("b.rs", type_glob), ("c.rs", qualified)]).len(),
        1,
        "`use Al::*; type Al = Align;` must match qualified `Align::*` on partial 4/5"
    );

    // 4. Inline `mod` starts a fresh import scope; unresolved prelude matches explicit crate import;
    //    `crate::platform::windows::Mode` is NOT misclassified as external `windows` SDK.
    let inline_and_windows = r#"
        use winit::window::Theme;
        fn outer(t: Theme) -> u8 { match t { Theme::Light => 1, Theme::Dark => 2, Theme::High => 3, Theme::Auto => 4 } }
        mod local_mod {
            pub enum Theme { Light, Dark, High, Auto }
            fn inner(t: Theme) -> u8 { match t { Theme::Light => 1, Theme::Dark => 2, Theme::High => 3, Theme::Auto => 4 } }
        }
    "#;
    assert!(scan_with_crates(&[("a.rs", inline_and_windows, Some("nve_ui"))]).is_empty());

    let prelude_a = "use nve_dioxus::prelude::*;\nfn fa(a: FlexAlign) -> u8 { match a { FlexAlign::Stretch => 0, FlexAlign::FlexStart => 1, FlexAlign::Center => 2, FlexAlign::FlexEnd => 3 } }";
    let explicit_b = "use nve_abi::FlexAlign;\nfn fb(a: FlexAlign) -> u8 { match a { FlexAlign::Stretch => 0, FlexAlign::FlexStart => 1, FlexAlign::Center => 2, FlexAlign::FlexEnd => 3 } }";
    assert_eq!(scan_with_crates(&[("a.rs", prelude_a, Some("app")), ("b.rs", explicit_b, Some("web"))]).len(), 1);

    let core_src = "pub enum AlignItems { Stretch, FlexStart, Center, FlexEnd }\npub enum FlexAlign { Stretch, FlexStart, Center, FlexEnd }\npub enum WinMode { A, B, C, D }";
    let conv_src = r#"
        use taffy::AlignItems;
        use crate::FlexAlign as Fa;
        pub fn convert_internal(a: nve_components::tables::AlignItems) -> Fa {
            match a {
                nve_components::tables::AlignItems::Stretch => Fa::Stretch,
                nve_components::tables::AlignItems::FlexStart => Fa::FlexStart,
                nve_components::tables::AlignItems::Center => Fa::Center,
                nve_components::tables::AlignItems::FlexEnd => Fa::FlexEnd,
            }
        }
        pub fn convert_win_mod(a: crate::platform::windows::WinMode) -> Fa {
            match a {
                crate::platform::windows::WinMode::A => Fa::Stretch,
                crate::platform::windows::WinMode::B => Fa::FlexStart,
                crate::platform::windows::WinMode::C => Fa::Center,
                crate::platform::windows::WinMode::D => Fa::FlexEnd,
            }
        }
    "#;
    let ast_c = syn::parse_file(core_src).expect("valid");
    let ast_u = syn::parse_file(conv_src).expect("valid");
    let mut violations = Vec::new();
    scan_duplicate_enums_and_converters(
        &[
            ("packages/core/components/src/lib.rs".into(), core_src, &ast_c),
            ("packages/backends/x/src/lib.rs".into(), conv_src, &ast_u),
        ],
        &mut violations,
    );
    assert!(violations.iter().any(|v| v.reason.contains("`fn convert_internal`")));
    assert!(
        violations.iter().any(|v| v.reason.contains("`fn convert_win_mod`")),
        "`crate::platform::windows::WinMode` must NOT be misclassified as external `windows` SDK: {violations:?}"
    );
}

#[test]
fn pass4_binding_pats_nested_or_bool_mirror_loops_macros_if_let_and_nested_items() {
    // 1. `norm_binding_pat_inner` must keep `None` / `MAX` verbatim (never turn into `$lN`).
    let none_vs_other = r#"
        fn a(k: Kind, v: Option<i32>) -> i32 {
            match k {
                Kind::A => if let None = v { 1 } else { 0 },
                Kind::B => if let None = v { 2 } else { 0 },
                Kind::C => if let None = v { 3 } else { 0 },
                Kind::D => if let None = v { 4 } else { 0 },
            }
        }
        fn b(k: Kind, v: Option<i32>) -> i32 {
            match k {
                Kind::A => if let other = v { 1 } else { 0 },
                Kind::B => if let other = v { 2 } else { 0 },
                Kind::C => if let other = v { 3 } else { 0 },
                Kind::D => if let other = v { 4 } else { 0 },
            }
        }
        fn c(k: Kind, v: i32) -> i32 {
            match k {
                Kind::A => if let MAX = v { 1 } else { 0 },
                Kind::B => if let MAX = v { 2 } else { 0 },
                Kind::C => if let MAX = v { 3 } else { 0 },
                Kind::D => if let MAX = v { 4 } else { 0 },
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", none_vs_other)]).is_empty(),
        "`if let None = v`, `if let other = v`, and `if let MAX = v` must NEVER collide"
    );

    // 2. Nested `Pat::Or` `(A(x) | B(x)) | C(x)` writes back shared `$lN` bindings.
    let nested_or = r#"
        fn or_a(k: Kind, v: Res) -> i32 {
            match k {
                Kind::A => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 1 } else { 0 },
                Kind::B => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 2 } else { 0 },
                Kind::C => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 3 } else { 0 },
                Kind::D => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 4 } else { 0 },
            }
        }
        fn or_b(k: Kind, v: Res) -> i32 {
            match k {
                Kind::A => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 1 } else { 0 },
                Kind::B => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 2 } else { 0 },
                Kind::C => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 3 } else { 0 },
                Kind::D => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 4 } else { 0 },
            }
        }
    "#;
    assert_eq!(scan_sources(&[("a.rs", nested_or)]).len(), 1);

    // 3. Non-vacuous 4-arm `MirrorAtom::Bool` vs `MirrorAtom::Str` test + direct atom check.
    let bool_lit: syn::Lit = syn::parse_str("true").expect("lit");
    let str_lit: syn::Lit = syn::parse_str("\"true\"").expect("lit");
    assert_eq!(lit_to_key_and_atom(&bool_lit, false).map(|(_, a)| a), Some(MirrorAtom::Bool(true)));
    assert_eq!(lit_to_key_and_atom(&str_lit, false).map(|(_, a)| a), Some(MirrorAtom::Str("true".into())));
    let bool_vs_str_4arm = r#"
        fn flag_to_bool(f: Flag) -> bool {
            match f { Flag::A => true, Flag::B => false, Flag::C => true, Flag::D => false }
        }
        fn str_to_flag(s: &str) -> Option<Flag> {
            match s { "true" => Some(Flag::A), "false" => Some(Flag::B), "true" => Some(Flag::C), "false" => Some(Flag::D), _ => None }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", bool_vs_str_4arm)]).is_empty(),
        "4-arm MirrorAtom::Bool must NOT mirror with MirrorAtom::Str(\"true\"/\"false\")"
    );

    // 4. Positive identical `loop`/`async`, `Stmt::Macro`, 2-branch `Pat::Or` `if let`, arm-block `use`,
    //    tuple overflow skip, and nested `ImplItem::Const` + nested `Item::Trait` default method symbols.
    let positive_misc = r#"
        fn a_loop(k: Kind, xs: &[i32]) {
            match k {
                Kind::A => for x in xs { push(x); }, Kind::B => while cond() { step(); },
                Kind::C => loop { if done() { break; } }, Kind::D => { let _ = async { fetch().await }; },
            }
        }
        fn b_loop(k: Kind, ys: &[i32]) {
            match k {
                Kind::A => for y in ys { push(y); }, Kind::B => while cond() { step(); },
                Kind::C => loop { if done() { break; } }, Kind::D => { let _ = async { fetch().await }; },
            }
        }
        fn stmt_macro_a(out: &mut String, a: Align) {
            write!(out, "{}", match a { Align::Start => "s", Align::Center => "c", Align::End => "e", Align::Stretch => "t" }).ok();
        }
        fn stmt_macro_b(out: &mut String, a: Align) {
            assert_eq!(match a { Align::Start => "s", Align::Center => "c", Align::End => "e", Align::Stretch => "t" }, "s");
        }
        fn if_let_2branch(a: Align) -> bool {
            if let Align::Start | Align::Center = a { true }
            else if let Align::End | Align::Stretch = a { false }
            else { false }
        }
        fn match_2branch(a: Align) -> bool {
            match a { Align::Start | Align::Center => true, Align::End | Align::Stretch => false }
        }
        fn arm_use_a(k: Kind) -> Target {
            match k {
                Kind::A => { use crate::Target as T; T::One } Kind::B => { use crate::Target as T; T::Two }
                Kind::C => { use crate::Target as T; T::Three } Kind::D => { use crate::Target as T; T::Four }
            }
        }
        fn arm_use_b(k: Kind) -> Target {
            match k { Kind::A => Target::One, Kind::B => Target::Two, Kind::C => Target::Three, Kind::D => Target::Four }
        }
        fn host() {
            trait Marker {
                const MAP: u8;
                fn def_map(a: Align) -> u8 { match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 } }
            }
            struct S;
            impl Marker for S {
                const MAP: u8 = match Align::Start { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 };
            }
        }
        fn big_a(x: (E, E)) -> u8 {
            match x { (E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I, E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I) => 1, _ => 0 }
        }
        fn big_b(x: (E, E)) -> u8 {
            match x { (E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I, E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I) => 1, _ => 0 }
        }
    "#;
    let dups = scan_sources(&[("a.rs", positive_misc)]);
    assert_eq!(dups.len(), 5, "expected 5 hits (loop, Stmt::Macro, 2-branch if-let, arm-use, nested trait<->const; overflow skipped): {dups:?}");
    assert!(
        dups.iter().any(|d| d.symbol_a == "host::<S as Marker>::MAP" && d.symbol_b == "host::Marker::def_map"),
        "nested ImplItem::Const and nested Item::Trait default method must qualify cleanly: {dups:?}"
    );
}
