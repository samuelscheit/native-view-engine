//! Stack-safety, nesting bounds, long-flat-spec scaling, doc preservation, and prop-default /
//! nested-option validation tests for the `syn`-based hybrid `.rs` spec frontend.

use nve_codegen::{CodegenError, parse_hybrid_rust_spec};

/// `(line, col, message)` of the parse error `source` must raise, or the unexpected outcome.
fn try_location(source: &str) -> Result<(usize, usize, String), String> {
    match parse_hybrid_rust_spec(source) {
        Err(CodegenError::Parse { line, col, message }) => Ok((line, col, message)),
        other => Err(format!("expected a parse error, got {other:?}")),
    }
}

/// Runs `f` on a thread with a 2 MiB stack (the default for spawned threads): the parser never
/// recurses on its caller's stack, so a small one must be enough.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("no stack overflow")
}

/// Every prefix / infix / postfix / keyword / group form, 100k deep, is rejected by the path bound
/// before `syn` runs, whatever the caller's stack (here 2 MiB).
#[test]
fn deep_nesting_of_every_form_is_rejected_on_a_small_stack() {
    let deep = 100_000;
    let rep = |s: &str| s.repeat(deep);
    let types = [
        rep("*const ") + "u8",
        rep("&") + "u8",
        rep("Option<") + "u8" + &rep(">"),
        rep("(") + "u8" + &rep(")"),
        rep("[") + "u8" + &rep("; 1]"),
        rep("fn() -> ") + "u8",
        rep("extern \"C\" fn() -> ") + "u8",
    ];
    let exprs = [
        rep("-") + "1",
        rep("!") + "1",
        rep("return ") + "1",
        rep("|| ") + "1",
        rep("|x,| ") + "1",
        rep("a = ") + "1",
        rep("a += ") + "1",
        "1".to_owned() + &rep(" + 1"),
        "a".to_owned() + &rep(".b"),
        "f".to_owned() + &rep("()"),
        "a".to_owned() + &rep("[0]"),
        "a".to_owned() + &rep("?"),
        "1".to_owned() + &rep(" as u8"),
        rep("if a { 1 } else ") + "{ 1 }",
        rep("match a { _ => ") + "1" + &rep(" }"),
        rep("loop { break ") + "1" + &rep(" }"),
        rep("while a { ") + &rep(" }"),
        rep("for x in a { ") + &rep(" }"),
        rep("{ let a = ") + "1" + &rep("; a }"),
        rep("become ") + "1",
        rep("..") + "1",
        rep("1..=") + "1",
        rep("x @ ") + "1",
        rep("move || ") + "1",
    ];
    let mut sources: Vec<String> = types
        .iter()
        .map(|ty| format!("#[nve_hybrid_struct]\npub struct S {{ pub x: {ty} }}"))
        .collect();
    sources.extend(
        exprs
            .iter()
            .map(|e| format!("#[nve_hybrid_const]\npub const X: u32 = {e};")),
    );
    let results = on_small_stack(move || {
        sources
            .iter()
            .map(|s| try_location(s).map(|(_, _, m)| m))
            .collect::<Vec<_>>()
    });
    for (i, result) in results.into_iter().enumerate() {
        let message = result.unwrap_or_else(|e| panic!("form #{i}: {e}"));
        assert!(message.contains("nesting too deep"), "form #{i}: {message}");
    }
}

/// Realistic inputs never trip the bound: forms nested just under it, long runs of `//!` docs
/// and of flat consts (a segment ends at `;`), and hundreds of items.
#[test]
fn nesting_under_the_bound_and_long_flat_specs_still_parse() {
    let ok = |source: String| {
        on_small_stack(move || {
            parse_hybrid_rust_spec(&source)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    };
    ok(format!(
        "#[nve_hybrid_struct]\npub struct S {{ pub x: {}u8{} }}",
        "Vec<".repeat(40),
        ">".repeat(40)
    ))
    .expect("40 Vec<");
    ok(format!(
        "#[nve_hybrid_struct]\npub struct S {{ pub x: {}u8 }}",
        "*const ".repeat(60)
    ))
    .expect("60 *const");
    let not_nesting = |e: String| assert!(!e.contains("nesting too deep"), "{e}");
    not_nesting(
        ok(format!(
            "#[nve_hybrid_struct]\npub struct S {{ pub x: [u8; {}1{}] }}",
            "(".repeat(100),
            ")".repeat(100)
        ))
        .expect_err("a parenthesized array length is not a literal"),
    );
    let docs = "//! doc line\n".repeat(150);
    ok(format!("{docs}#[nve_hybrid_const]\npub const X: u32 = 1;")).expect("150 //! lines");
    let consts: String = (0..70)
        .map(|i| format!("#[nve_hybrid_const]\npub const F{i}: u32 = 1 << {i};\n"))
        .collect();
    not_nesting(ok(consts).expect_err("`1 << i` is not an unsigned literal"));
    // The heaviest recursive forms just under the bound: accepted by the pre-check (the value
    // itself is then rejected), and `syn` parses, spans, and drops them on the 2 MiB stack.
    // `(form, tokens per repetition)`: the bound counts tokens.
    for (form, tokens) in [
        ("-", 1),
        ("!", 1),
        ("return ", 1),
        ("|| ", 2),
        ("&", 1),
        ("a = ", 2),
        ("move || ", 3),
    ] {
        not_nesting(
            ok(format!(
                "#[nve_hybrid_const]\npub const X: u32 = {}1;",
                form.repeat(115 / tokens)
            ))
            .expect_err("not an unsigned literal"),
        );
    }
    not_nesting(
        ok(format!(
            "#[nve_hybrid_const]\npub const X: u32 = 1{};",
            " + 1".repeat(55)
        ))
        .expect_err("not a literal"),
    );
    let structs: String = (0..300)
        .map(|i| format!("/// Doc.\n#[nve_hybrid_struct]\npub struct S{i} {{ pub x: u32 }}\n"))
        .collect();
    ok(structs).expect("300 items");
    let attrs: String = (0..200).map(|i| format!("#[doc = \"{i}\"]\n")).collect();
    ok(format!("{attrs}#[nve_hybrid_const]\npub const X: u32 = 1;")).expect("200 outer attributes");
}

#[test]
fn block_docs_and_prop_default_ranges() {
    let spec = parse_hybrid_rust_spec(
        "/**\n * First line.\n * Second line.\n */\n#[nve_hybrid_const]\npub const X: u32 = 1;",
    )
    .expect("block doc");
    assert_eq!(
        spec.consts[0].doc.as_deref(),
        Some("First line.\nSecond line.")
    );
    for (ty, default) in [
        ("u8", "300"),
        ("i32", "4294967295"),
        ("f32", "1e300"),
        ("u32", "-1"),
    ] {
        let source = format!(
            "#[nve_hybrid_view]\npub trait V {{ #[prop(default = {default})] fn x(&self) -> {ty}; }}"
        );
        assert!(
            matches!(
                parse_hybrid_rust_spec(&source),
                Err(CodegenError::Validation(_))
            ),
            "default {default} must not fit {ty}"
        );
    }
    assert!(
        parse_hybrid_rust_spec(
            "#[nve_hybrid_view]\npub trait V { #[prop(default = 255)] fn x(&self) -> u8; }"
        )
        .is_ok()
    );
}

/// Largest path bound the frontend accepts (`MAX_PATH_TOKENS` in `frontend/hybrid_spec/stack.rs`).
const MAX_PATH_TOKENS: usize = 2048;

/// `source` is valid Rust: `syn` parses all of it (on a generous stack), so the frontend's own
/// parse of it recursed through the whole nest too.
fn assert_valid_rust(name: &str, source: &str) {
    let src = source.to_owned();
    let valid = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || syn::parse_file(&src).map(drop).map_err(|e| e.to_string()))
        .expect("spawn syn checker")
        .join()
        .expect("syn checker");
    assert!(valid.is_ok(), "{name} is not valid Rust: {valid:?}");
}

/// The frontend's outcome for `source` (on the caller's 2 MiB stack): accepted, or a located
/// parse error that is not the depth cap (never a thread / validation failure).
fn parse_without_depth_error(name: &str, source: &str) {
    let src = source.to_owned();
    let result = on_small_stack(move || parse_hybrid_rust_spec(&src));
    if let Err(err) = result {
        assert!(
            matches!(err, CodegenError::Parse { .. }),
            "{name}: expected Ok or a parse error, got {err}"
        );
        if let CodegenError::Parse { message, .. } = err {
            assert!(!message.contains("nesting too deep"), "{name}: {message}");
        }
    }
}

/// Every recursive form, nested to about 1,950 path tokens (just under the cap), parses, lowers,
/// and drops on the parse thread's stack: the stack size is sound for the worst forms (`&&..&u8`,
/// `((u8))`, `{{1}}`, `&&&1`, `let &&&x`, ...), whatever their grammar. Forms that cross the
/// count restarts (`;`, `,`, file-level items) nest that deep *per* statement / element / item.
#[test]
fn every_recursive_form_parses_on_its_sized_stack() {
    // (form, path tokens per repetition, prefix repeated, suffix repeated, innermost)
    let types: [(&str, usize, &str, &str, &str); 14] = [
        ("*const", 2, "*const ", "", "u8"),
        ("&", 1, "&", "", "u8"),
        ("&'struct", 3, "&'struct ", "", "u8"),
        ("&'union mut", 4, "&'union mut ", "", "u8"),
        ("Option<", 3, "Option<", ">", "u8"),
        ("(", 1, "(", ")", "u8"),
        ("(u8,", 1, "(u8, ", ")", "u8"),
        ("[; 1]", 1, "[", "; 1]", "u8"),
        ("[&&&; 1]", 4, "[&&& ", "; 1]", "u8"),
        ("fn() ->", 4, "fn() -> ", "", "u8"),
        ("fn(", 2, "fn(", ")", "u8"),
        ("extern C fn", 6, "extern \"C\" fn() -> ", "", "u8"),
        ("for<'a, 'b> fn", 14, "for<'a, 'b> fn() -> ", "", "u8"),
        ("dyn Fn() ->", 5, "dyn Fn() -> ", "", "u8"),
    ];
    let exprs: [(&str, usize, &str, &str, &str); 43] = [
        ("-", 1, "-", "", "1"),
        ("!", 1, "!", "", "1"),
        ("*", 1, "*", "", "1"),
        ("&", 1, "&", "", "1"),
        ("&mut", 2, "&mut ", "", "1"),
        ("{", 1, "{", "}", "1"),
        ("(", 1, "(", ")", "1"),
        ("[", 1, "[", "]", "1"),
        ("return", 1, "return ", "", "1"),
        ("||", 2, "|| ", "", "1"),
        ("|x,|", 4, "|x,| ", "", "1"),
        ("|| |x,|", 6, "|| |x,| ", "", "1"),
        ("|&_,|-", 6, "|&_,|- ", "", "1"),
        ("move ||", 3, "move || ", "", "1"),
        ("|| {", 3, "|| { ", "}", "1"),
        ("a =", 2, "a = ", "", "1"),
        ("union =", 2, "union = ", "", "1"),
        ("union.union =", 4, "union.union = ", "", "1"),
        ("f::<u8, u8> =", 9, "f::<u8, u8> = ", "", "1"),
        ("a +=", 3, "a += ", "", "1"),
        ("a.b", 2, "", ".b", "a"),
        ("()", 1, "", "()", "f"),
        ("f(", 2, "f(", ")", "1"),
        ("f(1, 2,", 2, "f(1, 2, ", ")", "1"),
        ("[1,", 1, "[1, ", "]", "1"),
        ("[0]", 1, "", "[0]", "a"),
        ("?", 1, "", "?", "a"),
        (".await", 2, "", ".await", "x"),
        ("as u8", 2, "", " as u8", "1"),
        ("if else", 4, "if a { 1 } else ", "", "{ 1 }"),
        ("if union", 5, "if union {} else ", "", "{}"),
        ("match", 6, "match a { _ => ", " }", "1"),
        ("loop break", 3, "loop { break ", " }", "1"),
        ("'a: loop {", 5, "'a: loop { ", "}", ""),
        ("while", 3, "while a { ", " }", ""),
        ("for x in a {", 5, "for x in a { ", " }", ""),
        ("{ let a =", 4, "{ let a = ", "; a }", "1"),
        ("unsafe", 2, "unsafe { ", "}", ""),
        ("const {", 2, "const { ", " }", "1"),
        ("S { x:", 4, "S { x: ", " }", "1"),
        ("a::<", 5, "a::<", ">", "u8"),
        ("become", 1, "become ", "", "1"),
        ("yield", 1, "yield ", "", "1"),
    ];
    // Inside a statement / pattern: `let &&&x`, `let [[[x]]]`, `let (((x,),),)`.
    let pats: [(&str, usize, &str, &str, &str); 3] = [
        ("let &", 1, "&", "", "x"),
        ("let [", 1, "[", "]", "x"),
        ("let (,)", 1, "(", ",)", "x"),
    ];
    let nest = |(_, per, prefix, suffix, inner): (&str, usize, &str, &str, &str)| {
        let n = (MAX_PATH_TOKENS - 100) / per;
        format!("{}{inner}{}", prefix.repeat(n), suffix.repeat(n))
    };
    let mut sources: Vec<(String, String)> = Vec::new();
    for form in types {
        sources.push((
            form.0.into(),
            format!(
                "#[nve_hybrid_struct]\npub struct S {{ pub x: {} }}",
                nest(form)
            ),
        ));
    }
    for form in exprs {
        sources.push((
            form.0.into(),
            format!("#[nve_hybrid_const]\npub const X: u32 = {};", nest(form)),
        ));
    }
    for form in pats {
        sources.push((
            form.0.into(),
            format!(
                "#[nve_hybrid_const]\npub const X: u32 = {{ let {} = 1; 1 }};",
                nest(form)
            ),
        ));
    }
    sources.push((
        "mod m {".into(),
        nest(("mod m {", 3, "mod m { ", "}", "")),
    ));
    // Restart-crossing forms: each statement / element / item nests to the cap on its own.
    let deep_neg = "-".repeat(MAX_PATH_TOKENS - 100);
    sources.push((
        "3 statements".into(),
        format!(
            "#[nve_hybrid_const]\npub const X: u32 = {{ {deep_neg}x; {deep_neg}x; {deep_neg}x }};"
        ),
    ));
    sources.push(("let-else".into(), format!("#[nve_hybrid_const]\npub const X: u32 = {{ let Some(x) = {deep_neg}a else {{ return }}; {deep_neg}x }};")));
    sources.push((
        "3 tuple elements".into(),
        format!("#[nve_hybrid_const]\npub const X: u32 = ({deep_neg}1, {deep_neg}1, {deep_neg}1);"),
    ));
    let deep_ref = "&".repeat(MAX_PATH_TOKENS - 100);
    sources.push((
        "3 items".into(),
        (0..3)
            .map(|i| format!("#[nve_hybrid_struct]\npub struct S{i} {{ pub x: {deep_ref}u8 }}\n"))
            .collect(),
    ));
    sources.push(("3 fields".into(), format!("#[nve_hybrid_struct]\npub struct S {{ pub a: [{deep_ref}u8; 1], pub b: {deep_ref}u8, pub c: {deep_ref}u8 }}")));
    for (name, source) in &sources {
        assert_valid_rust(name, source);
        parse_without_depth_error(name, source);
    }
}

/// The path bound is exact at its cap: at the cap the spec reaches `syn`, one token more is a
/// located error before it. (`#[..]` restarts; `pub const X: u32 = -..-1;` is 8 + k tokens.)
#[test]
fn the_path_bound_cap_is_exact() {
    let spec = |k: usize| {
        format!(
            "#[nve_hybrid_const]\npub const X: u32 = {}1;",
            "-".repeat(k)
        )
    };
    let at_cap = spec(MAX_PATH_TOKENS - 8);
    assert_valid_rust("at cap", &at_cap);
    match parse_hybrid_rust_spec(&at_cap) {
        Err(CodegenError::Parse { message, .. }) => {
            assert!(!message.contains("nesting too deep"), "{message}");
        }
        other => panic!("expected the lowering's parse error, got {other:?}"),
    }
    match parse_hybrid_rust_spec(&spec(MAX_PATH_TOKENS - 7)) {
        Err(CodegenError::Parse { line, message, .. }) => {
            assert_eq!(line, 2);
            assert!(message.contains("nesting too deep"), "{message}");
        }
        other => panic!("expected the depth error, got {other:?}"),
    }
}

/// Long flat specs never trip the bound: items, members, fields, variants, and doc lines restart
/// the count.
#[test]
fn long_flat_specs_parse() {
    let blocks: String = (0..300)
        .map(|i| {
            format!(
                "extern \"C\" {{\n    #[nve_hybrid_c_fn]\n    pub fn f{i}(a: u32) -> u32;\n}}\n"
            )
        })
        .collect();
    assert_eq!(
        parse_hybrid_rust_spec(&blocks)
            .expect("300 extern blocks")
            .c_functions
            .len(),
        300
    );
    let consts: String = (0..3000)
        .map(|i| {
            format!("/// Doc {i}.\n/// More.\n#[nve_hybrid_const]\npub const C{i}: u32 = {i};\n")
        })
        .collect();
    assert_eq!(
        parse_hybrid_rust_spec(&consts)
            .expect("3000 consts")
            .consts
            .len(),
        3000
    );
    let members: String = (0..3000)
        .map(|i| format!("    /// Doc.\n    fn m{i}(&self, a: u32) -> u32;\n"))
        .collect();
    let spec = parse_hybrid_rust_spec(&format!(
        "#[nve_hybrid_module]\npub trait T {{\n{members}}}"
    ))
    .expect("3000 members");
    assert_eq!(spec.modules[0].methods.len(), 3000);
    let fields: String = (0..1000)
        .map(|i| format!("    /// Doc.\n    pub f{i}: Option<Vec<u8>>,\n"))
        .collect();
    let spec = parse_hybrid_rust_spec(&format!(
        "#[nve_hybrid_struct]\npub struct S {{\n{fields}}}"
    ))
    .expect("1000 fields");
    assert_eq!(spec.structs[0].fields.len(), 1000);
    let variants: String = (0..1000).map(|i| format!("    V{i} = {i},\n")).collect();
    let spec = parse_hybrid_rust_spec(&format!("#[nve_hybrid_enum]\npub enum E {{\n{variants}}}"))
        .expect("1000 variants");
    assert_eq!(spec.enums[0].variants.len(), 1000);
    let header = "//! Header line.\n".repeat(2000);
    parse_hybrid_rust_spec(&format!(
        "{header}#[nve_hybrid_const]\npub const X: u32 = 1;"
    ))
    .expect("2000 //! lines");
}

/// `///` Markdown survives verbatim; only `/** */` line decoration is stripped.
#[test]
fn line_docs_keep_markdown_and_block_docs_keep_indentation() {
    let spec = parse_hybrid_rust_spec(
        "/// * bullet\n/// **bold** and *em*\n#[nve_hybrid_const]\npub const X: u32 = 1;\n\
         /**\n * Title.\n *\n *     code();\n * * nested bullet\n */\n#[nve_hybrid_const]\npub const Y: u32 = 2;",
    )
    .expect("docs");
    assert_eq!(
        spec.consts[0].doc.as_deref(),
        Some("* bullet\n**bold** and *em*")
    );
    assert_eq!(
        spec.consts[1].doc.as_deref(),
        Some("Title.\n\n    code();\n* nested bullet")
    );
    // A block doc that starts with an indented code line keeps that indentation.
    let spec = parse_hybrid_rust_spec(
        "/**\n *     indented();\n */\n#[nve_hybrid_const]\npub const Z: u32 = 3;",
    )
    .expect("indented block doc");
    assert_eq!(spec.consts[0].doc.as_deref(), Some("    indented();"));
}

/// `usize` defaults fit `u32` (32-bit hosts); negative defaults never fit unsigned props.
#[test]
fn unsigned_and_usize_prop_defaults_are_range_checked() {
    let prop = |ty: &str, default: &str| {
        parse_hybrid_rust_spec(&format!(
            "#[nve_hybrid_view]\npub trait V {{ #[prop(default = {default})] fn x(&self) -> {ty}; }}"
        ))
    };
    for (ty, default) in [
        ("usize", "4294967295"),
        ("usize", "0"),
        ("u8", "255"),
        ("u16", "65535"),
    ] {
        prop(ty, default).unwrap_or_else(|e| panic!("{ty} = {default}: {e}"));
    }
    for (ty, default) in [
        ("usize", "4294967296"),
        ("usize", "-1"),
        ("u8", "-1"),
        ("u16", "-1"),
        ("u8", "256"),
    ] {
        assert!(
            matches!(prop(ty, default), Err(CodegenError::Validation(_))),
            "{ty} = {default} must be rejected"
        );
    }
}

/// `Option<Option<T>>` / `Option<Value>`: `None` and `Some(null)` would both encode as `Null`.
#[test]
fn ambiguous_nested_options_are_rejected() {
    for ty in [
        "Option<Option<u32>>",
        "Option<NveValue>",
        "Vec<Option<Option<String>>>",
    ] {
        let source = format!("#[nve_hybrid_module]\npub trait T {{ fn f(&self, a: {ty}); }}");
        match parse_hybrid_rust_spec(&source) {
            Err(CodegenError::Validation(message)) => {
                assert!(message.contains("ambiguous on the wire"), "{ty}: {message}");
            }
            other => panic!("{ty}: expected a validation error, got {other:?}"),
        }
    }
    parse_hybrid_rust_spec(
        "#[nve_hybrid_module]\npub trait T { fn f(&self, a: Option<Vec<Option<u32>>>); }",
    )
    .expect("an option inside a sequence is unambiguous");
}
