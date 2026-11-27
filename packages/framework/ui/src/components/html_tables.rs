//! Reusable `nve-web-html` [`TableAttr`] builders for HTML5 semantic components
//! (`<dialog>`, `<details>`, `<summary>`, `<form>`, `[popover]`, invoker commands, `data-*`).

use nve_dioxus::{table_attr_fn, TableAttr};
use nve_web_html::{
    ButtonType, ClosedBy, DataAttrList, FormMethod, HtmlDataAttrs, HtmlForm, HtmlFormControl,
    HtmlInvoker, HtmlSemantics, PopoverAction, PopoverMode, SemanticElement,
};

pub(crate) fn html_invoker_table(
    button_type: Option<String>,
    name: Option<String>,
    value: Option<String>,
    command: Option<String>,
    commandfor: Option<String>,
    popovertarget: Option<String>,
    popovertargetaction: Option<String>,
) -> TableAttr {
    table_attr_fn(move |tx, node| {
        if command.is_some() || commandfor.is_some() || popovertarget.is_some() {
            let mut inv = tx.edit::<HtmlInvoker>(node);
            if let Some(ref c) = command {
                inv = inv.command(c);
            }
            if let Some(ref cf) = commandfor {
                inv = inv.command_for(cf);
            }
            if let Some(ref pt) = popovertarget {
                inv = inv.popover_target(pt);
            }
            let _ = inv;
        }
        if let Some(ref action) = popovertargetaction {
            let pa = match action.trim() {
                "show" => PopoverAction::Show,
                "hide" => PopoverAction::Hide,
                _ => PopoverAction::Toggle,
            };
            tx.edit::<HtmlSemantics>(node).popover_action(pa);
        }
        if button_type.is_some() || name.is_some() || value.is_some() {
            let mut fc = tx.edit::<HtmlFormControl>(node);
            if let Some(ref bt) = button_type {
                let parsed = match bt.trim() {
                    "button" => ButtonType::Button,
                    "submit" => ButtonType::Submit,
                    "reset" => ButtonType::Reset,
                    _ => ButtonType::Unset,
                };
                fc = fc.button_type(parsed);
            }
            if let Some(ref n) = name {
                fc = fc.name(n);
            }
            if let Some(ref v) = value {
                fc = fc.value(v);
            }
            let _ = fc;
        }
    })
}

pub(crate) fn html_details_table(open: bool, group_name: Option<String>) -> TableAttr {
    table_attr_fn(move |tx, node| {
        tx.edit::<HtmlSemantics>(node)
            .element(SemanticElement::Details)
            .open(open);
        if let Some(ref name) = group_name {
            tx.edit::<HtmlFormControl>(node).name(name);
        }
    })
}

pub(crate) fn html_summary_table() -> TableAttr {
    table_attr_fn(move |tx, node| {
        tx.edit::<HtmlSemantics>(node)
            .element(SemanticElement::Summary);
    })
}

pub(crate) fn html_dialog_table(open: bool, closed_by: String) -> TableAttr {
    table_attr_fn(move |tx, node| {
        let cb = match closed_by.trim() {
            "any" => ClosedBy::Any,
            "closerequest" => ClosedBy::CloseRequest,
            "none" => ClosedBy::None,
            _ => ClosedBy::Unset,
        };
        tx.edit::<HtmlSemantics>(node)
            .element(SemanticElement::Dialog)
            .open(open)
            .closed_by(cb);
    })
}

pub(crate) fn html_form_table(
    method: String,
    action: Option<String>,
    target: Option<String>,
    novalidate: bool,
    dirty: bool,
    restored: bool,
) -> TableAttr {
    table_attr_fn(move |tx, node| {
        tx.edit::<HtmlSemantics>(node).element(SemanticElement::Form);
        let m = match method.trim() {
            "get" | "GET" => FormMethod::Get,
            "post" | "POST" => FormMethod::Post,
            "dialog" | "DIALOG" => FormMethod::Dialog,
            _ => FormMethod::Unset,
        };
        let mut f = tx.edit::<HtmlForm>(node).method(m).novalidate(novalidate);
        if let Some(ref act) = action {
            f = f.action(act);
        }
        if let Some(ref tgt) = target {
            f = f.target(tgt);
        }
        let _ = f;
        let attrs = DataAttrList::from_pairs(vec![
            (
                "nve-dirty".to_owned(),
                if dirty { "true" } else { "false" }.to_owned(),
            ),
            (
                "nve-draft-restored".to_owned(),
                if restored { "true" } else { "false" }.to_owned(),
            ),
        ]);
        tx.edit::<HtmlDataAttrs>(node).entries(Some(attrs));
    })
}

pub(crate) fn html_form_control_name(name: Option<String>) -> TableAttr {
    table_attr_fn(move |tx, node| {
        if let Some(ref n) = name {
            tx.edit::<HtmlFormControl>(node).name(n);
        }
    })
}

pub(crate) fn html_data_attrs(pairs: Vec<(String, String)>) -> TableAttr {
    table_attr_fn(move |tx, node| {
        if !pairs.is_empty() {
            tx.edit::<HtmlDataAttrs>(node)
                .entries(Some(DataAttrList::from_pairs(pairs.clone())));
        }
    })
}

pub(crate) fn html_popover_table(
    popover_mode: String,
    open: bool,
    data_pairs: Vec<(String, String)>,
) -> TableAttr {
    table_attr_fn(move |tx, node| {
        let mode = match popover_mode.trim() {
            "auto" => PopoverMode::Auto,
            "manual" => PopoverMode::Manual,
            "hint" => PopoverMode::Hint,
            _ => PopoverMode::None,
        };
        tx.edit::<HtmlSemantics>(node)
            .popover(mode)
            .data_nve_open(open);
        if !data_pairs.is_empty() {
            tx.edit::<HtmlDataAttrs>(node)
                .entries(Some(DataAttrList::from_pairs(data_pairs.clone())));
        }
    })
}
