//! Browser `wasm32` leaf provider for `SERVICE_LINKING` (`0x0013`).
//!
//! Enforces `validate_url` plus a strict browser scheme allowlist (`http`, `https`, `mailto`,
//! `tel`) rejecting `javascript:`, `data:`, `file:`, and other dangerous schemes before calling
//! `window.open(url, '_blank', 'noopener,noreferrer')`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_LINKING;
use nve_host_services::{
    pack_typed, validate_url, CompletionSink, HandlerReply, HostServiceHandler, ServiceError,
};
pub use nve_linking::LinkingClient;
use nve_linking::{dispatch_linking_wire, LinkingBackend};

/// Allowed external URL schemes in a browser context (case-insensitive ASCII).
pub const ALLOWED_WEB_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

/// Returns `true` if `url` starts with an explicitly allowlisted browser URL scheme
/// (`http`, `https`, `mailto`, `tel`) with zero heap allocation.
#[inline]
pub fn is_allowed_web_url_scheme(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    ALLOWED_WEB_SCHEMES
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

/// Validates `url` structurally via [`validate_url`] and enforces [`ALLOWED_WEB_SCHEMES`],
/// rejecting `javascript:`, `data:`, `file:`, and all non-allowlisted schemes.
#[inline]
pub fn validate_web_url(url: &str) -> Result<(), ServiceError> {
    validate_url(url)?;
    if !is_allowed_web_url_scheme(url) {
        return Err(ServiceError::InvalidArg(
            "URL scheme not in browser allowlist (http, https, mailto, tel)".into(),
        ));
    }
    Ok(())
}

/// Browser [`LinkingBackend`] & [`HostServiceHandler`] for [`SERVICE_LINKING`].
#[derive(Debug, Default, Clone, Copy)]
pub struct WebLinking;

impl WebLinking {
    pub const fn new() -> Self {
        Self
    }
}

impl LinkingBackend for WebLinking {
    fn open_url(&self, url: &str) -> Result<(), ServiceError> {
        validate_web_url(url)?;
        #[cfg(target_arch = "wasm32")]
        {
            if wasm_impl::open_url(url) {
                Ok(())
            } else {
                Err(ServiceError::Unsupported)
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Err(ServiceError::Unsupported)
        }
    }

    fn can_open_url(&self, url: &str) -> Result<bool, ServiceError> {
        validate_url(url)?;
        Ok(is_allowed_web_url_scheme(url))
    }

    fn initial_url(&self) -> Result<Option<String>, ServiceError> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(wasm_impl::initial_url())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(None)
        }
    }
}

impl HostServiceHandler for WebLinking {
    fn service_id(&self) -> u32 {
        SERVICE_LINKING
    }

    fn handle(
        &self,
        method_id: u32,
        _request_id: u64,
        request: &[u8],
        _done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        dispatch_linking_wire(self, method_id, request).map(HandlerReply::ready)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn LinkingBackend>(self))
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = "
export function nve_web_open_url(url) {
  try {
    if (typeof window === 'undefined' || typeof window.open !== 'function') return false;
    window.open(url, '_blank', 'noopener,noreferrer');
    return true;
  } catch (_) {
    return false;
  }
}

export function nve_web_initial_url() {
  try {
    if (typeof window !== 'undefined' && window.location && typeof window.location.href === 'string') {
      return window.location.href;
    }
  } catch (_) {}
  return '';
}
")]
    extern "C" {
        fn nve_web_open_url(url: &str) -> bool;
        fn nve_web_initial_url() -> String;
    }

    pub(super) fn open_url(url: &str) -> bool {
        nve_web_open_url(url)
    }

    pub(super) fn initial_url() -> Option<String> {
        let href = nve_web_initial_url();
        (!href.is_empty()).then_some(href)
    }
}
