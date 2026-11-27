//! Standalone custom Web Element / HTML tag escape hatch (`packages/modules/web-element`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::{NveTable, Res, Resource, Str};

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

impl Resource for WebElementAttrs {
    const KIND: Key64 = Key64::of("nve.res.web_element_attrs");

    fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        let res = nve_codec::ByteWriter::with_buf(out, |w| -> Result<(), nve_codec::CodecError> {
            w.write_len_max(self.0.len(), MAX_ATTRS)?;
            for (k, v) in self.0.iter() {
                w.write_str_max(k, MAX_STR_BYTES)?;
                w.write_str_max(v, MAX_STR_BYTES)?;
            }
            Ok(())
        });
        if res.is_err() {
            out.truncate(start);
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = nve_codec::ByteReader::new(bytes);
        let out = r
            .read_vec(MAX_ATTRS, 8, |r| {
                let k = r.read_string(MAX_STR_BYTES)?;
                let v = r.read_string(MAX_STR_BYTES)?;
                Ok((k, v))
            })
            .ok()?;
        r.finish().ok()?;
        Some(Self(Arc::from(out)))
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
        assert_eq!(attrs.0.as_ref(), &[("theme".to_owned(), "dark".to_owned())]);

        let mut buf = Vec::new();
        attrs.encode(&mut buf);
        assert_eq!(WebElementAttrs::decode(&buf).unwrap(), *attrs);
        assert!(WebElementAttrs::decode(&[]).is_none());
    }
}
