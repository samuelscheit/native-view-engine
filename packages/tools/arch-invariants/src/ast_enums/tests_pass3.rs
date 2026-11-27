//! Regression tests covering all Pass-2 dual-review P1/P2 findings in `ast_enums`:
//! - structured `variant_head` (tuples, string literals with `(`/`{`/` if `), symmetric `shared_pats`,
//!   and multi-guard grouping (restoring `rewrite_named_type`),
//! - `FreeVarScanner` rejecting struct field/method/path poisoning (`dst.align`), closure/`for`/`if let`
//!   enclosing locals, and nested `fn` local-scope reset,
//! - `{self}` imports, inline `mod` scope isolation, local defs vs unresolved single-segment types,
//!   `is_external_sdk_path` with `use taffy::AlignItems;` + `nve_components::tables::AlignItems`,
//! - chained alias-then-glob (`use crate::Align as Al; use Al::*;`), aliased converters,
//! - `Pat::Or` shared `$lN` bindings, 2-branch `if let` chains with `Pat::Or`, tuple overflow skip,
//!   `MirrorAtom::Bool` vs `"true"`, identical `loop`/`async` bodies, `Stmt::Macro` matches,
//!   and arm-block `use` ingestion.

use super::converters::scan_duplicate_enums_and_converters;
use super::fingerprint::{
    scan_duplicate_enum_match_arms, scan_duplicate_enum_match_arms_with_crates, DuplicateKind,
};

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
                Ty::Result { ok, err } => {
                    rewrite_a(ok, enums, aliases);
                    rewrite_a(err, enums, aliases);
                }
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
                Ty::Result { ok, err } => {
                    rewrite_a(ok, enums, aliases);
                    rewrite_a(err, enums, aliases);
                }
                _ => {}
            }
        }
    "#;
    // Symmetry check: both `(a, b)` and `(b, a)` must catch this duplicate.
    assert_eq!(
        scan_sources(&[("a.rs", guarded_a), ("b.rs", unguarded_b)]).len(),
        1,
        "multi-guard split on Struct vs single if/else arm must collide (a, b)"
    );
    assert_eq!(
        scan_sources(&[("b.rs", unguarded_b), ("a.rs", guarded_a)]).len(),
        1,
        "multi-guard split on Struct vs single if/else arm must collide symmetrically (b, a)"
    );

    // 2. String literal patterns containing `(`, `{`, or `" if "` must keep distinct variant_heads.
    let lit_a = r#"
        fn parse_a(s: &str) -> Option<Tok> {
            match s {
                "a (1)" => Some(Tok::A), "a (2)" => Some(Tok::B),
                "a {x}" => Some(Tok::C), "a if z" => Some(Tok::D),
                "a (extra)" => Some(Tok::E), _ => None,
            }
        }
        fn parse_b(s: &str) -> Option<Tok> {
            match s {
                "a (1)" => Some(Tok::A), "a (2)" => Some(Tok::B),
                "a {x}" => Some(Tok::C), "a if z" => Some(Tok::D),
                "a (other)" => Some(Tok::F), _ => None,
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", lit_a)]).len(),
        1,
        "string literals containing '(', '{{', ' if ' must not collapse into the same variant_head"
    );
}

#[test]
fn free_var_scanner_avoids_field_method_poisoning_and_tracks_closures_and_nested_fns() {
    // 1. Field name `.align` matching param name `align` must NOT poison `$r0` pre-seeding!
    let field_poison = r#"
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
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", field_poison)]).len(),
        1,
        "struct field `.align` must not poison `$rN` pre-seeding for param `align`"
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
    assert!(
        scan_sources(&[("a.rs", closure_swapped)]).is_empty(),
        "closure params must be tracked in enclosing_locals"
    );

    // 3. Nested `fn` resets `enclosing_locals` (cannot capture outer fn locals).
    let nested_fn = r#"
        fn outer(out: &mut Style, align: Align) {
            fn inner_a(dst: &mut Style, a: Align) {
                match a { Align::Start => dst.x = 1, Align::Center => dst.x = 2, Align::End => dst.x = 3, Align::Stretch => dst.x = 4 }
            }
            let _ = (out, align);
        }
        fn top_b(dst: &mut Style, a: Align) {
            match a { Align::Start => dst.x = 1, Align::Center => dst.x = 2, Align::End => dst.x = 3, Align::Stretch => dst.x = 4 }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", nested_fn)]).len(),
        1,
        "nested fn must reset enclosing_locals and collide with top-level equivalent"
    );
}

#[test]
fn self_imports_inline_mod_isolation_unresolved_prelude_and_sdk_path_precision() {
    // 1. `{self}` imports (`use crate::tables::{self, ...}` and `use nve_abi::{self, ...}`).
    let self_imp_a = r#"
        use nve_components::tables::{self, AlignItems};
        use nve_abi::{self, FlexDirection};
        pub fn to_abi(d: tables::FlexDirection) -> nve_abi::FlexDirection {
            match d {
                tables::FlexDirection::Column => nve_abi::FlexDirection::Column,
                tables::FlexDirection::Row => nve_abi::FlexDirection::Row,
                tables::FlexDirection::ColumnReverse => nve_abi::FlexDirection::ColumnReverse,
                tables::FlexDirection::RowReverse => nve_abi::FlexDirection::RowReverse,
            }
        }
    "#;
    let self_imp_b = r#"
        use nve_abi::FlexDirection;
        use nve_components::tables::FlexDirection as TableFlexDir;
        pub fn from_abi(d: FlexDirection) -> TableFlexDir {
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
    assert_eq!(dups.len(), 1, "`use ...::{{self}}` imports must resolve crate origins cleanly: {dups:?}");
    assert_eq!(dups[0].kind, DuplicateKind::Mirrored);

    // 2. Inline `mod` starts a fresh import scope (does NOT inherit parent `use winit::window::Theme;`).
    let inline_mod_src = r#"
        use winit::window::Theme;
        fn outer(t: Theme) -> u8 {
            match t { Theme::Light => 1, Theme::Dark => 2, Theme::High => 3, Theme::Auto => 4 }
        }
        mod local_mod {
            pub enum Theme { Light, Dark, High, Auto }
            fn inner(t: Theme) -> u8 {
                match t { Theme::Light => 1, Theme::Dark => 2, Theme::High => 3, Theme::Auto => 4 }
            }
        }
    "#;
    assert!(
        scan_with_crates(&[("a.rs", inline_mod_src, Some("nve_ui"))]).is_empty(),
        "inline `mod` must NOT inherit parent `use winit::window::Theme;`"
    );

    // 3. Unresolved single-segment type (`use nve_dioxus::prelude::*;`) matches explicit `nve_abi::FlexAlign`.
    let prelude_a = r#"
        use nve_dioxus::prelude::*;
        fn f_a(a: FlexAlign) -> u8 {
            match a { FlexAlign::Stretch => 0, FlexAlign::FlexStart => 1, FlexAlign::Center => 2, FlexAlign::FlexEnd => 3 }
        }
    "#;
    let explicit_b = r#"
        use nve_abi::FlexAlign;
        fn f_b(a: FlexAlign) -> u8 {
            match a { FlexAlign::Stretch => 0, FlexAlign::FlexStart => 1, FlexAlign::Center => 2, FlexAlign::FlexEnd => 3 }
        }
    "#;
    assert_eq!(
        scan_with_crates(&[
            ("app.rs", prelude_a, Some("studio_app")),
            ("abi.rs", explicit_b, Some("nve_backend_web")),
        ])
        .len(),
        1,
        "unresolved prelude-imported type must match explicit workspace crate import"
    );

    // 4. `use taffy::AlignItems;` must NOT misclassify `nve_components::tables::AlignItems` as SDK
    //    in `converters.rs`, AND aliased internal converters (`use Align as Al;`) MUST be caught.
    let core_src = "pub enum AlignItems { Stretch, FlexStart, Center, FlexEnd }\npub enum FlexAlign { Stretch, FlexStart, Center, FlexEnd }";
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
    assert!(
        violations.iter().any(|v| v.reason.contains("`fn convert_internal`")),
        "qualified internal path + aliased return must be caught even when `use taffy::AlignItems;` is in scope: {violations:?}"
    );
}

#[test]
fn pass3_p2_edge_cases_loops_stmt_macro_alias_glob_if_let_or_pat_or_overflow_and_bool_mirror() {
    // 1. Identical `loop`/`async` bodies DO collide, and `Stmt::Macro` (`write!`/`assert_eq!`) is visited.
    let positive_loops_and_macro = r#"
        fn a_loop(k: Kind, xs: &[i32]) {
            match k {
                Kind::A => for x in xs { push(x); },
                Kind::B => while cond() { step(); },
                Kind::C => loop { if done() { break; } },
                Kind::D => { let _ = async { fetch().await }; },
            }
        }
        fn b_loop(k: Kind, ys: &[i32]) {
            match k {
                Kind::A => for y in ys { push(y); },
                Kind::B => while cond() { step(); },
                Kind::C => loop { if done() { break; } },
                Kind::D => { let _ = async { fetch().await }; },
            }
        }
        fn stmt_macro_a(out: &mut String, a: Align) {
            write!(out, "{}", match a { Align::Start => "s", Align::Center => "c", Align::End => "e", Align::Stretch => "t" }).ok();
        }
        fn stmt_macro_b(out: &mut String, a: Align) {
            assert_eq!(match a { Align::Start => "s", Align::Center => "c", Align::End => "e", Align::Stretch => "t" }, "s");
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", positive_loops_and_macro)]).len(),
        2,
        "identical loop/async arms and match inside Stmt::Macro must collide"
    );

    // 2. Chained alias-then-glob (`use crate::Align as Al; use Al::*;`), 2-branch `if let` with `Pat::Or`,
    //    arm-block `use`, and shared `$lN` across `Pat::Or` in `if let`.
    let chained_and_if_let = r#"
        use crate::Align as Al;
        use Al::*;
        fn f_a(a: Al) -> bool {
            if let Start | Center = a { true }
            else if let End | Stretch = a { false }
            else { false }
        }
        fn f_b(a: Align) -> bool {
            match a {
                Align::Start | Align::Center => true,
                Align::End | Align::Stretch => false,
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", chained_and_if_let)]).len(),
        1,
        "chained alias-then-glob + 2-branch Pat::Or if-let chain must collide with match"
    );

    let or_binding_a = r#"
        fn or_a(k: Kind, v: Res) -> i32 {
            match k {
                Kind::A => if let Res::X(n) | Res::Y(n) = v { n + 1 } else { 0 },
                Kind::B => if let Res::X(n) | Res::Y(n) = v { n + 2 } else { 0 },
                Kind::C => if let Res::X(n) | Res::Y(n) = v { n + 3 } else { 0 },
                Kind::D => if let Res::X(n) | Res::Y(n) = v { n + 4 } else { 0 },
            }
        }
        fn or_b(k: Kind, v: Res) -> i32 {
            match k {
                Kind::A => if let Res::Y(m) | Res::X(m) = v { m + 1 } else { 0 },
                Kind::B => if let Res::Y(m) | Res::X(m) = v { m + 2 } else { 0 },
                Kind::C => if let Res::Y(m) | Res::X(m) = v { m + 3 } else { 0 },
                Kind::D => if let Res::Y(m) | Res::X(m) = v { m + 4 } else { 0 },
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", or_binding_a)]).len(),
        1,
        "`Pat::Or` in `if let` must share `$lN` across alternatives regardless of order"
    );

    // 3. `MirrorAtom::Bool(true)` must NOT collide with `MirrorAtom::Str("true")`.
    let bool_vs_str = r#"
        fn flag_to_bool(f: Flag) -> bool {
            match f { Flag::A => true, Flag::B => false, Flag::C => true, Flag::D => false }
        }
        fn str_to_flag(s: &str) -> Option<Flag> {
            match s { "true" => Some(Flag::A), "false" => Some(Flag::B), "x" => Some(Flag::C), "y" => Some(Flag::D), _ => None }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", bool_vs_str)]).is_empty(),
        "MirrorAtom::Bool(true) must NOT mirror with MirrorAtom::Str(\"true\")"
    );

    // 4. Tuple `Pat::Or` overflow (>64 combinations) skips the site cleanly rather than truncating.
    let overflow_src = r#"
        fn big_a(x: (E, E)) -> u8 {
            match x {
                (E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I,
                 E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I) => 1,
                _ => 0,
            }
        }
        fn big_b(x: (E, E)) -> u8 {
            match x {
                (E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I,
                 E::A | E::B | E::C | E::D | E::E | E::F | E::G | E::H | E::I) => 1,
                _ => 0,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", overflow_src)]).is_empty(),
        "tuple Pat::Or expansion overflow (>64) must skip the site cleanly"
    );

    // 5. Arm-block `use` alias ingestion & nested `ImplItem::Const` symbol disambiguation.
    let arm_use_and_const = r#"
        fn arm_use_a(k: Kind) -> Target {
            match k {
                Kind::A => { use crate::Target as T; T::One }
                Kind::B => { use crate::Target as T; T::Two }
                Kind::C => { use crate::Target as T; T::Three }
                Kind::D => { use crate::Target as T; T::Four }
            }
        }
        fn arm_use_b(k: Kind) -> Target {
            match k { Kind::A => Target::One, Kind::B => Target::Two, Kind::C => Target::Three, Kind::D => Target::Four }
        }
        fn host() {
            trait Marker { const MAP: u8; }
            struct S;
            impl Marker for S {
                const MAP: u8 = match Align::Start { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 };
            }
        }
        fn plain_map(a: Align) -> u8 {
            match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 }
        }
    "#;
    let dups = scan_sources(&[("a.rs", arm_use_and_const)]);
    assert_eq!(dups.len(), 2, "arm-block `use` and nested ImplItem::Const must both collide: {dups:?}");
    assert!(
        dups.iter().any(|d| d.symbol_a == "host::<S as Marker>::MAP" || d.symbol_b == "host::<S as Marker>::MAP"),
        "nested ImplItem::Const must include impl_prefix: {dups:?}"
    );
}
