//! Comprehensive tests for `nve-mdx`: Markdown/GFM/MDX parsing, frontmatter, `_meta.toml`,
//! directives, compile-time validation, syntax highlighting, `NVS2` search, codegen, and fuzzing.

use std::time::Instant;

use nve_mdx::{
    Block, CalloutKind, CompileOptions, ComponentRegistry, ComponentSchema, DiagnosticKind,
    DirectiveBlock, HydrateStrategy, Inline, NVS2_MAGIC, NVS2_VERSION, NavNode, PropSchema,
    PropType, PropValue, SearchIndex, SearchIndexError, SiteCodegenOptions, SiteSource, TableAlign,
    TokenKind, compile_document, compile_site, compile_site_with_validation,
    generate_rust_module, highlight_code, parse_frontmatter, parse_meta_toml,
};

#[test]
fn frontmatter_parses_yaml_and_toml_subsets_with_exact_body_line() {
    let src = "\u{feff}---\ntitle: \"NVE Architecture\"\ndescription: Zero-copy binary IR\norder = 10\nbadge: \"Stable\"\nicon: cpu\ntoc: false\ndraft = false\ncanonical: https://nve.dev/arch\ntags: [rust, fabric, skia]\nauthor = \"Core Team\"\nplatforms:\n  - macOS\n  - iOS\n  - Android\n---\n\n# Heading on line 18\n";
    let (fm, body, body_line) = parse_frontmatter(src);
    assert_eq!(fm.title.as_deref(), Some("NVE Architecture"));
    assert_eq!(fm.description.as_deref(), Some("Zero-copy binary IR"));
    assert_eq!(fm.order, Some(10));
    assert_eq!(fm.badge.as_deref(), Some("Stable"));
    assert_eq!(fm.icon.as_deref(), Some("cpu"));
    assert!(!fm.toc);
    assert!(!fm.draft);
    assert_eq!(fm.canonical.as_deref(), Some("https://nve.dev/arch"));
    assert_eq!(fm.tags, vec!["rust", "fabric", "skia"]);
    assert_eq!(fm.meta.get("author").map(String::as_str), Some("Core Team"));
    assert_eq!(
        fm.arrays.get("platforms"),
        Some(&vec!["macOS".into(), "iOS".into(), "Android".into()])
    );
    assert_eq!(body_line, 17);
    assert!(body.contains("# Heading on line 18"));
}

#[test]
fn full_markdown_gfm_directives_mdx_and_rsx_live() {
    let doc = r#"---
title: CommandBuffer Guide
description: Batched binary IR
order: 2
badge: v2
custom_owner: Runtime Team
---

# CommandBuffer Guide

Welcome to **{frontmatter.title}** by _{frontmatter.custom_owner}_ with ~~legacy JSON~~ and ``code `tick` ``!
See [Routing](/guides/routing "Router") and ![Diagram](/img/ir.png) and <https://nve.dev> or https://example.org/docs.

## Heading Duplicate

## Heading Duplicate

### Heading Duplicate

- [x] Zero-copy binary IR
- [ ] Pending item
  1. Nested ordered first
  2. Nested ordered second

> Quoted **note** with inline <Badge label="Inline" /> component.

---

| Backend | Latency | Alignment |
| :--- | :---: | ---: |
| Skia | `0.4ms` | Right |
| Fabric \| Yoga | `0.6ms` | Right |

```rust title="src/frame.rs" highlight="1,3-4" showLineNumbers
fn submit_frame(buf: &[u8]) -> u32 {
    let len = buf.len() as u32;
    // Highlighted lines 3 and 4
    len
}
```

~~~rsx live title="Interactive Counter"
rsx! {
    Button { label: "Increment" }
}
~~~

:::callout{kind="warning" title="ABI Invariant"}
Never cross FFI per-prop!
:::

:::tabs
::tab{label="Rust"}
Use `nve-engine`.
::tab{label="Swift"}
Use `NveSwiftUIKit`.
:::

:::steps
::step{title="Install"}
Run `cargo build`.
::step{title="Run"}
Run `cargo nve bench`.
:::

:::cards
::card{title="Router" href="/guides/routing" description="Typed routes"}
::card{title="UI Kit" href="/guides/ui" description="Primitives"}
:::

:::demo{id="counter-demo" hydrate="interaction" height="320"}
Live counter preview.
:::

<StatCard label={frontmatter.title} count={42} ratio={3.5} enabled={true} featured />

<FeatureBox title="Nested MDX" level={2}>
Inside **paired** tag!

<Badge label="Child" />
</FeatureBox>
"#;

    let page = compile_document("/guides/ir", "guides/ir.mdx", doc);
    assert_eq!(page.title, "CommandBuffer Guide");
    assert_eq!(page.description, "Batched binary IR");

    // Duplicate heading slug disambiguation: `heading-duplicate`, `-1`, `-2`.
    let toc_ids: Vec<&str> = page.toc.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(
        toc_ids,
        vec![
            "commandbuffer-guide",
            "heading-duplicate",
            "heading-duplicate-1",
            "heading-duplicate-2"
        ]
    );

    // Check frontmatter interpolation inside paragraph.
    assert!(page.plain_text.contains("CommandBuffer Guide"));
    assert!(page.plain_text.contains("Runtime Team"));

    // Check live rsx block extraction.
    assert_eq!(page.live_blocks.len(), 1);
    assert_eq!(page.live_blocks[0].lang, "rsx");
    assert_eq!(
        page.live_blocks[0].title.as_deref(),
        Some("Interactive Counter")
    );
    assert!(page.live_blocks[0].code.contains("Button"));

    // Check Rust code block meta & highlight lines.
    let rust_cb = page
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::CodeBlock(cb) if cb.lang.as_deref() == Some("rust") => Some(cb),
            _ => None,
        })
        .expect("rust code block");
    assert_eq!(rust_cb.title.as_deref(), Some("src/frame.rs"));
    assert_eq!(rust_cb.highlight_lines, vec![1, 3, 4]);
    assert!(rust_cb.show_line_numbers);
    assert!(!rust_cb.live);
    assert!(rust_cb.highlighted[0].highlighted);
    assert!(!rust_cb.highlighted[1].highlighted);
    assert!(rust_cb.highlighted[2].highlighted);
    assert!(rust_cb.highlighted[3].highlighted);

    // Check GFM table alignments & escaped pipe cell.
    let table = page
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Table {
                alignments,
                headers,
                rows,
                ..
            } => Some((alignments, headers, rows)),
            _ => None,
        })
        .expect("table");
    assert_eq!(
        table.0,
        &vec![TableAlign::Left, TableAlign::Center, TableAlign::Right]
    );
    assert_eq!(table.1.len(), 3);
    assert_eq!(table.2.len(), 2);

    // Check nested checklist + ordered sublist.
    let list = page
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::List { items, .. } => Some(items),
            _ => None,
        })
        .expect("list");
    assert_eq!(list[0].checked, Some(true));
    assert_eq!(list[1].checked, Some(false));
    assert!(list[1].blocks.iter().any(|b| matches!(b, Block::List { ordered: true, .. })));

    // Check directives (callout, tabs, steps, cards, demo).
    let mut seen_callout = false;
    let mut seen_tabs = false;
    let mut seen_steps = false;
    let mut seen_cards = false;
    let mut seen_demo = false;
    for b in &page.blocks {
        if let Block::Directive(d) = b {
            match d {
                DirectiveBlock::Callout { kind, title, .. } => {
                    assert_eq!(*kind, CalloutKind::Warning);
                    assert_eq!(title.as_deref(), Some("ABI Invariant"));
                    seen_callout = true;
                }
                DirectiveBlock::Tabs { items, .. } => {
                    assert_eq!(items.len(), 2);
                    assert_eq!(items[0].label, "Rust");
                    assert_eq!(items[1].label, "Swift");
                    seen_tabs = true;
                }
                DirectiveBlock::Steps { items, .. } => {
                    assert_eq!(items.len(), 2);
                    assert_eq!(items[0].title, "Install");
                    assert_eq!(items[1].title, "Run");
                    seen_steps = true;
                }
                DirectiveBlock::Cards { items, .. } => {
                    assert_eq!(items.len(), 2);
                    assert_eq!(items[0].href.as_deref(), Some("/guides/routing"));
                    seen_cards = true;
                }
                DirectiveBlock::Demo {
                    id,
                    hydrate,
                    height,
                    ..
                } => {
                    assert_eq!(id, "counter-demo");
                    assert_eq!(*hydrate, HydrateStrategy::Interaction);
                    assert_eq!(*height, Some(320));
                    seen_demo = true;
                }
                DirectiveBlock::Custom { .. } => {}
            }
        }
    }
    assert!(seen_callout && seen_tabs && seen_steps && seen_cards && seen_demo);

    // Check self-closing & paired MDX components with typed props + interpolation.
    let stat = page
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Component(c) if c.name == "StatCard" => Some(c),
            _ => None,
        })
        .expect("StatCard");
    assert!(stat.self_closing);
    assert_eq!(
        stat.prop("label"),
        Some(&PropValue::String("CommandBuffer Guide".into()))
    );
    assert_eq!(stat.prop("count"), Some(&PropValue::Int(42)));
    assert_eq!(stat.prop("ratio"), Some(&PropValue::Float(3.5)));
    assert_eq!(stat.prop("enabled"), Some(&PropValue::Bool(true)));
    assert_eq!(stat.prop("featured"), Some(&PropValue::Bool(true)));

    let feature = page
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Component(c) if c.name == "FeatureBox" => Some(c),
            _ => None,
        })
        .expect("FeatureBox");
    assert!(!feature.self_closing);
    assert_eq!(feature.prop("level"), Some(&PropValue::Int(2)));
    assert!(feature
        .children
        .iter()
        .any(|b| matches!(b, Block::Component(c) if c.name == "Badge")));

    // Check inline autolinks & inline component in blockquote.
    let has_autolink = page.blocks.iter().any(|b| match b {
        Block::Paragraph { content, .. } => content
            .iter()
            .any(|i| matches!(i, Inline::Autolink { url, .. } if url == "https://nve.dev")),
        _ => false,
    });
    assert!(has_autolink);
}

#[test]
fn meta_toml_nav_tree_prev_next_and_from_dir() {
    let meta = r#"
index = "Welcome"
"---" = "Core Guides"
guides = { title = "Guides", badge = "New", collapsed = true, icon = "book" }
github = { title = "GitHub", href = "https://github.com/nve/nve", badge = "OSS" }
"#;
    let parsed = parse_meta_toml(meta);
    assert_eq!(parsed.len(), 4);
    assert!(parsed[1].separator);
    assert_eq!(parsed[1].title.as_deref(), Some("Core Guides"));
    assert!(parsed[2].collapsed);
    assert_eq!(parsed[3].href.as_deref(), Some("https://github.com/nve/nve"));

    let mut src = SiteSource::new();
    src.insert("_meta.toml", meta);
    src.insert("index.mdx", "---\ntitle: Home\n---\n# Home\n");
    src.insert("guides/_meta.toml", "start = \"Quickstart\"\nadvanced = \"Advanced\"\n");
    src.insert("guides/index.mdx", "---\ntitle: Guides Home\n---\n# Guides\n");
    src.insert("guides/start.mdx", "---\ntitle: Quickstart\n---\n# Start\n");
    src.insert("guides/advanced.mdx", "---\ntitle: Advanced\n---\n# Advanced\n");
    src.insert("draft.mdx", "---\ntitle: Secret\ndraft: true\n---\n# Draft\n");

    let site = compile_site(&src, &CompileOptions::default());
    assert_eq!(site.pages.len(), 4); // draft excluded by default
    assert!(matches!(site.nav.items[0], NavNode::Page { ref title, .. } if title == "Welcome"));
    assert!(matches!(site.nav.items[1], NavNode::Separator { .. }));
    assert!(matches!(
        site.nav.items[2],
        NavNode::Folder {
            collapsed: true,
            ref children,
            ..
        } if children.len() == 2
    ));
    assert!(matches!(site.nav.items[3], NavNode::ExternalLink { .. }));

    // Flattened PrevNext trail: `/` -> `/guides` -> `/guides/start` -> `/guides/advanced`.
    let pn_root = site.prev_next.get("/").unwrap();
    assert!(pn_root.prev.is_none());
    assert_eq!(pn_root.next.as_ref().unwrap().route, "/guides");
    let pn_start = site.prev_next.get("/guides/start").unwrap();
    assert_eq!(pn_start.prev.as_ref().unwrap().route, "/guides");
    assert_eq!(pn_start.next.as_ref().unwrap().route, "/guides/advanced");

    // Verify filesystem SiteSource::from_dir on a temporary directory.
    let tmp = std::env::temp_dir().join(format!("nve-mdx-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("sub")).unwrap();
    std::fs::write(tmp.join("_meta.toml"), "index = \"Root\"\n").unwrap();
    std::fs::write(tmp.join("index.md"), "# Root\n").unwrap();
    std::fs::write(tmp.join("sub/page.mdx"), "# Sub Page\n").unwrap();
    let disk_src = SiteSource::from_dir(&tmp).unwrap();
    assert_eq!(disk_src.files.len(), 3);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn validator_catches_component_schema_and_link_errors_with_exact_lines() {
    let mut registry = ComponentRegistry::new();
    registry.register(
        ComponentSchema::new("Metric")
            .with_prop(PropSchema::required("label", PropType::String))
            .with_prop(PropSchema::required("value", PropType::Int))
            .allow_children(false),
    );

    let mut src = SiteSource::new();
    src.insert(
        "index.mdx",
        "---\ntitle: Home\n---\n# Intro\n\n<UnknownWidget foo=\"bar\" />\n<Metric label=\"FPS\" value=\"not-an-int\" extra={true} />\n<Metric value={60} />\n\nSee [Missing](/does/not/exist) and [BadAnchor](#no-such-heading) and [CrossBad](/other#missing) and [CrossOk](/other#valid-anchor).\n",
    );
    src.insert("other.mdx", "---\ntitle: Other\n---\n# Valid Anchor\n");
    src.insert("untitled.md", "No title or H1 here.\n");

    let (_site, diags) =
        compile_site_with_validation(&src, &CompileOptions::default(), &registry);

    let find_diag = |kind: DiagnosticKind| diags.iter().find(|d| d.kind == kind);
    let d_unknown = find_diag(DiagnosticKind::UnknownComponent).expect("UnknownComponent");
    assert_eq!(d_unknown.file, "index.mdx");
    assert_eq!(d_unknown.line, 6);

    let d_type = find_diag(DiagnosticKind::PropTypeMismatch).expect("PropTypeMismatch");
    assert_eq!(d_type.line, 7);

    let d_extra = find_diag(DiagnosticKind::UnknownProp).expect("UnknownProp");
    assert_eq!(d_extra.line, 7);

    let d_missing = find_diag(DiagnosticKind::MissingRequiredProp).expect("MissingRequiredProp");
    assert_eq!(d_missing.line, 8);

    let d_broken_route = find_diag(DiagnosticKind::BrokenInternalLink).expect("BrokenInternalLink");
    assert_eq!(d_broken_route.line, 10);

    let broken_anchors: Vec<_> = diags
        .iter()
        .filter(|d| d.kind == DiagnosticKind::BrokenAnchor)
        .collect();
    assert_eq!(broken_anchors.len(), 2);
    assert!(broken_anchors.iter().all(|d| d.line == 10));

    let d_empty_title = find_diag(DiagnosticKind::EmptyTitle).expect("EmptyTitle");
    assert_eq!(d_empty_title.file, "untitled.md");
}

#[test]
fn syntax_highlighter_covers_all_required_languages() {
    let cases: &[(&str, &str, TokenKind)] = &[
        ("rust", "#[inline]\npub fn run() { println!(\"hi\", 42); /* c */ }", TokenKind::Macro),
        ("cpp", "#include <vector>\n@interface View\nint main() { return 0x10; }", TokenKind::Attribute),
        ("swift", "@MainActor func draw() -> CGFloat { #available(iOS 17) }", TokenKind::Macro),
        ("kotlin", "@Composable fun Counter(n: Int) = \"val\"", TokenKind::Attribute),
        ("java", "public class App { int x = 7; }", TokenKind::Keyword),
        ("dart", "@override Widget build(BuildContext ctx) => 'ok';", TokenKind::TypeName),
        ("tsx", "export const App = (): Promise<void> => `hi`;", TokenKind::StringLit),
        ("bash", "# comment\nexport FOO=\"bar\"\necho $FOO", TokenKind::Macro),
        ("toml", "[package]\nname = \"nve-mdx\"\nedition = 2024 # ok", TokenKind::Attribute),
        ("json", "{\"count\": 12, \"ok\": true}", TokenKind::Number),
        ("cmake", "add_library(nve STATIC ${SRCS})", TokenKind::Function),
        ("diff", "@@ -1,2 +1,2 @@\n-old\n+new", TokenKind::DiffAdd),
    ];

    for &(lang, code, expected_kind) in cases {
        let lines = highlight_code(Some(lang), code, &[1]);
        assert!(!lines.is_empty(), "empty highlight for {lang}");
        assert!(lines[0].highlighted, "line 1 should be highlighted for {lang}");
        let has_kind = lines
            .iter()
            .flat_map(|l| l.tokens.iter())
            .any(|t| t.kind == expected_kind);
        assert!(has_kind, "expected {expected_kind:?} token in {lang} output");
        // Reconstructed text must match original code verbatim.
        let rebuilt: String = lines
            .iter()
            .map(|l| l.tokens.iter().map(|t| t.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rebuilt, code, "lossless highlight text mismatch for {lang}");
    }

    // Also verify DiffDel explicitly.
    let diff_lines = highlight_code(Some("diff"), "-removed\n+added", &[]);
    assert_eq!(diff_lines[0].tokens[0].kind, TokenKind::DiffDel);
    assert_eq!(diff_lines[1].tokens[0].kind, TokenKind::DiffAdd);
}

#[test]
fn nvs2_search_roundtrip_ranking_trigrams_speed_and_fuzzing() {
    let mut src = SiteSource::new();
    src.insert(
        "title-hit.mdx",
        "---\ntitle: QuantumLayout Engine\n---\nGeneral overview of layout.\n",
    );
    src.insert(
        "heading-hit.mdx",
        "---\ntitle: Architecture\n---\n## QuantumLayout Pipeline\nDetails here.\n",
    );
    src.insert(
        "code-hit.mdx",
        "---\ntitle: Low Level API\n---\nUse `QuantumLayout` directly in Rust.\n",
    );
    src.insert(
        "body-hit.mdx",
        "---\ntitle: Notes\n---\nMentions quantumlayout in plain body prose.\n",
    );

    let site = compile_site(&src, &CompileOptions::default());
    let encoded = site.search_index.encode();
    assert_eq!(&encoded[..4], NVS2_MAGIC);
    assert_eq!(u16::from_le_bytes([encoded[4], encoded[5]]), NVS2_VERSION);

    let decoded = SearchIndex::decode(&encoded).expect("valid NVS2 decode");
    assert_eq!(decoded, site.search_index);

    // Ranking accuracy: title (4.0) > heading (2.5) > code (2.0) > body (1.0).
    let t0 = Instant::now();
    let hits = decoded.query("QuantumLayout", 10);
    let elapsed = t0.elapsed();
    assert!(
        elapsed.as_millis() < 50,
        "query took unexpectedly long: {elapsed:?}"
    );
    let routes: Vec<&str> = hits.iter().map(|h| h.route.as_str()).collect();
    assert_eq!(routes[0], "/title-hit");
    assert_eq!(routes[1], "/heading-hit");
    assert_eq!(routes[2], "/code-hit");
    assert_eq!(routes[3], "/body-hit");
    assert!(hits[0].score > hits[1].score);
    assert!(hits[1].score > hits[2].score);
    assert!(hits[2].score > hits[3].score);

    // Prefix & trigram substring matching (`uantum` is middle of `quantumlayout`).
    assert!(!decoded.query("quant", 5).is_empty());
    assert!(!decoded.query("uantum", 5).is_empty());

    // Adversarial decoding: every truncation length must return Err without panicking.
    for cut in 0..encoded.len() {
        assert!(
            SearchIndex::decode(&encoded[..cut]).is_err(),
            "truncation at {cut} unexpectedly succeeded"
        );
    }

    // Bit-flipped payload -> ChecksumMismatch.
    let mut flipped = encoded.clone();
    flipped[12] ^= 0x01;
    assert!(matches!(
        SearchIndex::decode(&flipped),
        Err(SearchIndexError::ChecksumMismatch { .. })
    ));

    // Forged huge `doc_count = u32::MAX` with valid recomputed XXH64 checksum -> UnexpectedEof (no OOM!).
    let mut forged = Vec::new();
    forged.extend_from_slice(NVS2_MAGIC);
    forged.extend_from_slice(&NVS2_VERSION.to_le_bytes());
    forged.extend_from_slice(&0u16.to_le_bytes());
    forged.extend_from_slice(&u32::MAX.to_le_bytes()); // doc_count
    forged.extend_from_slice(&0u32.to_le_bytes()); // token_count
    forged.extend_from_slice(&0u32.to_le_bytes()); // trigram_count
    let ck = nve_hash::xxh64(&forged, 0);
    forged.extend_from_slice(&ck.to_le_bytes());
    assert_eq!(
        SearchIndex::decode(&forged),
        Err(SearchIndexError::UnexpectedEof)
    );
}

#[test]
fn rust_codegen_is_deterministic_and_embeds_live_blocks_and_search() {
    let mut src = SiteSource::new();
    src.insert(
        "index.mdx",
        "---\ntitle: Home\n---\n# Welcome\n\n```rsx live\nrsx! { div { \"hi\" } }\n```\n",
    );
    let site = compile_site(&src, &CompileOptions::default());
    let a = generate_rust_module(&site, &SiteCodegenOptions::default());
    let b = generate_rust_module(&site, &SiteCodegenOptions::default());
    assert_eq!(a, b);
    assert!(a.contains("pub const PAGES:"));
    assert!(a.contains("pub const LIVE_BLOCKS:"));
    assert!(a.contains("pub const SEARCH_INDEX_BYTES:"));
    assert!(a.contains("pub fn find_page("));
}

#[test]
fn adversarial_parser_fuzzing_never_panics() {
    let deep_lists: String = (0..120).map(|i| format!("{}- item {i}\n", "  ".repeat(i))).collect();
    let deep_quotes: String = (0..120).map(|i| format!("{} hi\n", "> ".repeat(i + 1))).collect();
    let deep_tags: String = (0..90).map(|_| "<Box>\n").collect::<String>() + "leaf\n";
    let deep_inline = "**__~~".repeat(80) + "text" + &"~~__**".repeat(80);
    let unclosed_everything = "---\ntitle: \"unterminated\n```rust\nfn x(\n:::tabs\n::tab{label=\"unclosed\n<Broken prop={{{{{{ a=\"\\";
    let tricky_utf8 = "# 🚀 日本語 — **éàü**\n\n`\\xFF\\u{` [bad]((((()))) ![img](a \"b\\) <Tag x=\"\\n\\t\\\"\" />\n:::callout{kind=\"\"}\n| a | b |\n| --- | --- |\n| \\| | `|` |\n";

    let inputs = [
        "",
        "---",
        "---\n---",
        &deep_lists,
        &deep_quotes,
        &deep_tags,
        &deep_inline,
        unclosed_everything,
        tricky_utf8,
    ];

    for (idx, sample) in inputs.iter().enumerate() {
        let page = compile_document(&format!("/fuzz/{idx}"), &format!("fuzz/{idx}.mdx"), sample);
        let idx_bin = SearchIndex::build(&[page]).encode();
        let decoded = SearchIndex::decode(&idx_bin).expect("fuzz search roundtrip");
        let _ = decoded.query("item", 5);
    }
}
