//! Comprehensive positive & negative unit tests for `ast_enums`:
//! - structural match-arm fingerprinting (same-file & cross-file, same-direction & mirrored),
//! - assignments, indexed/compound assignments, setters/builders, blocks, tuples, struct literals, `write!`,
//! - payload variants (`TupleStruct`, `Struct`), `Pat::Or`, `matches!`, `use Enum::*`, guards, `#[cfg(test)]`,
//! - zero false positives (different bodies, `< 4` arms, `Option`/`Result`/`bool`, distinct enum qualifiers,
//!   noop-only overlaps, cross-language partial overlaps, `Pat::Or` inflation, cardinal vs ordinal subsets),
//! - shrink-only ratchet baseline (new hits, ceiling growth, stale entries, unsorted/zero entries),
//! - workspace-wide structural duplicate `pub enum`s and `TryFrom`/unnamed enum converters.

use super::baseline::{evaluate_duplicates_with_ratchet, AllowlistedDuplicate};
use super::converters::scan_duplicate_enums_and_converters;
use super::fingerprint::{scan_duplicate_enum_match_arms, DuplicateKind};

fn scan_sources(sources: &[(&str, &str)]) -> Vec<super::MatchDuplicate> {
    let parsed: Vec<(&str, &str, syn::File)> = sources
        .iter()
        .map(|&(path, src)| (path, src, syn::parse_file(src).expect("valid rust snippet")))
        .collect();
    let refs: Vec<(&str, &str, &syn::File)> = parsed
        .iter()
        .map(|(p, s, ast)| (*p, *s, ast))
        .collect();
    scan_duplicate_enum_match_arms(&refs)
}

#[test]
fn detects_same_file_and_cross_file_same_direction_expression_matches_with_renamed_locals() {
    let file_a = r#"
        impl Align {
            pub fn css(self) -> &'static str {
                match self {
                    Self::Start => "start",
                    Self::Center => "center",
                    Self::End => "end",
                    Self::Stretch => "stretch",
                }
            }
            pub fn css_copy(self) -> &'static str {
                match self {
                    Self::End => "end",
                    Self::Start => "start",
                    Self::Stretch => "stretch",
                    Self::Center => "center",
                }
            }
        }
    "#;
    let file_b = r#"
        fn align_str(val: Align) -> &'static str {
            match val {
                Align::Stretch => "stretch",
                Align::Start => "start",
                Align::End => "end",
                Align::Center => "center",
            }
        }
    "#;
    let dups = scan_sources(&[("a.rs", file_a), ("b.rs", file_b)]);
    assert_eq!(dups.len(), 3, "expected 1 same-file + 2 cross-file duplicates: {dups:?}");
    assert!(dups.iter().all(|d| d.kind == DuplicateKind::SameDirection));
    assert!(dups.iter().all(|d| d.enum_name == "Align" && d.overlap_arms == 4));
}

#[test]
fn detects_same_file_and_cross_file_mirrored_matches() {
    let file_a = r#"
        impl Overflow {
            pub fn as_str(self) -> &'static str {
                match self {
                    Self::Visible => "visible",
                    Self::Hidden => "hidden",
                    Self::Clip => "clip",
                    Self::Scroll => "scroll",
                }
            }
            pub fn from_str_local(raw: &str) -> Option<Self> {
                match raw {
                    "visible" => Some(Self::Visible),
                    "hidden" => Some(Self::Hidden),
                    "clip" => Some(Self::Clip),
                    "scroll" => Some(Self::Scroll),
                    _ => None,
                }
            }
        }
    "#;
    let file_b = r#"
        pub fn parse_overflow(s: &str) -> Option<Overflow> {
            match s {
                "visible" => Some(Overflow::Visible),
                "hidden" => Some(Overflow::Hidden),
                "clip" => Some(Overflow::Clip),
                "scroll" => Some(Overflow::Scroll),
                _ => None,
            }
        }
    "#;
    let dups = scan_sources(&[("a.rs", file_a), ("b.rs", file_b)]);
    let mirrored: Vec<_> = dups.iter().filter(|d| d.kind == DuplicateKind::Mirrored).collect();
    let same: Vec<_> = dups.iter().filter(|d| d.kind == DuplicateKind::SameDirection).collect();
    assert_eq!(mirrored.len(), 2, "expected same-file + cross-file mirrored pairs: {dups:?}");
    assert_eq!(same.len(), 1, "expected cross-file LitToEnum duplicate: {dups:?}");
}

#[test]
fn detects_field_indexed_compound_assignments_setters_blocks_tuples_structs_and_write_macros() {
    // 1. Field + indexed + compound assignments with renamed receivers (`dst` vs `out`) & block arms.
    let assign_a = r#"
        fn apply_a(dst: &mut Style, slots: &mut [u32], a: Align) {
            match a {
                Align::Start => { dst.align = 1; slots[0] += 1; }
                Align::Center => { dst.align = 2; slots[0] += 2; }
                Align::End => { dst.align = 3; slots[0] += 3; }
                Align::Stretch => { dst.align = 4; slots[0] += 4; }
            }
        }
    "#;
    let assign_b = r#"
        fn apply_b(out: &mut Style, buf: &mut [u32], align: Align) {
            match align {
                Align::Start => { out.align = 1; buf[0] += 1; }
                Align::Center => { out.align = 2; buf[0] += 2; }
                Align::End => { out.align = 3; buf[0] += 3; }
                Align::Stretch => { out.align = 4; buf[0] += 4; }
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", assign_a), ("b.rs", assign_b)]).len(),
        1,
        "field/indexed/compound assignment arms with renamed receivers must collide"
    );

    // 2. Setter / builder method calls.
    let builder_a = r#"
        fn set_a(b: &mut Builder, a: Align) {
            match a {
                Align::Start => b.set_align("flex-start"),
                Align::Center => b.set_align("center"),
                Align::End => b.set_align("flex-end"),
                Align::Stretch => b.set_align("stretch"),
            }
        }
    "#;
    let builder_b = r#"
        fn set_b(w: &mut Builder, x: Align) {
            match x {
                Align::Start => w.set_align("flex-start"),
                Align::Center => w.set_align("center"),
                Align::End => w.set_align("flex-end"),
                Align::Stretch => w.set_align("stretch"),
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", builder_a), ("b.rs", builder_b)]).len(),
        1,
        "setter/builder method call arms must collide"
    );

    // 3. Multi-stmt block + `let` + `write!` macro with renamed inline format captures.
    let rich_a = r#"
        fn emit_a(f: &mut String, k: Kind) {
            match k {
                Kind::A => { let tag = "a"; write!(f, "{tag}"); }
                Kind::B => { let tag = "b"; write!(f, "{tag}"); }
                Kind::C => { let tag = "c"; write!(f, "{tag}"); }
                Kind::D => { let tag = "d"; write!(f, "{tag}"); }
            }
        }
    "#;
    let rich_b = r#"
        fn emit_b(out: &mut String, kind: Kind) {
            match kind {
                Kind::A => { let name = "a"; write!(out, "{name}"); }
                Kind::B => { let name = "b"; write!(out, "{name}"); }
                Kind::C => { let name = "c"; write!(out, "{name}"); }
                Kind::D => { let name = "d"; write!(out, "{name}"); }
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", rich_a), ("b.rs", rich_b)]).len(),
        1,
        "block + let + write! macro arms with renamed locals must collide"
    );

    // 4. Tuple + struct-literal arms with reordered struct fields.
    let struct_a = r#"
        fn pair_a(k: Kind) -> (Edge, u8) {
            match k {
                Kind::A => (Edge { left: 1, right: 2 }, 0),
                Kind::B => (Edge { left: 2, right: 3 }, 1),
                Kind::C => (Edge { left: 3, right: 4 }, 2),
                Kind::D => (Edge { left: 4, right: 5 }, 3),
            }
        }
    "#;
    let struct_b = r#"
        fn pair_b(k: Kind) -> (Edge, u8) {
            match k {
                Kind::A => (Edge { right: 2, left: 1 }, 0),
                Kind::B => (Edge { right: 3, left: 2 }, 1),
                Kind::C => (Edge { right: 4, left: 3 }, 2),
                Kind::D => (Edge { right: 5, left: 4 }, 3),
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", struct_a), ("b.rs", struct_b)]).len(),
        1,
        "tuple + struct-literal arms (even with reordered fields) must collide"
    );
}

#[test]
fn detects_payload_variants_struct_patterns_or_patterns_guards_glob_imports_and_matches_macro() {
    let payload_a = r#"
        fn paint_a(out: &mut Canvas, cmd: DrawCmd) {
            match cmd {
                DrawCmd::Color(c) => out.set_color(c),
                DrawCmd::Rect { x, y, .. } => out.rect(x, y),
                DrawCmd::Opacity(a) if a > 0.0 => out.alpha(a),
                DrawCmd::Clip | DrawCmd::Restore => out.pop(),
                _ => {}
            }
        }
    "#;
    let payload_b = r#"
        fn paint_b(dst: &mut Canvas, item: DrawCmd) {
            match item {
                DrawCmd::Color(col) => dst.set_color(col),
                DrawCmd::Rect { y: py, x: px, .. } => dst.rect(px, py),
                DrawCmd::Opacity(alpha) if alpha > 0.0 => dst.alpha(alpha),
                DrawCmd::Clip | DrawCmd::Restore => dst.pop(),
                _ => {}
            }
        }
    "#;
    let dups = scan_sources(&[("a.rs", payload_a), ("b.rs", payload_b)]);
    assert_eq!(dups.len(), 1, "payload TupleStruct + Struct + guard + Or must collide: {dups:?}");
    assert_eq!(dups[0].overlap_arms, 5);

    // Glob `use Enum::*;` matches qualified `Enum::Variant` and another glob import.
    let glob_a = r#"
        use crate::Align::*;
        fn f_a(a: Align) -> &'static str {
            match a { Start => "start", Center => "center", End => "end", Stretch => "stretch" }
        }
    "#;
    let glob_b = r#"
        fn f_b(a: Align) -> &'static str {
            match a { Align::Start => "start", Align::Center => "center", Align::End => "end", Align::Stretch => "stretch" }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", glob_a), ("b.rs", glob_b)]).len(),
        1,
        "glob `use Align::*;` must match qualified `Align::*`"
    );

    // `matches!` macro vs equivalent `match` expression.
    let pred_a = r#"
        fn is_inline_a(t: Tag) -> bool {
            matches!(t, Tag::Span | Tag::Em | Tag::Strong | Tag::Code)
        }
    "#;
    let pred_b = r#"
        fn is_inline_b(t: Tag) -> bool {
            match t {
                Tag::Span | Tag::Em | Tag::Strong | Tag::Code => true,
                _ => false,
            }
        }
    "#;
    assert_eq!(
        scan_sources(&[("a.rs", pred_a), ("b.rs", pred_b)]).len(),
        1,
        "`matches!` macro and equivalent `match` predicate must collide"
    );
}

#[test]
fn zero_false_positives_on_distinct_bodies_guards_noops_small_prelude_and_distinct_enums() {
    let distinct_bodies = r#"
        fn to_css(a: Align) -> &'static str {
            match a {
                Align::Start => "flex-start",
                Align::Center => "center",
                Align::End => "flex-end",
                Align::Stretch => "stretch",
            }
        }
        fn to_skia(a: Align) -> skia_safe::Align {
            match a {
                Align::Start => skia_safe::Align::Left,
                Align::Center => skia_safe::Align::Center,
                Align::End => skia_safe::Align::Right,
                Align::Stretch => skia_safe::Align::Justify,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", distinct_bodies)]).is_empty(),
        "different body fingerprints on same enum must NOT flag"
    );

    let small_prelude_and_cfg_test = r#"
        fn small_a(a: Align) -> u8 { match a { Align::Start => 0, Align::Center => 1, Align::End => 2 } }
        fn small_b(a: Align) -> u8 { match a { Align::Start => 0, Align::Center => 1, Align::End => 2 } }
        fn opt_a(x: Option<u32>) -> u32 { match x { Some(v) => v, None => 0 } }
        fn opt_b(y: Option<u32>) -> u32 { match y { Some(v) => v, None => 0 } }
        fn res_a(r: Result<u32, u32>) -> u32 { match r { Ok(v) => v, Err(e) => e } }
        fn res_b(r: Result<u32, u32>) -> u32 { match r { Ok(v) => v, Err(e) => e } }
        fn bool_a(b: bool) -> u8 { match b { true => 1, false => 0 } }
        fn bool_b(b: bool) -> u8 { match b { true => 1, false => 0 } }
        #[cfg(test)]
        fn test_only(a: Align) -> u8 { match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 } }
        fn prod_only(a: Align) -> u8 { match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 } }
    "#;
    assert!(
        scan_sources(&[("a.rs", small_prelude_and_cfg_test)]).is_empty(),
        "< 4 arms, Option/Result/bool, and #[cfg(test)] items must NOT flag"
    );

    let distinct_enums = r#"
        fn align_items_css(a: AlignItems) -> &'static str {
            match a {
                AlignItems::Start => "start",
                AlignItems::Center => "center",
                AlignItems::End => "end",
                AlignItems::Stretch => "stretch",
            }
        }
        fn align_content_css(a: AlignContent) -> &'static str {
            match a {
                AlignContent::Start => "start",
                AlignContent::Center => "center",
                AlignContent::End => "end",
                AlignContent::Stretch => "stretch",
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", distinct_enums)]).is_empty(),
        "different enum type qualifiers sharing variant names must NOT collide"
    );

    // Noop-only overlaps (ignoring 4 variants with `{}` while handling different variants) must NOT flag.
    let noop_only = r#"
        fn handle_a(e: Ev) {
            match e {
                Ev::A | Ev::B | Ev::C | Ev::D => {}
                Ev::E => run_e(),
                Ev::F => run_f(),
            }
        }
        fn handle_b(e: Ev) {
            match e {
                Ev::A | Ev::B | Ev::C | Ev::D => {}
                Ev::E => other_e(),
                Ev::F => other_f(),
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", noop_only)]).is_empty(),
        "noop-only arm overlaps must NOT flag"
    );

    // Guarded vs unguarded arms must NOT collide.
    let guard_mismatch = r#"
        fn g_a(k: Kind) -> u8 {
            match k {
                Kind::A(v) if v > 0 => 1,
                Kind::B(v) if v > 0 => 2,
                Kind::C(v) if v > 0 => 3,
                Kind::D(v) if v > 0 => 4,
                _ => 0,
            }
        }
        fn g_b(k: Kind) -> u8 {
            match k {
                Kind::A(_) => 1,
                Kind::B(_) => 2,
                Kind::C(_) => 3,
                Kind::D(_) => 4,
                _ => 0,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", guard_mismatch)]).is_empty(),
        "guarded arms must NOT collide with unguarded arms"
    );

    // Cross-language partial overlap & Pat::Or inflation must NOT false-positive.
    let cross_lang = r#"
        fn c_ty(t: Ty) -> &'static str {
            match t {
                Ty::Bool => "bool",
                Ty::U8 => "uint8_t",
                Ty::U16 => "uint16_t",
                Ty::I32 => "int32_t",
                Ty::U32 => "uint32_t",
                Ty::I64 => "int64_t",
                Ty::U64 => "uint64_t",
                Ty::F32 => "float",
                Ty::F64 => "double",
                Ty::Str => "const char*",
                Ty::Val => "NveValue",
                Ty::Void => "void",
            }
        }
        fn dart_ty(t: Ty) -> &'static str {
            match t {
                Ty::Bool => "bool",
                Ty::U8 | Ty::U16 | Ty::I32 | Ty::U32 | Ty::I64 | Ty::U64 => "int",
                Ty::F32 | Ty::F64 => "double",
                Ty::Str => "String",
                Ty::Val => "NveValue",
                Ty::Void => "void",
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", cross_lang)]).is_empty(),
        "cross-language mappers sharing only 4/12 variants must NOT false-positive"
    );

    // Distinct CLDR cardinal vs ordinal rules where Pat::Or arms only partially overlap must NOT false-positive.
    let cldr_rules = r#"
        fn cardinal_cy(n: u64) -> Plural {
            match n {
                0 => Plural::Zero,
                1 => Plural::One,
                2 => Plural::Two,
                3 => Plural::Few,
                6 => Plural::Many,
                _ => Plural::Other,
            }
        }
        fn ordinal_cy(n: u64) -> Plural {
            match n {
                0 | 7 | 8 | 9 => Plural::Zero,
                1 => Plural::One,
                2 => Plural::Two,
                3 | 4 => Plural::Few,
                5 | 6 => Plural::Many,
                _ => Plural::Other,
            }
        }
    "#;
    assert!(
        scan_sources(&[("a.rs", cldr_rules)]).is_empty(),
        "partially overlapping Pat::Or literal arms (cardinal vs ordinal) must NOT false-positive"
    );
}

#[test]
fn ratchet_enforces_new_hits_ceiling_growth_stale_entries_and_sort_order() {
    let src_a = r#"
        fn f1(a: Align) -> u8 {
            match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 }
        }
    "#;
    let src_b = r#"
        fn f2(a: Align) -> u8 {
            match a { Align::Start => 1, Align::Center => 2, Align::End => 3, Align::Stretch => 4 }
        }
    "#;
    let dups = scan_sources(&[("a.rs", src_a), ("b.rs", src_b)]);
    assert_eq!(dups.len(), 1);

    // 1. Empty allowlist -> fails immediately as active violation.
    let rep_new = evaluate_duplicates_with_ratchet(&dups, &[]);
    assert_eq!(rep_new.active_violations.len(), 1);
    assert!(rep_new.allowlisted_violations.is_empty());

    // 2. Exact allowlist entry -> 0 active violations, 1 allowlisted (for Post1B1D strict mode).
    let allow = [AllowlistedDuplicate {
        kind: DuplicateKind::SameDirection,
        file_a: "a.rs",
        symbol_a: "f1",
        file_b: "b.rs",
        symbol_b: "f2",
        enum_name: "Align",
        expected_count: 1,
        tracking: "test",
        reason: "legacy",
    }];
    let rep_ok = evaluate_duplicates_with_ratchet(&dups, &allow);
    assert!(rep_ok.active_violations.is_empty(), "{:?}", rep_ok.active_violations);
    assert_eq!(rep_ok.allowlisted_violations.len(), 1);

    // 3. Stale allowlist entry (duplicate removed) -> fails immediately!
    let rep_stale = evaluate_duplicates_with_ratchet(&[], &allow);
    assert_eq!(rep_stale.active_violations.len(), 1);
    assert!(rep_stale.active_violations[0].reason.contains("Stale"));

    // 4. Ceiling exceeded (2nd duplicate in same symbol pair) -> fails immediately!
    let two_dups = vec![dups[0].clone(), dups[0].clone()];
    let rep_grew = evaluate_duplicates_with_ratchet(&two_dups, &allow);
    assert_eq!(rep_grew.active_violations.len(), 1);
    assert!(rep_grew.active_violations[0].reason.contains("grew to 2"));

    // 5. Unsorted / zero-count allowlist -> fails immediately!
    let bad_allow = [
        AllowlistedDuplicate {
            kind: DuplicateKind::Mirrored,
            file_a: "z.rs",
            symbol_a: "z",
            file_b: "z.rs",
            symbol_b: "z2",
            enum_name: "Z",
            expected_count: 0,
            tracking: "test",
            reason: "bad",
        },
        allow[0],
    ];
    let rep_bad = evaluate_duplicates_with_ratchet(&dups, &bad_allow);
    assert!(
        rep_bad.active_violations.iter().any(|v| v.reason.contains("strictly sorted")),
        "unsorted allowlist must fail: {:?}",
        rep_bad.active_violations
    );
    assert!(
        rep_bad.active_violations.iter().any(|v| v.reason.contains("expected_count == 0")),
        "zero-count allowlist entry must fail: {:?}",
        rep_bad.active_violations
    );
}

#[test]
fn converters_catches_structural_duplicate_enums_try_from_and_unnamed_converters() {
    let core_src = r#"
        pub enum WireTransport { Tcp, Quic, Ws, Uds, Ble }
    "#;
    let host_src = r#"
        pub enum PairTransport { Tcp, Quic, Ws, Uds, Ble }
        impl TryFrom<PairTransport> for WireTransport {
            type Error = ();
            fn try_from(v: PairTransport) -> Result<Self, ()> {
                match v {
                    PairTransport::Tcp => Ok(Self::Tcp),
                    PairTransport::Quic => Ok(Self::Quic),
                    PairTransport::Ws => Ok(Self::Ws),
                    PairTransport::Uds => Ok(Self::Uds),
                    PairTransport::Ble => Ok(Self::Ble),
                }
            }
        }
        pub fn arbitrary_name(raw: u8) -> Option<WireTransport> {
            match raw {
                0 => Some(WireTransport::Tcp),
                1 => Some(WireTransport::Quic),
                2 => Some(WireTransport::Ws),
                _ => None,
            }
        }
        pub fn http_code_lookup(status: u16) -> Option<WireTransport> {
            match status {
                200 => Some(WireTransport::Tcp),
                404 => Some(WireTransport::Quic),
                500 => Some(WireTransport::Ws),
                _ => None,
            }
        }
    "#;
    let ast_core = syn::parse_file(core_src).expect("valid core snippet");
    let ast_host = syn::parse_file(host_src).expect("valid host snippet");
    let mut violations = Vec::new();
    scan_duplicate_enums_and_converters(
        &[("packages/core/wire/src/lib.rs".into(), core_src, &ast_core)],
        &[("hosts/skia-shell/src/pair.rs".into(), host_src, &ast_host)],
        &mut violations,
    );
    assert!(
        violations.iter().any(|v| v.reason.contains("Structurally identical `pub enum PairTransport`")),
        "expected structural duplicate enum violation: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.reason.contains("impl TryFrom<PairTransport> for WireTransport")),
        "expected TryFrom converter violation: {violations:?}"
    );
    assert!(
        !violations.iter().any(|v| v.reason.contains("`fn try_from`")),
        "must not double-report fn try_from inside already-flagged impl TryFrom: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.reason.contains("`fn arbitrary_name`")),
        "unnamed compact discriminant decoder must be caught: {violations:?}"
    );
    assert!(
        !violations.iter().any(|v| v.reason.contains("`fn http_code_lookup`")),
        "non-compact HTTP status lookup must NOT be flagged as WordEnum discriminant: {violations:?}"
    );
}
