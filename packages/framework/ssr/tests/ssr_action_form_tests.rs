//! `nve-ssr` streams real `<ActionForm>` markup only through the app's `web_backend` builder:
//! 1. A settled page renders `<form action method>` (the 0-JS progressive-enhancement fallback).
//! 2. A pending `<QuerySuspense>` slot inside `<ActionForm>` fails closed with
//!    `InvalidSlotPlacement(<form>)` BEFORE headers (a DSD `<slot>` cannot live inside `<form>`).
//! 3. A bare `WebBackend::new` builder silently loses `action`/`method` (only the ARIA-role
//!    `<form>` survives), breaking the 0-JS submit path: why the builder is mandatory on
//!    `SsrPageOptions::new`.

mod support;

use minicbor::{Decode, Encode};
use nve_abi::SurfaceId;
use nve_backend_web::{StreamingError, WebBackend};
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_query_dioxus::prelude::*;
use nve_rpc::{FieldErrors, FormDecode, FormPairs, procedures};
use nve_ssr::{SsrError, SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};

use support::*;

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct SaveNoteInput {
    #[n(0)]
    body: String,
}

impl FormDecode for SaveNoteInput {
    fn from_form(form: &FormPairs) -> Result<Self, FieldErrors> {
        Ok(Self {
            body: form.get("body").unwrap_or("").to_owned(),
        })
    }
}

procedures! {
    pub action SaveNote = "ssr.test.v1/SaveNote" (SaveNoteInput) -> PanelData, PanelErr;
}

#[component]
fn SettledFormApp() -> Element {
    let client = dioxus_core::consume_context::<ErasedQueryClient>();
    rsx! {
        QueryClientProvider {
            client: client,
            ActionForm::<SaveNote> {
                action: "/notes/save".to_owned(),
                text { "Save note" }
            }
        }
    }
}

#[component]
fn PendingSlotInFormApp() -> Element {
    let client = dioxus_core::consume_context::<ErasedQueryClient>();
    rsx! {
        QueryClientProvider {
            client: client,
            ActionForm::<SaveNote> {
                action: "/notes/save".to_owned(),
                QuerySuspense {
                    id: Some("form-slot".into()),
                    fallback: rsx! { text { "Loading..." } },
                    SlowPanelInner {}
                }
            }
        }
    }
}

async fn render(
    pool: &SsrLocalPool,
    web_backend: fn() -> WebBackend,
    app: fn() -> Element,
    transport: ControlledTransport,
) -> Result<String, SsrError> {
    let resp = pool
        .render_streaming(
            SsrPageOptions::new("Forms", web_backend),
            // Bare `NveStreamSlot`s stay pending until the deadline by contract.
            StreamPlan::default().with_total_deadline(core::time::Duration::from_millis(50)),
            erased_ssr_query_client(transport),
            move |c| DioxusSurface::new(SurfaceId::ROOT, app).with_context(c),
        )
        .await?;
    Ok(collect_all_html(resp.into_body()).await)
}

#[tokio::test]
async fn action_form_streams_form_tag_and_pending_slot_inside_form_fails_before_headers() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");

    let html = render(&pool, web_backend, SettledFormApp, ControlledTransport::default())
        .await
        .expect("settled form page renders");
    assert!(html.contains("<form "), "{html}");
    assert!(html.contains("action=\"/notes/save\""), "{html}");
    assert!(html.contains("method=\"post\""), "{html}");
    assert!(html.contains("Save note"), "{html}");

    let gated = ControlledTransport::default();
    gated.gate("slow");
    let err = render(&pool, web_backend, PendingSlotInFormApp, gated)
        .await
        .expect_err("pending slot inside <ActionForm> must fail before headers");
    assert!(
        matches!(
            err,
            SsrError::Streaming(StreamingError::InvalidSlotPlacement(ref msg)) if msg.contains("<form>")
        ),
        "expected InvalidSlotPlacement(<form>), got {err:?}"
    );

    let bare = render(&pool, WebBackend::new, SettledFormApp, ControlledTransport::default())
        .await
        .expect("bare backend still renders");
    assert!(
        !bare.contains("action=\"/notes/save\""),
        "bare WebBackend::new has no HtmlForm plugin; the builder must stay mandatory: {bare}"
    );
}

#[component]
fn InputSlotFormApp() -> Element {
    let client = dioxus_core::consume_context::<ErasedQueryClient>();
    rsx! {
        QueryClientProvider {
            client: client,
            NveStreamSlot {
                slot_id: "draft",
                role: StreamSlotRole::Input,
                ActionForm::<SaveNote> {
                    action: "/notes/save".to_owned(),
                    text { "Save note" }
                }
            }
            NveStreamSlot { slot_id: "feed", text { "Feed" } }
        }
    }
}

#[tokio::test]
async fn input_stream_slot_role_reaches_ssr_html() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");
    let html = render(&pool, web_backend, InputSlotFormApp, ControlledTransport::default())
        .await
        .expect("input slot page renders");
    assert_eq!(html.matches("data-nve-slot-role=\"input\"").count(), 1, "{html}");
    assert_eq!(html.matches("data-nve-slot-role=\"output\"").count(), 1, "{html}");
    let input_at = html.find("data-nve-slot-role=\"input\"").expect("input role");
    let form_at = html.find("<form ").expect("form");
    assert!(input_at < form_at, "the input slot wraps the form: {html}");
}
