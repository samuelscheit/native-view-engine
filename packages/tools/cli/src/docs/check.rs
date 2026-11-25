//! `cargo nve docs check` — validates frontmatter, internal links, `#anchors`, `_meta.toml`
//! ordering entries, duplicate routes, and `<LiveDemo>` IDs against `PlaygroundRegistry`.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use nve_docs::{PlaygroundRegistry, default_component_registry};
use nve_mdx::{
    Block, CompileOptions, CompiledPage, CompiledSite, Diagnostic, DiagnosticSeverity,
    DirectiveBlock, Inline, PropValue, SiteSource, compile_site_with_validation, parse_meta_toml,
};

/// Unified diagnostic emitted by `nve docs check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocsDiagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for DocsDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file, self.line, self.column, self.message
        )
    }
}

/// Complete validation summary returned by [`validate_content_dir`].
pub(crate) struct CheckResult {
    pub site: CompiledSite,
    pub diagnostics: Vec<DocsDiagnostic>,
    pub page_count: usize,
    pub meta_count: usize,
    pub demo_count: usize,
    pub search_bytes: usize,
}

/// Load, compile, and strictly validate a documentation content directory.
pub(crate) fn validate_content_dir(content_dir: &Path) -> Result<CheckResult, String> {
    if !content_dir.is_dir() {
        return Err(format!(
            "content directory does not exist: {}",
            content_dir.display()
        ));
    }

    let source = SiteSource::from_dir(content_dir)
        .map_err(|e| format!("failed to read {}: {e}", content_dir.display()))?;
    let meta_count = source
        .files
        .keys()
        .filter(|k| k.ends_with("_meta.toml"))
        .count();

    let registry = default_component_registry();
    let (site, mdx_diags) =
        compile_site_with_validation(&source, &CompileOptions::default(), &registry);

    let mut diagnostics: Vec<DocsDiagnostic> = mdx_diags
        .into_iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .map(from_mdx_diag)
        .collect();

    validate_meta_manifests(&source, &mut diagnostics);

    let playgrounds = nve_docs_site::playground_registry();
    let mut demo_ids_seen = BTreeSet::new();
    for page in &site.pages {
        validate_page_demos(page, &playgrounds, &mut demo_ids_seen, &mut diagnostics);
    }

    diagnostics.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.column.cmp(&b.column))
    });

    let page_count = site.pages.len();
    let search_bytes = site.search_index.encode().len();

    Ok(CheckResult {
        site,
        diagnostics,
        page_count,
        meta_count,
        demo_count: demo_ids_seen.len(),
        search_bytes,
    })
}

/// CLI entrypoint for `cargo nve docs check`.
pub(crate) fn run_check(content_dir: &Path) -> Result<(), String> {
    let report = validate_content_dir(content_dir)?;
    if !report.diagnostics.is_empty() {
        for diag in &report.diagnostics {
            eprintln!("  [error] {diag}");
        }
        return Err(format!(
            "docs check failed with {} error(s) across {} pages",
            report.diagnostics.len(),
            report.page_count
        ));
    }

    println!(
        "✓ docs check passed: {} pages, {} _meta.toml manifests, {} live demos, search.nvs1 ({:.1} KiB)",
        report.page_count,
        report.meta_count,
        report.demo_count,
        report.search_bytes as f64 / 1024.0
    );
    Ok(())
}

fn from_mdx_diag(d: Diagnostic) -> DocsDiagnostic {
    DocsDiagnostic {
        file: d.file,
        line: d.line,
        column: d.column,
        message: d.message,
    }
}

fn validate_meta_manifests(source: &SiteSource, out: &mut Vec<DocsDiagnostic>) {
    for (rel_path, content) in &source.files {
        if !rel_path.ends_with("_meta.toml") {
            continue;
        }
        let dir_prefix = rel_path
            .strip_suffix("_meta.toml")
            .unwrap_or("")
            .trim_end_matches('/');

        let mut direct_stems = BTreeSet::new();
        let mut child_dirs = BTreeSet::new();
        let search_prefix = if dir_prefix.is_empty() {
            String::new()
        } else {
            format!("{dir_prefix}/")
        };

        for file_key in source.files.keys() {
            let Some(rest) = file_key.strip_prefix(&search_prefix) else {
                continue;
            };
            if let Some((sub, _)) = rest.split_once('/') {
                child_dirs.insert(sub.to_string());
            } else if let Some(stem) = rest.strip_suffix(".mdx").or_else(|| rest.strip_suffix(".md"))
            {
                direct_stems.insert(stem.to_string());
            }
        }

        for entry in parse_meta_toml(content) {
            if entry.separator || entry.href.is_some() || entry.hidden {
                continue;
            }
            if !direct_stems.contains(&entry.key) && !child_dirs.contains(&entry.key) {
                out.push(DocsDiagnostic {
                    file: rel_path.clone(),
                    line: 1,
                    column: 1,
                    message: format!(
                        "_meta.toml references missing page or directory '{}'",
                        entry.key
                    ),
                });
            }
        }
    }
}

fn validate_page_demos(
    page: &CompiledPage,
    playgrounds: &PlaygroundRegistry,
    seen_ids: &mut BTreeSet<String>,
    out: &mut Vec<DocsDiagnostic>,
) {
    walk_blocks_for_demos(&page.blocks, &page.source_path, playgrounds, seen_ids, out);
}

fn walk_blocks_for_demos(
    blocks: &[Block],
    file: &str,
    playgrounds: &PlaygroundRegistry,
    seen_ids: &mut BTreeSet<String>,
    out: &mut Vec<DocsDiagnostic>,
) {
    for block in blocks {
        match block {
            Block::Directive(DirectiveBlock::Demo {
                id, line, blocks, ..
            }) => {
                check_demo_id(id, file, *line, 1, playgrounds, seen_ids, out);
                walk_blocks_for_demos(blocks, file, playgrounds, seen_ids, out);
            }
            Block::Directive(DirectiveBlock::Callout { blocks, .. })
            | Block::Directive(DirectiveBlock::Custom { blocks, .. })
            | Block::Blockquote { blocks, .. } => {
                walk_blocks_for_demos(blocks, file, playgrounds, seen_ids, out);
            }
            Block::Directive(DirectiveBlock::Tabs { items, .. }) => {
                for item in items {
                    walk_blocks_for_demos(&item.blocks, file, playgrounds, seen_ids, out);
                }
            }
            Block::Directive(DirectiveBlock::Steps { items, .. }) => {
                for item in items {
                    walk_blocks_for_demos(&item.blocks, file, playgrounds, seen_ids, out);
                }
            }
            Block::Directive(DirectiveBlock::Cards { items, .. }) => {
                for item in items {
                    walk_blocks_for_demos(&item.blocks, file, playgrounds, seen_ids, out);
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    walk_blocks_for_demos(&item.blocks, file, playgrounds, seen_ids, out);
                }
            }
            Block::Component(comp) => {
                if (comp.name == "LiveDemo" || comp.name == "Demo")
                    && let Some(PropValue::String(id)) = comp.prop("id")
                {
                    check_demo_id(
                        id,
                        file,
                        comp.pos.line,
                        comp.pos.column,
                        playgrounds,
                        seen_ids,
                        out,
                    );
                }
                walk_blocks_for_demos(&comp.children, file, playgrounds, seen_ids, out);
            }
            Block::Heading { content, .. } | Block::Paragraph { content, .. } => {
                walk_inlines_for_demos(content, file, playgrounds, seen_ids, out);
            }
            Block::Table { headers, rows, .. } => {
                for h in headers {
                    walk_inlines_for_demos(h, file, playgrounds, seen_ids, out);
                }
                for row in rows {
                    for cell in row {
                        walk_inlines_for_demos(cell, file, playgrounds, seen_ids, out);
                    }
                }
            }
            Block::CodeBlock(_) | Block::ThematicBreak { .. } => {}
        }
    }
}

fn walk_inlines_for_demos(
    inlines: &[Inline],
    file: &str,
    playgrounds: &PlaygroundRegistry,
    seen_ids: &mut BTreeSet<String>,
    out: &mut Vec<DocsDiagnostic>,
) {
    for inline in inlines {
        match inline {
            Inline::InlineComponent(comp) => {
                if (comp.name == "LiveDemo" || comp.name == "Demo")
                    && let Some(PropValue::String(id)) = comp.prop("id")
                {
                    check_demo_id(
                        id,
                        file,
                        comp.pos.line,
                        comp.pos.column,
                        playgrounds,
                        seen_ids,
                        out,
                    );
                }
            }
            Inline::Strong(c) | Inline::Emphasis(c) | Inline::Strikethrough(c) => {
                walk_inlines_for_demos(c, file, playgrounds, seen_ids, out);
            }
            Inline::Link { text, .. } => {
                walk_inlines_for_demos(text, file, playgrounds, seen_ids, out);
            }
            _ => {}
        }
    }
}

fn check_demo_id(
    id: &str,
    file: &str,
    line: usize,
    column: usize,
    playgrounds: &PlaygroundRegistry,
    seen_ids: &mut BTreeSet<String>,
    out: &mut Vec<DocsDiagnostic>,
) {
    seen_ids.insert(id.to_string());
    if !playgrounds.contains(id) {
        out.push(DocsDiagnostic {
            file: file.to_string(),
            line,
            column,
            message: format!(
                "Unknown LiveDemo id '{id}' (registered demos: {})",
                playgrounds.ids().join(", ")
            ),
        });
    }
}
