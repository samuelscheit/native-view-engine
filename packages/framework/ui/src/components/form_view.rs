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
use nve_dioxus::prelude::*;

use crate::components::actions::{Button, ButtonSize, ButtonVariant};
use crate::components::forms::Input;
use crate::form_draft::FormController;
use crate::style_buf::TypedStyleBuilder;
use crate::theme::use_theme_rgba;

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
    controller: Signal<FormController>,
    on_submit: Option<EventHandler<FormSubmitPayload>>,
}

/// Reads the enclosing [`Form`]'s [`FormController`] signal when called inside `<Form>`.
pub fn use_form_context() -> Option<Signal<FormController>> {
    try_consume_context::<FormSubmitContext>().map(|ctx| ctx.controller)
}

fn dispatch_form_submit(
    mut ctrl: Signal<FormController>,
    on_submit: Option<EventHandler<FormSubmitPayload>>,
) {
    let persist_key = ctrl.read().persist_key().map(str::to_owned);
    let outcome = ctrl.write().submit_with(|_| Ok(()));
    if let Ok(values) = outcome
        && let Some(handler) = on_submit
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
    #[props(default)] id: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let (theme, _colors) = use_theme_rgba();

    let persist_key_init = persist_key.clone();
    let fallback_ctrl = use_signal(move || match persist_key_init {
        Some(key) => {
            let mut c = FormController::with_default_persistence(key);
            let _ = c.restore_draft();
            c
        }
        None => FormController::new(),
    });
    let active_ctrl = controller.unwrap_or(fallback_ctrl);

    let did_restore = use_signal(|| false);
    if !*did_restore.peek() {
        *did_restore.write_unchecked() = true;
        if controller.is_some() {
            let _ = active_ctrl.write_unchecked().restore_draft();
        }
    }

    use_hook(|| {
        provide_context(FormSubmitContext {
            controller: active_ctrl,
            on_submit,
        });
    });

    let form_style = TypedStyleBuilder::new()
        .flex_col()
        .gap(theme.spacing.md)
        .width_pct(100.0)
        .finish();

    let method_str = method.as_html_method();
    let dirty_flag = if active_ctrl.read().is_dirty() {
        "true"
    } else {
        "false"
    };
    let restored_flag = if active_ctrl.read().has_restored_draft() {
        "true"
    } else {
        "false"
    };

    rsx! {
        form {
            id: id,
            test_id: test_id,
            method: "{method_str}",
            action: action,
            target: target,
            novalidate: novalidate,
            accessibility_role: "form",
            "data-nve-dirty": "{dirty_flag}",
            "data-nve-draft-restored": "{restored_flag}",
            nve_style: form_style,
            style: style,
            onsubmit: move |_: SubmitEvent| {
                dispatch_form_submit(active_ctrl, on_submit);
            },
            {children}
        }
    }
}

/// Bound text input inside [`Form`]: emits `<input name="...">`, syncs with [`FormController`],
/// saves dirty drafts automatically, marks touched on blur, and displays visible validation errors.
#[component]
pub fn FormField(
    name: String,
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
    #[props(default)] style: Option<String>,
) -> Element {
    let ctx = try_consume_context::<FormSubmitContext>();
    let value = ctx
        .map(|c| c.controller.read().text_value(&name))
        .unwrap_or_default();
    let error = ctx.and_then(|c| c.controller.read().visible_error(&name).map(str::to_owned));
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
            style: style,
            on_change: move |next: String| {
                if let Some(mut c) = ctx.map(|x| x.controller) {
                    c.write().set_text(&name_change, next);
                }
            },
            on_blur: move |_| {
                if let Some(mut c) = ctx.map(|x| x.controller) {
                    let _ = c.write().touch(&name_blur);
                }
            },
            on_submit: move |_| {
                if let Some(c) = ctx {
                    dispatch_form_submit(c.controller, c.on_submit);
                }
            },
        }
    }
}

/// Submit button inside [`Form`] emitting `<button type="submit" name="..." value="...">` on Web
/// and triggering [`FormController`] validation + submission on Native/Headless press.
#[component]
pub fn FormSubmitButton(
    #[props(default)] variant: ButtonVariant,
    #[props(default)] size: ButtonSize,
    #[props(default)] loading: bool,
    #[props(default)] disabled: bool,
    #[props(default)] name: Option<String>,
    #[props(default)] value: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
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
            style: style,
            on_press: move |_| {
                if let Some(c) = ctx {
                    dispatch_form_submit(c.controller, c.on_submit);
                }
            },
            {children}
        }
    }
}
