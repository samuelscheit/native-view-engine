//! Standalone custom Web Element / HTML tag escape hatch (`packages/modules/web-element`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::{
    read_str_words, validate_str_words, write_str_words, FlatResource, NveTable, Res, Str,
};

const MAX_ATTRS: usize = 512;
const MAX_STR_BYTES: usize = 64 * 1024;

/// Sorted attribute pairs attached to a [`WebElement`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebElementAttrs(pub Arc<[(String, String)]>);

impl WebElementAttrs {
    pub fn from_pairs(mut pairs: Vec<(String, String)>) -> Self {
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs.dedup_by(|a, b| a.0 == b.0);
        Self(Arc::from(pairs))
    }
}

/// Zero-copy borrowed view over flat [`WebElementAttrs`] words (`[count, (key_str, val_str)*]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebElementAttrsView<'a>(&'a [u32]);

impl<'a> WebElementAttrsView<'a> {
    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() { 0 } else { self.0[0] as usize }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn iter(self) -> WebElementAttrsIter<'a> {
        WebElementAttrsIter {
            words: self.0,
            pos: 1,
            remaining: self.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebElementAttrsIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for WebElementAttrsIter<'a> {
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

impl FlatResource for WebElementAttrs {
    const KIND: Key64 = Key64::of("nve.res.web_element_attrs");
    type View<'a> = WebElementAttrsView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_ATTRS {
            return false;
        }
        let start = out.len();
        out.push(self.0.len() as u32);
        for (k, v) in self.0.iter() {
            if !write_str_words(out, k, MAX_STR_BYTES) || !write_str_words(out, v, MAX_STR_BYTES) {
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
        if count > MAX_ATTRS {
            return false;
        }
        let mut pos = 1usize;
        for _ in 0..count {
            if !validate_str_words(words, &mut pos, MAX_STR_BYTES)
                || !validate_str_words(words, &mut pos, MAX_STR_BYTES)
            {
                return false;
            }
        }
        pos == words.len()
    }

    #[inline(always)]
    fn view(words: &[u32]) -> WebElementAttrsView<'_> {
        WebElementAttrsView(words)
    }
}

/// Arbitrary custom Web Element table (`nve.web_element`).
#[derive(NveTable)]
#[nve_table(name = "nve.web_element", storage = "sparse", flags = "publish")]
pub struct WebElement {
    #[effect(realize)]
    pub tag: Str,
    #[effect(semantics, paint)]
    pub attrs: Res<WebElementAttrs>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    #[test]
    fn web_element_table_and_attrs_roundtrip() {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<WebElement>(n)
            .tag("my-chart")
            .attrs(Some(WebElementAttrs::from_pairs(vec![(
                "theme".into(),
                "dark".into(),
            )])));
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let row = store.try_row::<WebElement>(n).unwrap();
        assert_eq!(row.tag(), "my-chart");
        let attrs = store.resource::<WebElementAttrs>(row.attrs()).unwrap();
        let got: Vec<(&str, &str)> = attrs.iter().collect();
        assert_eq!(got, vec![("theme", "dark")]);
        assert!(!WebElementAttrs::validate(&[]));
        assert!(!WebElementAttrs::validate(&[0x7FFF_FFFF]));
    }
}
