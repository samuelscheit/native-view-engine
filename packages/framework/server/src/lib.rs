#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod auth;
pub mod config;
pub mod csrf;
pub mod direct;
pub mod registry;
pub mod response;
pub mod server;

pub use auth::{
    extract_bearer_token_strict, extract_cookie_value, extract_credential, make_bearer_header,
    AnonymousAuth, AuthCredential, AuthError, AuthResolver,
};
pub use config::{FormRoute, ServerConfig};
pub use csrf::{
    build_session_cookie, validate_same_origin_redirect, CsrfPolicy, CsrfVerdict, PolicyRejection,
};
pub use direct::DirectTransport;
pub use registry::{Cx, ProcMeta, RpcRegistry, RpcRegistryBuilder};
pub use response::{parse_rpc_http_reply, protocol_error_response, FormOutcome, PreflightError};
pub use server::{PreparedRequest, Server};
