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

/// Mutable entry points (`boot.js`, `manifest.json`, test routes): always revalidated, so a
/// rebuild is picked up on the next load.
const CACHE_NO_STORE: &str = "no-store";
/// `hosts/web-shell/build.sh` publishes every bundle file under `pkg/<bundle>/<content-hash>/`
/// and never rewrites a published hash directory, so those bytes are cached forever.
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// Hex length of `build.sh`'s `PUBLISHED_HASH` directory names.
const CONTENT_HASH_DIR_LEN: usize = 16;

/// Builds a `200 OK` uncached `nosniff` response with `mime` content type.
pub fn static_response(mime: &'static str, body: impl Into<Bytes>) -> Response<HttpBody> {
    cached_static_response(mime, CACHE_NO_STORE, body)
}

fn cached_static_response(
    mime: &'static str,
    cache_control: &'static str,
    body: impl Into<Bytes>,
) -> Response<HttpBody> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, mime)
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(CACHE_CONTROL, cache_control)
        .body(HttpBody::full(body.into()))
        .expect("static response builds cleanly")
}

/// `immutable` for files inside a `build.sh` content-hash directory, `no-store` otherwise.
fn asset_cache_control(rel: &str) -> &'static str {
    match rel.split_once('/') {
        Some((dir, _))
            if dir.len() == CONTENT_HASH_DIR_LEN
                && dir.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            CACHE_IMMUTABLE
        }
        _ => CACHE_NO_STORE,
    }
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
        Ok(bytes) => cached_static_response(mime, asset_cache_control(rel), bytes),
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
    use super::{asset_cache_control, resolve_safe_subpath, CACHE_IMMUTABLE, CACHE_NO_STORE};

    #[test]
    fn only_content_hash_directory_assets_are_immutable() {
        let hashed = "78862642eae79244/nve_web_shell_bg.wasm";
        assert_eq!(asset_cache_control(hashed), CACHE_IMMUTABLE);
        assert_eq!(asset_cache_control("78862642eae79244/snippets/a/b.js"), CACHE_IMMUTABLE);
        for mutable in [
            "manifest.json",
            "boot.js",
            "78862642EAE79244/x.wasm",
            "78862642eae7924/x.wasm",
            "78862642eae792440/x.wasm",
            "78862642eae7924g/x.wasm",
            "staging/x.wasm",
        ] {
            assert_eq!(asset_cache_control(mutable), CACHE_NO_STORE, "{mutable}");
        }
    }

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
