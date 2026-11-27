//! Static responses: the external `/pkg/issues/boot.js` ES module (zero inline JS) and
//! path-traversal-safe `/pkg/issues/*` WASM bundle assets.

use std::path::{Component, Path, PathBuf};

use bytes::Bytes;
use http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use http::{Response, StatusCode};
use nve_http_hyper::HttpBody;
use nve_rpc::RpcStatus;
use nve_server::protocol_error_response;

/// Request path of the external WASM boot module (served by the Rust server itself, so
/// `hosts/web-shell/build.sh` wiping `pkg/issues` never deletes it).
pub const BOOT_JS_PATH: &str = "/pkg/issues/boot.js";

/// Static boot module: resolves the content-hashed bundle via `manifest.json`, instantiates the
/// WASM with a streaming fetch, and mounts `IssuesApp` onto the SSR `#app` host.
pub const BOOT_JS: &str = "\
const m = await (await fetch('/pkg/issues/manifest.json', { cache: 'no-cache' })).json();
const wasmRes = fetch('/pkg/issues/' + m.wasm);
const mod = await import('/pkg/issues/' + m.js);
await mod.default({ module_or_path: wasmRes });
window.__nveIssuesMod = mod;
window.__nveIssuesController = mod.start_issues_app('app', '');
";

/// Builds a `200 OK` uncached `nosniff` response with `mime` content type.
pub fn static_response(mime: &'static str, body: impl Into<Bytes>) -> Response<HttpBody> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, mime)
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(CACHE_CONTROL, "no-store")
        .body(HttpBody::full(body.into()))
        .expect("static response builds cleanly")
}

/// Plain-text protocol error response (`404`, `401`, `500`, ...) as an [`HttpBody`].
pub fn error_response(status: RpcStatus, message: &str) -> Response<HttpBody> {
    protocol_error_response(status, message).map(HttpBody::from)
}

pub fn boot_js_response() -> Response<HttpBody> {
    static_response("application/javascript; charset=utf-8", BOOT_JS)
}

/// Reads `rel` under `root` (blocking I/O; run via `spawn_blocking`).
pub fn read_wasm_asset_sync(root: &Path, rel: &str) -> Response<HttpBody> {
    let not_found = || error_response(RpcStatus::NotFound, "asset not found");
    let Some(target) = resolve_safe_subpath(root, rel) else {
        return not_found();
    };
    let mime = match target.extension().and_then(|e| e.to_str()) {
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json; charset=utf-8",
        _ => return not_found(),
    };
    match std::fs::read(&target) {
        Ok(bytes) => static_response(mime, bytes),
        Err(_) => not_found(),
    }
}

fn resolve_safe_subpath(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.contains('\\') {
        return None;
    }
    let mut joined = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(seg) => joined.push(seg),
            _ => return None,
        }
    }
    let canon_root = std::fs::canonicalize(root).ok()?;
    let canon_target = std::fs::canonicalize(&joined).ok()?;
    if canon_target.starts_with(&canon_root) && canon_target.is_file() {
        Some(canon_target)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_safe_subpath;

    #[test]
    fn resolve_safe_subpath_rejects_parent_dir_even_inside_root() {
        let tmp = std::env::temp_dir().join(format!(
            "nve-issues-subpath-unit-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let pkg = tmp.join("pkg");
        let sub = pkg.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(pkg.join("manifest.json"), b"{}").unwrap();
        std::fs::write(tmp.join("outside.json"), b"{}").unwrap();

        assert!(resolve_safe_subpath(&pkg, "manifest.json").is_some());
        // Resolves INSIDE `pkg` after canonicalization, so this fails unless `Component::Normal`
        // explicitly rejects `..` components.
        assert!(resolve_safe_subpath(&pkg, "sub/../manifest.json").is_none());
        assert!(resolve_safe_subpath(&pkg, "../outside.json").is_none());
        assert!(resolve_safe_subpath(&pkg, "sub\\manifest.json").is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
