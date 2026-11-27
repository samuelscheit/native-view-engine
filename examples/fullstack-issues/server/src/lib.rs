//! Reference fullstack issue-tracker server library wiring `nve-rpc`, `nve-server`, and
//! `nve-http-hyper`.

pub mod auth;
pub mod store;

use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Body;
use nve_example_issues_api::{
    AddComment, AddCommentInput, GetIssue, IssueFilter, IssueId, ListComments, ListIssues,
    RenameIssue, RenameIssueInput,
};
use nve_http_hyper::NveHyperService;
use nve_server::{
    protocol_error_response, AuthResolver, CsrfPolicy, Cx, RpcRegistry, Server, ServerConfig,
};

use crate::auth::{Actor, IssuesAuth};
use crate::store::IssueStore;

pub type IssuesServer = Server<IssueStore, Actor, IssuesAuth>;

/// Builds the sans-I/O `IssuesServer` registering all 5 procedures (with static cache tags /
/// invalidations declared in `procedures!`, plus built-in progressive form routes for
/// `/issues/rename` and `/issues/comment`).
pub fn build_issues_server<I, S>(store: IssueStore, allowed_origins: I) -> IssuesServer
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

    let config = ServerConfig::default()
        .with_form_route::<RenameIssue>("/issues/rename", "/issues")
        .with_form_route::<AddComment>("/issues/comment", "/issues");

    Server::new(
        store,
        reg.build(),
        IssuesAuth,
        CsrfPolicy::allow_origins(allowed_origins),
        config,
    )
}

/// Thin composite HTTP service serving `GET /issues` (the 0-JS `303 See Other` landing page) and
/// delegating `/rpc/*`, `/issues/rename`, and `/issues/comment` directly to [`NveHyperService`].
#[derive(Clone)]
pub struct IssuesHttpService {
    rpc: NveHyperService<IssueStore, Actor, IssuesAuth>,
}

impl IssuesHttpService {
    pub fn new(server: IssuesServer) -> Self {
        Self {
            rpc: NveHyperService::new(server),
        }
    }

    fn render_issues_page(&self) -> Response<Full<Bytes>> {
        let issues = self.rpc.server().state().list(&IssueFilter::default());
        let mut html = String::from("<!doctype html><html><body><h1>Issues</h1><ul>");
        for issue in &issues {
            html.push_str("<li>#");
            html.push_str(&issue.id.0.to_string());
            html.push_str(": ");
            push_html_escaped(&issue.title, &mut html);
            html.push_str("</li>");
        }
        html.push_str("</ul></body></html>");
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Full::new(Bytes::from(html)))
            .expect("static HTML response builds cleanly")
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
