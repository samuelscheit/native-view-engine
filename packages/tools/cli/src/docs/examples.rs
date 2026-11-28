//! Pluggable `hosts/web-shell` WASM example bundle discovery and NVE Portal Bar wrapper emission
//! for `/examples/studio/` and `/examples/kitchen-sink/`.
//!
//! Supports BOTH:
//! - Unified `hosts/web-shell` bundles (`index.html` + `pkg/<slug>/manifest.json`, which names the
//!   entry files in the content-hashed `pkg/<slug>/<hash>/` directory `build.sh` publishes)
//! - Future per-app bundles (`<dist>/studio/` and `<dist>/kitchen-sink/`) with zero code changes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nve_abi::{FrameId, SurfaceId};
use nve_backend_web::{
    inject_ssg_with_scripts_into_shell_html, prerender_nve_app_frame0, SsrOutput,
    PRE_HYDRATION_REPLAY_SCRIPT_TAG, WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG,
};
use super::portal::render_portal_wrapper_html;
use nve_engine::Viewport;
use nve_example_kitchen_sink::KitchenSinkModule;
use nve_studio_workspace::StudioWorkspaceModule;

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
        description: "Multi-pane workspace, 100k Fenwick VirtualList, X-Ray inspector, and NveCrdtDelta CRDTs in WASM.",
    },
    ExampleAppSpec {
        slug: "kitchen-sink",
        title: "NVE Kitchen Sink Gallery",
        description: "Interactive showcase of every nve-ui component, theme/density/i18n switching, Canvas draw lists, and ErrorBoundary recovery.",
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
    pub kitchen_sink_mode: &'static str,
    pub pages_emitted: usize,
}

/// Discover, stage, and wrap `/examples/studio/` and `/examples/kitchen-sink/` in the NVE Portal Bar.
pub(crate) fn stage_example_apps(
    repo_root: &Path,
    out_dir: &Path,
    base_path: &str,
    web_shell_dist: Option<&Path>,
    with_examples: bool,
    skip_wasm: bool,
) -> Result<ExamplesStageSummary, String> {
    let mut resolved = resolve_all_bundles(repo_root, web_shell_dist, skip_wasm);
    let missing = |resolved: &[(ExampleAppSpec, DiscoveredBundle)]| -> Vec<&'static str> {
        resolved.iter().filter(|(_, b)| matches!(b, DiscoveredBundle::Fallback)).map(|(s, _)| s.slug).collect()
    };

    // Build whenever any app lacks a bundle (a partial `pkg/` must not leave fallback pages).
    if with_examples && !missing(&resolved).is_empty() {
        run_web_shell_build(repo_root)?;
        resolved = resolve_all_bundles(repo_root, web_shell_dist, false);
    }
    // An explicit `--web-shell-dist` promises live apps: a missing bundle is an error, not a
    // silent fallback page (those are for `--skip-wasm` / bundle-less builds).
    let unbuilt = missing(&resolved);
    if let Some(dist) = web_shell_dist.filter(|_| !unbuilt.is_empty()) {
        return Err(format!(
            "--web-shell-dist {} has no WASM bundle for {unbuilt:?}; run `./hosts/web-shell/build.sh --dev {}` first",
            dist.display(), unbuilt.join(" ")
        ));
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
                let app_index = inner_dir.join("index.html");
                inject_embed_topbar_hider(&app_index)?;
                inject_cold_ssr_and_bootstrap(&app_index, spec.slug)?;
                ("per-app", Some("./app/index.html?ws=off&embed=1".to_string()))
            }
            DiscoveredBundle::Unified { root_dir } => {
                if !staged_unified {
                    stage_unified_runtime(&root_dir, &runtime_dir)?;
                    staged_unified = true;
                }
                let app_html_name = format!("{}.html", spec.slug);
                let app_html_path = runtime_dir.join(&app_html_name);
                fs::copy(runtime_dir.join("index.html"), &app_html_path).map_err(|e| {
                    format!(
                        "failed to stage cold-SSR entry {}: {e}",
                        app_html_path.display()
                    )
                })?;
                inject_cold_ssr_and_bootstrap(&app_html_path, spec.slug)?;
                (
                    "unified",
                    Some(format!(
                        "../_runtime/{app_html_name}?app={}&ws=off&embed=1",
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
        } else if spec.slug == "kitchen-sink" {
            summary.kitchen_sink_mode = mode_label;
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
    // 2. Fall back to the unified bundle (`<dir>/index.html` + `<dir>/pkg/<slug>/manifest.json`).
    for dir in candidates {
        if is_unified_bundle_dir(dir, slug) {
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

fn is_unified_bundle_dir(dir: &Path, slug: &str) -> bool {
    dir.join("index.html").is_file() && dir.join("pkg").join(slug).join("manifest.json").is_file()
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
        "==> Building hosts/web-shell WASM example bundles ({} studio kitchen-sink)...",
        script.display()
    );
    let status = Command::new("bash")
        .current_dir(repo_root)
        .arg(&script)
        .args(["studio", "kitchen-sink"])
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

/// A web backend with an app family's shared plugin set registered BEFORE the first commit, so
/// Frame-0 tags and placement checks match the browser shell.
fn example_web_backend(
    register: fn(&mut nve_backend_web::WebTableRegistry),
) -> nve_backend_web::WebBackend {
    let mut web = nve_backend_web::WebBackend::new();
    register(web.table_registry_mut());
    web
}

/// Pre-renders default Frame-0 SSG HTML + Option 5 CSS for any example `NveApp` via the generic
/// engine-level [`prerender_nve_app_frame0`] helper so brand-new cold visitors paint at 0ms.
fn render_example_default_ssr(slug: &str) -> Result<Option<SsrOutput>, String> {
    use nve_engine::{NveApp, StoreBackend};

    let vp = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 1.0,
    };
    let ssr = match slug {
        "studio" => {
            let (store, backend) =
                prerender_nve_app_frame0::<StudioWorkspaceModule>(
                    example_web_backend(nve_studio_workspace::register_web_systems),
                    vp,
                )
                    .map_err(|e| {
                        format!("failed to prerender Frame-0 SSG for example '{slug}': {e}")
                    })?;
            backend.render_ssr(&store)
        }
        "kitchen-sink" => {
            let mut app = KitchenSinkModule::create(SurfaceId::ROOT);
            let mut web = example_web_backend(nve_example_kitchen_sink::register_web_systems);
            web.set_viewport(SurfaceId::ROOT, vp);
            app.commit_to(FrameId(1), &mut web)
                .map_err(|e| format!("failed to commit kitchen-sink Frame-0 SSG: {e}"))?;
            web.render_ssr(app.surface().store())
        }
        _ => return Ok(None),
    };
    Ok(Some(ssr))
}

/// Injects pre-rendered default Frame-0 SSG plus `<script data-nve-snapshot-boot>` and
/// `<script data-nve-replay>` into a staged example HTML file.
fn inject_cold_ssr_and_bootstrap(index_path: &Path, slug: &str) -> Result<(), String> {
    if !index_path.is_file() {
        return Ok(());
    }
    let html = fs::read_to_string(index_path)
        .map_err(|e| format!("failed to read {}: {e}", index_path.display()))?;
    let scripts = format!(
        "{WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG}\n{PRE_HYDRATION_REPLAY_SCRIPT_TAG}"
    );
    let patched = if let Some(ssr) = render_example_default_ssr(slug)? {
        inject_ssg_with_scripts_into_shell_html(&html, "app", &ssr, &scripts)
    } else {
        html
    };
    fs::write(index_path, patched)
        .map_err(|e| format!("failed to write {}: {e}", index_path.display()))
}

pub(super) fn join_base(base_path: &str, path: &str) -> String {
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
