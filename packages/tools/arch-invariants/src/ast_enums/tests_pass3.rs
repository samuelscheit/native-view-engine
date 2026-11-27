//! Regression tests covering Pass-2, Pass-3, and Pass-4 dual-review findings in `ast_enums`:
//! - structured `variant_head` (tuples, string literals with `(`/`{`/` if `), symmetric `shared_pats`,
//!   multi-guard `SynUnit` grouping (`rewrite_named_type`) and `matched_heads` anti-inflation gates,
//! - `FreeVarScanner` rejecting struct field/method poisoning (`dst.align` / `dst.align(1)`),
//!   closure/`for`/`if let` enclosing locals, and nested `fn` / `Item::Trait` local-scope reset,
//! - `{self}` imports (module + PascalCase enum), single-segment `use LocalEnum::*;` & `use self::Al::*;`,
//!   reverse-order `use Al::*; use ... as Al;` and `type Al = Align; use Al::*;`, inline `mod` scope
//!   isolation, distinct-crate local enums, unresolved prelude types, local `mod windows;` shadowing
//!   `EXTERNAL_SDK_ROOTS`, and aliased converters,
//! - `norm_binding_pat_inner` & `norm_subpat` uppercase non-binding preservation (`None`/`MAX`/`MIN`),
//!   nested `Pat::Or` `(A(x) | B(x)) | C(x)` binding sharing, 2-branch `Pat::Or` `if let` chains,
//!   tuple overflow skip (including nested match scanning), 4-arm `MirrorAtom::Bool` vs `Str`,
//!   identical `loop`/`async` bodies, `Stmt::Macro` matches, arm-block `use`, and trait const defaults.

use super::converters::scan_duplicate_enums_and_converters;
use super::fingerprint::{
    scan_duplicate_enum_match_arms, scan_duplicate_enum_match_arms_with_crates, DuplicateKind,
};
use super::legacy_tables::scan_match_tables;
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
    //    per side. Old `split(['(', '{'])` collapsed into 2 heads (`"a "` ×5 with 3/5 = 60% < 75%
    //    `MIN_ARM_INTERNAL_MATCH_PCT`, and `"a"` ×1), failing `matched_syn = 1/2 = 50% < 65%`.
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
    //    Old `split('(')` collapsed all 6 tuples to `""` -> 4/6 = 66.7% < 75% (`MIN_ARM_INTERNAL_MATCH_PCT`).
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

    // 4. Non-vacuous multi-guard anti-inflation tests exercising BOTH head ratio gates:
    //    Case A (`matched_head_count / shared_pats >= 70%`): 3 heads (`A`x3, `B`, disagreeing `C`).
    //    `matched_syn = 2/3 = 66.7% >= 65%` passes syntactic gate; old `shared_arms = 4/3` falsely
    //    passed; ONLY `matched_head_count = 2/3 = 66.7% < 70%` rejects it.
    let multi_guard_shared_gate = r#"
        fn mg_a(k: Kind) -> u8 {
            match k {
                Kind::A(x) if x == 1 => 1, Kind::A(x) if x == 2 => 2, Kind::A(x) if x == 3 => 3,
                Kind::B => 4, Kind::C => 5, _ => 0,
            }
        }
        fn mg_b(k: Kind) -> u8 {
            match k {
                Kind::A(x) if x == 1 => 1, Kind::A(x) if x == 2 => 2, Kind::A(x) if x == 3 => 3,
                Kind::B => 4, Kind::C => 99, _ => 0,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", multi_guard_shared_gate)]).is_empty(),
        "Case A: 3 guarded rows on A must not inflate matched_head_count/shared_pats (2/3 < 70%)"
    );

    //    Case B (`matched_head_count / min_heads >= 60%`): 6 heads each (`A`x4 + `B` shared, 4
    //    disjoint heads per side). `matched_syn = 2/2 = 100%` and `matched_head_count/shared_pats = 2/2 = 100%`
    //    both pass; old `shared_arms = 5/6 = 83% >= 60%` falsely passed; ONLY `matched_head_count = 2/6 = 33% < 60%` rejects it.
    let multi_guard_coverage_gate = r#"
        fn cov_a(k: Kind) -> u8 {
            match k {
                Kind::A(x) if x == 1 => 1, Kind::A(x) if x == 2 => 2,
                Kind::A(x) if x == 3 => 3, Kind::A(x) if x == 4 => 4,
                Kind::B => 5, Kind::C => 6, Kind::D => 7, Kind::E => 8, Kind::F => 9, _ => 0,
            }
        }
        fn cov_b(k: Kind) -> u8 {
            match k {
                Kind::A(x) if x == 1 => 1, Kind::A(x) if x == 2 => 2,
                Kind::A(x) if x == 3 => 3, Kind::A(x) if x == 4 => 4,
                Kind::B => 5, Kind::P => 10, Kind::Q => 11, Kind::R => 12, Kind::S => 13, _ => 0,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", multi_guard_coverage_gate)]).is_empty(),
        "Case B: 4 guarded rows on A must not inflate matched_head_count/min_heads (2/6 < 60%)"
    );
}

#[test]
fn free_var_scanner_avoids_field_method_poisoning_and_tracks_closures_and_nested_fns() {
    let field_and_method_poison = r#"
        fn apply_a(align: Align, dst: &mut Style) {
            match align { Align::Start => dst.align = 1, Align::Center => dst.align = 2, Align::End => dst.align = 3, Align::Stretch => dst.align = 4 }
        }
        fn apply_b(a: Align, out: &mut Style) {
            match a { Align::Start => out.align = 1, Align::Center => out.align = 2, Align::End => out.align = 3, Align::Stretch => out.align = 4 }
        }
        fn call_a(align: Align, dst: &mut Style) {
            match align { Align::Start => dst.align(1), Align::Center => dst.align(2), Align::End => dst.align(3), Align::Stretch => dst.align(4) }
        }
        fn call_b(a: Align, out: &mut Style) {
            match a { Align::Start => out.align(1), Align::Center => out.align(2), Align::End => out.align(3), Align::Stretch => out.align(4) }
        }
    "#;
    assert_eq!(scan_sources(&[("a.rs", field_and_method_poison)]).len(), 2);

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
    assert_eq!(scan_sources(&[("a.rs", nested_fn)]).len(), 1);
}

#[test]
fn self_imports_local_globs_reverse_globs_inline_mod_and_sdk_shadowing() {
    // 1. Non-vacuous PascalCase `{self}` tests (positive AND negative with 4 `Theme::*` arms).
    let sdk_self_4 = r#"
        use winit::window::Theme::{self, Light};
        fn w_a(t: Theme) -> u8 { match t { Light => 0, Theme::Dark => 1, Theme::High => 2, Theme::Auto => 3 } }
    "#;
    let sdk_explicit_4 = r#"
        fn w_b(t: winit::window::Theme) -> u8 {
            match t {
                winit::window::Theme::Light => 0, winit::window::Theme::Dark => 1,
                winit::window::Theme::High => 2, winit::window::Theme::Auto => 3,
            }
        }
    "#;
    assert_eq!(
        scan_with_crates(&[("a.rs", sdk_self_4, Some("h1")), ("b.rs", sdk_explicit_4, Some("h2"))]).len(),
        1,
        "positive `use winit::window::Theme::{{self, Light}};` must match explicit `winit::window::Theme`"
    );

    let sdk_self_5 = r#"
        use winit::window::Theme::{self, Light};
        fn w_a(t: Theme) -> u8 { match t { Light => 0, Theme::Dark => 1, Theme::High => 2, Theme::Auto => 3, Theme::System => 4 } }
    "#;
    let local_theme_5 = r#"
        pub enum Theme { Light, Dark, High, Auto, System }
        fn l_a(t: Theme) -> u8 { match t { Theme::Light => 0, Theme::Dark => 1, Theme::High => 2, Theme::Auto => 3, Theme::System => 4 } }
    "#;
    assert!(
        scan_with_crates(&[("w.rs", sdk_self_5, Some("nve_host")), ("l.rs", local_theme_5, Some("nve_ui"))]).is_empty(),
        "negative 5-arm (4 `Theme::*`) `use winit::window::Theme::{{self, Light}};` must NOT collide with local `Theme`"
    );

    // 2. Module `{self}` mirror + single-segment `use Foo::*;` cross-crate isolation + `use self::Al::*;`.
    let mod_self_a = r#"
        use nve_components::tables::{self, AlignItems};
        use nve_abi::FlexDirection::{self, Column};
        pub fn to_abi(d: tables::FlexDirection) -> FlexDirection {
            match d {
                tables::FlexDirection::Column => Column, tables::FlexDirection::Row => FlexDirection::Row,
                tables::FlexDirection::ColumnReverse => FlexDirection::ColumnReverse, tables::FlexDirection::RowReverse => FlexDirection::RowReverse,
            }
        }
    "#;
    let mod_self_b = r#"
        use nve_abi::{self, FlexDirection};
        use nve_components::tables::FlexDirection as TableFlexDir;
        pub fn from_abi(d: nve_abi::FlexDirection) -> TableFlexDir {
            match d {
                FlexDirection::Column => TableFlexDir::Column, FlexDirection::Row => TableFlexDir::Row,
                FlexDirection::ColumnReverse => TableFlexDir::ColumnReverse, FlexDirection::RowReverse => TableFlexDir::RowReverse,
            }
        }
    "#;
    let mir = scan_with_crates(&[("a.rs", mod_self_a, Some("nve_backend_headless")), ("b.rs", mod_self_b, Some("nve_dioxus"))]);
    assert_eq!(mir.len(), 1);
    assert_eq!(mir[0].kind, DuplicateKind::Mirrored);

    let local_glob_a = "pub enum Foo { A, B, C, D }\nfn fa(x: Foo) -> u8 { use Foo::*; match x { A => 1, B => 2, C => 3, D => 4 } }";
    let local_enum_b = "pub enum Foo { A, B, C, D }\nfn fb(x: Foo) -> u8 { match x { Foo::A => 1, Foo::B => 2, Foo::C => 3, Foo::D => 4 } }";
    assert!(
        scan_with_crates(&[("a.rs", local_glob_a, Some("crate_a")), ("b.rs", local_enum_b, Some("crate_b"))]).is_empty(),
        "single-segment `use Foo::*;` on a local enum in crate_a must NOT collide with crate_b's `Foo`"
    );

    // 3. Non-vacuous reverse-order `rev_glob`, `type_glob`, and `use self::Al::*;` tests.
    let rev_glob_a = "use Al::*;\nuse crate::Align as Al;\nfn fa(a: Align) -> u8 { match a { Start => 1, Center => 2, End => 3, Stretch => 4, Extra => 99 } }";
    let rev_glob_b = "use self::Al::*;\nuse crate::Align as Al;\nfn fb(a: Align) -> u8 { match a { Start => 1, Center => 2, End => 3, Stretch => 4, Extra => 5 } }";
    let fake_al = "pub enum Al { Start, Center, End, Stretch, Extra }\nfn fal(a: Al) -> u8 { match a { Al::Start => 1, Al::Center => 2, Al::End => 3, Al::Stretch => 4, Al::Extra => 99 } }";
    let type_glob = "use Al::*;\ntype Al = Align;\nfn ft(a: Align) -> u8 { match a { Start => 1, Center => 2, End => 3, Stretch => 4, Extra => 99 } }";
    let qualified = "fn fq(a: Align) -> u8 { match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4, Align::Extra => 5 } }";

    assert!(
        scan_sources(&[("a.rs", rev_glob_a), ("al.rs", fake_al)]).is_empty(),
        "reverse-order `use Al::*; use crate::Align as Al;` must leave zero stale `Al` in glob_enums"
    );
    let rev_pair = scan_sources(&[("a.rs", rev_glob_a), ("b.rs", rev_glob_b)]);
    assert_eq!(rev_pair.len(), 1, "two reverse-order/self::Al glob fns must collide on 4/5 partial dup");
    assert_eq!(rev_pair[0].enum_name, "Align");
    assert_eq!(
        scan_sources(&[("t.rs", type_glob), ("q.rs", qualified)]).len(),
        1,
        "`use Al::*; type Al = Align;` must collide on true 4/5 partial dup with qualified `Align::*`"
    );

    // 4. Inline `mod` scope isolation, unresolved prelude match, and local `mod windows;` shadowing SDK.
    let inline_mod = r#"
        use winit::window::Theme;
        fn outer(t: Theme) -> u8 { match t { Theme::Light => 1, Theme::Dark => 2, Theme::High => 3, Theme::Auto => 4 } }
        mod local_mod {
            pub enum Theme { Light, Dark, High, Auto }
            fn inner(t: Theme) -> u8 { match t { Theme::Light => 1, Theme::Dark => 2, Theme::High => 3, Theme::Auto => 4 } }
        }
    "#;
    assert!(scan_with_crates(&[("a.rs", inline_mod, Some("nve_ui"))]).is_empty());

    let prelude_a = "use nve_dioxus::prelude::*;\nfn fa(a: FlexAlign) -> u8 { match a { FlexAlign::Stretch => 0, FlexAlign::FlexStart => 1, FlexAlign::Center => 2, FlexAlign::FlexEnd => 3 } }";
    let explicit_b = "use nve_abi::FlexAlign;\nfn fb(a: FlexAlign) -> u8 { match a { FlexAlign::Stretch => 0, FlexAlign::FlexStart => 1, FlexAlign::Center => 2, FlexAlign::FlexEnd => 3 } }";
    assert_eq!(scan_with_crates(&[("a.rs", prelude_a, Some("app")), ("b.rs", explicit_b, Some("web"))]).len(), 1);

    let local_win_mod = "use crate::platform::windows;\nfn f(m: windows::WinMode) -> u8 { match m { windows::WinMode::A => 1, windows::WinMode::B => 2, windows::WinMode::C => 3, windows::WinMode::D => 4 } }";
    let ext_win_sdk = "fn g(m: windows::WinMode) -> u8 { match m { windows::WinMode::A => 1, windows::WinMode::B => 2, windows::WinMode::C => 3, windows::WinMode::D => 4 } }";
    assert!(
        scan_with_crates(&[("a.rs", local_win_mod, Some("nve_host")), ("b.rs", ext_win_sdk, Some("nve_win"))]).is_empty(),
        "local `use crate::platform::windows;` must shadow `EXTERNAL_SDK_ROOTS` and not collide with external `windows`"
    );

    let core_src = "pub enum AlignItems { Stretch, FlexStart, Center, FlexEnd }\npub enum FlexAlign { Stretch, FlexStart, Center, FlexEnd }\npub enum WinMode { A, B, C, D }";
    let conv_src = r#"
        use taffy::AlignItems;
        use crate::FlexAlign as Fa;
        use crate::platform::windows;
        pub fn convert_internal(a: nve_components::tables::AlignItems) -> Fa {
            match a {
                nve_components::tables::AlignItems::Stretch => Fa::Stretch, nve_components::tables::AlignItems::FlexStart => Fa::FlexStart,
                nve_components::tables::AlignItems::Center => Fa::Center, nve_components::tables::AlignItems::FlexEnd => Fa::FlexEnd,
            }
        }
        pub fn convert_win_mod(a: windows::WinMode) -> Fa {
            match a { windows::WinMode::A => Fa::Stretch, windows::WinMode::B => Fa::FlexStart, windows::WinMode::C => Fa::Center, windows::WinMode::D => Fa::FlexEnd }
        }
        pub fn nested_in_overflow(a: windows::WinMode) -> Fa {
            match (a, a) {
                (WinMode::A | WinMode::B | WinMode::C | WinMode::D | WinMode::E | WinMode::F | WinMode::G | WinMode::H | WinMode::I,
                 WinMode::A | WinMode::B | WinMode::C | WinMode::D | WinMode::E | WinMode::F | WinMode::G | WinMode::H | WinMode::I) => {
                    match a {
                        windows::WinMode::A => Fa::Stretch, windows::WinMode::B => Fa::FlexStart,
                        windows::WinMode::C => Fa::Center, windows::WinMode::D => Fa::FlexEnd,
                    }
                }
                _ => Fa::Stretch,
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
        "imported local `windows::WinMode` must NOT be misclassified as external `windows` SDK: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.reason.contains("`fn nested_in_overflow`")),
        "converter match nested inside an overflowed match arm body must still be scanned: {violations:?}"
    );

    let legacy_overflow = r#"
        pub fn disc_nested_in_overflow(x: (E, E), v: u8) -> Option<E> {
            match x {
                (E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I,
                 E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I) => Some(E::A),
                _ => match v { 0 => Some(E::A), 1 => Some(E::B), 2 => Some(E::C), _ => None },
            }
        }
    "#;
    let ast_l = syn::parse_file(legacy_overflow).expect("valid");
    let mut legacy_vs = Vec::new();
    scan_match_tables("packages/modules/x/src/lib.rs", legacy_overflow, &ast_l, &mut legacy_vs);
    assert!(
        legacy_vs.iter().any(|v| v.reason.contains("`fn disc_nested_in_overflow`")),
        "discriminant match nested inside an overflowed match arm body must still be scanned: {legacy_vs:?}"
    );
}

#[test]
fn pass4_binding_pats_subpat_consts_nested_or_bool_mirror_and_nested_items() {
    // 1. `norm_binding_pat_inner` (`None`/`MAX` vs `other`) AND `norm_subpat` (`Ev::A(MAX)` vs `Ev::A(MIN)`).
    let none_vs_other = r#"
        fn a(k: Kind, v: Option<i32>) -> i32 { match k { Kind::A => if let None = v { 1 } else { 0 }, Kind::B => if let None = v { 2 } else { 0 }, Kind::C => if let None = v { 3 } else { 0 }, Kind::D => if let None = v { 4 } else { 0 } } }
        fn b(k: Kind, v: Option<i32>) -> i32 { match k { Kind::A => if let other = v { 1 } else { 0 }, Kind::B => if let other = v { 2 } else { 0 }, Kind::C => if let other = v { 3 } else { 0 }, Kind::D => if let other = v { 4 } else { 0 } } }
        fn c(k: Kind, v: i32) -> i32 { match k { Kind::A => if let MAX = v { 1 } else { 0 }, Kind::B => if let MAX = v { 2 } else { 0 }, Kind::C => if let MAX = v { 3 } else { 0 }, Kind::D => if let MAX = v { 4 } else { 0 } } }
    "#;
    assert!(scan_sources(&[("a.rs", none_vs_other)]).is_empty());

    let subpat_consts = r#"
        fn max_fn(e: Ev) -> u8 { match e { Ev::A(MAX) => 1, Ev::B(MAX) => 2, Ev::C(MAX) => 3, Ev::D(MAX) => 4, _ => 0 } }
        fn min_fn(e: Ev) -> u8 { match e { Ev::A(MIN) => 1, Ev::B(MIN) => 2, Ev::C(MIN) => 3, Ev::D(MIN) => 4, _ => 0 } }
    "#;
    assert!(
        scan_sources(&[("a.rs", subpat_consts)]).is_empty(),
        "ALL_CAPS const subpatterns (`Ev::A(MAX)` vs `Ev::A(MIN)`) must NOT collapse to `_`"
    );

    // 2. Nested `Pat::Or` `(A(x) | B(x)) | C(x)` writes back shared `$lN` bindings.
    let nested_or = r#"
        fn or_a(k: Kind, v: Res) -> i32 { match k { Kind::A => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 1 } else { 0 }, Kind::B => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 2 } else { 0 }, Kind::C => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 3 } else { 0 }, Kind::D => if let (Res::X(n) | Res::Y(n)) | Res::Z(n) = v { n + 4 } else { 0 } } }
        fn or_b(k: Kind, v: Res) -> i32 { match k { Kind::A => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 1 } else { 0 }, Kind::B => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 2 } else { 0 }, Kind::C => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 3 } else { 0 }, Kind::D => if let Res::Z(m) | Res::Y(m) | Res::X(m) = v { m + 4 } else { 0 } } }
    "#;
    assert_eq!(scan_sources(&[("a.rs", nested_or)]).len(), 1);

    // 3. Non-vacuous 4-arm `MirrorAtom::Bool` vs `MirrorAtom::Str` + direct atom check.
    let bool_lit: syn::Lit = syn::parse_str("true").expect("lit");
    let str_lit: syn::Lit = syn::parse_str("\"true\"").expect("lit");
    assert_eq!(lit_to_key_and_atom(&bool_lit, false).map(|(_, a)| a), Some(MirrorAtom::Bool(true)));
    assert_eq!(lit_to_key_and_atom(&str_lit, false).map(|(_, a)| a), Some(MirrorAtom::Str("true".into())));
    let bool_vs_str_4arm = r#"
        fn flag_to_bool(f: Flag) -> bool { match f { Flag::A => true, Flag::B => false, Flag::C => true, Flag::D => false } }
        fn str_to_flag(s: &str) -> Option<Flag> { match s { "true" => Some(Flag::A), "false" => Some(Flag::B), "true" => Some(Flag::C), "false" => Some(Flag::D), _ => None } }
    "#;
    assert!(scan_sources(&[("a.rs", bool_vs_str_4arm)]).is_empty());

    // 4. Identical `loop`/`async`, `Stmt::Macro`, 2-branch `Pat::Or` `if let`, arm-block `use`,
    //    tuple overflow skip, and nested `ImplItem::Const` + `TraitItem::Fn` + `TraitItem::Const` defaults.
    let positive_misc = r#"
        fn a_loop(k: Kind, xs: &[i32]) { match k { Kind::A => for x in xs { push(x); }, Kind::B => while cond() { step(); }, Kind::C => loop { if done() { break; } }, Kind::D => { let _ = async { fetch().await }; } } }
        fn b_loop(k: Kind, ys: &[i32]) { match k { Kind::A => for y in ys { push(y); }, Kind::B => while cond() { step(); }, Kind::C => loop { if done() { break; } }, Kind::D => { let _ = async { fetch().await }; } } }
        fn stmt_macro_a(out: &mut String, a: Align) { write!(out, "{}", match a { Align::Start => "s", Align::Center => "c", Align::End => "e", Align::Stretch => "t" }).ok(); }
        fn stmt_macro_b(out: &mut String, a: Align) { assert_eq!(match a { Align::Start => "s", Align::Center => "c", Align::End => "e", Align::Stretch => "t" }, "s"); }
        fn if_let_2branch(a: Align) -> bool { if let Align::Start | Align::Center = a { true } else if let Align::End | Align::Stretch = a { false } else { false } }
        fn match_2branch(a: Align) -> bool { match a { Align::Start | Align::Center => true, Align::End | Align::Stretch => false } }
        fn arm_use_a(k: Kind) -> Target { match k { Kind::A => { use crate::Target as T; T::One } Kind::B => { use crate::Target as T; T::Two } Kind::C => { use crate::Target as T; T::Three } Kind::D => { use crate::Target as T; T::Four } } }
        fn arm_use_b(k: Kind) -> Target { match k { Kind::A => Target::One, Kind::B => Target::Two, Kind::C => Target::Three, Kind::D => Target::Four } }
        trait TopMarker {
            const TOP_MAP: u8 = match Align::Start { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 };
        }
        fn host() {
            trait Marker {
                const DEF_MAP: u8 = match Align::Start { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 };
                fn def_map(a: Align) -> u8 { match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 } }
            }
            struct S;
            impl Marker for S {
                const MAP: u8 = match Align::Start { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 };
            }
        }
        fn big_a(x: (E, E)) -> u8 { match x { (E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I, E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I) => 1, _ => 0 } }
        fn big_b(x: (E, E)) -> u8 { match x { (E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I, E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I) => 1, _ => 0 } }
    "#;
    let dups = scan_sources(&[("a.rs", positive_misc)]);
    assert_eq!(dups.len(), 10, "expected 10 hits (loop, Stmt::Macro, 2-branch if-let, arm-use, + C(4,2)=6 pairs among TOP_MAP/MAP/DEF_MAP/def_map; overflow skipped): {dups:?}");
    assert!(dups.iter().any(|d| d.symbol_a == "TopMarker::TOP_MAP" || d.symbol_b == "TopMarker::TOP_MAP"));
    assert!(dups.iter().any(|d| d.symbol_a == "host::Marker::DEF_MAP" || d.symbol_b == "host::Marker::DEF_MAP"));
}
