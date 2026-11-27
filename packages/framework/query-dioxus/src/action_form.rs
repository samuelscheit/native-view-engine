//! Progressive `<ActionForm<P>>` uniting 0-JS HTML `<form method="post">` PRG fallback with
//! hydrated Web WASM and Native local `FormDecode` validation + CBOR RPC submission.

use core::marker::PhantomData;

use dioxus_core::{spawn, Element, EventHandler};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{Signal, WritableExt};
use nve_dioxus::use_signal;
use nve_query::Shared;
use nve_rpc::{
    Action, CborCodec, DomainStatus, FieldErrors, FormDecode, FormPairs, Procedure, RpcCodec,
    RpcError,
};
use nve_ui::{Form, FormController, FormMethod, FormSubmitPayload};

use crate::hooks::use_action;

fn field_errors_to_pairs(errs: &FieldErrors) -> Vec<(String, String)> {
    errs.errors
        .iter()
        .map(|e| (e.field.clone(), e.message.clone()))
        .collect()
}

/// Progressive typed form bound to RPC [`Action`] procedure `P`:
/// - On 0-JS browsers: renders semantic `<form method="post" action="...">` hitting `nve-server`'s
///   `FormRoute` (`303 See Other` / `422 text/plain`).
/// - On hydrated Web WASM and Native (Skia/iOS/Android/Headless): validates locally via
///   `P::Args::from_form(&pairs)` (0ms inline field errors, zero network round-trip), submits typed
///   CBOR over `use_action::<P>()` (`POST /rpc/<P::NAME>`), preserves drafts on error, applies
///   server `DomainStatus::field_errors()` inline, and commits clean baseline + clears drafts ONLY
///   when the action succeeds.
#[component]
pub fn ActionForm<P>(
    action: String,
    #[props(default)] persist_key: Option<String>,
    #[props(default)] controller: Option<Signal<FormController>>,
    #[props(default)] reset_on_success: bool,
    #[props(default)] on_success: Option<EventHandler<P::Output>>,
    #[props(default)] on_error: Option<EventHandler<Shared<RpcError<P::Error>>>>,
    #[props(default)] id: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
    #[props(default)] _marker: PhantomData<fn() -> P>,
) -> Element
where
    P: Procedure<Effect = Action> + 'static,
    P::Args: FormDecode,
    P::Output: Clone + PartialEq,
    P::Error: DomainStatus + core::fmt::Debug + PartialEq,
    CborCodec: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    let action_hook = use_action::<P>();
    let persist_init = persist_key.clone();
    let fallback_ctrl = use_signal(move || match persist_init {
        Some(key) => {
            let mut c = FormController::with_default_persistence(key);
            let _ = c.restore_draft();
            c
        }
        None => FormController::new(),
    });
    let mut active_ctrl = controller.unwrap_or(fallback_ctrl);

    rsx! {
        Form {
            id: id,
            test_id: test_id,
            method: FormMethod::Post,
            action: Some(action),
            novalidate: true,
            persist_key: persist_key,
            controller: Some(active_ctrl),
            clear_draft_on_submit: false,
            style: style,
            on_submit: move |payload: FormSubmitPayload| {
                if action_hook.is_pending() {
                    return;
                }
                let pairs = FormPairs::from_pairs(payload.values.into_iter().collect());
                match P::Args::from_form(&pairs) {
                    Err(field_errs) => {
                        let list = field_errors_to_pairs(&field_errs);
                        active_ctrl.write().apply_server_errors(&list);
                    }
                    Ok(typed_args) => {
                        spawn(async move {
                            match action_hook.call(typed_args).await {
                                Ok(reply) => {
                                    if let Ok(mut ctrl) = active_ctrl.try_write() {
                                        if reset_on_success {
                                            ctrl.reset();
                                        } else {
                                            ctrl.mark_submit_succeeded();
                                        }
                                    }
                                    if let Some(cb) = on_success {
                                        cb.call(reply.value);
                                    }
                                }
                                Err(err) => {
                                    if let Ok(mut ctrl) = active_ctrl.try_write() {
                                        match err.as_ref() {
                                            RpcError::Domain { error, .. }
                                                if error.field_errors().is_some() =>
                                            {
                                                if let Some(fe) = error.field_errors() {
                                                    let list = field_errors_to_pairs(fe);
                                                    ctrl.apply_server_errors(&list);
                                                }
                                            }
                                            other => {
                                                ctrl.set_form_error(Some(other.to_string()));
                                            }
                                        }
                                    }
                                    if let Some(cb) = on_error {
                                        cb.call(err);
                                    }
                                }
                            }
                        });
                    }
                }
            },
            {children}
        }
    }
}
