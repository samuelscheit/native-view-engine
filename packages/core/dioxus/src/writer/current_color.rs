//! CSS `currentColor` borders: the sparse set of nodes whose winning border color is the symbolic
//! `currentColor`, re-resolved against each node's effective text color at the end of every
//! transaction, so own-color, ancestor-color, reparent, and replay-order changes all converge.

use nve_tables::{css_border_color_source, css_key_bits, mask_border_color_source};
use nve_store::NodeId;

use super::{StandaloneStyleVal, StoreMutationTx};

/// Whether a style key (or an inline style's accepted mask) can set the border color.
#[inline]
pub(super) fn key_sets_border_color(key: &str) -> bool {
    css_key_bits(key).is_some_and(|bits| mask_border_color_source(bits).is_some())
}

impl StoreMutationTx<'_, '_> {
    /// Recomputes whether `node`'s winning border color is `currentColor` (inline style beats
    /// standalone attributes; the last standalone border-color source wins).
    pub(crate) fn refresh_current_color_border(&mut self, node: NodeId) {
        let inline = self
            .inline_styles
            .get(node)
            .and_then(|&(_, mask)| mask_border_color_source(mask));
        let current = inline
            .or_else(|| {
                self.standalone_style_attrs
                    .get(node)?
                    .iter()
                    .rev()
                    .find_map(|(k, v)| match v {
                        StandaloneStyleVal::Str(s) => css_border_color_source(k, s),
                        StandaloneStyleVal::Float(_) | StandaloneStyleVal::Int(_) => None,
                    })
            })
            .unwrap_or(false);
        if current {
            self.current_color_borders.insert(node);
        } else {
            self.current_color_borders.remove(node);
        }
    }

    /// Resolves every tracked `currentColor` border (O(k·depth), writes only on change).
    pub(crate) fn resolve_current_color_borders(&mut self) {
        for node in self.current_color_borders.iter() {
            if self.tx.store().is_live(node) {
                nve_tables::resolve_current_color_border(self.tx, node);
            }
        }
    }
}
