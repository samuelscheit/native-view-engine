//! Progressive reusable [`Form`], [`FormField`], and [`FormSubmitButton`] components wrapping
//! [`crate::form::FormState`] and [`crate::form_draft::FormController`].
//!
//! - On Web without JS/WASM: emits semantic `<form method="post" action="...">` with native
//!   `<input name="...">` and `<button type="submit">` for zero-JS PRG (`303 See Other`) or
//!   Stage-2 `target="nve-sink"` (`204 No Content`) server actions.
//! - On live WASM and Native (Skia/Headless): validates fields in-app via [`FormController`],
//!   automatically saves/restores dirty drafts when `persist_key` is set, and clears saved drafts
//!   automatically upon verified submission.

use std::collections::BTreeMap;

use dioxus_core::{provide_context, try_consume_context, use_hook, EventHandler};
use dioxus_core_macro::Props;
use dioxus_signals::CopyValue;
use nve_dioxus::prelude::*;

use crate::components::actions::{Button, ButtonSize, ButtonVariant};
use crate::components::forms::Input;
use crate::form_draft::FormController;
use crate::components::roles::error_text;
use crate::surface::{SurfaceStyleExt, TypedTextStyle};
use crate::theme::use_theme;

/// HTTP submission method emitted on `<form method="...">`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormMethod {
    Get,
    #[default]
    Post,
}

impl FormMethod {
    #[inline]
    pub const fn as_html_method(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
        }
    }
}

/// Validated field snapshot delivered to [`Form`]'s `on_submit` handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSubmitPayload {
    pub values: BTreeMap<String, String>,
    pub persist_key: Option<String>,
}

#[derive(Clone, Copy)]
struct FormSubmitContext {
    controller: Signal<Signal<FormController>>,
    on_submit: CopyValue<Option<EventHandler<FormSubmitPayload>>>,
    clear_draft_on_submit: CopyValue<bool>,
}

/// Reads the enclosing [`Form`]'s active [`FormController`] signal when called inside `<Form>`,
/// staying synchronized if the parent swaps the `controller` prop.
pub fn use_form_context() -> Option<Signal<FormController>> {
    try_consume_context::<FormSubmitContext>().map(|ctx| *ctx.controller.read())
}

fn dispatch_form_submit(ctx: FormSubmitContext) {
    let mut ctrl = *ctx.controller.peek();
    let clear_draft = *ctx.clear_draft_on_submit.read();
    let handler = *ctx.on_submit.read();
    let persist_key = ctrl.read().persist_key().map(str::to_owned);
    let values = if clear_draft {
        ctrl.write().submit_with(|_| Ok(())).ok()
    } else {
        ctrl.write().validate_for_submit()
    };
    if let Some(values) = values
        && let Some(handler) = handler
    {
        handler.call(FormSubmitPayload { values, persist_key });
    }
}

/// Progressive, accessible `<Form>` component emitting a real HTML5 `<form>` on Web and wrapping
/// [`FormController`] validation + automatic draft persistence across Web and Native.
#[component]
pub fn Form(
    #[props(default)] method: FormMethod,
    #[props(default)] action: Option<String>,
    #[props(default)] target: Option<String>,
    #[props(default)] novalidate: bool,
    #[props(default)] persist_key: Option<String>,
    #[props(default)] controller: Option<Signal<FormController>>,
    #[props(default)] on_submit: Option<EventHandler<FormSubmitPayload>>,
    #[props(default = true)] clear_draft_on_submit: bool,
    #[props(default)] id: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let theme = use_theme();

    let has_external_ctrl = controller.is_some();
    let persist_key_init = persist_key.clone();
    let fallback_ctrl = use_signal(move || {
        if has_external_ctrl {
            FormController::new()
        } else if let Some(key) = persist_key_init {
            let mut c = FormController::with_default_persistence(key);
            let _ = c.restore_draft();
            c
        } else {
            FormController::new()
        }
    });
    let active_ctrl = controller.unwrap_or(fallback_ctrl);

    let mut last_restored_ctrl = use_hook(|| CopyValue::new(Option::<Signal<FormController>>::None));
    if has_external_ctrl && *last_restored_ctrl.read() != Some(active_ctrl) {
        last_restored_ctrl.set(Some(active_ctrl));
        if !active_ctrl.peek().has_restored_draft() && !active_ctrl.peek().is_dirty() {
            let _ = active_ctrl.write_unchecked().restore_draft();
        }
    }

    let ctrl_sig = use_signal(|| active_ctrl);
    if *ctrl_sig.peek() != active_ctrl {
        *ctrl_sig.write_unchecked() = active_ctrl;
    }
    let mut on_submit_slot = use_hook(|| CopyValue::new(on_submit));
    on_submit_slot.set(on_submit);
    let mut clear_draft_slot = use_hook(|| CopyValue::new(clear_draft_on_submit));
    clear_draft_slot.set(clear_draft_on_submit);

    let ctx = FormSubmitContext {
        controller: ctrl_sig,
        on_submit: on_submit_slot,
        clear_draft_on_submit: clear_draft_slot,
    };
    use_hook(|| {
        provide_context(ctx);
    });

    let form_style = TypedStyle::new()
        .flex_col()
        .gap(theme.spacing.md)
        .width_pct(100.0)
        .merge_opt(nve_style.as_ref());
    let form_err_style = TypedStyle::new().text_style(TypedTextStyle {
        color: error_text(&theme),
        font_size: theme.typography.ui.size,
        font_weight: 500,
        text_align: TextAlign::Left,
        opacity: 1.0,
    });

    let ctrl_read = active_ctrl.read();
    let form_table = super::html_tables::html_form_table(
        method.as_html_method().to_owned(),
        action,
        target,
        novalidate,
        ctrl_read.is_dirty(),
        ctrl_read.has_restored_draft(),
    );
    let form_error = ctrl_read.form_error().map(str::to_owned);
    drop(ctrl_read);

    rsx! {
        view {
            id: id,
            test_id: test_id,
            nve_table: form_table,
            accessibility_role: "form",
            nve_style: form_style,
            onsubmit: move |_: SubmitEvent| {
                dispatch_form_submit(ctx);
            },
            if let Some(ref err_msg) = form_error {
                text {
                    test_id: "form-error",
                    accessibility_role: "alert",
                    aria_live: "assertive",
                    nve_style: form_err_style,
                    "{err_msg}"
                }
            }
            {children}
        }
    }
}

/// Bound text input inside [`Form`]: emits `<input name="...">`, syncs with [`FormController`],
/// saves dirty drafts automatically, marks touched on blur, and displays visible validation errors.
#[component]
pub fn FormField(
    name: String,
    #[props(default)] initial_value: Option<String>,
    #[props(default)] label: Option<String>,
    #[props(default)] placeholder: Option<String>,
    #[props(default)] helper_text: Option<String>,
    #[props(default)] input_type: Option<String>,
    #[props(default)] secure: bool,
    #[props(default)] multiline: bool,
    #[props(default)] required: bool,
    #[props(default)] disabled: bool,
    #[props(default)] read_only: bool,
    #[props(default)] autocomplete: Option<AutofillHint>,
    #[props(default)] input_mode: Option<InputMode>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
) -> Element {
    let ctx = try_consume_context::<FormSubmitContext>();
    let active_ctrl = ctx.map(|c| *c.controller.read());
    if let Some(ctrl) = active_ctrl
        && ctrl.peek().state().value::<String>(&name).is_none()
    {
        let init = initial_value.as_deref().unwrap_or("");
        ctrl.write_unchecked().ensure_text_field(&name, init);
    }
    let value = active_ctrl
        .map(|ctrl| ctrl.read().text_value(&name))
        .unwrap_or_default();
    let error = active_ctrl.and_then(|ctrl| ctrl.read().visible_error(&name).map(str::to_owned));
    let field_id = test_id.clone().unwrap_or_else(|| format!("field-{name}"));
    let name_change = name.clone();
    let name_blur = name.clone();

    rsx! {
        Input {
            id: field_id.clone(),
            test_id: test_id.unwrap_or(field_id),
            name: name,
            input_type: input_type,
            required: required,
            label: label,
            placeholder: placeholder,
            helper_text: helper_text,
            error: error,
            value: value,
            secure: secure,
            multiline: multiline,
            disabled: disabled,
            read_only: read_only,
            autocomplete: autocomplete,
            input_mode: input_mode,
            nve_style: nve_style,
            on_change: move |next: String| {
                if let Some(mut ctrl) = ctx.map(|x| *x.controller.peek()) {
                    ctrl.write().set_text(&name_change, next);
                }
            },
            on_blur: move |_| {
                if let Some(mut ctrl) = ctx.map(|x| *x.controller.peek()) {
                    let _ = ctrl.write().touch(&name_blur);
                }
            },
            on_submit: move |_| {
                if let Some(c) = ctx {
                    dispatch_form_submit(c);
                }
            },
        }
    }
}

/// Hidden form parameter inside [`Form`]: emits `<input type="hidden" name="..." value="...">`
/// for 0-JS HTML submissions while synchronizing its value into [`FormController`] for native and
/// hydrated WASM submissions.
#[component]
pub fn FormHiddenField(name: String, value: String) -> Element {
    let ctx = try_consume_context::<FormSubmitContext>();
    if let Some(ctrl) = ctx.map(|c| *c.controller.read())
        && ctrl.peek().state().value::<String>(&name).map(String::as_str) != Some(value.as_str())
    {
        ctrl.write_unchecked()
            .register_text_field(name.clone(), value.clone());
    }
    rsx! {
        textinput {
            nve_table: super::html_tables::html_form_control_name(Some(name)),
            input_type: "hidden",
            value: "{value}",
        }
    }
}

/// Submit button inside [`Form`] emitting `<button type="submit" name="..." value="...">` on Web
/// and triggering [`FormController`] validation + submission on Native/Headless press.
///
/// On `wasm32`, `<button type="submit">` natively triggers the enclosing `<form onsubmit>` event
/// once; `on_press` submission dispatch is therefore gated to non-`wasm32` targets so clicks never
/// double-submit on Web.
#[component]
pub fn FormSubmitButton(
    #[props(default)] variant: ButtonVariant,
    #[props(default)] size: ButtonSize,
    #[props(default)] loading: bool,
    #[props(default)] disabled: bool,
    #[props(default)] name: Option<String>,
    #[props(default)] value: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    let ctx = try_consume_context::<FormSubmitContext>();

    rsx! {
        Button {
            variant: variant,
            size: size,
            loading: loading,
            disabled: disabled,
            button_type: "submit".to_owned(),
            name: name,
            value: value,
            test_id: test_id,
            nve_style: nve_style,
            on_press: move |_| {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(c) = ctx {
                    dispatch_form_submit(c);
                }
            },
            {children}
        }
    }
}
