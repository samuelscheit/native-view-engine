//! Regression tests covering all Pass-1 dual-review P1/P2 findings in `ast_enums`:
//! - per-match outer-variable alpha-renaming (opposite directions, swapped corners, bare outer vars),
//! - monotonic local shadowing, pattern-binding shadowing, and nested block scoping,
//! - structural loop/while/for/async/const bodies & `if let` / nested `match` pattern preservation,
//! - `use Enum as Alias;`, `type Alias = Enum;`, SDK aliases, same-name cross-crate enums (`snapshot` vs `apply`),
//! - `EnumA <-> EnumB` mirrors with distinct variant names,
//! - macro body traversal (`format!`/`write!`/`vec!`/`assert_eq!`/`Stmt::Macro`, `core::matches!`),
//! - glob `LitToEnum`, inner-block `use`, multi-glob partial duplicates, `if let` chains, `v @ Enum::A`,
//!   `Pat::Or` inside `Pat::Tuple`, cross-language `Pat::Or` inflation vs abort-wildcard subsets,
//! - `converters.rs` (non-core duplicate enums, imported SDK types, container params, unnamed converters),
//! - `#[cfg(any(test, ...))]` vs nested `#[cfg(test)]`, `<T as Trait>::method` / `mod::` symbols,
//! - zero double-reporting between `legacy_tables::scan_match_tables` and `fingerprint`.

use super::converters::scan_duplicate_enums_and_converters;
use super::fingerprint::{
    scan_duplicate_enum_match_arms, scan_duplicate_enum_match_arms_with_crates, DuplicateKind,
};
use super::legacy_tables::scan_match_tables;

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
fn per_match_outer_vars_reject_opposite_directions_and_swapped_corners() {
    // Non-bare outer-var expressions (`+ 1.0`) prove `$rN` pre-seeding distinguishes opposite mappings
    // without relying on `is_bare_outer_var`.
    let opposite_same_order = r#"
        fn edge_val(e: Edge, top: f32, right: f32, bottom: f32, left: f32) -> f32 {
            match e { Edge::Top => top + 1.0, Edge::Right => right + 1.0, Edge::Bottom => bottom + 1.0, Edge::Left => left + 1.0 }
        }
        fn opposite_edge_val(e: Edge, top: f32, right: f32, bottom: f32, left: f32) -> f32 {
            match e { Edge::Top => bottom + 1.0, Edge::Right => left + 1.0, Edge::Bottom => top + 1.0, Edge::Left => right + 1.0 }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", opposite_same_order)]).is_empty(),
        "opposite-direction non-bare outer-var expressions must NOT collide"
    );

    // Non-bare bodies with permuted parameter declarations are positionally alpha-equivalent
    // (`(e, p0, p1, p2, p3)` -> `Top => p0 + 1.0, ...`) and therefore collide, whereas pure bare
    // outer-var selectors remain rejected unconditionally by `is_bare_outer_var`.
    let positional_reordered_non_bare = r#"
        fn edge_a(e: Edge, top: f32, right: f32, bottom: f32, left: f32) -> f32 {
            match e { Edge::Top => top + 1.0, Edge::Right => right + 1.0, Edge::Bottom => bottom + 1.0, Edge::Left => left + 1.0 }
        }
        fn edge_b(e: Edge, bottom: f32, left: f32, top: f32, right: f32) -> f32 {
            match e { Edge::Top => bottom + 1.0, Edge::Right => left + 1.0, Edge::Bottom => top + 1.0, Edge::Left => right + 1.0 }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", positional_reordered_non_bare)]).len(),
        1,
        "non-bare positionally alpha-equivalent functions with permuted param names must collide"
    );

    let bare_reordered_params = r#"
        fn edge_a(e: Edge, top: f32, right: f32, bottom: f32, left: f32) -> f32 {
            match e { Edge::Top => top, Edge::Right => right, Edge::Bottom => bottom, Edge::Left => left }
        }
        fn edge_b(e: Edge, bottom: f32, left: f32, top: f32, right: f32) -> f32 {
            match e { Edge::Top => bottom, Edge::Right => left, Edge::Bottom => top, Edge::Left => right }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", bare_reordered_params)]).is_empty(),
        "bare outer-var selectors with reordered params must NOT collide"
    );

    let swapped_corners = r#"
        fn corners(c: Corner, a: f32, b: f32) -> ((f32, f32), (f32, f32)) {
            let primary = match c {
                Corner::TopLeft => (a, b), Corner::TopRight => (b, a),
                Corner::BottomLeft => (a, a), Corner::BottomRight => (b, b),
            };
            let inverted = match c {
                Corner::TopLeft => (b, a), Corner::TopRight => (a, b),
                Corner::BottomLeft => (b, b), Corner::BottomRight => (a, a),
            };
            (primary, inverted)
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", swapped_corners)]).is_empty(),
        "swapped corner tables referencing outer vars in opposite roles must NOT collide"
    );
}

#[test]
fn monotonic_local_shadowing_pattern_shadowing_and_block_scoping() {
    let shadow_diff = r#"
        fn f_a(k: Kind) -> i32 {
            match k {
                Kind::A => { let x = 1; let x = 2; let y = 3; x + y }
                Kind::B => { let x = 2; let x = 3; let y = 4; x + y }
                Kind::C => { let x = 3; let x = 4; let y = 5; x + y }
                Kind::D => { let x = 4; let x = 5; let y = 6; x + y }
            }
        }
        fn f_b(k: Kind) -> i32 {
            match k {
                Kind::A => { let a = 1; let b = 2; let c = 3; a + c }
                Kind::B => { let a = 2; let b = 3; let c = 4; a + c }
                Kind::C => { let a = 3; let b = 4; let c = 5; a + c }
                Kind::D => { let a = 4; let b = 5; let c = 6; a + c }
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", shadow_diff)]).is_empty(),
        "re-binding shadowed `x` must not collapse into first binding `a`"
    );

    let pat_shadow = r#"
        fn g_a(k: Kind) -> i32 {
            match k {
                Kind::A(x) => { let x = x + 1; x * 2 }
                Kind::B(x) => { let x = x + 2; x * 2 }
                Kind::C(x) => { let x = x + 3; x * 2 }
                Kind::D(x) => { let x = x + 4; x * 2 }
            }
        }
        fn g_b(k: Kind) -> i32 {
            match k {
                Kind::A(x) => { let y = x + 1; x * 2 }
                Kind::B(x) => { let y = x + 2; x * 2 }
                Kind::C(x) => { let y = x + 3; x * 2 }
                Kind::D(x) => { let y = x + 4; x * 2 }
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", pat_shadow)]).is_empty(),
        "local shadowing a pattern binding must resolve to $lN, not $pN"
    );

    let block_scope = r#"
        fn h_a(k: Kind) -> i32 {
            match k {
                Kind::A => { let x = 1; { let x = 10; drop(x); } x }
                Kind::B => { let x = 2; { let x = 20; drop(x); } x }
                Kind::C => { let x = 3; { let x = 30; drop(x); } x }
                Kind::D => { let x = 4; { let x = 40; drop(x); } x }
            }
        }
        fn h_b(k: Kind) -> i32 {
            match k {
                Kind::A => { let x = 1; { let y = 10; drop(y); } x }
                Kind::B => { let x = 2; { let y = 20; drop(y); } x }
                Kind::C => { let x = 3; { let y = 30; drop(y); } x }
                Kind::D => { let x = 4; { let y = 40; drop(y); } x }
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", block_scope)]).len(),
        1,
        "inner block local shadow must restore outer local binding after block exits"
    );
}

#[test]
fn distinct_loops_async_const_and_if_let_patterns_never_collapse() {
    let distinct_loops = r#"
        fn loops_a(k: Kind, xs: &[i32]) {
            match k {
                Kind::A => for x in xs { push_a(x); },
                Kind::B => while cond_b() { step_b(); },
                Kind::C => loop { break_c(); },
                Kind::D => { let _ = async { fetch_d().await }; },
            }
        }
        fn loops_b(k: Kind, ys: &[i32]) {
            match k {
                Kind::A => for y in ys { push_z(y); },
                Kind::B => while cond_z() { step_z(); },
                Kind::C => loop { break_z(); },
                Kind::D => { let _ = async { fetch_z().await }; },
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", distinct_loops)]).is_empty(),
        "distinct loop/while/for/async bodies must NEVER collapse into shared 'other'"
    );

    let distinct_if_let = r#"
        fn check_a(k: Kind, v: Result<i32, i32>) -> i32 {
            match k {
                Kind::A => if let Ok(x) = v { x + 1 } else { 0 },
                Kind::B => if let Ok(x) = v { x + 2 } else { 0 },
                Kind::C => if let Ok(x) = v { x + 3 } else { 0 },
                Kind::D => if let Ok(x) = v { x + 4 } else { 0 },
            }
        }
        fn check_b(k: Kind, v: Result<i32, i32>) -> i32 {
            match k {
                Kind::A => if let Err(y) = v { y + 1 } else { 0 },
                Kind::B => if let Err(y) = v { y + 2 } else { 0 },
                Kind::C => if let Err(y) = v { y + 3 } else { 0 },
                Kind::D => if let Err(y) = v { y + 4 } else { 0 },
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", distinct_if_let)]).is_empty(),
        "`if let Ok(x)` must NEVER collapse with `if let Err(y)`"
    );
}

#[test]
fn detects_matches_inside_macros_and_qualified_matches_macro() {
    let macro_a = r#"
        fn fmt_a(a: Align) -> String {
            format!("align={}", match a {
                Align::Start => "start", Align::Center => "center",
                Align::End => "end", Align::Stretch => "stretch",
            })
        }
        fn pred_a(t: Tag) -> bool {
            core::matches!(t, Tag::Span | Tag::Em | Tag::Strong | Tag::Code)
        }
    "#;
    let macro_b = r#"
        fn vec_b(a: Align) -> Vec<&'static str> {
            vec![match a {
                Align::Start => "start", Align::Center => "center",
                Align::End => "end", Align::Stretch => "stretch",
            }]
        }
        fn pred_b(t: Tag) -> bool {
            std::matches!(t, Tag::Span | Tag::Em | Tag::Strong | Tag::Code)
        }
    "#;
    let dups = scan_sources(&[("a.rs", macro_a), ("b.rs", macro_b)]);
    assert_eq!(
        dups.len(),
        2,
        "expected match inside format!/vec! AND core::matches! vs std::matches!: {dups:?}"
    );
}

#[test]
fn use_as_type_alias_sdk_alias_and_cross_crate_same_name_mirror() {
    let alias_a = r#"
        use crate::Align as Al;
        fn f_a(a: Al) -> u8 {
            match a { Al::Start => 1, Al::Center => 2, Al::End => 3, Al::Stretch => 4 }
        }
    "#;
    let alias_b = r#"
        type LocalAlign = Align;
        fn f_b(a: LocalAlign) -> u8 {
            match a { LocalAlign::Start => 1, LocalAlign::Center => 2, LocalAlign::End => 3, LocalAlign::Stretch => 4 }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", alias_a), ("b.rs", alias_b)]).len(),
        1,
        "`use Align as Al;` and `type LocalAlign = Align;` must collide"
    );

    let sdk_a = r#"
        use winit::window::Theme as WinitTheme;
        fn w_a(t: WinitTheme) -> u8 {
            match t { WinitTheme::Light => 0, WinitTheme::Dark => 1, WinitTheme::High => 2, WinitTheme::Auto => 3 }
        }
    "#;
    let local_theme = r#"
        fn local_t(t: Theme) -> u8 {
            match t { Theme::Light => 0, Theme::Dark => 1, Theme::High => 2, Theme::Auto => 3 }
        }
    "#;
    let sdk_b = r#"
        fn w_b(t: winit::window::Theme) -> u8 {
            match t {
                winit::window::Theme::Light => 0, winit::window::Theme::Dark => 1,
                winit::window::Theme::High => 2, winit::window::Theme::Auto => 3,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", sdk_a), ("b.rs", local_theme)]).is_empty(),
        "SDK alias `winit::window::Theme as WinitTheme` must NOT collide with local `Theme`"
    );
    assert_eq!(
        scan_sources(&[("a.rs", sdk_a), ("b.rs", sdk_b)]).len(),
        1,
        "SDK alias `WinitTheme` must collide with direct `winit::window::Theme`"
    );

    // Reproduces exact `snapshot.rs` <-> `apply.rs` cross-crate `FlexDirection` mirror.
    let snapshot_rs = r#"
        pub fn layout_snapshot_of(dir: nve_tables::tables::FlexDirection) -> nve_abi::FlexDirection {
            match dir {
                nve_tables::tables::FlexDirection::Column => nve_abi::FlexDirection::Column,
                nve_tables::tables::FlexDirection::Row => nve_abi::FlexDirection::Row,
                nve_tables::tables::FlexDirection::ColumnReverse => nve_abi::FlexDirection::ColumnReverse,
                nve_tables::tables::FlexDirection::RowReverse => nve_abi::FlexDirection::RowReverse,
            }
        }
    "#;
    let apply_rs = r#"
        use nve_abi::FlexDirection;
        use nve_tables::tables::FlexDirection as TableFlexDir;
        pub fn apply_dir(dir: FlexDirection) -> TableFlexDir {
            match dir {
                FlexDirection::Column => TableFlexDir::Column,
                FlexDirection::Row => TableFlexDir::Row,
                FlexDirection::ColumnReverse => TableFlexDir::ColumnReverse,
                FlexDirection::RowReverse => TableFlexDir::RowReverse,
            }
        }
    "#;
    let dups = scan_with_crates(&[
        ("snapshot.rs", snapshot_rs, Some("nve_backend_headless")),
        ("apply.rs", apply_rs, Some("nve_dioxus")),
    ]);
    assert_eq!(dups.len(), 1, "cross-crate FlexDirection mirror must be caught: {dups:?}");
    assert_eq!(dups[0].kind, DuplicateKind::Mirrored);
    assert_eq!(dups[0].overlap_arms, 4);
}

#[test]
fn detects_enum_to_enum_mirror_with_distinct_variant_names() {
    let src = r#"
        fn key_to_action(k: KeyCode) -> Action {
            match k {
                KeyCode::ArrowUp => Action::MoveNorth, KeyCode::ArrowDown => Action::MoveSouth,
                KeyCode::ArrowLeft => Action::MoveWest, KeyCode::ArrowRight => Action::MoveEast,
            }
        }
        fn action_to_key(a: Action) -> KeyCode {
            match a {
                Action::MoveNorth => KeyCode::ArrowUp, Action::MoveSouth => KeyCode::ArrowDown,
                Action::MoveWest => KeyCode::ArrowLeft, Action::MoveEast => KeyCode::ArrowRight,
            }
        }
    "#;
    let dups = scan_sources(&[("a.rs", src)]);
    assert_eq!(dups.len(), 1, "EnumA <-> EnumB mirror with distinct variant names must be caught: {dups:?}");
    assert_eq!(dups[0].kind, DuplicateKind::Mirrored);
    assert_eq!(dups[0].overlap_arms, 4);
}

#[test]
fn detects_glob_lit_to_enum_inner_block_use_and_multi_glob_partial_duplicates() {
    let glob_lit_a = r#"
        use crate::MyEnum::*;
        fn parse_a(s: &str) -> Option<MyEnum> {
            match s { "a" => Some(Alpha), "b" => Some(Beta), "c" => Some(Gamma), "d" => Some(Delta), _ => None }
        }
    "#;
    let glob_lit_b = r#"
        fn parse_b(s: &str) -> Option<MyEnum> {
            match s {
                "a" => Some(MyEnum::Alpha), "b" => Some(MyEnum::Beta),
                "c" => Some(MyEnum::Gamma), "d" => Some(MyEnum::Delta), _ => None,
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", glob_lit_a), ("b.rs", glob_lit_b)]).len(),
        1,
        "glob `LitToEnum` must collide with qualified `LitToEnum`"
    );

    let block_use_a = r#"
        fn inner_a(a: Align) -> u8 {
            {
                use crate::Align as Al;
                match a { Al::Start => 1, Al::Center => 2, Al::End => 3, Al::Stretch => 4 }
            }
        }
    "#;
    let block_use_b = r#"
        fn inner_b(a: Align) -> u8 {
            match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", block_use_a), ("b.rs", block_use_b)]).len(),
        1,
        "inner-block `use Align as Al;` must resolve cleanly"
    );

    let multi_glob_a = r#"
        use crate::Foo::*;
        use crate::Bar::*;
        fn mg_a(f: Foo) -> u8 { match f { A => 1, B => 2, C => 3, D => 4, E => 99 } }
    "#;
    let multi_glob_b = r#"
        fn mg_b(f: Foo) -> u8 { match f { Foo::A => 1, Foo::B => 2, Foo::C => 3, Foo::D => 4, Foo::E => 5 } }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", multi_glob_a), ("b.rs", multi_glob_b)]).len(),
        1,
        "multi-glob `use Foo::*; use Bar::*;` partial duplicate (4/5) must match qualified `Foo::*`"
    );
}

#[test]
fn detects_if_let_chains_at_subpats_and_or_inside_tuple_patterns() {
    let if_let_a = r#"
        fn chain_a(a: Align) -> &'static str {
            if let Align::Start = a { "start" }
            else if let Align::Center = a { "center" }
            else if let Align::End = a { "end" }
            else if let Align::Stretch = a { "stretch" }
            else { "other" }
        }
    "#;
    let match_b = r#"
        fn match_b(a: Align) -> &'static str {
            match a { Align::Start => "start", Align::Center => "center", Align::End => "end", Align::Stretch => "stretch", _ => "other" }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", if_let_a), ("b.rs", match_b)]).len(),
        1,
        "`if let` chain with >= 4 branches must collide with equivalent `match`"
    );

    let at_a = r#"
        fn at_a(a: Align) -> u8 {
            match a { v @ Align::Start => 1, v @ Align::Center => 2, v @ Align::End => 3, v @ Align::Stretch => 4 }
        }
    "#;
    let plain_b = r#"
        fn plain_b(a: Align) -> u8 {
            match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", at_a), ("b.rs", plain_b)]).len(),
        1,
        "`v @ Enum::Variant` must be extracted and collide with plain `Enum::Variant`"
    );

    let tup_a = r#"
        fn def_a(t: PropType, d: DefVal) -> String {
            match (t, d) {
                (PropType::Int32 | PropType::IntEnum, DefVal::Int(i)) => i.to_string(),
                (PropType::String | PropType::Color, DefVal::Str(s)) => format!("\"{s}\""),
                _ => String::new(),
            }
        }
    "#;
    let tup_b = r#"
        fn def_b(t: PropType, d: DefVal) -> String {
            match (t, d) {
                (PropType::Int32, DefVal::Int(n)) => n.to_string(),
                (PropType::IntEnum, DefVal::Int(n)) => n.to_string(),
                (PropType::String, DefVal::Str(v)) => format!("\"{v}\""),
                (PropType::Color, DefVal::Str(v)) => format!("\"{v}\""),
                _ => String::new(),
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", tup_a), ("b.rs", tup_b)]).len(),
        1,
        "`Pat::Or` inside `Pat::Tuple` must expand and match flat tuple arms"
    );
}

#[test]
fn cfg_any_test_nested_cfg_test_symbols_and_cross_lang_wildcards() {
    let src = r#"
        mod inner {
            pub trait Enc { fn encode(&self) -> u8; }
            impl Enc for Align {
                #[cfg(any(test, feature = "debug"))]
                fn encode(&self) -> u8 {
                    // Identical to `dup_encode`: failing to skip `#[cfg(test)]` yields 2+ hits.
                    #[cfg(test)]
                    let _ignored = match self {
                        Self::Start => 1, Self::Center => 2, Self::End => 3, Self::Stretch => 4,
                    };
                    #[cfg(test)]
                    {
                        let _ignored_block = match self {
                            Self::Start => 1, Self::Center => 2, Self::End => 3, Self::Stretch => 4,
                        };
                    }
                    match self {
                        Self::Start => 1, Self::Center => 2, Self::End => 3, Self::Stretch => 4,
                    }
                }
            }
        }
        fn dup_encode(a: Align) -> u8 {
            match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 }
        }
    "#;
    let dups = scan_sources(&[("a.rs", src)]);
    assert_eq!(dups.len(), 1, "expected exactly 1 hit (skipping nested #[cfg(test)]): {dups:?}");
    assert_eq!(dups[0].symbol_a, "dup_encode");
    assert_eq!(dups[0].symbol_b, "inner::<Align as Enc>::encode");

    let cross_zero = r#"
        fn dart_default(ty: Ty) -> &'static str {
            match ty {
                Ty::Bool => "false",
                Ty::U8 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I64 | Ty::U64 => "0",
                Ty::F32 | Ty::F64 => "0.0",
                Ty::Enum => "MyEnum.first",
                Ty::Struct => "MyStruct()",
                Ty::Str => "''",
            }
        }
        fn rust_zero(ty: Ty) -> &'static str {
            match ty {
                Ty::Bool => "false",
                Ty::U8 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I64 | Ty::U64 | Ty::Enum => "0",
                Ty::F32 | Ty::F64 => "0.0",
                Ty::Struct if is_pod(ty) => "Default::default()",
                _ => "OwnedBytes::empty()",
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", cross_zero)]).is_empty(),
        "3/5 cross-language Bool/Int/Float zero-value coincidence must NOT false-positive"
    );

    let abort_subset = r#"
        fn pod_ty(ty: Ty) -> &'static str {
            match ty { Ty::Bool => "bool", Ty::U8 => "u8", Ty::I32 => "i32", Ty::F32 => "f32", _ => unreachable!("non-POD") }
        }
        fn owned_ty(ty: Ty) -> &'static str {
            match ty { Ty::Bool => "bool", Ty::U8 => "u8", Ty::I32 => "i32", Ty::F32 => "f32", Ty::Str => "String" }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", abort_subset)]).len(),
        1,
        "abort-wildcard subset (`unreachable!(...)`) MUST still be caught"
    );
}

#[test]
fn converters_hardening_and_zero_legacy_tables_double_report() {
    let core_src = r#"
        pub enum Align { Start, Center, End, Stretch }
        pub enum Flow { Row, Col, RowRev, ColRev }
    "#;
    let other_a = r#"
        pub enum StudioPanel { Inspector, Tree, Console, Metrics }
        use taffy::AlignItems;
        pub fn map_sdk(a: Align) -> AlignItems {
            match a { Align::Start => AlignItems::Start, Align::Center => AlignItems::Center, Align::End => AlignItems::End, Align::Stretch => AlignItems::Stretch }
        }
        pub fn slice_helper(items: &[Align]) -> Flow {
            match items.len() { 0 => Flow::Row, 1 => Flow::Col, _ => Flow::RowRev }
        }
        pub fn secret_internal_convert(a: Align) -> Flow {
            match a { Align::Start => Flow::Row, Align::Center => Flow::Col, Align::End => Flow::RowRev, Align::Stretch => Flow::ColRev }
        }
    "#;
    let other_b = r#"pub enum StudioPanel { Inspector, Tree, Console, Metrics }"#;
    let ast_core = syn::parse_file(core_src).expect("valid");
    let ast_a = syn::parse_file(other_a).expect("valid");
    let ast_b = syn::parse_file(other_b).expect("valid");
    let mut violations = Vec::new();
    scan_duplicate_enums_and_converters(
        &[
            ("packages/core/components/src/lib.rs".into(), core_src, &ast_core),
            ("examples/a/src/lib.rs".into(), other_a, &ast_a),
            ("examples/b/src/lib.rs".into(), other_b, &ast_b),
        ],
        &mut violations,
    );
    assert!(
        violations.iter().any(|v| v.reason.contains("Duplicate internal `pub enum StudioPanel`")),
        "same-name duplicate pub enum across two non-core crates must be caught: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.reason.contains("`fn secret_internal_convert`")),
        "unnamed internal NVE->NVE enum converter must be caught: {violations:?}"
    );
    assert!(
        !violations.iter().any(|v| v.reason.contains("`fn map_sdk`")),
        "imported SDK return type (`use taffy::AlignItems;`) must NOT be flagged: {violations:?}"
    );
    assert!(
        !violations.iter().any(|v| v.reason.contains("`fn slice_helper`")),
        "slice param `&[Align]` with integer match body must NOT be flagged as enum converter: {violations:?}"
    );

    let mirror_src = r#"
        impl TextAlign {
            pub fn as_css(self) -> &'static str {
                match self { Self::Left => "left", Self::Center => "center", Self::Right => "right", Self::Justify => "justify" }
            }
            pub fn parse_css(s: &str) -> Option<Self> {
                match s { "left" => Some(Self::Left), "center" => Some(Self::Center), "right" => Some(Self::Right), "justify" => Some(Self::Justify), _ => None }
            }
        }
    "#;
    let ast_m = syn::parse_file(mirror_src).expect("valid");
    let mut legacy_v = Vec::new();
    scan_match_tables("packages/backends/web/src/text.rs", mirror_src, &ast_m, &mut legacy_v);
    assert!(legacy_v.is_empty(), "legacy_tables must not duplicate same-file mirrored detection outside core: {legacy_v:?}");
    let fp_dups = scan_duplicate_enum_match_arms(&[("packages/backends/web/src/text.rs", mirror_src, &ast_m)]);
    assert_eq!(fp_dups.len(), 1);
    assert_eq!(fp_dups[0].kind, DuplicateKind::Mirrored);
}
