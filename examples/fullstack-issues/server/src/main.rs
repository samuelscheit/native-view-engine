use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use nve_example_issues_server::store::IssueStore;
use nve_example_issues_server::{build_issues_server_with_cors, IssuesHttpService};
use nve_http_hyper::serve_service_with_shutdown;
use nve_server::CorsPolicy;
use tokio::net::TcpListener;

fn default_browser_origin(addr: SocketAddr) -> String {
    let ip = match addr.ip() {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        other => other,
    };
    let port = addr.port();
    match ip {
        IpAddr::V4(v4) => format!("http://{v4}:{port}"),
        IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind = std::env::var("NVE_ISSUES_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let origin = std::env::var("NVE_ISSUES_ORIGIN")
        .unwrap_or_else(|_| default_browser_origin(local_addr));

    println!("nve-example-issues-server listening on http://{local_addr} (allowed origin: {origin})");

    let cors = std::env::var("NVE_ISSUES_CORS_ORIGIN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|o| CorsPolicy::allow_origins([o]));
    let server = build_issues_server_with_cors(IssueStore::seeded(), [origin], cors);
    let mut svc = IssuesHttpService::new(server);
    if let Ok(wasm_dir) = std::env::var("NVE_ISSUES_WASM_DIR")
        && !wasm_dir.trim().is_empty()
    {
        svc = svc.with_wasm_dir(wasm_dir);
    }
    serve_service_with_shutdown(listener, svc, core::future::pending::<()>()).await?;
    Ok(())
}
