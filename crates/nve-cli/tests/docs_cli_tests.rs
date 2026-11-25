//! Integration tests for `cargo nve docs check` and `cargo nve docs build`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nve_mdx::{NVS1_MAGIC, SearchIndex};

fn nve_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nve")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve-docs-cli-test-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

#[test]
fn docs_check_passes_on_official_docs_content() {
    let output = Command::new(nve_bin())
        .current_dir(repo_root())
        .args(["docs", "check", "--content", "docs/content"])
        .output()
        .expect("run nve docs check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "docs check failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("30 pages") && stdout.contains("5 live demos"),
        "unexpected docs check summary: {stdout}"
    );
}

#[test]
fn docs_check_fails_non_zero_on_broken_link_and_unknown_demo() {
    let dir = temp_test_dir("broken");
    fs::write(
        dir.join("_meta.toml"),
        "index = \"Home\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("index.mdx"),
        "---\ntitle: Broken Test\n---\n\nSee [Missing](/does/not/exist) and <LiveDemo id=\"unknown-demo-xyz\" />\n",
    )
    .unwrap();

    let output = Command::new(nve_bin())
        .args(["docs", "check", "--content"])
        .arg(&dir)
        .output()
        .expect("run nve docs check on broken content");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "expected non-zero exit code for broken link + unknown demo"
    );
    assert!(
        stderr.contains("Broken internal link") && stderr.contains("unknown-demo-xyz"),
        "expected both broken link and unknown LiveDemo diagnostics, got:\n{stderr}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn docs_build_skip_wasm_emits_all_30_semantic_html_pages_sitemap_and_nvs1() {
    let out_dir = temp_test_dir("build-base");
    let output = Command::new(nve_bin())
        .current_dir(repo_root())
        .args([
            "docs",
            "build",
            "--content",
            "docs/content",
            "--skip-wasm",
            "--base-path",
            "/native-view-engine",
            "--site-url",
            "https://samuelscheit.github.io/native-view-engine",
            "--out",
        ])
        .arg(&out_dir)
        .output()
        .expect("run nve docs build --skip-wasm");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "docs build failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Verify top-level static artifacts
    assert!(out_dir.join(".nojekyll").is_file());
    assert!(out_dir.join("404.html").is_file());
    assert!(out_dir.join("sitemap.xml").is_file());
    assert!(out_dir.join("search.nvs1").is_file());

    // Verify NVS1 binary search index decodes and answers queries
    let nvs1_bytes = fs::read(out_dir.join("search.nvs1")).expect("read search.nvs1");
    assert_eq!(&nvs1_bytes[..4], NVS1_MAGIC);
    let index = SearchIndex::decode(&nvs1_bytes).expect("decode NVS1 index");
    let hits = index.query("VirtualList", 5);
    assert!(!hits.is_empty(), "expected search hits for VirtualList");

    // Verify sitemap contains canonical URLs
    let sitemap = fs::read_to_string(out_dir.join("sitemap.xml")).expect("read sitemap.xml");
    assert!(sitemap.contains("https://samuelscheit.github.io/native-view-engine/guides/ui-components/"));

    // Verify all 30 HTML files exist
    let html_files = count_html_files(&out_dir);
    // 30 route index.html files + 404.html = 31 HTML files
    assert_eq!(
        html_files, 31,
        "expected 30 route index.html files + 404.html, found {html_files}"
    );

    // Inspect root and nested page semantic HTML5 + base_path links + Option 5 CSS
    let root_html = fs::read_to_string(out_dir.join("index.html")).expect("read root index.html");
    assert!(root_html.contains("<!doctype html>"));
    assert!(root_html.contains("<style data-nve-base>"));
    assert!(root_html.contains("<style data-nve-ssr data-nve-hashes=\""));
    assert!(root_html.contains("<h1"));
    assert!(root_html.contains("<nav"));
    assert!(root_html.contains("<main"));
    assert!(root_html.contains("<article"));
    assert!(
        root_html.contains("href=\"/native-view-engine/getting-started/quickstart\""),
        "expected base_path-prefixed internal hrefs"
    );

    let guide_html = fs::read_to_string(out_dir.join("guides/ui-components/index.html"))
        .expect("read guides/ui-components/index.html");
    assert!(guide_html.contains("<h1") && guide_html.contains("data-nve-island=\"ui-playground\""));

    let _ = fs::remove_dir_all(&out_dir);
}

fn count_html_files(dir: &Path) -> usize {
    let mut count = 0;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            count += count_html_files(&p);
        } else if p.extension().and_then(|e| e.to_str()) == Some("html") {
            count += 1;
        }
    }
    count
}
