#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod config;
#[cfg(target_arch = "wasm32")]
mod js_fetch;
pub mod transport;

pub use config::{CredentialsMode, FetchConfig, DEFAULT_FETCH_TIMEOUT_MS};
pub use transport::{
    attach_browser_lifecycle, browser_sleep_ms, BrowserClock, FetchTransport, LifecycleGuard,
};
