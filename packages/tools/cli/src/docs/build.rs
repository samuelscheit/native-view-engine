//! `cargo nve docs build` — emits static Semantic HTML5 + Option 5 CSS pages for every route,
//! binary `search.nvs2`, `sitemap.xml`, `.nojekyll`, `404.html`, and optional `wasm32`
//! progressive hydration bundle.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use nve_backend_web::SsrDocumentOptions;
use nve_docs::DocsViewport;
use nve_mdx::CompiledSite;

use crate::docs::check::validate_content_dir;
use crate::docs::examples::{EXAMPLE_APPS, ExamplesStageSummary, stage_example_apps};

/// Build configuration for `cargo nve docs build`.
#[derive(Debug, Clone)]
pub(crate) struct BuildOptions {
    pub content: PathBuf,
    pub out: PathBuf,
    pub base_path: String,
    pub site_url: String,
    pub skip_wasm: bool,
    pub web_shell_dist: Option<PathBuf>,
    pub with_examples: bool,
}

/// Metrics returned after a successful static + optional WASM build.
#[derive(Debug, Clone)]
pub(crate) struct BuildSummary {
    pub pages_emitted: usize,
    pub total_html_bytes: usize,
    pub total_css_bytes: usize,
    pub search_index_bytes: usize,
    pub wasm_raw_bytes: Option<usize>,
    pub wasm_gzip_bytes: Option<usize>,
    pub examples: ExamplesStageSummary,
}

/// Theme-independent document layout chrome; colors (canvas, ink, `theme-color`, every appearance)
/// come from `nve_docs_site::render_docs_ssr`'s head.
const DOCS_HEAD_CHROME: &str = concat!(
    "<style data-nve-docs-chrome>",
    "*,*::before,*::after{box-sizing:border-box;}",
    "html,body{margin:0;padding:0;width:100%;height:100%;",
    "font-family:Inter,-apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,Helvetica,Arial,sans-serif;",
    "-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility;}",
    "#app{width:100%;height:100%;}",
    "#app>[data-nve-id=\"1\"]{width:100%;height:100%;}",
    "[data-testid=\"docs-root\"]{width:100%;height:100vh;overflow:hidden;}",
    "[data-testid=\"docs-body\"]{min-height:0;overflow:hidden;flex:1 1 0%;}",
    "[data-testid=\"docs-navbar\"]{border-top:0!important;border-left:0!important;border-right:0!important;}",
    "[data-testid=\"docs-sidebar-desktop\"]{border-top:0!important;border-left:0!important;border-bottom:0!important;min-height:0;height:100%;}",
    "[data-testid=\"docs-sidebar-scroll\"]{min-height:0;overflow-y:auto;scrollbar-width:thin;}",
    "[data-testid=\"docs-toc\"]{border-top:0!important;border-right:0!important;border-bottom:0!important;min-height:0;height:100%;overflow-y:auto;}",
    "[data-testid=\"docs-main-scroll\"]{min-height:0;height:100%;overflow-y:auto;align-items:center!important;scrollbar-width:thin;}",
    "[data-testid=\"docs-custom-scroll\"]{min-height:0;width:100%;height:100%;overflow-y:auto;overflow-x:hidden;align-items:center!important;scrollbar-width:thin;}",
    "[data-testid=\"docs-article-container\"]{flex-shrink:0!important;padding-bottom:64px!important;}",
    "[data-testid=\"landing-page\"],[data-testid=\"examples-gallery-page\"]{flex-shrink:0!important;}",
    "[data-testid=\"landing-hero-glow\"]{background:radial-gradient(ellipse 68% 58% at 50% 0%,rgba(56,189,248,0.11),transparent 72%);}",
    "[data-theme=\"light\"] [data-testid=\"landing-hero-glow\"]{background:radial-gradient(ellipse 68% 58% at 50% 0%,rgba(2,132,199,0.08),transparent 72%);}",
    "[data-testid=\"landing-hero-h1\"] span,[data-testid=\"examples-hero-h1\"] span{letter-spacing:-0.03em;line-height:1.1;}",
    "[data-testid=\"docs-sidebar-overlay\"]{position:fixed!important;inset:0!important;z-index:400!important;}",
    "[data-testid=\"docs-search-dialog-backdrop\"]{position:fixed!important;inset:0!important;z-index:1000!important;}",
    "[data-testid=\"docs-inlines\"] a{text-underline-offset:3px;}",
    "[data-testid=\"landing-cta-row\"] a,[data-testid=\"hero-tab-live\"],[data-testid=\"hero-tab-code\"],[data-testid=\"hero-tab-ir\"],[data-testid=\"landing-cli-copy\"],[data-testid^=\"examples-pg-tab-\"],[data-testid^=\"hero-host-\"]{white-space:nowrap!important;}",
    "[data-testid=\"bench-bar-legend\"],[data-testid=\"bento-host-pills\"],[data-testid=\"bench-controls-bar\"]{flex-wrap:wrap!important;}",
    "a,button,[role=\"button\"],[role=\"link\"],[role=\"tab\"]{cursor:pointer;}",
    "a:hover,button:hover,[role=\"button\"]:hover,[role=\"link\"]:hover,[role=\"tab\"]:hover{opacity:0.92;}",
    "a{color:inherit;text-decoration:none;}",
    "a:focus-visible,button:focus-visible,input:focus-visible{outline:2px solid #38bdf8;outline-offset:1px;}",
    "code,pre{font-family:ui-monospace,\"SFMono-Regular\",Menlo,Monaco,Consolas,monospace;}",
    "</style>"
);

/// Execute the full static documentation site build pipeline.
pub(crate) fn run_build(opts: &BuildOptions) -> Result<BuildSummary, String> {
    let report = validate_content_dir(&opts.content)?;
    if !report.diagnostics.is_empty() {
        for diag in &report.diagnostics {
            eprintln!("  [error] {diag}");
        }
        return Err(format!(
            "docs build aborted: {} validation error(s)",
            report.diagnostics.len()
        ));
    }

    let base_path = normalize_base_path(&opts.base_path);
    let site_url = opts.site_url.trim().trim_end_matches('/').to_string();

    fs::create_dir_all(&opts.out)
        .map_err(|e| format!("failed to create output dir {}: {e}", opts.out.display()))?;

    let (wasm_raw_bytes, wasm_gzip_bytes) = if opts.skip_wasm {
        (None, None)
    } else {
        let (raw, gz) = build_wasm_hydration_bundle(&opts.out)?;
        (Some(raw), gz)
    };

    let site = Arc::new(report.site);
    let site_cfg = nve_docs_site::nve_docs_site_config(&base_path);
    let has_wasm = !opts.skip_wasm;

    // 1. Emit binary NVS2 search index.
    let search_bytes = site
        .search_index
        .encode()
        .map_err(|e| format!("failed to encode search.nvs2: {e}"))?;
    fs::write(opts.out.join("search.nvs2"), &search_bytes)
        .map_err(|e| format!("failed to write search.nvs2: {e}"))?;

    // 2. Emit .nojekyll for GitHub Pages.
    fs::write(opts.out.join(".nojekyll"), b"")
        .map_err(|e| format!("failed to write .nojekyll: {e}"))?;

    // 3. Emit sitemap.xml (Landing, Examples Gallery, Live WASM Examples, and all /docs/** pages).
    let sitemap = build_sitemap_xml(&site, &site_cfg, &site_url);
    fs::write(opts.out.join("sitemap.xml"), &sitemap)
        .map_err(|e| format!("failed to write sitemap.xml: {e}"))?;

    // 4. Emit static semantic HTML5 + Option 5 CSS pages for:
    //    - "/" (Landing Page)
    //    - "/examples" (Examples Gallery)
    //    - "/docs" & "/docs/<slug>" (all compiled MDX docs pages)
    let mut total_html_bytes = 0usize;
    let mut total_css_bytes = 0usize;
    let mut pages_emitted = 0usize;
    let body_scripts = has_wasm.then(|| hydration_bootstrap_script(&base_path));

    let custom_ssr_routes: [(&str, &str, &str); 2] = [
        (
            "/",
            "Native View Engine — Native-speed UI in Rust",
            "Write declarative UI once in Rust with binary FFI and one binary ABI across 10 native, GPU Skia, and Web WASM hosts.",
        ),
        (
            "/examples",
            "Interactive Examples & Live Playgrounds — Native View Engine",
            "Launch lazy-loaded WebAssembly apps (Aether Workspace + NVE Studio, Kitchen Sink) and explore 5 live embedded docs playgrounds.",
        ),
    ];

    for (route, title, desc) in custom_ssr_routes {
        let (html_len, css_len) = emit_ssr_page(
            &site,
            &opts.out,
            route,
            &base_path,
            &site_url,
            title,
            Some(desc),
            body_scripts.as_deref(),
        )?;
        total_html_bytes += html_len;
        total_css_bytes += css_len;
        pages_emitted += 1;
    }

    for page in &site.pages {
        let public_route = site_cfg.docs_route_for_content(&page.route);
        let page_title = format!("{} — Native View Engine", page.title);
        let desc = (!page.description.is_empty()).then_some(page.description.as_str());
        let (html_len, css_len) = emit_ssr_page(
            &site,
            &opts.out,
            public_route.as_str(),
            &base_path,
            &site_url,
            &page_title,
            desc,
            body_scripts.as_deref(),
        )?;
        total_html_bytes += html_len;
        total_css_bytes += css_len;
        pages_emitted += 1;
    }

    // 5. Stage lazy-loaded live WASM example apps (/examples/studio/ and /examples/kitchen-sink/).
    let repo_root = find_workspace_root();
    let examples = stage_example_apps(
        &repo_root,
        &opts.out,
        &base_path,
        opts.web_shell_dist.as_deref(),
        opts.with_examples,
        opts.skip_wasm,
    )?;
    pages_emitted += examples.pages_emitted;

    // 6. Emit 404.html fallback (hydrates client-side for GitHub Pages SPA routes).
    let not_found_html = render_404_page(&site, &base_path, &site_url, body_scripts.as_deref())?;
    fs::write(opts.out.join("404.html"), &not_found_html)
        .map_err(|e| format!("failed to write 404.html: {e}"))?;

    let summary = BuildSummary {
        pages_emitted,
        total_html_bytes,
        total_css_bytes,
        search_index_bytes: search_bytes.len(),
        wasm_raw_bytes,
        wasm_gzip_bytes,
        examples,
    };

    print_build_summary(&opts.out, &summary);
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
fn emit_ssr_page(
    site: &Arc<CompiledSite>,
    out_dir: &Path,
    public_route: &str,
    base_path: &str,
    site_url: &str,
    title: &str,
    description: Option<&str>,
    body_scripts: Option<&str>,
) -> Result<(usize, usize), String> {
    let ssr = nve_docs_site::render_docs_ssr(site, public_route, base_path, DocsViewport::desktop_default())?;
    let ssr_preview = ssr.output();
    let css_bytes = ssr_preview.base_css.len() + ssr_preview.style_css.len() + ssr.head.len();

    let canonical = canonical_url_for(site_url, public_route);
    let doc_html = ssr.backend.render_ssr_document(
        ssr.surface.store(),
        &SsrDocumentOptions {
            title: title.to_string(),
            description: description.map(str::to_string),
            canonical_url: Some(canonical),
            og_image: None,
            lang: Some("en".to_string()),
            head_extra: Some(format!("{}{DOCS_HEAD_CHROME}", ssr.head)),
            body_scripts: body_scripts.map(str::to_string),
            enable_event_replay: body_scripts.is_some(),
            style_nonce: None,
        },
    );

    let out_file = route_output_path(out_dir, public_route);
    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    fs::write(&out_file, &doc_html)
        .map_err(|e| format!("failed to write {}: {e}", out_file.display()))?;
    Ok((doc_html.len(), css_bytes))
}

fn print_build_summary(out: &Path, s: &BuildSummary) {
    println!(
        "✓ docs build complete -> {} ({} pages | examples: studio={}, kitchen-sink={})",
        out.display(),
        s.pages_emitted,
        s.examples.studio_mode,
        s.examples.kitchen_sink_mode,
    );
    println!(
        "  HTML: {:.1} KiB | CSS (inlined across pages): {:.1} KiB | search.nvs2: {:.1} KiB",
        s.total_html_bytes as f64 / 1024.0,
        s.total_css_bytes as f64 / 1024.0,
        s.search_index_bytes as f64 / 1024.0
    );
    if let Some(raw) = s.wasm_raw_bytes {
        match s.wasm_gzip_bytes {
            Some(gz) => println!(
                "  WASM: {} bytes ({:.1} KiB raw, {:.1} KiB gzip)",
                raw,
                raw as f64 / 1024.0,
                gz as f64 / 1024.0
            ),
            None => println!("  WASM: {} bytes ({:.1} KiB raw)", raw, raw as f64 / 1024.0),
        }
    }
}

pub(crate) fn normalize_base_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

pub(crate) fn route_output_path(out_dir: &Path, route: &str) -> PathBuf {
    let trimmed = route.trim().trim_matches('/');
    if trimmed.is_empty() {
        out_dir.join("index.html")
    } else {
        out_dir.join(trimmed).join("index.html")
    }
}

fn canonical_url_for(site_url: &str, route: &str) -> String {
    let trimmed = route.trim().trim_matches('/');
    if trimmed.is_empty() {
        format!("{site_url}/")
    } else {
        format!("{site_url}/{trimmed}/")
    }
}

fn build_sitemap_xml(
    site: &CompiledSite,
    site_cfg: &nve_docs::DocsSiteConfig,
    site_url: &str,
) -> String {
    let mut xml = String::with_capacity((site.pages.len() + 4) * 96 + 160);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for static_route in ["/", "/examples"] {
        let loc = canonical_url_for(site_url, static_route);
        let _ = writeln!(xml, "  <url><loc>{}</loc></url>", escape_xml(&loc));
    }
    for app in EXAMPLE_APPS {
        let loc = canonical_url_for(site_url, &format!("/examples/{}", app.slug));
        let _ = writeln!(xml, "  <url><loc>{}</loc></url>", escape_xml(&loc));
    }
    for page in &site.pages {
        let public_route = site_cfg.docs_route_for_content(&page.route);
        let loc = canonical_url_for(site_url, public_route.as_str());
        let _ = writeln!(xml, "  <url><loc>{}</loc></url>", escape_xml(&loc));
    }
    xml.push_str("</urlset>\n");
    xml
}

fn escape_xml(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn hydration_bootstrap_script(base_path: &str) -> String {
    let assets_prefix = if base_path.is_empty() {
        "/assets".to_string()
    } else {
        format!("{base_path}/assets")
    };
    let escaped_base = base_path.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "<script type=\"module\">\n\
         import init, {{ hydrate_docs_app }} from \"{assets_prefix}/nve_docs_site.js\";\n\
         (async () => {{\n\
           try {{\n\
             await init();\n\
             const controller = hydrate_docs_app(\"app\", \"{escaped_base}\");\n\
             window.__NVE_DOCS__ = controller;\n\
             window.dispatchEvent(new CustomEvent(\"nve:docs-hydrated\", {{ detail: {{ adopted: controller.adopted_nodes, mismatches: controller.mismatches, replayed_events: controller.replayed_events, reconciled_dirty_controls: controller.reconciled_dirty_controls }} }}));\n\
           }} catch (err) {{\n\
             console.error(\"[nve-docs] WASM hydration failed:\", err);\n\
             window.__NVE_DOCS_ERROR__ = String(err);\n\
           }}\n\
         }})();\n\
         </script>"
    )
}

fn render_404_page(
    site: &Arc<CompiledSite>,
    base_path: &str,
    site_url: &str,
    body_scripts: Option<&str>,
) -> Result<String, String> {
    let ssr = nve_docs_site::render_docs_ssr(site, "/", base_path, DocsViewport::desktop_default())?;
    Ok(ssr.backend.render_ssr_document(
        ssr.surface.store(),
        &SsrDocumentOptions {
            title: "Native View Engine — Documentation".to_string(),
            description: Some("Native View Engine documentation".to_string()),
            canonical_url: Some(format!("{site_url}/")),
            og_image: None,
            lang: Some("en".to_string()),
            head_extra: Some(format!("{}{DOCS_HEAD_CHROME}", ssr.head)),
            body_scripts: body_scripts.map(str::to_string),
            enable_event_replay: body_scripts.is_some(),
            style_nonce: None,
        },
    ))
}

fn find_workspace_root() -> PathBuf {
    if let Ok(mut cur) = env::current_dir() {
        loop {
            let candidate = cur.join("Cargo.toml");
            if candidate.is_file()
                && fs::read_to_string(&candidate)
                    .map(|s| s.contains("[workspace]"))
                    .unwrap_or(false)
            {
                return cur;
            }
            if !cur.pop() {
                break;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn build_wasm_hydration_bundle(out_dir: &Path) -> Result<(usize, Option<usize>), String> {
    let repo_root = find_workspace_root();
    let target_dir = match env::var("CARGO_TARGET_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let p = PathBuf::from(dir);
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        }
        _ => repo_root.join("target"),
    };

    println!("==> Compiling nve-docs-site (wasm32-unknown-unknown, wasm-release)...");
    let status = Command::new("cargo")
        .current_dir(&repo_root)
        .env("CARGO_TARGET_DIR", &target_dir)
        .args([
            "rustc",
            "--locked",
            "-p",
            "nve-docs-site",
            "--lib",
            "--crate-type",
            "cdylib",
            "--no-default-features",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            "wasm-release",
        ])
        .status()
        .map_err(|e| format!("failed to invoke cargo for wasm32 build: {e}"))?;
    if !status.success() {
        return Err(format!("cargo wasm32 build failed with status {status}"));
    }

    let wasm_input = target_dir
        .join("wasm32-unknown-unknown")
        .join("wasm-release")
        .join("nve_docs_site.wasm");
    if !wasm_input.is_file() {
        return Err(format!(
            "expected compiled WASM artifact at {}",
            wasm_input.display()
        ));
    }

    let assets_dir = out_dir.join("assets");
    fs::create_dir_all(&assets_dir)
        .map_err(|e| format!("failed to create {}: {e}", assets_dir.display()))?;

    let wasm_bindgen = env::var("WASM_BINDGEN").unwrap_or_else(|_| "wasm-bindgen".to_string());
    println!("==> Running {wasm_bindgen} --target web -> {}", assets_dir.display());
    let bg_status = Command::new(&wasm_bindgen)
        .args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(&assets_dir)
        .arg(&wasm_input)
        .status()
        .map_err(|e| {
            format!(
                "failed to run '{wasm_bindgen}' (install with `cargo install wasm-bindgen-cli`): {e}"
            )
        })?;
    if !bg_status.success() {
        return Err(format!("wasm-bindgen failed with status {bg_status}"));
    }

    let wasm_out = assets_dir.join("nve_docs_site_bg.wasm");
    let wasm_opt = env::var("WASM_OPT").unwrap_or_else(|_| "wasm-opt".to_string());
    let opt_tmp = assets_dir.join("nve_docs_site_bg.opt.wasm");
    if let Ok(opt_status) = Command::new(&wasm_opt)
        .args([
            "-Oz",
            "--enable-bulk-memory",
            "--enable-nontrapping-float-to-int",
            "--enable-sign-ext",
            "--enable-mutable-globals",
        ])
        .arg(&wasm_out)
        .arg("-o")
        .arg(&opt_tmp)
        .status()
    {
        if opt_status.success() && opt_tmp.is_file() {
            let _ = fs::rename(&opt_tmp, &wasm_out);
        } else {
            let _ = fs::remove_file(&opt_tmp);
        }
    }

    let raw_bytes = fs::metadata(&wasm_out)
        .map_err(|e| format!("failed to stat {}: {e}", wasm_out.display()))?
        .len() as usize;

    let gzip_bytes = Command::new("gzip")
        .args(["-9", "-c"])
        .arg(&wasm_out)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout.len());

    Ok((raw_bytes, gzip_bytes))
}
