//! Semantic HTML5 tag & attribute resolution (`<dialog>`, `[popover]`, `command`/`commandfor`,
//! `<details>`/`<summary>`, `<form>`, canonical ARIA/tabindex/data-* attributes) shared across
//! SSR (`render_ssr`), mirror HTML (`render_html`), live `wasm32` DOM, and hydration.

use nve_abi::hash::HashMap;
use nve_abi::{ComponentKind, RawPropValue};

use crate::html::write_attr;
use crate::input_attrs::is_truthy_raw_prop;

/// Resolved semantic HTML5 tag metadata for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticTag {
    pub tag: &'static str,
    pub wrap_code: bool,
}

pub fn component_to_html_tag(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Root | ComponentKind::View | ComponentKind::ScrollView => "div",
        ComponentKind::Text => "span",
        ComponentKind::Image => "img",
        ComponentKind::TextInput => "input",
        ComponentKind::Pressable => "button",
        ComponentKind::SkiaCanvas => "canvas",
        ComponentKind::Custom => "div",
    }
}

/// Returns `true` if `key` can affect semantic HTML5 tag resolution.
#[inline]
pub fn is_semantic_tag_prop(key: &str) -> bool {
    matches!(
        key,
        "semanticTag"
            | "popover"
            | "accessibilityRole"
            | "accessibility_role"
            | "ariaLevel"
            | "aria-level"
            | "aria_level"
            | "href"
    )
}

fn parse_aria_level(raw_props: &HashMap<String, RawPropValue>) -> u8 {
    let val = raw_props
        .get("ariaLevel")
        .or_else(|| raw_props.get("aria-level"))
        .or_else(|| raw_props.get("aria_level"));
    let level = match val {
        Some(RawPropValue::I64(n)) => *n,
        Some(RawPropValue::F64(n)) if n.is_finite() => *n as i64,
        Some(RawPropValue::String(s)) => s.trim().parse::<i64>().unwrap_or(2),
        _ => 2,
    };
    level.clamp(1, 6) as u8
}

/// Resolves semantic HTML5 tags from `semanticTag`, `accessibilityRole`, `ariaLevel`, `popover`,
/// and `href`.
pub fn resolve_semantic_tag(
    kind: ComponentKind,
    raw_props: &HashMap<String, RawPropValue>,
) -> SemanticTag {
    if let Some(RawPropValue::String(explicit)) = raw_props.get("semanticTag") {
        match explicit.trim().to_ascii_lowercase().as_str() {
            "dialog" => return SemanticTag { tag: "dialog", wrap_code: false },
            "details" => return SemanticTag { tag: "details", wrap_code: false },
            "summary" => return SemanticTag { tag: "summary", wrap_code: false },
            "form" => return SemanticTag { tag: "form", wrap_code: false },
            "p" => return SemanticTag { tag: "p", wrap_code: false },
            "span" => return SemanticTag { tag: "span", wrap_code: false },
            "strong" => return SemanticTag { tag: "strong", wrap_code: false },
            "em" => return SemanticTag { tag: "em", wrap_code: false },
            "s" => return SemanticTag { tag: "s", wrap_code: false },
            "code" => return SemanticTag { tag: "code", wrap_code: false },
            "kbd" => return SemanticTag { tag: "kbd", wrap_code: false },
            "br" => return SemanticTag { tag: "br", wrap_code: false },
            "table" => return SemanticTag { tag: "table", wrap_code: false },
            "thead" => return SemanticTag { tag: "thead", wrap_code: false },
            "tbody" => return SemanticTag { tag: "tbody", wrap_code: false },
            "tfoot" => return SemanticTag { tag: "tfoot", wrap_code: false },
            "tr" => return SemanticTag { tag: "tr", wrap_code: false },
            "th" => return SemanticTag { tag: "th", wrap_code: false },
            "td" => return SemanticTag { tag: "td", wrap_code: false },
            "select" => return SemanticTag { tag: "select", wrap_code: false },
            _ => {}
        }
    }
    if let Some(RawPropValue::String(role)) = raw_props
        .get("accessibilityRole")
        .or_else(|| raw_props.get("accessibility_role"))
    {
        match role.trim().to_ascii_lowercase().as_str() {
            "heading" => {
                let tag = match parse_aria_level(raw_props) {
                    1 => "h1",
                    2 => "h2",
                    3 => "h3",
                    4 => "h4",
                    5 => "h5",
                    _ => "h6",
                };
                return SemanticTag { tag, wrap_code: false };
            }
            "navigation" => return SemanticTag { tag: "nav", wrap_code: false },
            "main" => return SemanticTag { tag: "main", wrap_code: false },
            "article" => return SemanticTag { tag: "article", wrap_code: false },
            "complementary" => return SemanticTag { tag: "aside", wrap_code: false },
            "banner" => return SemanticTag { tag: "header", wrap_code: false },
            "contentinfo" => return SemanticTag { tag: "footer", wrap_code: false },
            "form" => return SemanticTag { tag: "form", wrap_code: false },
            "dialog" if !raw_props.contains_key("popover") => {
                return SemanticTag { tag: "dialog", wrap_code: false };
            }
            "link" => {
                if matches!(raw_props.get("href"), Some(RawPropValue::String(h)) if !h.trim().is_empty()) {
                    return SemanticTag { tag: "a", wrap_code: false };
                }
            }
            "code" => return SemanticTag { tag: "code", wrap_code: false },
            "codeblock" => return SemanticTag { tag: "pre", wrap_code: true },
            _ => {}
        }
    }
    SemanticTag {
        tag: component_to_html_tag(kind),
        wrap_code: false,
    }
}

/// Tag-only metadata props that never appear as HTML attributes on DOM elements.
#[inline]
pub fn is_tag_only_raw_prop(key: &str) -> bool {
    matches!(key, "semanticTag" | "ariaLevel" | "aria-level" | "aria_level")
}

/// Canonical HTML attribute name for camelCase/snake_case ARIA, focus, and semantic HTML5 keys.
pub fn canonical_html_attr_name(key: &str) -> Option<&'static str> {
    match key {
        "closedBy" | "closed_by" | "closedby" => Some("closedby"),
        "commandFor" | "command_for" | "commandfor" => Some("commandfor"),
        "popoverTarget" | "popover_target" | "popovertarget" => Some("popovertarget"),
        "popoverTargetAction" | "popover_target_action" | "popovertargetaction" => {
            Some("popovertargetaction")
        }
        "noValidate" | "no_validate" | "novalidate" => Some("novalidate"),
        "tabIndex" | "tab_index" | "tab-index" | "tabindex" => Some("tabindex"),
        "ariaChecked" | "aria_checked" | "aria-checked" => Some("aria-checked"),
        "ariaSelected" | "aria_selected" | "aria-selected" => Some("aria-selected"),
        "ariaExpanded" | "aria_expanded" | "aria-expanded" => Some("aria-expanded"),
        "ariaModal" | "aria_modal" | "aria-modal" => Some("aria-modal"),
        "ariaLive" | "aria_live" | "aria-live" => Some("aria-live"),
        "ariaControls" | "aria_controls" | "aria-controls" => Some("aria-controls"),
        "ariaLabelledBy" | "aria_labelledby" | "aria-labelledby" => Some("aria-labelledby"),
        "ariaDescribedBy" | "aria_describedby" | "aria-describedby" => Some("aria-describedby"),
        "ariaInvalid" | "aria_invalid" | "aria-invalid" => Some("aria-invalid"),
        "ariaOrientation" | "aria_orientation" | "aria-orientation" => Some("aria-orientation"),
        "ariaActiveDescendant" | "aria_activedescendant" | "aria-activedescendant" => {
            Some("aria-activedescendant")
        }
        _ => None,
    }
}

fn write_string_prop_if_present(
    raw_props: &HashMap<String, RawPropValue>,
    keys: &[&str],
    html_attr: &str,
    out: &mut String,
) {
    for &k in keys {
        if let Some(val) = raw_props.get(k) {
            match val {
                RawPropValue::String(s) if !s.trim().is_empty() => {
                    write_attr(html_attr, s.trim(), out);
                    return;
                }
                RawPropValue::I64(n) => {
                    let mut buf = String::new();
                    nve_abi::num::write_i64(&mut buf, *n);
                    write_attr(html_attr, &buf, out);
                    return;
                }
                RawPropValue::Bool(true) if html_attr == "popover" => {
                    write_attr("popover", "auto", out);
                    return;
                }
                _ => return,
            }
        }
    }
}

fn write_bool_attr_if_truthy(
    raw_props: &HashMap<String, RawPropValue>,
    keys: &[&str],
    html_attr: &str,
    out: &mut String,
) {
    for &k in keys {
        if let Some(val) = raw_props.get(k) {
            if is_truthy_raw_prop(val) {
                out.push(' ');
                out.push_str(html_attr);
            }
            return;
        }
    }
}

/// Emits native HTML5 semantic attributes (`id`, `slot`, `<dialog>` `open`/`closedby`,
/// `command`/`commandfor`, `popover`/`popovertarget`/`popovertargetaction`, `<details>` `open`,
/// `<form>` `method`/`action`/`target`/`novalidate`, `<button>` `type`/`name`/`value`,
/// `<input>` `name`/`type`/`required`, canonical `aria-*`, `tabindex`, and `data-*`) shared by
/// `render_ssr` and `render_html` for guaranteed SSR/mirror/live DOM parity.
pub(crate) fn write_semantic_html_attrs(
    kind: ComponentKind,
    tag: &str,
    raw_props: &HashMap<String, RawPropValue>,
    out: &mut String,
) {
    write_string_prop_if_present(raw_props, &["id"], "id", out);
    write_string_prop_if_present(raw_props, &["slot"], "slot", out);

    // `<dialog>` and `<details>` open state & closedby behavior.
    if matches!(tag, "dialog" | "details") {
        write_bool_attr_if_truthy(raw_props, &["open"], "open", out);
    }
    if tag == "dialog" {
        write_string_prop_if_present(raw_props, &["closedby", "closedBy", "closed_by"], "closedby", out);
    }

    // Popover API attributes.
    if let Some(pop_val) = raw_props.get("popover") {
        match pop_val {
            RawPropValue::String(s) => {
                let t = s.trim();
                if t.is_empty() {
                    write_attr("popover", "auto", out);
                } else {
                    write_attr("popover", t, out);
                }
            }
            RawPropValue::Bool(true) => write_attr("popover", "auto", out),
            _ => {}
        }
    }
    write_string_prop_if_present(
        raw_props,
        &["popovertarget", "popoverTarget", "popover_target"],
        "popovertarget",
        out,
    );
    write_string_prop_if_present(
        raw_props,
        &["popovertargetaction", "popoverTargetAction", "popover_target_action"],
        "popovertargetaction",
        out,
    );

    // Invoker Commands API (`command` / `commandfor`).
    write_string_prop_if_present(raw_props, &["command"], "command", out);
    write_string_prop_if_present(
        raw_props,
        &["commandfor", "commandFor", "command_for"],
        "commandfor",
        out,
    );

    // Native `<form>` attributes.
    if tag == "form" {
        write_string_prop_if_present(raw_props, &["method"], "method", out);
        write_string_prop_if_present(raw_props, &["action"], "action", out);
        write_string_prop_if_present(raw_props, &["target"], "target", out);
        write_string_prop_if_present(raw_props, &["enctype"], "enctype", out);
        write_bool_attr_if_truthy(raw_props, &["novalidate", "noValidate", "no_validate"], "novalidate", out);
    }

    // Native form-associated attributes (`name`, `type`, `value` on buttons, `required`, `form`).
    write_string_prop_if_present(raw_props, &["name"], "name", out);
    let is_secure_input = kind == ComponentKind::TextInput
        && raw_props
            .get("secureTextEntry")
            .or_else(|| raw_props.get("secure_text_entry"))
            .is_some_and(is_truthy_raw_prop);
    if !is_secure_input {
        write_string_prop_if_present(raw_props, &["type", "button_type", "input_type"], "type", out);
    }
    if kind != ComponentKind::TextInput {
        write_string_prop_if_present(raw_props, &["value"], "value", out);
    }
    if kind == ComponentKind::TextInput {
        write_bool_attr_if_truthy(raw_props, &["required"], "required", out);
        write_string_prop_if_present(raw_props, &["minlength"], "minlength", out);
        write_string_prop_if_present(raw_props, &["maxlength"], "maxlength", out);
        write_string_prop_if_present(raw_props, &["pattern"], "pattern", out);
    }
    write_string_prop_if_present(raw_props, &["form"], "form", out);
    write_string_prop_if_present(raw_props, &["formaction"], "formaction", out);
    write_string_prop_if_present(raw_props, &["formmethod"], "formmethod", out);

    // Canonical focus & ARIA state attributes.
    write_string_prop_if_present(raw_props, &["tabIndex", "tab_index", "tabindex"], "tabindex", out);
    for (keys, attr) in [
        (&["ariaModal", "aria-modal"][..], "aria-modal"),
        (&["ariaExpanded", "aria-expanded"], "aria-expanded"),
        (&["ariaControls", "aria-controls"], "aria-controls"),
        (&["ariaLabelledBy", "aria-labelledby"], "aria-labelledby"),
        (&["ariaDescribedBy", "aria-describedby"], "aria-describedby"),
        (&["ariaChecked", "aria-checked"], "aria-checked"),
        (&["ariaSelected", "aria-selected"], "aria-selected"),
        (&["ariaLive", "aria-live"], "aria-live"),
        (&["ariaInvalid", "aria-invalid"], "aria-invalid"),
    ] {
        write_string_prop_if_present(raw_props, keys, attr, out);
    }

    // Deterministic sorted pass-through for custom `data-*` attributes (excluding backend-owned).
    let mut data_keys: Vec<&str> = raw_props
        .keys()
        .map(String::as_str)
        .filter(|k| {
            k.starts_with("data-")
                && !matches!(
                    *k,
                    "data-nve-id"
                        | "data-nve-ssr-hash"
                        | "data-nve-draw-ops"
                        | "data-nve-island"
                        | "data-nve-hydrate"
                        | "data-testid"
                )
        })
        .collect();
    data_keys.sort_unstable();
    for key in data_keys {
        if let Some(val) = raw_props.get(key) {
            match val {
                RawPropValue::String(s) => write_attr(key, s, out),
                RawPropValue::Bool(true) => write_attr(key, "true", out),
                RawPropValue::I64(n) => {
                    let mut buf = String::new();
                    nve_abi::num::write_i64(&mut buf, *n);
                    write_attr(key, &buf, out);
                }
                _ => {}
            }
        }
    }
}

/// Pure, native-testable decision for the `<dialog closedby="any">` backdrop-hit fallback when
/// `'closedBy' in HTMLDialogElement.prototype` is absent in the browser.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn should_close_dialog_on_backdrop_click(
    closedby_attr: Option<&str>,
    supports_native_closedby: bool,
    target_is_dialog: bool,
    click_x: f64,
    click_y: f64,
    rect_left: f64,
    rect_right: f64,
    rect_top: f64,
    rect_bottom: f64,
) -> bool {
    if supports_native_closedby || !target_is_dialog {
        return false;
    }
    if !closedby_attr.is_some_and(|v| v.trim().eq_ignore_ascii_case("any")) {
        return false;
    }
    click_x < rect_left || click_x > rect_right || click_y < rect_top || click_y > rect_bottom
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_supports_dialog_closedby() {
  return typeof HTMLDialogElement !== 'undefined' && 'closedBy' in HTMLDialogElement.prototype;
}
export function nve_sync_popover_state(el, open) {
  if (!el || !el.isConnected) return;
  try {
    if (open && typeof el.showPopover === 'function' && !el.matches(':popover-open')) {
      el.showPopover();
    } else if (!open && typeof el.hidePopover === 'function' && el.matches(':popover-open')) {
      el.hidePopover();
    }
  } catch (_) {}
}
")]
extern "C" {
    pub(crate) fn nve_supports_dialog_closedby() -> bool;
    pub(crate) fn nve_sync_popover_state(el: &web_sys::Element, open: bool);
}

/// Synchronizes native `HTMLDialogElement` `showModal()` / `show()` / `close()` state on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn sync_dialog_open_state(el: &web_sys::Element, open: bool, is_modal: bool) {
    use wasm_bindgen::JsCast;
    let Some(dialog) = el.dyn_ref::<web_sys::HtmlDialogElement>() else {
        return;
    };
    if !dialog.is_connected() {
        if open {
            let _ = dialog.set_attribute("open", "");
        } else {
            let _ = dialog.remove_attribute("open");
        }
        return;
    }
    if open {
        if is_modal {
            let already_modal = dialog.matches(":modal").unwrap_or(false);
            if !already_modal {
                if dialog.open() {
                    dialog.close();
                }
                let _ = dialog.show_modal();
            }
        } else if !dialog.open() {
            dialog.show();
        }
    } else if dialog.open() {
        dialog.close();
    }
}

/// 6-line `ev.target === dialog` backdrop-hit fallback when `'closedBy' in HTMLDialogElement.prototype`
/// is absent. Returns `true` if this click closed the dialog.
#[cfg(target_arch = "wasm32")]
pub(crate) fn try_dialog_backdrop_fallback(ev: &web_sys::MouseEvent) -> bool {
    use wasm_bindgen::JsCast;
    let Some(target) = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlDialogElement>().ok()) else {
        return false;
    };
    if !target.open() {
        return false;
    }
    let rect = target.get_bounding_client_rect();
    let should_close = should_close_dialog_on_backdrop_click(
        target.get_attribute("closedby").as_deref(),
        nve_supports_dialog_closedby(),
        true,
        f64::from(ev.client_x()),
        f64::from(ev.client_y()),
        rect.left(),
        rect.right(),
        rect.top(),
        rect.bottom(),
    );
    if should_close {
        target.close();
    }
    should_close
}
