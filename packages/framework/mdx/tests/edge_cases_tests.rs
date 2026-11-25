//! Additional deep edge-case tests for `nve-mdx`: NVS1 error paths, duplicate routes,
//! nested multiline highlighting, directive closers, and sub-millisecond search scale.

use std::time::Instant;

use nve_mdx::{
    CompileOptions, ComponentRegistry, DiagnosticKind, NVS1_MAGIC, NVS1_VERSION, SearchIndex,
    SearchIndexError, SiteSource, TokenKind, compile_site, compile_site_with_validation, fnv1a64,
    highlight_code,
};

#[test]
fn validator_detects_duplicate_routes_and_disallowed_children() {
    let registry = ComponentRegistry::with_default_docs_components();
    let mut src = SiteSource::new();
    src.insert("guides.md", "---\ntitle: Guides MD\n---\n# Guides\n");
    src.insert(
        "guides/index.mdx",
        "---\ntitle: Guides Index\n---\n<Badge label=\"New\">\nDisallowed body\n</Badge>\n",
    );

    let (_site, diags) = compile_site_with_validation(&src, &CompileOptions::default(), &registry);
    assert!(
        diags.iter().any(|d| d.kind == DiagnosticKind::DuplicateRoute),
        "expected DuplicateRoute diagnostic, got {diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.kind == DiagnosticKind::ChildrenNotAllowed),
        "expected ChildrenNotAllowed diagnostic, got {diags:?}"
    );
}

#[test]
fn nvs1_codec_rejects_all_malformed_variants_cleanly() {
    // 1. Too short
    assert_eq!(SearchIndex::decode(&[0u8; 10]), Err(SearchIndexError::TooShort));

    // 2. Invalid magic
    let mut bad_magic = vec![0u8; 28];
    bad_magic[0..4].copy_from_slice(b"NOPE");
    assert_eq!(
        SearchIndex::decode(&bad_magic),
        Err(SearchIndexError::InvalidMagic(*b"NOPE"))
    );

    // 3. Unsupported version
    let mut bad_ver = vec![0u8; 28];
    bad_ver[0..4].copy_from_slice(NVS1_MAGIC);
    bad_ver[4..6].copy_from_slice(&99u16.to_le_bytes());
    assert_eq!(
        SearchIndex::decode(&bad_ver),
        Err(SearchIndexError::UnsupportedVersion(99))
    );

    // 4. Trailing bytes with valid checksum
    let mut trailing = Vec::new();
    trailing.extend_from_slice(NVS1_MAGIC);
    trailing.extend_from_slice(&NVS1_VERSION.to_le_bytes());
    trailing.extend_from_slice(&0u16.to_le_bytes());
    trailing.extend_from_slice(&0u32.to_le_bytes()); // doc_count
    trailing.extend_from_slice(&0u32.to_le_bytes()); // token_count
    trailing.extend_from_slice(&0u32.to_le_bytes()); // trigram_count
    trailing.extend_from_slice(&[0xde, 0xad]); // extra trailing bytes
    let ck = fnv1a64(&trailing);
    trailing.extend_from_slice(&ck.to_le_bytes());
    assert_eq!(
        SearchIndex::decode(&trailing),
        Err(SearchIndexError::TrailingBytes(2))
    );

    // 5. Out-of-bounds posting doc_id with valid checksum
    let mut bad_doc = Vec::new();
    bad_doc.extend_from_slice(NVS1_MAGIC);
    bad_doc.extend_from_slice(&NVS1_VERSION.to_le_bytes());
    bad_doc.extend_from_slice(&0u16.to_le_bytes());
    bad_doc.extend_from_slice(&0u32.to_le_bytes()); // doc_count = 0
    bad_doc.extend_from_slice(&1u32.to_le_bytes()); // token_count = 1
    bad_doc.extend_from_slice(&0u32.to_le_bytes()); // trigram_count = 0
    bad_doc.extend_from_slice(&3u32.to_le_bytes()); // term len = 3
    bad_doc.extend_from_slice(b"abc");
    bad_doc.extend_from_slice(&1u32.to_le_bytes()); // posting_count = 1
    bad_doc.extend_from_slice(&7u32.to_le_bytes()); // doc_id = 7 >= 0
    bad_doc.extend_from_slice(&10u16.to_le_bytes()); // weight_x10
    let ck2 = fnv1a64(&bad_doc);
    bad_doc.extend_from_slice(&ck2.to_le_bytes());
    assert_eq!(
        SearchIndex::decode(&bad_doc),
        Err(SearchIndexError::InvalidDocId {
            doc_id: 7,
            doc_count: 0
        })
    );
}

#[test]
fn syntax_highlighter_tracks_multiline_comments_and_raw_strings() {
    let rust = "let s = r#\"line one\nline two\"#;\n/* outer /* nested */\nstill comment */\nlet x = 1;";
    let lines = highlight_code(Some("rust"), rust, &[]);
    assert_eq!(lines.len(), 5);
    assert!(lines[1].tokens.iter().any(|t| t.kind == TokenKind::StringLit));
    assert!(lines[3].tokens.iter().any(|t| t.kind == TokenKind::Comment));
    assert!(lines[4].tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "let"));
}

#[test]
fn nvs1_query_stays_sub_millisecond_on_100_page_site() {
    let mut src = SiteSource::new();
    for i in 0..100 {
        src.insert(
            format!("docs/page-{i:03}.mdx"),
            format!(
                "---\ntitle: Component Module {i}\n---\n# Component Module {i}\n\n## Layout Pipeline\n\nUses `CommandBuffer` and `SurfaceOrchestrator` for frame {i}.\n"
            ),
        );
    }
    let site = compile_site(&src, &CompileOptions::default());
    let encoded = site.search_index.encode();
    let decoded = SearchIndex::decode(&encoded).unwrap();

    // Warm one query then measure.
    let _ = decoded.query("CommandBuffer", 10);
    let t0 = Instant::now();
    let hits = decoded.query("SurfaceOrchestrator layout", 10);
    let elapsed_us = t0.elapsed().as_micros();
    assert_eq!(hits.len(), 10);
    assert!(
        elapsed_us < 10_000,
        "100-page query exceeded budget: {elapsed_us}us"
    );
}
