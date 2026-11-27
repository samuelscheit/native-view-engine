//! `GET /issues`: per-request isolated streaming SSR of the shared `IssuesApp` with pre-headers
//! critical data, out-of-order `<QuerySuspense>` slots, and a strict zero-inline-JS nonce CSP.

use http::header::{HeaderValue, CONTENT_SECURITY_POLICY};
use http::Response;
use nve_backend_web::WebBackend;
use nve_dioxus::DioxusSurface;
use nve_example_issues_api::{
    parse_comments_issue_query, parse_focus_issue_query, GetIssue, IssueFilter, ListIssues,
};
use nve_example_issues_client::{IssuesApp, IssuesAppProps};
use nve_http_hyper::HttpBody;
use nve_query_dioxus::erased_ssr_query_client;
use nve_rpc::{RpcError, RpcStatus};
use nve_server::DirectTransport;
use nve_ssr::{generate_csp_nonce, SsrError, SsrLocalPool, SsrPageOptions, StreamPlan};
use nve_ui::theme::Theme;
use nve_ui::ColorHex;

use crate::assets::{error_response, BOOT_JS_PATH};
use crate::auth::Actor;
use crate::IssuesServer;

pub const PAGE_TITLE: &str = "NVE Fullstack Issues";

/// Per-request SSR backend with the issues family's shared plugin set (the browser shell
/// registers the same one), so `<ActionForm>` streams real `<form action method>` markup (the 0-JS
/// fallback) and `<form>` placement hygiene sees the true tag.
pub fn issues_web_backend() -> WebBackend {
    let mut backend = WebBackend::new();
    nve_example_issues_client::register_web_systems(backend.table_registry_mut());
    backend
}

/// Document-level CSS outside the Dioxus tree: zero body margin, full-height `#app` flex column
/// (so the root `scrollview` fills the viewport in both DSD and hydrated modes), and the app's
/// theme background behind it.
fn document_head_extra(nonce: &str) -> String {
    let background = Theme::DARK.surface.subtle.hex();
    format!(
        "<link rel=\"icon\" href=\"data:,\">\
         <style nonce=\"{nonce}\">html,body{{margin:0;height:100%;background:{background};\
         color-scheme:dark;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,\
         sans-serif}}#app{{height:100%;display:flex;flex-direction:column}}</style>"
    )
}

/// Strict zero-inline-JS CSP: scripts only from `'self'` (plus WASM compilation), `<style>` only
/// with the per-request nonce; `style=""` attributes stay allowed for SSR node styles.
pub fn content_security_policy(nonce: &str) -> String {
    format!(
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
         style-src 'self' 'nonce-{nonce}'; style-src-attr 'unsafe-inline'; connect-src 'self'; \
         img-src 'self' data:; object-src 'none'; base-uri 'self'; form-action 'self'"
    )
}

/// Maps a pre-headers critical query failure onto its HTTP status (`404` for missing issues).
fn critical_error_response<E>(err: &RpcError<E>) -> Response<HttpBody> {
    match err {
        RpcError::Domain { status, .. } => error_response(*status, "requested resource failed"),
        RpcError::Protocol { status, message } => error_response(*status, message),
        RpcError::Codec(_) | RpcError::Transport(_) => {
            error_response(RpcStatus::Internal, "critical SSR query failed")
        }
    }
}

fn ssr_error_response(err: &SsrError) -> Response<HttpBody> {
    let status = match err {
        SsrError::PoolOverloaded | SsrError::PoolClosed => RpcStatus::Unavailable,
        _ => RpcStatus::Internal,
    };
    error_response(status, &err.to_string())
}

/// Renders `/issues` for `actor`. `query` is the raw URL query (`?comments_issue=`, `?issue=`).
pub async fn render_issues_page(
    server: IssuesServer,
    pool: SsrLocalPool,
    actor: Actor,
    query: Option<String>,
    boot_wasm: bool,
) -> Response<HttpBody> {
    let query = query.as_deref();
    let featured_issue_id = parse_comments_issue_query(query);
    let client = erased_ssr_query_client(DirectTransport::with_principal(server, actor));

    // Pre-headers critical data: unknown focus issue is a real `404`, and the issue list is
    // settled into `seq=0`. Stats + featured comments stay unprefetched and stream out of order.
    if let Some(focus_id) = parse_focus_issue_query(query)
        && let Err(err) = client.prefetch::<GetIssue>(&focus_id).await
    {
        return critical_error_response(&*err);
    }
    if let Err(err) = client.prefetch::<ListIssues>(&IssueFilter::default()).await {
        return critical_error_response(&*err);
    }

    let nonce = match generate_csp_nonce() {
        Ok(nonce) => nonce,
        Err(err) => return ssr_error_response(&err),
    };
    let mut page = SsrPageOptions::new(PAGE_TITLE, issues_web_backend)
        .with_style_nonce(nonce.clone())
        .with_head_extra(document_head_extra(&nonce));
    if boot_wasm {
        page = page
            .with_modulepreload_src(BOOT_JS_PATH)
            .with_async_boot_src(BOOT_JS_PATH);
    }

    let rendered = pool
        .render_streaming(page, StreamPlan::default(), client, move |c| {
            DioxusSurface::new_with_props(
                1,
                IssuesApp,
                IssuesAppProps {
                    client: c,
                    sleep: None,
                    featured_issue_id,
                },
            )
        })
        .await;
    match rendered {
        Ok(mut resp) => {
            let csp = HeaderValue::from_str(&content_security_policy(&nonce))
                .expect("base64 nonce is a valid header value");
            resp.headers_mut().insert(CONTENT_SECURITY_POLICY, csp);
            resp
        }
        Err(err) => ssr_error_response(&err),
    }
}
