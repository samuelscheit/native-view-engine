//! Reference fullstack issue-tracker server library wiring `nve-rpc`, `nve-server`, and
//! `nve-http-hyper`.

pub mod auth;
pub mod store;

use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::path::{Component, Path, PathBuf};

use bytes::Bytes;
use http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use http::{Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Body;
use nve_example_issues_api::{
    AddComment, AddCommentInput, CloseIssue, CloseIssueInput, CreateIssue, CreateIssueInput,
    GetIssue, IssueFilter, IssueId, ListComments, ListIssues, RenameIssue, RenameIssueInput,
};
use nve_http_hyper::NveHyperService;
use nve_server::{
    protocol_error_response, AuthResolver, CorsPolicy, CsrfPolicy, Cx, RpcRegistry, Server,
    ServerConfig,
};

use crate::auth::{Actor, IssuesAuth};
use crate::store::IssueStore;

pub type IssuesServer = Server<IssueStore, Actor, IssuesAuth>;

/// Builds the sans-I/O `IssuesServer` registering all 7 procedures (with static cache tags /
/// invalidations declared in `procedures!`, plus built-in progressive form routes for
/// `/issues/create`, `/issues/close`, `/issues/rename`, and `/issues/comment`).
pub fn build_issues_server<I, S>(store: IssueStore, allowed_origins: I) -> IssuesServer
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    build_issues_server_with_cors(store, allowed_origins, None)
}

/// Same as [`build_issues_server`], with optional opt-in [`CorsPolicy`] for cross-origin RPC.
pub fn build_issues_server_with_cors<I, S>(
    store: IssueStore,
    allowed_origins: I,
    cors: Option<CorsPolicy>,
) -> IssuesServer
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut reg = RpcRegistry::<IssueStore, Actor>::builder();

    reg.register::<ListIssues, _, _>(|cx: Cx<IssueStore, Actor>, filter: IssueFilter| async move {
        Ok(cx.state.list(&filter))
    });

    reg.register::<GetIssue, _, _>(|cx: Cx<IssueStore, Actor>, id: IssueId| async move {
        cx.state.get(id)
    });

    reg.register::<ListComments, _, _>(|cx: Cx<IssueStore, Actor>, id: IssueId| async move {
        cx.state.list_comments(id)
    });

    reg.register_with_form::<CreateIssue, _, _>(
        |cx: Cx<IssueStore, Actor>, input: CreateIssueInput| async move {
            cx.state.create(&input)
        },
    );

    reg.register_with_form::<CloseIssue, _, _>(
        |cx: Cx<IssueStore, Actor>, input: CloseIssueInput| async move {
            cx.state.close(&input)
        },
    );

    reg.register_with_form::<RenameIssue, _, _>(
        |cx: Cx<IssueStore, Actor>, input: RenameIssueInput| async move {
            cx.state.rename(&input)
        },
    );

    reg.register_with_form::<AddComment, _, _>(
        |cx: Cx<IssueStore, Actor>, input: AddCommentInput| async move {
            cx.state.add_comment(&cx.principal.username, &input)
        },
    );

    let mut config = ServerConfig::default()
        .with_form_route::<CreateIssue>("/issues/create", "/issues")
        .with_form_route::<CloseIssue>("/issues/close", "/issues")
        .with_form_route::<RenameIssue>("/issues/rename", "/issues")
        .with_form_route::<AddComment>("/issues/comment", "/issues");
    if let Some(policy) = cors {
        config = config.with_cors(policy);
    }

    Server::new(
        store,
        reg.build(),
        IssuesAuth,
        CsrfPolicy::allow_origins(allowed_origins),
        config,
    )
}

/// Thin composite HTTP service serving `GET /issues` (the 0-JS `303 See Other` landing page with
/// progressive WASM boot when configured), optional static `/pkg/issues/*` WASM assets, and
/// delegating `/rpc/*` + `/issues/{create,close,rename,comment}` directly to [`NveHyperService`].
#[derive(Clone)]
pub struct IssuesHttpService {
    rpc: NveHyperService<IssueStore, Actor, IssuesAuth>,
    wasm_dir: Option<PathBuf>,
}

impl IssuesHttpService {
    pub fn new(server: IssuesServer) -> Self {
        Self {
            rpc: NveHyperService::new(server),
            wasm_dir: None,
        }
    }

    /// Configures a local directory holding the built `hosts/web-shell/pkg/issues` WASM bundle
    /// served safely under `/pkg/issues/*` and booted progressively on `GET /issues`.
    pub fn with_wasm_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.wasm_dir = Some(dir.into());
        self
    }

    fn render_issues_page(&self) -> Response<Full<Bytes>> {
        let issues = self.rpc.server().state().list(&IssueFilter::default());
        let mut html = String::from(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
             <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
             <link rel=\"icon\" href=\"data:,\">\
             <title>NVE Fullstack Issues</title>\
             <style>html,body{margin:0;padding:0;min-height:100vh;background:#0b1120;color:#e2e8f0;\
             font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;}\
             #app{min-height:100vh;display:flex;flex-direction:column;}\
             .ssr-fallback{max-width:760px;margin:24px auto;padding:16px;}</style></head>\
             <body><div id=\"app\"><div class=\"ssr-fallback\"><h1>Issues</h1>\
             <form method=\"post\" action=\"/issues/create\">\
             <input name=\"title\" placeholder=\"New issue title\" required>\
             <button type=\"submit\">Create Issue</button></form><ul>",
        );
        for issue in &issues {
            html.push_str("<li>#");
            html.push_str(&issue.id.0.to_string());
            html.push_str(": ");
            push_html_escaped(&issue.title, &mut html);
            html.push_str("</li>");
        }
        html.push_str("</ul></div></div>");
        if self.wasm_dir.is_some() {
            html.push_str(
                "<script type=\"module\">\
                 const m=await(await fetch('/pkg/issues/manifest.json',{cache:'no-cache'})).json();\
                 const wasmRes=fetch('/pkg/issues/'+m.wasm);\
                 const mod=await import('/pkg/issues/'+m.js);\
                 await mod.default({module_or_path:wasmRes});\
                 window.__nveIssuesMod=mod;\
                 window.__nveIssuesController=mod.start_issues_app('app','');\
                 </script>",
            );
        }
        html.push_str("</body></html>");
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
            .header(CACHE_CONTROL, "no-store")
            .body(Full::new(Bytes::from(html)))
            .expect("static HTML response builds cleanly")
    }
}

fn read_wasm_asset_sync(root: &Path, rel: &str) -> Response<Full<Bytes>> {
    let Some(target) = resolve_safe_subpath(root, rel) else {
        return protocol_error_response(nve_rpc::RpcStatus::NotFound, "asset not found")
            .map(Full::new);
    };
    let mime = match target.extension().and_then(|e| e.to_str()) {
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json; charset=utf-8",
        _ => {
            return protocol_error_response(nve_rpc::RpcStatus::NotFound, "asset not found")
                .map(Full::new);
        }
    };
    let Ok(bytes) = std::fs::read(&target) else {
        return protocol_error_response(nve_rpc::RpcStatus::NotFound, "asset not found")
            .map(Full::new);
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, mime)
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(bytes)))
        .expect("static asset response builds cleanly")
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

fn push_html_escaped(input: &str, out: &mut String) {
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

impl<B> hyper::service::Service<Request<B>> for IssuesHttpService
where
    B: Body<Data = Bytes> + Unpin + Send + 'static,
    B::Error: core::fmt::Display,
{
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<B>) -> Self::Future {
        if *req.method() == Method::GET && req.uri().path().starts_with("/pkg/issues/") {
            let Some(root) = self.wasm_dir.clone() else {
                let resp =
                    protocol_error_response(nve_rpc::RpcStatus::NotFound, "wasm assets disabled")
                        .map(Full::new);
                return Box::pin(async move { Ok(resp) });
            };
            let rel = req
                .uri()
                .path()
                .strip_prefix("/pkg/issues/")
                .unwrap_or("")
                .to_owned();
            return Box::pin(async move {
                let resp = tokio::task::spawn_blocking(move || read_wasm_asset_sync(&root, &rel))
                    .await
                    .unwrap_or_else(|_| {
                        protocol_error_response(
                            nve_rpc::RpcStatus::Internal,
                            "asset reader task failed",
                        )
                        .map(Full::new)
                    });
                Ok(resp)
            });
        }
        if *req.method() == Method::GET && req.uri().path() == "/issues" {
            let auth_res = IssuesAuth.try_resolve_sync(self.rpc.server().state(), req.headers());
            let resp = match auth_res {
                Some(Ok(_actor)) => self.render_issues_page(),
                Some(Err(err)) => {
                    protocol_error_response(err.status, &err.message).map(Full::new)
                }
                None => protocol_error_response(
                    nve_rpc::RpcStatus::Unauthenticated,
                    "authentication required",
                )
                .map(Full::new),
            };
            return Box::pin(async move { Ok(resp) });
        }
        hyper::service::Service::call(&self.rpc, req)
    }
}

impl<B> tower_service::Service<Request<B>> for IssuesHttpService
where
    B: Body<Data = Bytes> + Unpin + Send + 'static,
    B::Error: core::fmt::Display,
{
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        hyper::service::Service::call(self, req)
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
