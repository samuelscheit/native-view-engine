//! Reference fullstack issue-tracker server library wiring `nve-rpc`, `nve-server`,
//! `nve-http-hyper`, and streaming `nve-ssr` of the shared `nve-example-issues-client` UI.

pub mod assets;
pub mod auth;
mod service;
pub mod ssr;
pub mod store;
mod test_hooks;

use nve_example_issues_api::{
    AddComment, AddCommentInput, CloseIssue, CloseIssueInput, CreateIssue, CreateIssueInput,
    GetIssue, GetWorkspaceStats, IssueFilter, IssueId, ListComments, ListIssues, RenameIssue,
    RenameIssueInput, StatsRequest,
};
use nve_server::{CorsPolicy, CsrfPolicy, Cx, RpcRegistry, Server, ServerConfig};

use crate::auth::{Actor, IssuesAuth};
use crate::store::IssueStore;

pub use service::IssuesHttpService;

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
        cx.state.wait_comments_hook().await;
        cx.state.list_comments(id)
    });

    reg.register::<GetWorkspaceStats, _, _>(
        |cx: Cx<IssueStore, Actor>, _req: StatsRequest| async move {
            cx.state.wait_stats_hook().await;
            Ok(cx.state.stats())
        },
    );

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
