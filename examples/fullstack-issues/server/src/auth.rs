//! Dual-platform demo auth resolver proving Web `HttpOnly` cookies and Native `Bearer` tokens
//! resolve cleanly into one shared `Actor` principal.

use http::HeaderMap;
use nve_rpc::BoxFuture;
use nve_server::{extract_credential, AuthCredential, AuthError, AuthResolver};

use crate::store::IssueStore;

pub const SESSION_COOKIE_NAME: &str = "nve_issues_session";
pub const DEMO_WEB_COOKIE_TOKEN: &str = "web-demo-session";
pub const DEMO_NATIVE_BEARER_TOKEN: &str = "native-demo-bearer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthTransportKind {
    Cookie,
    Bearer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub username: String,
    pub via: AuthTransportKind,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IssuesAuth;

impl IssuesAuth {
    fn resolve_sync(headers: &HeaderMap) -> Result<Actor, AuthError> {
        match extract_credential(headers, SESSION_COOKIE_NAME) {
            AuthCredential::Bearer(DEMO_NATIVE_BEARER_TOKEN) => Ok(Actor {
                username: "native-alice".to_owned(),
                via: AuthTransportKind::Bearer,
            }),
            AuthCredential::Cookie(DEMO_WEB_COOKIE_TOKEN) => Ok(Actor {
                username: "web-bob".to_owned(),
                via: AuthTransportKind::Cookie,
            }),
            AuthCredential::Bearer(_) | AuthCredential::Cookie(_) => {
                Err(AuthError::unauthenticated("invalid session credentials"))
            }
            AuthCredential::None => Err(AuthError::unauthenticated("authentication required")),
        }
    }
}

impl AuthResolver<IssueStore, Actor> for IssuesAuth {
    #[inline]
    fn try_resolve_sync(
        &self,
        _state: &IssueStore,
        headers: &HeaderMap,
    ) -> Option<Result<Actor, AuthError>> {
        Some(Self::resolve_sync(headers))
    }

    fn resolve<'a>(
        &'a self,
        _state: &'a IssueStore,
        headers: &'a HeaderMap,
    ) -> BoxFuture<'a, Result<Actor, AuthError>> {
        let resolved = Self::resolve_sync(headers);
        Box::pin(async move { resolved })
    }
}
