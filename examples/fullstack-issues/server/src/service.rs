//! Composite [`IssuesHttpService`]: streaming SSR `GET /issues`, the external boot module,
//! static `/pkg/issues/*` assets, opt-in `/_test/*` hooks, and [`NveHyperService`] for the rest.

use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::path::PathBuf;

use bytes::Bytes;
use http::header::{CACHE_CONTROL, LOCATION, SET_COOKIE, X_CONTENT_TYPE_OPTIONS};
use http::{Method, Request, Response, StatusCode};
use hyper::body::Body;
use nve_http_hyper::{HttpBody, NveHyperService};
use nve_rpc::RpcStatus;
use nve_server::{build_session_cookie, AuthResolver};
use nve_ssr::SsrLocalPool;

use crate::assets::{boot_js_response, error_response, read_wasm_asset_sync, BOOT_JS_PATH};
use crate::auth::{Actor, IssuesAuth, DEMO_WEB_COOKIE_TOKEN, SESSION_COOKIE_NAME};
use crate::ssr::{render_demo_login_page, render_issues_page};
use crate::store::IssueStore;
use crate::test_hooks::{handle_test_route, TEST_ROUTE_PREFIX};
use crate::IssuesServer;

const PKG_PREFIX: &str = "/pkg/issues/";
const DEMO_SESSION_MAX_AGE_SECS: u64 = 86_400;

type ResponseFuture = Pin<Box<dyn Future<Output = Result<Response<HttpBody>, Infallible>> + Send>>;

#[inline]
fn ready(resp: Response<HttpBody>) -> ResponseFuture {
    Box::pin(async move { Ok(resp) })
}

fn see_other_redirect(location: &'static str, set_cookie: Option<&str>) -> Response<HttpBody> {
    let mut builder = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(LOCATION, location)
        .header(CACHE_CONTROL, "no-store")
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(cookie) = set_cookie {
        builder = builder.header(SET_COOKIE, cookie);
    }
    builder
        .body(HttpBody::full(Bytes::new()))
        .expect("redirect response builds cleanly")
}

/// Thin composite HTTP service (cheap `Clone`) usable from Hyper and Tower/Axum.
#[derive(Clone)]
pub struct IssuesHttpService {
    rpc: NveHyperService<IssueStore, Actor, IssuesAuth>,
    ssr_pool: SsrLocalPool,
    wasm_dir: Option<PathBuf>,
    enable_test_hooks: bool,
}

impl IssuesHttpService {
    /// Creates the service with a default-sized dedicated [`SsrLocalPool`].
    ///
    /// # Panics
    /// When the SSR worker threads cannot be spawned.
    pub fn new(server: IssuesServer) -> Self {
        Self::with_ssr_pool(
            server,
            SsrLocalPool::new().expect("spawn nve-ssr LocalSet worker pool"),
        )
    }

    pub fn with_ssr_pool(server: IssuesServer, ssr_pool: SsrLocalPool) -> Self {
        Self {
            rpc: NveHyperService::new(server),
            ssr_pool,
            wasm_dir: None,
            enable_test_hooks: false,
        }
    }

    /// Serves the built `hosts/web-shell/pkg/issues` bundle under `/pkg/issues/*` and boots it
    /// from the SSR page via the external `/pkg/issues/boot.js` module.
    pub fn with_wasm_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.wasm_dir = Some(dir.into());
        self
    }

    /// Mounts the deterministic `/_test/*` stream-control routes (E2E only; `404` otherwise).
    pub fn with_test_hooks(mut self, enable: bool) -> Self {
        self.enable_test_hooks = enable;
        self
    }

    fn serve_pkg_asset(&self, path: &str) -> ResponseFuture {
        let Some(root) = self.wasm_dir.clone() else {
            return ready(error_response(RpcStatus::NotFound, "wasm assets disabled"));
        };
        // Matched before the filesystem so `build.sh` wiping `pkg/issues` never removes it.
        if path == BOOT_JS_PATH {
            return ready(boot_js_response());
        }
        let rel = path.strip_prefix(PKG_PREFIX).unwrap_or("").to_owned();
        Box::pin(async move {
            Ok(tokio::task::spawn_blocking(move || read_wasm_asset_sync(&root, &rel))
                .await
                .unwrap_or_else(|_| {
                    error_response(RpcStatus::Internal, "asset reader task failed")
                }))
        })
    }

    fn serve_root_page<B>(&self, req: &Request<B>) -> ResponseFuture {
        let server = self.rpc.server();
        if matches!(
            IssuesAuth.try_resolve_sync(server.state(), req.headers()),
            Some(Ok(_))
        ) {
            return ready(see_other_redirect("/issues", None));
        }
        ready(render_demo_login_page(self.wasm_dir.is_some()))
    }

    fn serve_demo_login<B>(&self, req: &Request<B>) -> ResponseFuture {
        let server = self.rpc.server();
        let csrf = server.csrf();
        if !csrf.allows_cookies() {
            return ready(error_response(
                RpcStatus::PermissionDenied,
                "cookie login disabled by bearer-only policy",
            ));
        }
        if let Err(rej) = csrf.check_origin(req.method(), req.headers()) {
            return ready(error_response(rej.status, rej.reason));
        }
        let secure = csrf
            .allowed_origins()
            .iter()
            .all(|origin| origin.starts_with("https://"));
        let cookie = build_session_cookie(
            SESSION_COOKIE_NAME,
            DEMO_WEB_COOKIE_TOKEN,
            secure,
            Some(DEMO_SESSION_MAX_AGE_SECS),
        )
        .expect("static demo session cookie is valid");
        ready(see_other_redirect("/issues", Some(&cookie)))
    }

    fn serve_demo_logout<B>(&self, req: &Request<B>) -> ResponseFuture {
        let server = self.rpc.server();
        let csrf = server.csrf();
        if let Err(rej) = csrf.check_origin(req.method(), req.headers()) {
            return ready(error_response(rej.status, rej.reason));
        }
        let secure_attr = if csrf.allows_cookies()
            && csrf
                .allowed_origins()
                .iter()
                .all(|origin| origin.starts_with("https://"))
        {
            "; Secure"
        } else {
            ""
        };
        let clear = format!(
            "{SESSION_COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax{secure_attr}; Max-Age=0"
        );
        ready(see_other_redirect("/", Some(&clear)))
    }

    fn serve_issues_page<B>(&self, req: &Request<B>) -> ResponseFuture {
        let server = self.rpc.server();
        let actor = match IssuesAuth.try_resolve_sync(server.state(), req.headers()) {
            Some(Ok(actor)) => actor,
            Some(Err(err)) => return ready(error_response(err.status, &err.message)),
            None => {
                return ready(error_response(
                    RpcStatus::Unauthenticated,
                    "authentication required",
                ));
            }
        };
        let page = render_issues_page(
            server.clone(),
            self.ssr_pool.clone(),
            actor,
            req.uri().query().map(str::to_owned),
            self.wasm_dir.is_some(),
        );
        Box::pin(async move { Ok(page.await) })
    }
}

impl<B> hyper::service::Service<Request<B>> for IssuesHttpService
where
    B: Body<Data = Bytes> + Unpin + Send + 'static,
    B::Error: core::fmt::Display,
{
    type Response = Response<HttpBody>;
    type Error = Infallible;
    type Future = ResponseFuture;

    fn call(&self, req: Request<B>) -> Self::Future {
        let path = req.uri().path();
        if *req.method() == Method::GET && path == "/" {
            return self.serve_root_page(&req);
        }
        if *req.method() == Method::POST && path == "/demo-login" {
            return self.serve_demo_login(&req);
        }
        if *req.method() == Method::POST && path == "/demo-logout" {
            return self.serve_demo_logout(&req);
        }
        if *req.method() == Method::GET && path.starts_with(PKG_PREFIX) {
            return self.serve_pkg_asset(path);
        }
        if *req.method() == Method::GET && path == "/issues" {
            return self.serve_issues_page(&req);
        }
        if path.starts_with(TEST_ROUTE_PREFIX) {
            if !self.enable_test_hooks {
                return ready(error_response(RpcStatus::NotFound, "not found"));
            }
            return ready(handle_test_route(
                self.rpc.server().state(),
                req.method(),
                path,
                req.uri().query(),
            ));
        }
        let rpc = hyper::service::Service::call(&self.rpc, req);
        Box::pin(async move { rpc.await.map(|resp| resp.map(HttpBody::from)) })
    }
}

impl<B> tower_service::Service<Request<B>> for IssuesHttpService
where
    B: Body<Data = Bytes> + Unpin + Send + 'static,
    B::Error: core::fmt::Display,
{
    type Response = Response<HttpBody>;
    type Error = Infallible;
    type Future = ResponseFuture;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        hyper::service::Service::call(self, req)
    }
}
