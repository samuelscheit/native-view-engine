//! Track 1B-1d staged debt scanners and permanent Track 1B-1e codegen guards:
//! - `#[derive(NveTable)]` field hygiene (`#[bits(N)]`, magic sentinel defaults, split bool sentinels)
//! - Hand-rolled `Unset` enum variants, manual `SET_*` masks, Tier-1 `encode_bytes`/`decode_bytes`
//! - Dioxus numeric `AttributeValue::{Float,Int}` -> `String` round-trips
//! - Platform purity (`packages/core/*` zero CSS/HTML formatters & zero `motion`/`MotionClock`)
//! - Banned 4-letter format acronyms
//! - Dart-web / TypeScript 53-bit integer safety & legacy codegen path bans (always enforced)

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Fields, ImplItem, Item, Type};

use crate::ast_layout::{has_attr_list_ident, last_type_ident};
use crate::codegen_53bit;
use crate::{
    is_binary_ext, is_cfg_test_attr, is_self_invariant_file, is_test_or_bench_rs, line_at,
    RepoIndex, Violation, DOC_EXCLUDES,
};

const MAGIC_SENTINEL_DEFAULTS: &[&str] = &[
    "default(u8::MAX)",
    "default(u16::MAX)",
    "default(u32::MAX)",
    "default(i16::MIN)",
    "default(i32::MIN)",
    "default(f32::NAN)",
    "default(f32::INFINITY)",
    "default(f32::NEG_INFINITY)",
];

const BANNED_4_LETTER_ACRONYMS: &[&str] = &[
    "NVH1", "NVW1", "NVT1", "NVD1", "NVC1", "NVWS1", "NVB1", "NVNP", "NVNC", "NVERELOC", "NWS1",
    "NVE_MET1", "NVE1", "NVP1", "NK1",
];

const BANNED_CORE_CSS_FN_PREFIXES: &[&str] = &[
    "write_css",
    "as_css",
    "as_html",
    "to_css_keyword",
    "write_html",
    "format_css",
    "parse_box_shadow",
];

const LEGACY_CODEGEN_PATHS: &[&str] = &[
    "packages/core/codegen/src/schema/wire.rs",
    "packages/core/codegen/src/schema/reloc",
    "packages/core/codegen/src/frontend/hybrid_rust",
    "packages/core/codegen/src/schema/scene_abi.rs",
    "packages/core/codegen/src/schema/hybrid/core_abi.rs",
    "packages/core/codegen/src/targets/hash",
];

/// Scans production Rust files under `packages/` for `#[derive(NveTable)]` field hygiene,
/// `Unset` sentinel enum variants, manual `SET_*` mask constants, Tier-1 `encode_bytes`/
/// `decode_bytes` on `#[repr(C)]` files, and numeric RSX -> `String` round-trips in `nve-dioxus`.
pub fn scan_table_and_codec_hygiene(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    out: &mut Vec<Violation>,
) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }

    let check_unset = rel.starts_with("packages/core/tables/")
        || (rel.starts_with("packages/modules/") && rel.matches('/').count() >= 4);
    let check_set_masks = rel.starts_with("packages/core/tables/");
    let in_core = rel.starts_with("packages/core/");
    let in_dioxus = rel.starts_with("packages/core/dioxus/src/");
    let has_repr_c = file_has_repr_c_struct(&ast.items);

    scan_table_items(
        rel,
        raw,
        &ast.items,
        check_unset,
        check_set_masks,
        in_core && has_repr_c,
        in_dioxus,
        out,
    );
}

#[expect(clippy::too_many_arguments, reason = "internal recursive AST walker sharing immutable file scope flags")]
fn scan_table_items(
    rel: &str,
    raw: &str,
    items: &[Item],
    check_unset: bool,
    check_set_masks: bool,
    check_tier1_bytes: bool,
    in_dioxus: bool,
    out: &mut Vec<Violation>,
) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    scan_table_items(
                        rel,
                        raw,
                        sub,
                        check_unset,
                        check_set_masks,
                        check_tier1_bytes,
                        in_dioxus,
                        out,
                    );
                }
            }
            Item::Struct(st)
                if !is_cfg_test_attr(&st.attrs)
                    && has_attr_list_ident(&st.attrs, "derive", "NveTable") =>
            {
                if let Fields::Named(named) = &st.fields {
                    let all_names: Vec<String> = named
                        .named
                        .iter()
                        .filter_map(|f| f.ident.as_ref().map(ToString::to_string))
                        .collect();
                    for field in &named.named {
                        let Some(ident) = &field.ident else { continue };
                        let fname = ident.to_string();
                        let ln = ident.span().start().line;
                        let ty_name = last_type_ident(&field.ty).unwrap_or_default();

                        if let Some(bits_n) = extract_bits_attr(&field.attrs)
                            && !is_valid_narrowing_bits(&field.ty, bits_n)
                        {
                            out.push(Violation::new(
                                rel,
                                ln,
                                line_at(raw, ln),
                                format!(
                                    "Redundant/invalid `#[bits({bits_n})]` on `{fname}: {ty_name}` in #[derive(NveTable)]; only valid when strictly narrowing an integer primitive (`u8`/`u16`/`u32`/`i8`/`i16`/`i32`)"
                                ),
                            ));
                        }
                        for attr in &field.attrs {
                            let compact: String = quote_attr_compact(attr);
                            for &sentinel in MAGIC_SENTINEL_DEFAULTS {
                                if compact.contains(sentinel) {
                                    out.push(Violation::new(
                                        rel,
                                        ln,
                                        line_at(raw, ln),
                                        format!(
                                            "Magic sentinel `#[{sentinel}]` in NveTable is banned; use `Option<T>`"
                                        ),
                                    ));
                                }
                            }
                        }
                        if is_split_bool_sentinel(&fname, &field.ty, &all_names) {
                            out.push(Violation::new(
                                rel,
                                ln,
                                line_at(raw, ln),
                                format!(
                                    "Split boolean presence sentinel `{fname}` in NveTable is banned; fold into `Option<T>`"
                                ),
                            ));
                        }
                    }
                }
            }
            Item::Enum(en) if check_unset && !is_cfg_test_attr(&en.attrs) => {
                for variant in &en.variants {
                    if variant.ident == "Unset" {
                        let ln = variant.ident.span().start().line;
                        out.push(Violation::new(
                            rel,
                            ln,
                            line_at(raw, ln),
                            format!(
                                "Hand-rolled `Unset` sentinel variant in enum `{}` is banned; model optionality with `Option<{}>`",
                                en.ident, en.ident
                            ),
                        ));
                    }
                }
            }
            Item::Const(c)
                if check_set_masks
                    && !is_cfg_test_attr(&c.attrs)
                    && c.ident.to_string().starts_with("SET_") =>
            {
                let ln = c.ident.span().start().line;
                out.push(Violation::new(
                    rel,
                    ln,
                    line_at(raw, ln),
                    "Manual `SET_*` mask constants are banned; use `#[nve_table(inherited)]`",
                ));
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                if check_tier1_bytes
                    && matches!(f.sig.ident.to_string().as_str(), "encode_bytes" | "decode_bytes")
                {
                    let ln = f.sig.ident.span().start().line;
                    out.push(Violation::new(
                        rel,
                        ln,
                        line_at(raw, ln),
                        "Hand-written `encode_bytes`/`decode_bytes` on Tier 1 `#[repr(C)]` POD is banned; pass typed memory or derive `#[derive(NveCodec)]`",
                    ));
                }
                if in_dioxus {
                    scan_dioxus_numeric_to_string(rel, raw, &f.block, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                for ii in &imp.items {
                    match ii {
                        ImplItem::Const(c)
                            if check_set_masks
                                && !is_cfg_test_attr(&c.attrs)
                                && c.ident.to_string().starts_with("SET_") =>
                        {
                            let ln = c.ident.span().start().line;
                            out.push(Violation::new(
                                rel,
                                ln,
                                line_at(raw, ln),
                                "Manual `SET_*` mask constants are banned; use `#[nve_table(inherited)]`",
                            ));
                        }
                        ImplItem::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                            if check_tier1_bytes
                                && matches!(
                                    f.sig.ident.to_string().as_str(),
                                    "encode_bytes" | "decode_bytes"
                                )
                            {
                                let ln = f.sig.ident.span().start().line;
                                out.push(Violation::new(
                                    rel,
                                    ln,
                                    line_at(raw, ln),
                                    "Hand-written `encode_bytes`/`decode_bytes` on Tier 1 `#[repr(C)]` POD is banned; pass typed memory or derive `#[derive(NveCodec)]`",
                                ));
                            }
                            if in_dioxus {
                                scan_dioxus_numeric_to_string(rel, raw, &f.block, out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// Scans `packages/core/*` for CSS/HTML formatters/parsers (belonging in `nve-backend-web`) and
/// stateful `motion`/`MotionClock` references (belonging in `packages/modules/motion/`).
pub fn scan_platform_purity(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<Violation>) {
    if !rel.starts_with("packages/core/") || is_cfg_test_attr(&ast.attrs) {
        return;
    }
    if rel.contains("motion") {
        out.push(Violation::new(
            rel,
            1,
            rel,
            "Stateful runtime engine `motion` must live in standalone `packages/modules/motion/`, never in core crates",
        ));
    }

    struct PurityScan<'a> {
        rel: &'a str,
        raw: &'a str,
        out: &'a mut Vec<Violation>,
    }

    impl PurityScan<'_> {
        fn check_fn_name(&mut self, ident: &syn::Ident) {
            let name = ident.to_string();
            if BANNED_CORE_CSS_FN_PREFIXES.iter().any(|p| name.starts_with(p)) {
                let ln = ident.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    "Web CSS/HTML formatters/parsers belong in `nve-backend-web`, never `packages/core/*`",
                ));
            }
        }
    }

    impl<'ast> Visit<'ast> for PurityScan<'_> {
        fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
            if is_cfg_test_attr(&node.attrs) {
                return;
            }
            if node.ident == "motion" {
                let ln = node.ident.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    "Stateful runtime engine `motion` must live in standalone `packages/modules/motion/`, never in core crates",
                ));
            }
            visit::visit_item_mod(self, node);
        }

        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if is_cfg_test_attr(&node.attrs) {
                return;
            }
            self.check_fn_name(&node.sig.ident);
            visit::visit_item_fn(self, node);
        }

        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            if is_cfg_test_attr(&node.attrs) {
                return;
            }
            self.check_fn_name(&node.sig.ident);
            visit::visit_impl_item_fn(self, node);
        }

        fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
            if is_cfg_test_attr(&node.attrs) {
                return;
            }
            self.check_fn_name(&node.sig.ident);
            visit::visit_trait_item_fn(self, node);
        }

        fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
            if !is_cfg_test_attr(&node.attrs) {
                visit::visit_item_impl(self, node);
            }
        }

        fn visit_ident(&mut self, i: &'ast syn::Ident) {
            if i == "MotionClock" {
                let ln = i.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    "Stateful runtime engine `motion` must live in standalone `packages/modules/motion/`, never in core crates",
                ));
            }
        }
    }

    PurityScan {
        rel,
        raw,
        out,
    }
    .visit_file(ast);
}

/// Scans all tracked non-binary text files outside `DOC_EXCLUDES` for banned 4-letter acronyms.
pub fn check_banned_4_letter_acronyms(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || DOC_EXCLUDES.contains(&rel.as_str())
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        scanned += 1;
        scan_banned_acronyms_str(rel, &content, &mut violations);
    }
    assert!(scanned > 0, "banned acronym scanner must scan > 0 tracked files");
    violations
}

pub fn scan_banned_acronyms_str(rel: &str, content: &str, out: &mut Vec<Violation>) {
    for (idx, line) in content.lines().enumerate() {
        for &acr in BANNED_4_LETTER_ACRONYMS {
            if line_contains_ident(line, acr) {
                out.push(Violation::new(
                    rel,
                    idx + 1,
                    line,
                    format!("Banned legacy 4-letter format acronym `{acr}`; use clear self-documenting names"),
                ));
            }
        }
    }
}

/// Scans `packages/core/codegen/src/targets` for Dart-web 53-bit `const int 0x...` safety and
/// bans legacy codegen paths (`targets/**/*wire.rs`, `targets/**/reloc*`, `targets/**/*core_abi.rs`,
/// `targets/**/scene_abi.rs`, `targets/hash`, `schema/wire.rs`, `schema/reloc`,
/// `schema/hybrid/core_abi.rs`, `frontend/hybrid_rust`, `schema/scene_abi.rs`).
pub fn check_codegen_hygiene_and_dart_53bit(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for rel in idx.rs_files_under("packages/core/codegen/src/targets") {
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        let Some(src) = idx.read_text(&rel) else {
            continue;
        };
        scanned += 1;
        if rel.contains("/dart/") || rel.ends_with("/dart.rs") {
            codegen_53bit::scan_dart_53bit_safety(&rel, &src, &mut violations);
        }
        if rel.contains("/typescript/") || rel.ends_with("/typescript.rs") {
            codegen_53bit::scan_ts_53bit_safety(&rel, &src, &mut violations);
        }
        let file = rel.rsplit('/').next().unwrap_or("");
        if file.ends_with("wire.rs")
            || file.starts_with("reloc")
            || file.ends_with("core_abi.rs")
            || file == "scene_abi.rs"
        {
            violations.push(Violation::new(
                &rel,
                1,
                &rel,
                "Legacy codegen target (`*wire.rs` / `reloc*` / `*core_abi.rs` / `scene_abi.rs`) must stay deleted: host ABI data is rendered from `templates/` by `src/abi`",
            ));
        }
    }
    assert!(scanned > 0, "codegen hygiene scanner must scan > 0 target files");

    for &legacy_path in LEGACY_CODEGEN_PATHS {
        if idx.root.join(legacy_path).exists() {
            violations.push(Violation::new(
                legacy_path,
                1,
                legacy_path,
                "Legacy codegen schema/frontend path must be deleted post-1B-1e",
            ));
        }
    }
    violations
}

fn file_has_repr_c_struct(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Struct(st) if !is_cfg_test_attr(&st.attrs) => {
            has_attr_list_ident(&st.attrs, "repr", "C")
        }
        Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => m
            .content
            .as_ref()
            .is_some_and(|(_, sub)| file_has_repr_c_struct(sub)),
        _ => false,
    })
}

fn quote_attr_compact(attr: &Attribute) -> String {
    match &attr.meta {
        syn::Meta::List(list) => {
            let mut s = format!("{}({})", list.path.segments.last().map(|p| p.ident.to_string()).unwrap_or_default(), list.tokens);
            s.retain(|c| !c.is_whitespace());
            s
        }
        _ => String::new(),
    }
}

fn extract_bits_attr(attrs: &[Attribute]) -> Option<usize> {
    for attr in attrs {
        if attr.path().is_ident("bits")
            && let Ok(syn::Lit::Int(int)) = attr.parse_args::<syn::Lit>()
            && let Ok(n) = int.base10_parse::<usize>()
        {
            return Some(n);
        }
    }
    None
}

fn is_valid_narrowing_bits(ty: &Type, bits_n: usize) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    if tp.qself.is_some() || tp.path.segments.len() != 1 {
        return false;
    }
    let natural = match tp.path.segments[0].ident.to_string().as_str() {
        "u8" | "i8" => 8,
        "u16" | "i16" => 16,
        "u32" | "i32" => 32,
        _ => return false,
    };
    bits_n > 0 && bits_n < natural
}

fn is_split_bool_sentinel(fname: &str, ty: &Type, all_fields: &[String]) -> bool {
    if last_type_ident(ty).as_deref() != Some("bool") {
        return false;
    }
    if fname.ends_with("_enabled")
        || fname.ends_with("_set")
        || fname.ends_with("_present")
        || fname.ends_with("_specified")
    {
        return true;
    }
    fname
        .strip_prefix("has_")
        .is_some_and(|stem| all_fields.iter().any(|other| other == stem))
}

fn scan_dioxus_numeric_to_string(
    rel: &str,
    raw: &str,
    block: &syn::Block,
    out: &mut Vec<Violation>,
) {
    struct ArmScan<'a> {
        rel: &'a str,
        raw: &'a str,
        out: &'a mut Vec<Violation>,
    }
    struct PatHasNumericAttr(bool);
    impl<'ast> Visit<'ast> for PatHasNumericAttr {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs.windows(2).any(|w| {
                w[0] == "AttributeValue" && (w[1] == "Float" || w[1] == "Int")
            }) {
                self.0 = true;
            }
            visit::visit_path(self, p);
        }
    }
    struct BodyAllocString(bool);
    impl<'ast> Visit<'ast> for BodyAllocString {
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if node.method == "to_string" {
                self.0 = true;
            }
            visit::visit_expr_method_call(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if node.path.segments.last().is_some_and(|s| s.ident == "format") {
                self.0 = true;
            }
        }
    }
    impl<'ast> Visit<'ast> for ArmScan<'_> {
        fn visit_arm(&mut self, arm: &'ast syn::Arm) {
            let mut pat = PatHasNumericAttr(false);
            pat.visit_pat(&arm.pat);
            if pat.0 {
                let mut body = BodyAllocString(false);
                body.visit_expr(&arm.body);
                if body.0 {
                    let ln = arm.pat.span().start().line;
                    self.out.push(Violation::new(
                        self.rel,
                        ln,
                        line_at(self.raw, ln),
                        "Numeric RSX `f64`/`i64` -> `String` -> parse round-trip in `nve-dioxus` is banned; write typed numeric value directly",
                    ));
                }
            }
            visit::visit_arm(self, arm);
        }
    }
    ArmScan { rel, raw, out }.visit_block(block);
}

fn line_contains_ident(line: &str, ident: &str) -> bool {
    let bytes = line.as_bytes();
    let nb = ident.as_bytes();
    let mut pos = 0;
    while let Some(idx) = line[pos..].find(ident) {
        let abs = pos + idx;
        let before_ok = abs == 0 || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_');
        let after = abs + nb.len();
        let after_ok =
            after >= bytes.len() || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if before_ok && after_ok {
            return true;
        }
        pos = abs + 1;
    }
    false
}
