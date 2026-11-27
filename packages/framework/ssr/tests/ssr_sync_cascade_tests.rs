//! A three-hop synchronous render cascade (leaf throws -> inner `ErrorBoundary` fallback whose
//! `on_error` sets an ancestor signal -> ancestor re-renders a throwing banner -> outer
//! `ErrorBoundary` fallback) settles inside the SSR shell: no intermediate state, no slot.

mod support;

use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_query_dioxus::prelude::*;
use nve_ssr::{SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};

use support::*;

#[component]
fn ThrowingLeaf() -> Element {
    Err(ui_error("leaf failed"))?;
    rsx! { text { "unreachable" } }
}

#[component]
fn StatusBanner(status: String) -> Element {
    if !status.is_empty() {
        Err(ui_error(format!("banner rejected {status}")))?;
    }
    rsx! { text { "status ok" } }
}

#[component]
fn CascadeApp() -> Element {
    let mut status = use_signal(String::new);
    rsx! {
        view {
            text { "sibling stays" }
            ErrorBoundary {
                fallback: |r: UiErrorReport, _| rsx! { text { "OUTER:{r.message}" } },
                StatusBanner { status: status() }
            }
            ErrorBoundary {
                fallback: |r: UiErrorReport, _| rsx! { text { "INNER:{r.message}" } },
                on_error: move |r: UiErrorReport| status.set(r.message),
                ThrowingLeaf {}
            }
        }
    }
}

#[tokio::test]
async fn three_hop_sync_error_cascade_settles_in_ssr_shell() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");
    let resp = pool
        .render_streaming(
            SsrPageOptions::new("Cascade", web_backend),
            StreamPlan::default(),
            erased_ssr_query_client(ControlledTransport::default()),
            |c| DioxusSurface::new(1, CascadeApp).with_context(c),
        )
        .await
        .expect("cascade page renders");
    let html = collect_all_html(resp.into_body()).await;
    assert!(html.contains("sibling stays"), "{html}");
    assert!(html.contains("INNER:leaf failed"), "{html}");
    assert!(html.contains("OUTER:banner rejected leaf failed"), "{html}");
    assert!(
        !html.contains("status ok") && !html.contains("unreachable"),
        "{html}"
    );
    assert!(
        !html.contains("data-nve-slot"),
        "sync cascade must not stream: {html}"
    );
}
