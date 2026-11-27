use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use nve_example_issues_server::store::IssueStore;
use nve_example_issues_server::{build_issues_server_with_cors, IssuesHttpService};
use nve_http_hyper::serve_service_with_shutdown;
use nve_server::CorsPolicy;
use tokio::net::TcpListener;

fn default_browser_origins(addr: SocketAddr) -> Vec<String> {
    let ip = match addr.ip() {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        other => other,
    };
    let port = addr.port();
    let primary = match ip {
        IpAddr::V4(v4) => format!("http://{v4}:{port}"),
        IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
    };
    let mut origins = vec![primary];
    if ip.is_loopback() {
        origins.push(format!("http://localhost:{port}"));
    }
    origins
}

/// Resolves the built `hosts/web-shell/pkg/issues` bundle directory.
///
/// Respects explicit `NVE_ISSUES_WASM_DIR` (empty value explicitly disables WASM for 0-JS mode).
/// When unset, auto-detects a local `hosts/web-shell/pkg/issues/manifest.json` from cwd or repo root.
fn resolve_wasm_dir() -> Option<PathBuf> {
    match std::env::var("NVE_ISSUES_WASM_DIR") {
        Ok(raw) => {
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
        }
        Err(_) => {
            let candidates = [
                PathBuf::from("hosts/web-shell/pkg/issues"),
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../hosts/web-shell/pkg/issues"),
            ];
            candidates
                .into_iter()
                .find(|dir| dir.join("manifest.json").is_file())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind = std::env::var("NVE_ISSUES_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let origins = match std::env::var("NVE_ISSUES_ORIGIN") {
        Ok(raw) if !raw.trim().is_empty() => vec![raw],
        _ => default_browser_origins(local_addr),
    };

    println!(
        "nve-example-issues-server listening on http://{local_addr} (allowed origin: {})",
        origins.join(", ")
    );
    println!("  open http://{local_addr}/ in a browser to sign in and launch /issues");

    let cors = std::env::var("NVE_ISSUES_CORS_ORIGIN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|o| CorsPolicy::allow_origins([o]));
    let server = build_issues_server_with_cors(IssueStore::seeded(), origins, cors);
    let test_hooks = std::env::var("NVE_ISSUES_TEST_HOOKS").is_ok_and(|v| v == "1");
    if test_hooks {
        println!("WARNING: NVE_ISSUES_TEST_HOOKS=1 mounts /_test/* stream-control routes (E2E only)");
    }
    let mut svc = IssuesHttpService::new(server).with_test_hooks(test_hooks);
    if let Some(wasm_dir) = resolve_wasm_dir() {
        println!(
            "  wasm bundle: {} (streaming SSR + WASM hydration enabled)",
            wasm_dir.display()
        );
        svc = svc.with_wasm_dir(wasm_dir);
    } else {
        println!(
            "  wasm bundle: disabled (0-JS SSR mode; run ./hosts/web-shell/build.sh --dev issues to enable hydration)"
        );
    }
    serve_service_with_shutdown(listener, svc, core::future::pending::<()>()).await?;
    Ok(())
}
