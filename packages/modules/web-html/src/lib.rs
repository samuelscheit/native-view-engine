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

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::{
    read_str_words, validate_str_words, write_str_words, FlatResource, NveTable, Res, Str, WordEnum,
};

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

/// Structural HTML5 element tag override; wrap in `Option<SemanticElement>` (`None` = derive
/// from cross-platform core tables).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum SemanticElement {
    Dialog = 0,
    Details = 1,
    Summary = 2,
    Form = 3,
    Paragraph = 4,
    Span = 5,
    Strong = 6,
    Em = 7,
    Strike = 8,
    Code = 9,
    Kbd = 10,
    Br = 11,
    Pre = 12,
    Table = 13,
    Thead = 14,
    Tbody = 15,
    Tr = 16,
    Th = 17,
    Td = 18,
    Nav = 19,
    Main = 20,
    Article = 21,
    Aside = 22,
    Header = 23,
    Footer = 24,
    Section = 25,
    Button = 26,
    Label = 27,
}

impl SemanticElement {
    /// Canonical HTML tag name.
    #[inline]
    pub const fn as_html_tag(self) -> &'static str {
        match self {
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
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        const TABLE: &[(&str, SemanticElement)] = &[
            ("dialog", SemanticElement::Dialog),
            ("details", SemanticElement::Details),
            ("summary", SemanticElement::Summary),
            ("form", SemanticElement::Form),
            ("p", SemanticElement::Paragraph),
            ("span", SemanticElement::Span),
            ("strong", SemanticElement::Strong),
            ("em", SemanticElement::Em),
            ("s", SemanticElement::Strike),
            ("code", SemanticElement::Code),
            ("kbd", SemanticElement::Kbd),
            ("br", SemanticElement::Br),
            ("pre", SemanticElement::Pre),
            ("table", SemanticElement::Table),
            ("thead", SemanticElement::Thead),
            ("tbody", SemanticElement::Tbody),
            ("tr", SemanticElement::Tr),
            ("th", SemanticElement::Th),
            ("td", SemanticElement::Td),
            ("nav", SemanticElement::Nav),
            ("main", SemanticElement::Main),
            ("article", SemanticElement::Article),
            ("aside", SemanticElement::Aside),
            ("header", SemanticElement::Header),
            ("footer", SemanticElement::Footer),
            ("section", SemanticElement::Section),
            ("button", SemanticElement::Button),
            ("label", SemanticElement::Label),
        ];
        TABLE
            .iter()
            .find_map(|&(name, tag)| s.eq_ignore_ascii_case(name).then_some(tag))
    }
}

/// HTML `<dialog closedby="...">` attribute value; wrap in `Option<ClosedBy>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum ClosedBy {
    Any = 0,
    CloseRequest = 1,
    None = 2,
}

impl ClosedBy {
    #[inline]
    pub const fn as_html_attr(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::CloseRequest => "closerequest",
            Self::None => "none",
        }
    }
}

/// HTML `[popover="..."]` attribute value; wrap in `Option<PopoverMode>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum PopoverMode {
    Auto = 0,
    Manual = 1,
    Hint = 2,
}

impl PopoverMode {
    #[inline]
    pub const fn as_html_attr(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::Hint => "hint",
        }
    }
}

/// HTML `[popovertargetaction="..."]` attribute value; wrap in `Option<PopoverAction>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum PopoverAction {
    Toggle = 0,
    Show = 1,
    Hide = 2,
}

impl PopoverAction {
    #[inline]
    pub const fn as_html_attr(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Show => "show",
            Self::Hide => "hide",
        }
    }
}

/// HTML5 structural semantics, `<dialog>`/`<details>` `open` state, and `[popover]`.
#[derive(NveTable)]
#[nve_table(name = "nve.html.semantics", storage = "sparse", flags = "publish")]
pub struct HtmlSemantics {
    #[effect(realize, semantics)]
    pub element: Option<SemanticElement>,
    #[effect(semantics)]
    pub open: bool,
    #[effect(semantics)]
    pub closed_by: Option<ClosedBy>,
    #[effect(semantics, realize)]
    pub popover: Option<PopoverMode>,
    #[effect(semantics)]
    pub popover_action: Option<PopoverAction>,
    #[effect(semantics)]
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

/// HTML `<form method="...">` / `formmethod` value; wrap in `Option<FormMethod>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum FormMethod {
    Get = 0,
    Post = 1,
    Dialog = 2,
}

impl FormMethod {
    #[inline]
    pub const fn as_html_attr(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Dialog => "dialog",
        }
    }
}

/// HTML `<form enctype="...">` value; wrap in `Option<FormEnctype>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum FormEnctype {
    UrlEncoded = 0,
    Multipart = 1,
    Plain = 2,
}

impl FormEnctype {
    #[inline]
    pub const fn as_html_attr(self) -> &'static str {
        match self {
            Self::UrlEncoded => "application/x-www-form-urlencoded",
            Self::Multipart => "multipart/form-data",
            Self::Plain => "text/plain",
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
    pub method: Option<FormMethod>,
    #[effect(semantics)]
    pub enctype: Option<FormEnctype>,
    #[effect(semantics)]
    pub novalidate: bool,
}

/// HTML `<button type="...">` value; wrap in `Option<ButtonType>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum ButtonType {
    Button = 0,
    Submit = 1,
    Reset = 2,
}

impl ButtonType {
    #[inline]
    pub const fn as_html_attr(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::Submit => "submit",
            Self::Reset => "reset",
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
    pub button_type: Option<ButtonType>,
    #[effect(semantics)]
    pub form_method: Option<FormMethod>,
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
const MAX_DATA_ATTR_STR_BYTES: usize = 64 * 1024;

/// Zero-copy borrowed view over flat [`DataAttrList`] words (`[count, (key_str, val_str)*]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataAttrListView<'a>(&'a [u32]);

impl<'a> DataAttrListView<'a> {
    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() { 0 } else { self.0[0] as usize }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn iter(self) -> DataAttrListIter<'a> {
        DataAttrListIter {
            words: self.0,
            pos: 1,
            remaining: self.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataAttrListIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for DataAttrListIter<'a> {
    type Item = (&'a str, &'a str);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let k = read_str_words(self.words, &mut self.pos)?;
        let v = read_str_words(self.words, &mut self.pos)?;
        self.remaining -= 1;
        Some((k, v))
    }
}

impl FlatResource for DataAttrList {
    const KIND: Key64 = Key64::of("nve.res.data_attrs");
    type View<'a> = DataAttrListView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_DATA_ATTRS {
            return false;
        }
        let start = out.len();
        out.push(self.0.len() as u32);
        for (k, v) in self.0.iter() {
            if !write_str_words(out, k, MAX_DATA_ATTR_STR_BYTES)
                || !write_str_words(out, v, MAX_DATA_ATTR_STR_BYTES)
            {
                out.truncate(start);
                return false;
            }
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&count_u32) = words.first() else {
            return false;
        };
        let count = count_u32 as usize;
        if count > MAX_DATA_ATTRS {
            return false;
        }
        let mut pos = 1usize;
        for _ in 0..count {
            if !validate_str_words(words, &mut pos, MAX_DATA_ATTR_STR_BYTES)
                || !validate_str_words(words, &mut pos, MAX_DATA_ATTR_STR_BYTES)
            {
                return false;
            }
        }
        pos == words.len()
    }

    #[inline(always)]
    fn view(words: &[u32]) -> DataAttrListView<'_> {
        DataAttrListView(words)
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
            .element(Some(SemanticElement::Dialog))
            .open(true)
            .closed_by(Some(ClosedBy::CloseRequest))
            .popover(Some(PopoverMode::Auto));
        tx.edit::<HtmlInvoker>(n)
            .command("show-modal")
            .command_for("dlg-1")
            .popover_target("pop-1");
        tx.edit::<HtmlForm>(n)
            .action("/submit")
            .method(Some(FormMethod::Post))
            .enctype(Some(FormEnctype::Multipart))
            .novalidate(true);
        tx.edit::<HtmlFormControl>(n)
            .name("email")
            .button_type(Some(ButtonType::Submit));
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
            Some(ButtonType::Submit)
        );

        let list = store
            .resource::<DataAttrList>(store.try_row::<HtmlDataAttrs>(n).unwrap().entries())
            .unwrap();
        assert_eq!(list.len(), 2);
        let pairs: Vec<(&str, &str)> = list.iter().collect();
        assert_eq!(pairs, vec![("data-state", "open"), ("data-x", "42")]);
        assert!(!DataAttrList::validate(&[]));
        assert!(!DataAttrList::validate(&[0x7FFF_FFFF]));
    }
}
