//! `GET /issues`: per-request isolated streaming SSR of the shared `IssuesApp` with pre-headers
//! critical data, out-of-order `<QuerySuspense>` slots, and a strict zero-inline-JS nonce CSP.

use core::ops::ControlFlow;
use core::time::Duration;

use http::header::{
    HeaderValue, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use http::{Response, StatusCode};
use nve_abi::SurfaceId;
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
use nve_ui::theme::{ColorHex, Theme};

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

/// Upper bound on the pre-headers critical prefetch: a stalled upstream answers `503` instead of
/// leaving the browser waiting for headers indefinitely.
pub const CRITICAL_PREFETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Document-level CSS outside the Dioxus tree: zero body margin, full-height `#app` flex column
/// (so the root `scrollview` fills the viewport in both DSD and hydrated modes), and the app's
/// theme background behind it.
fn document_head_extra(nonce: &str) -> String {
    // The client root renders under `Theme::DARK` (`IssuesApp`).
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

/// Error page for `status`: client errors (`4xx`) carry `client_message`, server errors (`5xx`)
/// only their generic status code (`internal`, `unavailable`), never upstream or internal detail.
fn page_error_response(status: RpcStatus, client_message: &str) -> Response<HttpBody> {
    if status.http_status() >= 500 {
        error_response(status, status.code_str())
    } else {
        error_response(status, client_message)
    }
}

/// Maps a pre-headers critical query failure onto its HTTP status (`404` for missing issues).
pub fn critical_error_response<E>(err: &RpcError<E>) -> Response<HttpBody> {
    match err {
        RpcError::Domain { status, .. } => {
            page_error_response(*status, "requested resource failed")
        }
        RpcError::Protocol { status, message } => page_error_response(*status, message),
        RpcError::Codec(_) | RpcError::Transport(_) => page_error_response(RpcStatus::Internal, ""),
    }
}

/// Runs the pre-headers critical prefetch `fut` under `timeout`: `Break` carries the response to
/// answer before headers instead of rendering; elapsing breaks with a generic `503`.
pub async fn bounded_critical_prefetch(
    timeout: Duration,
    fut: impl Future<Output = ControlFlow<Response<HttpBody>>>,
) -> ControlFlow<Response<HttpBody>> {
    tokio::time::timeout(timeout, fut).await.unwrap_or_else(|_elapsed| {
        ControlFlow::Break(page_error_response(RpcStatus::Unavailable, ""))
    })
}

/// Maps an SSR orchestration failure onto `503` (pool saturated / closed) or `500`.
pub fn ssr_error_response(err: &SsrError) -> Response<HttpBody> {
    let status = match err {
        SsrError::PoolOverloaded | SsrError::PoolClosed => RpcStatus::Unavailable,
        _ => RpcStatus::Internal,
    };
    page_error_response(status, "")
}

/// Renders the zero-JS dark-theme landing / demo sign-in page for unauthenticated `GET /` visits.
///
/// Uses a fresh CSPRNG nonce and the same strict zero-inline-JS CSP as `/issues`; submitting the
/// form hits `POST /demo-login` (CSRF-checked) to mint the `HttpOnly; SameSite=Lax` demo cookie.
pub fn render_demo_login_page(boot_wasm: bool) -> Response<HttpBody> {
    let nonce = match generate_csp_nonce() {
        Ok(nonce) => nonce,
        Err(err) => return ssr_error_response(&err),
    };
    let theme = &Theme::DARK;
    let mode_note = if boot_wasm {
        "Streaming SSR + WASM hydration enabled (/pkg/issues/boot.js)."
    } else {
        "0-JS streaming SSR mode active (run ./hosts/web-shell/build.sh --dev issues for WASM hydration)."
    };
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{PAGE_TITLE} — Sign In</title>\
         <link rel=\"icon\" href=\"data:,\">\
         <style nonce=\"{nonce}\">\
         *{{box-sizing:border-box}}\
         html,body{{margin:0;min-height:100%;background:{bg};color:{fg};color-scheme:dark;\
         font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;\
         display:flex;align-items:center;justify-content:center;padding:24px}}\
         .card{{width:100%;max-width:440px;background:{surface};border:1px solid {border};\
         border-radius:12px;padding:24px;display:flex;flex-direction:column;gap:14px}}\
         h1{{margin:0;font-size:20px;line-height:1.25}}\
         p{{margin:0;font-size:14px;line-height:1.5;color:{secondary}}}\
         button{{width:100%;padding:10px 16px;border:0;border-radius:8px;background:{accent};\
         color:{accent_fg};font-size:14px;font-weight:600;cursor:pointer}}\
         button:focus-visible{{outline:2px solid {focus};outline-offset:2px}}\
         .meta{{font-size:12px;color:{muted};line-height:1.5}}\
         code{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}}\
         </style></head><body><main class=\"card\">\
         <h1>{PAGE_TITLE}</h1>\
         <p>Reference fullstack app demonstrating streaming SSR, out-of-order \
         <code>&lt;QuerySuspense&gt;</code>, strict nonce CSP, and fail-closed dual auth \
         (<code>HttpOnly</code> cookie on web, <code>Bearer</code> token on native).</p>\
         <form method=\"POST\" action=\"/demo-login\">\
         <button type=\"submit\">Sign in as web-bob &rarr;</button>\
         </form>\
         <p class=\"meta\">{mode_note}<br>Direct unauthenticated requests to \
         <code>/issues</code> intentionally return <code>401</code>.</p>\
         </main></body></html>",
        bg = theme.surface.subtle.hex(),
        fg = theme.text.primary.hex(),
        surface = theme.surface.base.hex(),
        border = theme.border.default.hex(),
        secondary = theme.text.secondary.hex(),
        muted = theme.text.tertiary.hex(),
        accent = theme.status.accent.solid.hex(),
        accent_fg = theme.status.accent.on_solid.hex(),
        focus = theme.border.focus.hex(),
    );
    let csp = HeaderValue::from_str(&content_security_policy(&nonce))
        .expect("base64 nonce is a valid header value");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(CONTENT_SECURITY_POLICY, csp)
        .body(HttpBody::full(html.into()))
        .expect("demo login page builds cleanly")
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
    let critical = bounded_critical_prefetch(CRITICAL_PREFETCH_TIMEOUT, async {
        if let Some(focus_id) = parse_focus_issue_query(query)
            && let Err(err) = client.prefetch::<GetIssue>(&focus_id).await
        {
            return ControlFlow::Break(critical_error_response(&*err));
        }
        if let Err(err) = client.prefetch::<ListIssues>(&IssueFilter::default()).await {
            return ControlFlow::Break(critical_error_response(&*err));
        }
        ControlFlow::Continue(())
    })
    .await;
    if let ControlFlow::Break(resp) = critical {
        return resp;
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
                SurfaceId::ROOT,
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
