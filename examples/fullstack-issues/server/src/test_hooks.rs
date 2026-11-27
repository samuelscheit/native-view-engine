//! Opt-in deterministic streaming E2E control routes (`/_test/*`), mounted only when
//! [`crate::IssuesHttpService::with_test_hooks`] is enabled (`NVE_ISSUES_TEST_HOOKS=1`).

use http::{Method, Response};
use nve_example_issues_api::parse_u64_query_param;
use nve_http_hyper::HttpBody;
use nve_rpc::RpcStatus;

use crate::assets::{error_response, static_response};
use crate::store::IssueStore;

pub const TEST_ROUTE_PREFIX: &str = "/_test/";

/// - `POST /_test/stream-control?stats_delay_ms=<u64>&comments_delay_ms=<u64>&hold_comments=0|1`
///   updates only the given [`IssueStore`] stream hooks.
/// - `GET /_test/stream-status` reports `comments_waiting=<N>`.
pub fn handle_test_route(
    store: &IssueStore,
    method: &Method,
    path: &str,
    query: Option<&str>,
) -> Response<HttpBody> {
    const TEXT: &str = "text/plain; charset=utf-8";
    match (method, path.strip_prefix(TEST_ROUTE_PREFIX)) {
        (&Method::POST, Some("stream-control")) => {
            if let Some(ms) = parse_u64_query_param(query, "stats_delay_ms") {
                store.set_stats_delay_ms(ms);
            }
            if let Some(ms) = parse_u64_query_param(query, "comments_delay_ms") {
                store.set_comments_delay_ms(ms);
            }
            if let Some(hold) = parse_u64_query_param(query, "hold_comments") {
                store.set_comments_gate_open(hold == 0);
            }
            static_response(TEXT, "ok")
        }
        (&Method::GET, Some("stream-status")) => static_response(
            TEXT,
            format!("comments_waiting={}", store.comments_waiting()),
        ),
        _ => error_response(RpcStatus::NotFound, "unknown test route"),
    }
}
