//! Pluggable `hosts/web-shell` WASM example bundle discovery and NVE Portal Bar wrapper emission
//! for `/examples/studio/` and `/examples/counter/`.
//!
//! Supports BOTH:
//! - Unified `hosts/web-shell` bundles (`index.html` + `pkg/nve_web_shell.js` + `*.wasm`)
//! - Future per-app bundles (`<dist>/studio/` and `<dist>/counter/`) with zero code changes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nve_docs_site::GITHUB_REPO_URL;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExampleAppSpec {
    pub slug: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

pub(crate) const EXAMPLE_APPS: [ExampleAppSpec; 2] = [
    ExampleAppSpec {
        slug: "studio",
        title: "Aether Workspace + NVE Studio",
        description: "Multi-pane workspace, 100k Fenwick VirtualList, X-Ray inspector, and NVC1 CRDTs in WASM.",
    },
    ExampleAppSpec {
        slug: "counter",
        title: "Interactive Counter",
        description: "Minimal NveApp proving fine-grained Dioxus signals and zero-JSON binary CommandBuffer patching.",
    },
];

#[derive(Debug, Clone)]
pub(crate) enum DiscoveredBundle {
    /// Future per-app layout: `<dir>/<slug>/` holds its own standalone bundle.
    PerApp { app_dir: PathBuf, shared_root: PathBuf },
    /// Unified layout: `<dir>/index.html` + `<dir>/pkg/` serves `?app=<slug>`.
    Unified { root_dir: PathBuf },
    /// No prebuilt WASM bundle available (e.g. `--skip-wasm` in fast CI/unit tests).
    Fallback,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExamplesStageSummary {
    pub studio_mode: &'static str,
    pub counter_mode: &'static str,
    pub pages_emitted: usize,
}

/// Discover, stage, and wrap `/examples/studio/` and `/examples/counter/` in the NVE Portal Bar.
pub(crate) fn stage_example_apps(
    repo_root: &Path,
    out_dir: &Path,
    base_path: &str,
    web_shell_dist: Option<&Path>,
    with_examples: bool,
    skip_wasm: bool,
) -> Result<ExamplesStageSummary, String> {
    let mut resolved = resolve_all_bundles(repo_root, web_shell_dist, skip_wasm);

    if with_examples
        && resolved
            .iter()
            .all(|(_, b)| matches!(b, DiscoveredBundle::Fallback))
    {
        run_web_shell_build(repo_root)?;
        resolved = resolve_all_bundles(repo_root, web_shell_dist, false);
    }

    // If any app uses the Unified bundle, stage it once under `<out>/examples/_runtime/`.
    let mut staged_unified = false;
    let runtime_dir = out_dir.join("examples").join("_runtime");

    let mut summary = ExamplesStageSummary::default();

    for (spec, bundle) in resolved {
        let app_out = out_dir.join("examples").join(spec.slug);
        fs::create_dir_all(&app_out)
            .map_err(|e| format!("failed to create {}: {e}", app_out.display()))?;

        let (mode_label, iframe_src) = match bundle {
            DiscoveredBundle::PerApp {
                app_dir,
                shared_root,
            } => {
                let inner_dir = app_out.join("app");
                copy_dir_recursive(&app_dir, &inner_dir)?;
                copy_top_level_shared_assets(&shared_root, &app_out)?;
                inject_embed_topbar_hider(&inner_dir.join("index.html"))?;
                ("per-app", Some("./app/index.html?ws=off&embed=1".to_string()))
            }
            DiscoveredBundle::Unified { root_dir } => {
                if !staged_unified {
                    stage_unified_runtime(&root_dir, &runtime_dir)?;
                    staged_unified = true;
                }
                (
                    "unified",
                    Some(format!(
                        "../_runtime/index.html?app={}&ws=off&embed=1",
                        spec.slug
                    )),
                )
            }
            DiscoveredBundle::Fallback => ("fallback", None),
        };

        let wrapper_html = render_portal_wrapper_html(spec, base_path, iframe_src.as_deref(), mode_label);
        fs::write(app_out.join("index.html"), wrapper_html)
            .map_err(|e| format!("failed to write {}/index.html: {e}", app_out.display()))?;

        summary.pages_emitted += 1;
        if spec.slug == "studio" {
            summary.studio_mode = mode_label;
        } else if spec.slug == "counter" {
            summary.counter_mode = mode_label;
        }
    }

    Ok(summary)
}

fn resolve_all_bundles(
    repo_root: &Path,
    web_shell_dist: Option<&Path>,
    skip_wasm: bool,
) -> Vec<(ExampleAppSpec, DiscoveredBundle)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(explicit) = web_shell_dist {
        let abs = if explicit.is_absolute() {
            explicit.to_path_buf()
        } else {
            repo_root.join(explicit)
        };
        candidates.push(abs.clone());
        // Today's `./hosts/web-shell/build.sh` outputs `hosts/web-shell/pkg/` directly next to
        // `hosts/web-shell/index.html`. When `--web-shell-dist hosts/web-shell/dist` is passed
        // before per-app `dist/` exists, automatically check the parent `hosts/web-shell/`.
        if let Some(parent) = abs.parent()
            && abs.file_name().and_then(|n| n.to_str()) == Some("dist")
        {
            candidates.push(parent.to_path_buf());
        }
    } else if !skip_wasm {
        candidates.push(repo_root.join("hosts/web-shell/dist"));
        candidates.push(repo_root.join("hosts/web-shell"));
    }

    EXAMPLE_APPS
        .into_iter()
        .map(|spec| {
            let bundle = discover_for_app(&candidates, spec.slug);
            (spec, bundle)
        })
        .collect()
}

fn discover_for_app(candidates: &[PathBuf], slug: &str) -> DiscoveredBundle {
    // 1. Prefer per-app bundles (`<dir>/<slug>/`) across all candidates.
    for dir in candidates {
        let per_app = dir.join(slug);
        if is_per_app_bundle_dir(&per_app) {
            return DiscoveredBundle::PerApp {
                app_dir: per_app,
                shared_root: dir.clone(),
            };
        }
    }
    // 2. Fall back to unified bundle (`<dir>/index.html` + `<dir>/pkg/` with `.wasm`/`.js`).
    for dir in candidates {
        if is_unified_bundle_dir(dir) {
            return DiscoveredBundle::Unified {
                root_dir: dir.clone(),
            };
        }
    }
    DiscoveredBundle::Fallback
}

fn is_per_app_bundle_dir(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let has_index = dir.join("index.html").is_file();
    let has_pkg = dir.join("pkg").is_dir() && dir_has_wasm_or_js(&dir.join("pkg"));
    let has_direct_wasm = dir_has_wasm_or_js(dir);
    has_index && (has_pkg || has_direct_wasm)
}

fn is_unified_bundle_dir(dir: &Path) -> bool {
    dir.join("index.html").is_file()
        && dir.join("pkg").is_dir()
        && dir_has_wasm_or_js(&dir.join("pkg"))
}

fn dir_has_wasm_or_js(dir: &Path) -> bool {
    dir_has_wasm_or_js_depth(dir, 2)
}

fn dir_has_wasm_or_js_depth(dir: &Path, max_depth: usize) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let p = entry.path();
        if p.is_file() {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("wasm" | "js" | "mjs")
            )
        } else if p.is_dir() && max_depth > 1 {
            dir_has_wasm_or_js_depth(&p, max_depth - 1)
        } else {
            false
        }
    })
}

fn run_web_shell_build(repo_root: &Path) -> Result<(), String> {
    let script = repo_root.join("hosts/web-shell/build.sh");
    if !script.is_file() {
        return Err(format!(
            "--with-examples requested, but {} does not exist",
            script.display()
        ));
    }
    println!(
        "==> Building hosts/web-shell WASM example bundles ({} studio counter)...",
        script.display()
    );
    let status = Command::new("bash")
        .current_dir(repo_root)
        .arg(&script)
        .args(["studio", "counter"])
        .status()
        .map_err(|e| format!("failed to execute {}: {e}", script.display()))?;
    if !status.success() {
        return Err(format!("hosts/web-shell/build.sh failed with status {status}"));
    }
    Ok(())
}

fn stage_unified_runtime(root_dir: &Path, runtime_dir: &Path) -> Result<(), String> {
    if runtime_dir.exists() {
        fs::remove_dir_all(runtime_dir)
            .map_err(|e| format!("failed to clean {}: {e}", runtime_dir.display()))?;
    }
    fs::create_dir_all(runtime_dir)
        .map_err(|e| format!("failed to create {}: {e}", runtime_dir.display()))?;

    let src_index = root_dir.join("index.html");
    let dst_index = runtime_dir.join("index.html");
    fs::copy(&src_index, &dst_index).map_err(|e| {
        format!(
            "failed to copy {} -> {}: {e}",
            src_index.display(),
            dst_index.display()
        )
    })?;
    inject_embed_topbar_hider(&dst_index)?;

    let src_pkg = root_dir.join("pkg");
    if src_pkg.is_dir() {
        copy_pkg_runtime_dir(&src_pkg, &runtime_dir.join("pkg"))?;
    }
    copy_top_level_shared_assets(root_dir, runtime_dir)?;
    Ok(())
}

/// Copies `hosts/web-shell/pkg/` while excluding the dev-only `pkg/dev/` bundle from production
/// static site outputs (saving ~310 KiB when `hosts/web-shell/build.sh` built all 3 bundles).
fn copy_pkg_runtime_dir(src_pkg: &Path, dst_pkg: &Path) -> Result<(), String> {
    fs::create_dir_all(dst_pkg)
        .map_err(|e| format!("failed to create {}: {e}", dst_pkg.display()))?;
    let entries = fs::read_dir(src_pkg)
        .map_err(|e| format!("failed to read {}: {e}", src_pkg.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to iterate {}: {e}", src_pkg.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() && name == "dev" {
            continue;
        }
        let target = dst_pkg.join(&name);
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else if path.is_file() {
            fs::copy(&path, &target).map_err(|e| {
                format!("failed to copy {} -> {}: {e}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

fn copy_top_level_shared_assets(src_dir: &Path, dst_dir: &Path) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(src_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if matches!(ext, "js" | "mjs" | "css" | "wasm") {
            let dest = dst_dir.join(entry.file_name());
            fs::copy(&path, &dest)
                .map_err(|e| format!("failed to copy {} -> {}: {e}", path.display(), dest.display()))?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("failed to create {}: {e}", dst.display()))?;
    let entries = fs::read_dir(src)
        .map_err(|e| format!("failed to read {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to iterate {}: {e}", src.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let target = dst.join(&name);
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else if path.is_file() {
            fs::copy(&path, &target).map_err(|e| {
                format!("failed to copy {} -> {}: {e}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

/// When a staged `index.html` is hosted inside our Portal Bar with `?embed=1`, suppress any
/// duplicate inner `.topbar` so only the single sleek NVE Portal Bar is visible.
fn inject_embed_topbar_hider(index_path: &Path) -> Result<(), String> {
    if !index_path.is_file() {
        return Ok(());
    }
    let html = fs::read_to_string(index_path)
        .map_err(|e| format!("failed to read {}: {e}", index_path.display()))?;
    if html.contains("data-nve-embed-helper") {
        return Ok(());
    }
    let snippet = concat!(
        "<script data-nve-embed-helper>",
        "if(new URLSearchParams(location.search).get('embed')==='1'){",
        "const s=document.createElement('style');",
        "s.textContent='.topbar{display:none!important;}';",
        "document.head.appendChild(s);",
        "}",
        "</script>"
    );
    let patched = if let Some(idx) = html.find("</head>") {
        format!("{}{snippet}{}", &html[..idx], &html[idx..])
    } else {
        format!("{snippet}{html}")
    };
    fs::write(index_path, patched)
        .map_err(|e| format!("failed to write {}: {e}", index_path.display()))
}

fn join_base(base_path: &str, path: &str) -> String {
    let b = base_path.trim().trim_matches('/');
    let p = path.trim().trim_start_matches('/');
    if b.is_empty() {
        if p.is_empty() {
            "/".to_string()
        } else {
            format!("/{p}")
        }
    } else if p.is_empty() {
        format!("/{b}/")
    } else {
        format!("/{b}/{p}")
    }
}

fn render_portal_wrapper_html(
    spec: ExampleAppSpec,
    base_path: &str,
    iframe_src: Option<&str>,
    bundle_mode: &str,
) -> String {
    let home_href = join_base(base_path, "");
    let examples_href = join_base(base_path, "examples/");
    let docs_href = join_base(base_path, "docs/");
    let studio_href = join_base(base_path, "examples/studio/");
    let counter_href = join_base(base_path, "examples/counter/");

    let studio_active = if spec.slug == "studio" { "true" } else { "false" };
    let counter_active = if spec.slug == "counter" { "true" } else { "false" };
    let initial_status = if iframe_src.is_some() {
        "Booting WASM…"
    } else {
        "WASM Not Built"
    };

    let main_body = match iframe_src {
        Some(src) => format!(
            r#"<iframe id="nve-example-frame" class="portal-frame" title="{title}" src="{src}" data-bundle-mode="{bundle_mode}"></iframe>"#,
            title = spec.title,
            src = src,
            bundle_mode = bundle_mode,
        ),
        None => format!(
            r#"<div class="fallback-wrap" data-nve-example-fallback="{slug}">
  <div class="fallback-card">
    <div class="fallback-pill">Lazy WASM Bundle Not Staged</div>
    <h1>{title}</h1>
    <p>{desc}</p>
    <p class="fallback-note">This static site was built with <code>--skip-wasm</code> (or without a prebuilt <code>hosts/web-shell</code> bundle). Build and include the live WebAssembly apps locally with:</p>
    <pre><code>./hosts/web-shell/build.sh
cargo run --locked -p nve-cli -- docs build --web-shell-dist hosts/web-shell/dist</code></pre>
    <div class="fallback-actions">
      <a class="btn-primary" href="{examples_href}">← Back to Examples</a>
      <a class="btn-secondary" href="{docs_href}">Read Documentation</a>
    </div>
  </div>
</div>"#,
            slug = spec.slug,
            title = spec.title,
            desc = spec.description,
            examples_href = examples_href,
            docs_href = docs_href,
        ),
    };

    format!(
        r##"<!doctype html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <meta name="theme-color" content="#09090b" />
  <title>{title} — Native View Engine</title>
  <style>
    *, *::before, *::after {{ box-sizing: border-box; }}
    html, body {{
      margin: 0; padding: 0; width: 100%; height: 100%;
      overflow: hidden;
      background: #09090b; color: #fafafa;
      font-family: Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      -webkit-font-smoothing: antialiased;
    }}
    .portal-shell {{ display: flex; flex-direction: column; width: 100%; height: 100vh; }}
    .portal-bar {{ height: 48px; flex-shrink: 0; width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 0 14px; background: #111114; border-bottom: 1px solid #27272a; font-size: 13px; }}
    .portal-left, .portal-right {{ display: flex; align-items: center; gap: 10px; min-width: 0; }}
    .brand-pill {{ display: inline-flex; align-items: center; gap: 8px; text-decoration: none; color: #fafafa; font-weight: 700; }}
    .brand-mark {{ padding: 2px 7px; border-radius: 6px; background: #0c2231; border: 1px solid #38bdf8; color: #38bdf8; font-size: 11px; font-weight: 700; letter-spacing: 0.02em; }}
    .nav-link {{ padding: 5px 10px; border-radius: 7px; color: #a1a1aa; text-decoration: none; font-weight: 500; border: 1px solid transparent; }}
    .nav-link:hover {{ color: #fafafa; background: #18181b; border-color: #27272a; }}
    .switcher {{ display: inline-flex; align-items: center; gap: 3px; padding: 3px; border-radius: 9px; background: #09090b; border: 1px solid #27272a; }}
    .switch-tab {{ padding: 4px 11px; border-radius: 6px; color: #a1a1aa; text-decoration: none; font-size: 12px; font-weight: 600; }}
    .switch-tab:hover {{ color: #fafafa; }}
    .switch-tab[data-active="true"] {{ background: #0c2231; color: #38bdf8; border: 1px solid rgba(56, 189, 248, 0.55); }}
    .status-pill {{ padding: 4px 10px; border-radius: 999px; background: #18181b; border: 1px solid #27272a; color: #a1a1aa; font-size: 11px; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }}
    .status-pill[data-ready="true"] {{ color: #4ade80; border-color: rgba(74, 222, 128, 0.45); }}
    .portal-main {{ flex: 1 1 0%; width: 100%; min-height: 0; display: flex; background: #09090b; }}
    .portal-frame {{ width: 100%; height: 100%; border: 0; display: block; background: #0b1120; }}
    .fallback-wrap {{ flex: 1; display: flex; align-items: center; justify-content: center; padding: 24px; }}
    .fallback-card {{ max-width: 620px; width: 100%; padding: 28px; border-radius: 14px; background: #111114; border: 1px solid #27272a; display: flex; flex-direction: column; gap: 14px; }}
    .fallback-pill {{ align-self: flex-start; padding: 3px 10px; border-radius: 999px; background: #0c2231; border: 1px solid #38bdf8; color: #38bdf8; font-size: 11px; font-weight: 700; }}
    .fallback-card h1 {{ margin: 0; font-size: 24px; color: #fafafa; }}
    .fallback-card p {{ margin: 0; font-size: 14px; color: #a1a1aa; line-height: 1.55; }}
    .fallback-card pre {{ margin: 0; padding: 14px; border-radius: 10px; background: #0c0c0f; border: 1px solid #27272a; overflow-x: auto; font-size: 12px; color: #38bdf8; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }}
    .fallback-actions {{ display: flex; gap: 10px; flex-wrap: wrap; padding-top: 6px; }}
    .btn-primary {{ padding: 9px 16px; border-radius: 8px; text-decoration: none; background: #38bdf8; color: #09090b; font-size: 13px; font-weight: 700; }}
    .btn-secondary {{ padding: 9px 16px; border-radius: 8px; text-decoration: none; background: #18181b; border: 1px solid #27272a; color: #fafafa; font-size: 13px; font-weight: 600; }}
    @media (max-width: 720px) {{ .hide-sm {{ display: none !important; }} }}
  </style>
</head>
<body>
  <div class="portal-shell" data-nve-example="{slug}" data-bundle-mode="{bundle_mode}">
    <header class="portal-bar" role="banner">
      <div class="portal-left">
        <a class="brand-pill" href="{home_href}" aria-label="Native View Engine home">
          <span class="brand-mark">NVE</span>
        </a>
        <a class="nav-link" data-testid="portal-back-examples" href="{examples_href}">← Back to Examples</a>
        <a class="nav-link hide-sm" data-testid="portal-docs-link" href="{docs_href}">Docs</a>
      </div>
      <div class="portal-right">
        <nav class="switcher" aria-label="Switch live example">
          <a class="switch-tab" data-testid="portal-switch-studio" href="{studio_href}" data-active="{studio_active}">Studio</a>
          <a class="switch-tab" data-testid="portal-switch-counter" href="{counter_href}" data-active="{counter_active}">Counter</a>
        </nav>
        <span id="portal-status" class="status-pill hide-sm" data-ready="false">{initial_status}</span>
        <a class="nav-link hide-sm" href="{github_url}" target="_blank" rel="noopener noreferrer">GitHub ↗</a>
      </div>
    </header>
    <main class="portal-main">
      {main_body}
    </main>
  </div>
  <script>
    (() => {{
      const frame = document.getElementById("nve-example-frame");
      const pill = document.getElementById("portal-status");
      if (!frame || !pill) return;
      const poll = () => {{
        try {{
          const doc = frame.contentDocument;
          if (!doc) return;
          const app = doc.getElementById("app");
          if (app && app.querySelector("[data-nve-id]")) {{
            pill.textContent = "WASM Live ({bundle_mode})";
            pill.dataset.ready = "true";
            return;
          }}
        }} catch {{}}
        setTimeout(poll, 150);
      }};
      frame.addEventListener("load", poll);
      poll();
    }})();
  </script>
</body>
</html>
"##,
        title = spec.title,
        slug = spec.slug,
        bundle_mode = bundle_mode,
        home_href = home_href,
        examples_href = examples_href,
        docs_href = docs_href,
        studio_href = studio_href,
        counter_href = counter_href,
        studio_active = studio_active,
        counter_active = counter_active,
        initial_status = initial_status,
        github_url = GITHUB_REPO_URL,
        main_body = main_body,
    )
}
