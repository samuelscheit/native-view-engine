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

    // Verify sitemap contains canonical URLs for Landing, Examples, and /docs/**
    let sitemap = fs::read_to_string(out_dir.join("sitemap.xml")).expect("read sitemap.xml");
    assert!(sitemap.contains("https://samuelscheit.github.io/native-view-engine/</loc>"));
    assert!(sitemap.contains("https://samuelscheit.github.io/native-view-engine/examples/</loc>"));
    assert!(
        sitemap.contains("https://samuelscheit.github.io/native-view-engine/examples/studio/</loc>")
    );
    assert!(
        sitemap.contains("https://samuelscheit.github.io/native-view-engine/docs/guides/ui-components/</loc>")
    );

    // Verify all 35 HTML files exist:
    // 1 Landing (/) + 1 Examples Gallery (/examples) + 2 Live Example Wrappers (/examples/{studio,counter})
    // + 30 Docs pages (/docs/**) + 404.html = 35 HTML files
    let html_files = count_html_files(&out_dir);
    assert_eq!(
        html_files, 35,
        "expected 34 route index.html files + 404.html, found {html_files}"
    );

    // Inspect Landing root index.html (must NOT download Studio WASM)
    let root_html = fs::read_to_string(out_dir.join("index.html")).expect("read root index.html");
    assert!(root_html.contains("<!doctype html>"));
    assert!(root_html.contains("<style data-nve-base>"));
    assert!(root_html.contains("<style data-nve-ssr data-nve-hashes=\""));
    assert!(root_html.contains("<h1"));
    assert!(root_html.contains("Native-speed UI in Rust"));
    assert!(!root_html.contains("nve_web_shell.js"));
    assert!(
        root_html.contains("href=\"/native-view-engine/docs/getting-started/quickstart\""),
        "expected base_path + /docs prefixed quickstart CTA href"
    );
    assert!(
        root_html.contains("href=\"/native-view-engine/examples/studio/\""),
        "expected base_path prefixed studio launch CTA href"
    );

    // Inspect /examples/ gallery and /examples/{studio,counter}/ fallback portal wrappers
    let examples_html =
        fs::read_to_string(out_dir.join("examples/index.html")).expect("read examples/index.html");
    assert!(examples_html.contains("Interactive Examples &amp; Live Playgrounds") || examples_html.contains("Interactive Examples"));
    let studio_html = fs::read_to_string(out_dir.join("examples/studio/index.html"))
        .expect("read examples/studio/index.html");
    assert!(studio_html.contains("Back to Examples") && studio_html.contains("data-nve-example-fallback=\"studio\""));
    let counter_html = fs::read_to_string(out_dir.join("examples/counter/index.html"))
        .expect("read examples/counter/index.html");
    assert!(counter_html.contains("Back to Examples") && counter_html.contains("data-nve-example-fallback=\"counter\""));

    // Inspect /docs/ and nested /docs/guides/ui-components/ semantic HTML5
    let docs_root_html =
        fs::read_to_string(out_dir.join("docs/index.html")).expect("read docs/index.html");
    assert!(docs_root_html.contains("<nav") && docs_root_html.contains("<main") && docs_root_html.contains("<article"));
    assert!(
        docs_root_html.contains("href=\"/native-view-engine/docs/getting-started/quickstart\""),
        "expected /docs prefixed internal links in docs index"
    );

    let guide_html = fs::read_to_string(out_dir.join("docs/guides/ui-components/index.html"))
        .expect("read docs/guides/ui-components/index.html");
    assert!(guide_html.contains("<h1") && guide_html.contains("data-nve-island=\"ui-playground\""));

    let _ = fs::remove_dir_all(&out_dir);
}

#[test]
fn docs_build_discovers_both_unified_and_per_app_web_shell_bundles() {
    // 1. Unified `build.sh` layout: `index.html` + `pkg/<bundle>/manifest.json` naming the entry
    //    files in a content-hashed `pkg/<bundle>/<hash>/` directory. The dev bundle is not shipped.
    let unified_src = temp_test_dir("ws-unified-src");
    fs::write(
        unified_src.join("index.html"),
        "<html><head></head><body><header class=\"topbar\"></header><main id=\"app\"></main></body></html>",
    )
    .unwrap();
    for (bundle, hash) in [("studio", "0123456789abcdef"), ("counter", "fedcba9876543210"), ("dev", "00112233aabbccdd")] {
        let dir = unified_src.join("pkg").join(bundle).join(hash);
        fs::create_dir_all(dir.join("snippets/nve-web-shell-1/")).unwrap();
        fs::write(dir.join("nve_web_shell.js"), "export default function(){}").unwrap();
        fs::write(dir.join("nve_web_shell_bg.wasm"), b"\0asm").unwrap();
        fs::write(dir.join("snippets/nve-web-shell-1/inline0.js"), "export const x = 1;").unwrap();
        fs::write(
            unified_src.join("pkg").join(bundle).join("manifest.json"),
            format!("{{\"js\":\"{hash}/nve_web_shell.js\",\"wasm\":\"{hash}/nve_web_shell_bg.wasm\"}}"),
        )
        .unwrap();
    }

    let out_unified = temp_test_dir("ws-unified-out");
    let status = Command::new(nve_bin())
        .current_dir(repo_root())
        .args([
            "docs",
            "build",
            "--content",
            "docs/content",
            "--skip-wasm",
            "--web-shell-dist",
        ])
        .arg(&unified_src)
        .arg("--out")
        .arg(&out_unified)
        .status()
        .expect("run docs build with unified web-shell");
    assert!(status.success());
    let runtime_pkg = out_unified.join("examples/_runtime/pkg");
    for file in [
        "studio/manifest.json",
        "studio/0123456789abcdef/nve_web_shell_bg.wasm",
        "studio/0123456789abcdef/snippets/nve-web-shell-1/inline0.js",
        "counter/manifest.json",
        "counter/fedcba9876543210/nve_web_shell.js",
    ] {
        assert!(runtime_pkg.join(file).is_file(), "{file} copied");
    }
    assert!(
        !runtime_pkg.join("dev").exists(),
        "dev-only bundle should be excluded from production static docs output"
    );
    let studio_unified = fs::read_to_string(out_unified.join("examples/studio/index.html")).unwrap();
    assert!(studio_unified.contains("../_runtime/index.html?app=studio&ws=off&embed=1"));
    assert!(studio_unified.contains("data-bundle-mode=\"unified\""));

    // A `pkg/` without the app's manifest is not a bundle for that app.
    let no_manifest_src = temp_test_dir("ws-no-manifest-src");
    fs::create_dir_all(no_manifest_src.join("pkg/studio/0123456789abcdef")).unwrap();
    fs::write(no_manifest_src.join("index.html"), "<html></html>").unwrap();
    fs::write(no_manifest_src.join("pkg/studio/0123456789abcdef/nve_web_shell_bg.wasm"), b"\0asm").unwrap();
    let out_no_manifest = temp_test_dir("ws-no-manifest-out");
    let status_nm = Command::new(nve_bin())
        .current_dir(repo_root())
        .args(["docs", "build", "--content", "docs/content", "--skip-wasm", "--web-shell-dist"])
        .arg(&no_manifest_src)
        .arg("--out")
        .arg(&out_no_manifest)
        .status()
        .expect("run docs build without manifests");
    assert!(status_nm.success());
    let studio_fallback = fs::read_to_string(out_no_manifest.join("examples/studio/index.html")).unwrap();
    assert!(!studio_fallback.contains("data-bundle-mode=\"unified\""), "{studio_fallback}");

    // 2. Future per-app layout (`<dist>/studio/` + `<dist>/counter/` + shared `nve_dom.js`)
    let per_app_src = temp_test_dir("ws-perapp-src");
    fs::create_dir_all(per_app_src.join("studio")).unwrap();
    fs::create_dir_all(per_app_src.join("counter")).unwrap();
    fs::write(per_app_src.join("nve_dom.js"), "// thin interpreter").unwrap();
    fs::write(
        per_app_src.join("studio/index.html"),
        "<html><head></head><body>studio per-app</body></html>",
    )
    .unwrap();
    fs::write(per_app_src.join("studio/studio.wasm"), b"\0asm").unwrap();
    fs::write(
        per_app_src.join("counter/index.html"),
        "<html><head></head><body>counter per-app</body></html>",
    )
    .unwrap();
    fs::write(per_app_src.join("counter/counter.wasm"), b"\0asm").unwrap();

    let out_per_app = temp_test_dir("ws-perapp-out");
    let status2 = Command::new(nve_bin())
        .current_dir(repo_root())
        .args([
            "docs",
            "build",
            "--content",
            "docs/content",
            "--skip-wasm",
            "--web-shell-dist",
        ])
        .arg(&per_app_src)
        .arg("--out")
        .arg(&out_per_app)
        .status()
        .expect("run docs build with per-app web-shell");
    assert!(status2.success());
    assert!(out_per_app.join("examples/studio/app/studio.wasm").is_file());
    assert!(out_per_app.join("examples/studio/nve_dom.js").is_file());
    assert!(out_per_app.join("examples/counter/app/counter.wasm").is_file());
    let studio_per = fs::read_to_string(out_per_app.join("examples/studio/index.html")).unwrap();
    assert!(studio_per.contains("./app/index.html?ws=off&embed=1"));
    assert!(studio_per.contains("data-bundle-mode=\"per-app\""));

    let _ = fs::remove_dir_all(&unified_src);
    let _ = fs::remove_dir_all(&out_unified);
    let _ = fs::remove_dir_all(&per_app_src);
    let _ = fs::remove_dir_all(&out_per_app);
    let _ = fs::remove_dir_all(&no_manifest_src);
    let _ = fs::remove_dir_all(&out_no_manifest);
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
