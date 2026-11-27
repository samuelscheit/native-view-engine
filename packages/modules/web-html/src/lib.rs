//! Standalone HTML5 / Web-specific tables (`packages/modules/web-html`).
//!
//! Pure-spec root crate owning strictly HTML5/Web tables so `packages/core/components` stays 100%
//! cross-platform:
//! - [`WebHtml`] (`nve.web_html`): raw/sanitized HTML fragments
//! - [`HtmlSemantics`] (`nve.html.semantics`): HTML5 tag overrides (`<dialog>`, `<details>`, `<nav>`,
//!   `<table>`, etc.), `open`, `closedby`, and `[popover]`
//! - [`HtmlInvoker`] (`nve.html.invoker`): HTML5 Invoker Commands (`command`, `commandfor`, `popovertarget`)
//! - [`HtmlForm`] (`nve.html.form`): `<form>` metadata (`action`, `method`, `enctype`, `novalidate`, `target`, `name`)
//! - [`HtmlFormControl`] (`nve.html.form_control`): form-associated control attributes (`name`, `form`, `value`, `formaction`, `type`, `formmethod`)
//! - [`HtmlDataAttrs`] (`nve.html.data_attrs`): custom `data-*` DOM attributes via [`DataAttrList`]
//!
//! Its leaf web adapter (`nve-web-html-web`) implements BOTH live DOM and SSR hooks uniformly via
//! `WebTableSystem`, with zero special-casing inside `nve-backend-web`.

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::{word_enum, NveTable, Res, Resource, Str};

/// Trusted HTML fragment node table (`nve.web_html`).
///
/// # Security Contract
///
/// `html` is an explicit trusted-HTML escape hatch emitted verbatim into SSR output and live DOM
/// `innerHTML`. Callers MUST only supply trusted static/author-controlled markup or HTML sanitized
/// with an allow-list HTML sanitizer; never write untrusted user input directly into `WebHtml::html`.
#[derive(NveTable)]
#[nve_table(name = "nve.web_html", storage = "sparse", flags = "publish")]
pub struct WebHtml {
    /// Wrapper HTML tag (empty = `"div"`). Validated against `is_safe_custom_tag` on web.
    #[effect(realize)]
    pub tag: Str,
    /// Trusted inner HTML fragment rendered verbatim into the wrapper on live DOM and SSR.
    #[effect(paint, semantics)]
    pub html: Str,
}

word_enum! {
    /// Structural HTML5 element tag override (`Auto` = derive from cross-platform core tables).
    pub enum SemanticElement: 5 {
        Auto = 0,
        Dialog = 1,
        Details = 2,
        Summary = 3,
        Form = 4,
        Paragraph = 5,
        Span = 6,
        Strong = 7,
        Em = 8,
        Strike = 9,
        Code = 10,
        Kbd = 11,
        Br = 12,
        Pre = 13,
        Table = 14,
        Thead = 15,
        Tbody = 16,
        Tr = 17,
        Th = 18,
        Td = 19,
        Nav = 20,
        Main = 21,
        Article = 22,
        Aside = 23,
        Header = 24,
        Footer = 25,
        Section = 26,
        Button = 27,
        Label = 28,
    }
}

impl SemanticElement {
    /// Canonical HTML tag name (`None` for `Auto`).
    #[inline]
    pub const fn as_html_tag(self) -> Option<&'static str> {
        Some(match self {
            Self::Auto => return None,
            Self::Dialog => "dialog",
            Self::Details => "details",
            Self::Summary => "summary",
            Self::Form => "form",
            Self::Paragraph => "p",
            Self::Span => "span",
            Self::Strong => "strong",
            Self::Em => "em",
            Self::Strike => "s",
            Self::Code => "code",
            Self::Kbd => "kbd",
            Self::Br => "br",
            Self::Pre => "pre",
            Self::Table => "table",
            Self::Thead => "thead",
            Self::Tbody => "tbody",
            Self::Tr => "tr",
            Self::Th => "th",
            Self::Td => "td",
            Self::Nav => "nav",
            Self::Main => "main",
            Self::Article => "article",
            Self::Aside => "aside",
            Self::Header => "header",
            Self::Footer => "footer",
            Self::Section => "section",
            Self::Button => "button",
            Self::Label => "label",
        })
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "dialog" => Self::Dialog,
            "details" => Self::Details,
            "summary" => Self::Summary,
            "form" => Self::Form,
            "p" => Self::Paragraph,
            "span" => Self::Span,
            "strong" => Self::Strong,
            "em" => Self::Em,
            "s" => Self::Strike,
            "code" => Self::Code,
            "kbd" => Self::Kbd,
            "br" => Self::Br,
            "pre" => Self::Pre,
            "table" => Self::Table,
            "thead" => Self::Thead,
            "tbody" => Self::Tbody,
            "tr" => Self::Tr,
            "th" => Self::Th,
            "td" => Self::Td,
            "nav" => Self::Nav,
            "main" => Self::Main,
            "article" => Self::Article,
            "aside" => Self::Aside,
            "header" => Self::Header,
            "footer" => Self::Footer,
            "section" => Self::Section,
            "button" => Self::Button,
            "label" => Self::Label,
            _ => Self::Auto,
        }
    }
}

word_enum! {
    pub enum ClosedBy: 2 { Unset = 0, Any = 1, CloseRequest = 2, None = 3 }
}

impl ClosedBy {
    #[inline]
    pub const fn as_html_attr(self) -> Option<&'static str> {
        match self {
            Self::Unset => None,
            Self::Any => Some("any"),
            Self::CloseRequest => Some("closerequest"),
            Self::None => Some("none"),
        }
    }
}

word_enum! {
    pub enum PopoverMode: 2 { None = 0, Auto = 1, Manual = 2, Hint = 3 }
}

impl PopoverMode {
    #[inline]
    pub const fn as_html_attr(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Auto => Some("auto"),
            Self::Manual => Some("manual"),
            Self::Hint => Some("hint"),
        }
    }
}

word_enum! {
    pub enum PopoverAction: 2 { Unset = 0, Toggle = 1, Show = 2, Hide = 3 }
}

impl PopoverAction {
    #[inline]
    pub const fn as_html_attr(self) -> Option<&'static str> {
        match self {
            Self::Unset => None,
            Self::Toggle => Some("toggle"),
            Self::Show => Some("show"),
            Self::Hide => Some("hide"),
        }
    }
}

/// HTML5 structural semantics, `<dialog>`/`<details>` `open` state, and `[popover]`.
#[derive(NveTable)]
#[nve_table(name = "nve.html.semantics", storage = "sparse", flags = "publish")]
pub struct HtmlSemantics {
    #[effect(realize, semantics)]
    #[bits(5)]
    pub element: SemanticElement,
    #[effect(semantics)]
    #[bits(1)]
    pub open: bool,
    #[effect(semantics)]
    #[bits(2)]
    pub closed_by: ClosedBy,
    #[effect(semantics, realize)]
    #[bits(2)]
    pub popover: PopoverMode,
    #[effect(semantics)]
    #[bits(2)]
    pub popover_action: PopoverAction,
    #[effect(semantics)]
    #[bits(1)]
    pub data_nve_open: bool,
}

/// HTML5 Declarative Invoker Commands (`command`, `commandfor`, `popovertarget`).
#[derive(NveTable)]
#[nve_table(name = "nve.html.invoker", storage = "sparse", flags = "publish")]
pub struct HtmlInvoker {
    #[effect(semantics)]
    pub popover_target: Str,
    #[effect(semantics)]
    pub command_for: Str,
    #[effect(semantics)]
    pub command: Str,
}

word_enum! {
    pub enum FormMethod: 2 { Unset = 0, Get = 1, Post = 2, Dialog = 3 }
}

impl FormMethod {
    #[inline]
    pub const fn as_html_attr(self) -> Option<&'static str> {
        match self {
            Self::Unset => None,
            Self::Get => Some("get"),
            Self::Post => Some("post"),
            Self::Dialog => Some("dialog"),
        }
    }
}

word_enum! {
    pub enum FormEnctype: 2 { Unset = 0, UrlEncoded = 1, Multipart = 2, Plain = 3 }
}

impl FormEnctype {
    #[inline]
    pub const fn as_html_attr(self) -> Option<&'static str> {
        match self {
            Self::Unset => None,
            Self::UrlEncoded => Some("application/x-www-form-urlencoded"),
            Self::Multipart => Some("multipart/form-data"),
            Self::Plain => Some("text/plain"),
        }
    }
}

/// HTML `<form>` container metadata.
#[derive(NveTable)]
#[nve_table(name = "nve.html.form", storage = "sparse", flags = "publish")]
pub struct HtmlForm {
    #[effect(semantics)]
    pub action: Str,
    #[effect(semantics)]
    pub target: Str,
    #[effect(semantics)]
    pub name: Str,
    #[effect(semantics)]
    #[bits(2)]
    pub method: FormMethod,
    #[effect(semantics)]
    #[bits(2)]
    pub enctype: FormEnctype,
    #[effect(semantics)]
    #[bits(1)]
    pub novalidate: bool,
}

word_enum! {
    pub enum ButtonType: 2 { Unset = 0, Button = 1, Submit = 2, Reset = 3 }
}

impl ButtonType {
    #[inline]
    pub const fn as_html_attr(self) -> Option<&'static str> {
        match self {
            Self::Unset => None,
            Self::Button => Some("button"),
            Self::Submit => Some("submit"),
            Self::Reset => Some("reset"),
        }
    }
}

/// HTML form-associated button/control attributes (`name`, `form`, `type`, `value`, `formaction`, `formmethod`).
#[derive(NveTable)]
#[nve_table(name = "nve.html.form_control", storage = "sparse", flags = "publish")]
pub struct HtmlFormControl {
    #[effect(semantics)]
    pub name: Str,
    #[effect(semantics)]
    pub form: Str,
    #[effect(semantics)]
    pub value: Str,
    #[effect(semantics)]
    pub form_action: Str,
    #[effect(semantics)]
    #[bits(2)]
    pub button_type: ButtonType,
    #[effect(semantics)]
    #[bits(2)]
    pub form_method: FormMethod,
}

/// Custom `data-*` attribute pairs (HTML5 DOM passthrough).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataAttrList(pub Arc<[(String, String)]>);

impl DataAttrList {
    pub fn from_pairs(mut pairs: Vec<(String, String)>) -> Self {
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs.dedup_by(|a, b| a.0 == b.0);
        Self(Arc::from(pairs))
    }
}

const MAX_DATA_ATTRS: usize = 256;

impl Resource for DataAttrList {
    const KIND: Key64 = Key64::of("nve.res.data_attrs");

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        for (k, v) in self.0.iter() {
            out.extend_from_slice(&(k.len() as u32).to_le_bytes());
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(&(v.len() as u32).to_le_bytes());
            out.extend_from_slice(v.as_bytes());
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut pos = 0usize;
        let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
            let end = pos.checked_add(n)?;
            let s = bytes.get(*pos..end)?;
            *pos = end;
            Some(s)
        };
        let u32v = |pos: &mut usize| -> Option<u32> {
            Some(u32::from_le_bytes(take(pos, 4)?.try_into().ok()?))
        };
        let str_owned = |pos: &mut usize| -> Option<String> {
            let len = u32v(pos)? as usize;
            Some(core::str::from_utf8(take(pos, len)?).ok()?.to_owned())
        };
        let n = u32v(&mut pos)? as usize;
        if n > MAX_DATA_ATTRS {
            return None;
        }
        let mut out = Vec::with_capacity(n.min(32));
        for _ in 0..n {
            out.push((str_owned(&mut pos)?, str_owned(&mut pos)?));
        }
        (pos == bytes.len()).then(|| Self(Arc::from(out)))
    }
}

/// Custom `data-*` attribute table (`nve.html.data_attrs`).
#[derive(NveTable)]
#[nve_table(name = "nve.html.data_attrs", storage = "sparse", flags = "publish")]
pub struct HtmlDataAttrs {
    #[effect(semantics)]
    pub entries: Res<DataAttrList>,
}

/// Registers every `nve-web-html` table in the process-global registry.
pub fn register_web_html_tables() {
    let _ = WebHtml::slot();
    let _ = HtmlSemantics::slot();
    let _ = HtmlInvoker::slot();
    let _ = HtmlForm::slot();
    let _ = HtmlFormControl::slot();
    let _ = HtmlDataAttrs::slot();
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    #[test]
    fn web_html_and_html5_tables_roundtrip() {
        register_web_html_tables();
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<WebHtml>(n)
            .tag("article")
            .html("<p>Hello <strong>HTML</strong></p>");
        tx.edit::<HtmlSemantics>(n)
            .element(SemanticElement::Dialog)
            .open(true)
            .closed_by(ClosedBy::CloseRequest)
            .popover(PopoverMode::Auto);
        tx.edit::<HtmlInvoker>(n)
            .command("show-modal")
            .command_for("dlg-1")
            .popover_target("pop-1");
        tx.edit::<HtmlForm>(n)
            .action("/submit")
            .method(FormMethod::Post)
            .enctype(FormEnctype::Multipart)
            .novalidate(true);
        tx.edit::<HtmlFormControl>(n)
            .name("email")
            .button_type(ButtonType::Submit);
        tx.edit::<HtmlDataAttrs>(n)
            .entries(Some(DataAttrList::from_pairs(vec![
                ("data-state".into(), "open".into()),
                ("data-x".into(), "42".into()),
            ])));
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        assert_eq!(store.try_row::<WebHtml>(n).unwrap().tag(), "article");
        assert!(store.try_row::<HtmlSemantics>(n).unwrap().open());
        assert_eq!(
            store.try_row::<HtmlInvoker>(n).unwrap().command(),
            "show-modal"
        );
        assert!(store.try_row::<HtmlForm>(n).unwrap().novalidate());
        assert_eq!(
            store.try_row::<HtmlFormControl>(n).unwrap().button_type(),
            ButtonType::Submit
        );

        let list = store
            .resource::<DataAttrList>(store.try_row::<HtmlDataAttrs>(n).unwrap().entries())
            .unwrap();
        assert_eq!(list.0.len(), 2);
        let mut enc = Vec::new();
        list.encode(&mut enc);
        assert_eq!(DataAttrList::decode(&enc).as_ref(), Some(list));
        assert!(DataAttrList::decode(&[]).is_none());
        assert!(DataAttrList::decode(&[0xFF, 0xFF, 0xFF, 0x7F]).is_none());
    }
}
