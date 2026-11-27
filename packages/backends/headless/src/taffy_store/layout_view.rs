//! Taffy tree adapter (`LayoutView`) over disjoint borrows of `NodeStore` and `TaffyStoreLayout`.

use nve_tables::tables::{Display as NveDisplay, Layout as NveLayout, TextContent};
use nve_engine::TextMeasurer;
use nve_store::{NodeId, NodeStore};

use taffy::geometry::Size;
use taffy::style::AvailableSpace;
use taffy::tree::{Cache, Layout, LayoutInput, LayoutOutput, RunMode};
use taffy::{
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_hidden_layout,
    compute_leaf_layout, compute_oof_layout, CacheTree, LayoutContainingBlock,
    LayoutFlexboxContainer, LayoutGridContainer, LayoutPartialTree, TraversePartialTree,
    TraverseTree,
};

use super::style_view::StyleView;
use super::{ResolvedTextStyle, UnroundedBox};

pub(super) struct LayoutView<'s> {
    pub(super) store: &'s NodeStore,
    pub(super) root_index: u32,
    pub(super) viewport: (f32, f32),
    pub(super) resolved_rtl: &'s [bool],
    pub(super) resolved_text: &'s [ResolvedTextStyle],
    pub(super) caches: &'s mut [Cache],
    pub(super) boxes: &'s mut [UnroundedBox],
    pub(super) hoisted: &'s mut [Vec<NodeId>],
    pub(super) measurer: &'s dyn TextMeasurer,
}

#[inline(always)]
fn ix(n: taffy::NodeId) -> usize {
    u64::from(n) as usize
}

#[inline(always)]
fn tid_from_nve(id: NodeId) -> taffy::NodeId {
    taffy::NodeId::from(id.index() as u64)
}

impl<'s> LayoutView<'s> {
    #[inline(always)]
    fn id_of(&self, n: taffy::NodeId) -> NodeId {
        self.store.id_at(ix(n)).unwrap_or(NodeId::NULL)
    }

    #[inline(always)]
    fn style(&self, n: taffy::NodeId) -> StyleView<'s> {
        let i = ix(n);
        StyleView {
            store: self.store,
            id: self.id_of(n),
            is_root: i as u32 == self.root_index,
            viewport: self.viewport,
            rtl: self.resolved_rtl[i],
        }
    }

    fn measure(
        &self,
        n: taffy::NodeId,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
    ) -> Size<f32> {
        let id = self.id_of(n);
        let Some(tc) = self.store.try_row::<TextContent>(id) else {
            return Size::ZERO;
        };
        let text = tc.text();
        if text.is_empty() {
            return Size::ZERO;
        }
        let resolved = self.resolved_text[ix(n)];
        let max_width = known.width.or(match available.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        });
        let (mw, mh) = self.measurer.measure_with_weight(
            text,
            resolved.font_size,
            resolved.font_weight,
            max_width,
        );
        let safe_w = if mw.is_finite() && mw >= 0.0 { mw } else { 0.0 };
        let safe_h = if mh.is_finite() && mh >= 0.0 { mh } else { 0.0 };
        Size {
            width: known.width.unwrap_or(safe_w),
            height: known.height.unwrap_or(safe_h),
        }
    }
}

type ChildIter<'a> =
    core::iter::Map<core::iter::Copied<core::slice::Iter<'a, NodeId>>, fn(NodeId) -> taffy::NodeId>;

impl TraversePartialTree for LayoutView<'_> {
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    #[inline]
    fn child_ids(&self, parent: taffy::NodeId) -> Self::ChildIter<'_> {
        self.store
            .children(self.id_of(parent))
            .iter()
            .copied()
            .map(tid_from_nve as fn(NodeId) -> taffy::NodeId)
    }

    #[inline]
    fn child_count(&self, parent: taffy::NodeId) -> usize {
        self.store.children(self.id_of(parent)).len()
    }

    #[inline]
    fn get_child_id(&self, parent: taffy::NodeId, child_index: usize) -> taffy::NodeId {
        tid_from_nve(self.store.children(self.id_of(parent))[child_index])
    }
}

impl TraverseTree for LayoutView<'_> {}

impl CacheTree for LayoutView<'_> {
    #[inline]
    fn cache_get(&self, n: taffy::NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        self.caches[ix(n)].get(input)
    }

    #[inline]
    fn cache_store(&mut self, n: taffy::NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.caches[ix(n)].store(input, output);
    }

    #[inline]
    fn cache_clear(&mut self, n: taffy::NodeId) {
        self.caches[ix(n)].clear();
    }
}

impl LayoutPartialTree for LayoutView<'_> {
    type CoreContainerStyle<'a>
        = StyleView<'a>
    where
        Self: 'a;
    type CustomIdent = String;

    #[inline]
    fn get_core_container_style(&self, n: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        self.style(n)
    }

    #[inline]
    fn set_unrounded_layout(&mut self, n: taffy::NodeId, layout: &Layout) {
        self.boxes[ix(n)] = UnroundedBox {
            x: layout.location.x,
            y: layout.location.y,
            w: layout.size.width,
            h: layout.size.height,
        };
    }

    fn compute_child_layout(&mut self, n: taffy::NodeId, inputs: LayoutInput) -> LayoutOutput {
        if inputs.run_mode == RunMode::PerformHiddenLayout {
            self.clear_hoisted_children(n);
            return compute_hidden_layout(self, n);
        }
        compute_cached_layout(self, n, inputs, |tree, n, inputs| {
            let id = tree.id_of(n);
            let display = if crate::is_display_none(tree.store, id) {
                NveDisplay::None
            } else {
                tree.store.row::<NveLayout>(id).display()
            };
            let has_children = !tree.store.children(id).is_empty();
            let mut out = match (display, has_children) {
                (NveDisplay::None, _) => {
                    tree.clear_hoisted_children(n);
                    compute_hidden_layout(tree, n)
                }
                (NveDisplay::Grid, true) => compute_grid_layout(tree, n, inputs),
                (_, true) => compute_flexbox_layout(tree, n, inputs),
                (_, false) => {
                    tree.clear_hoisted_children(n);
                    let ro: &Self = tree;
                    compute_leaf_layout(inputs, &ro.style(n), |_, _| 0.0, |k, a| {
                        ro.measure(n, k, a)
                    })
                }
            };
            if inputs.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, n, &mut out);
            }
            out
        })
    }
}

impl LayoutContainingBlock for LayoutView<'_> {
    type OofItemStyle<'a>
        = StyleView<'a>
    where
        Self: 'a;

    #[inline]
    fn get_oof_item_style(&self, n: taffy::NodeId) -> Self::OofItemStyle<'_> {
        self.style(n)
    }

    #[inline]
    fn clear_hoisted_children(&mut self, n: taffy::NodeId) {
        self.hoisted[ix(n)].clear();
    }

    #[inline]
    fn add_hoisted_children(&mut self, n: taffy::NodeId, hoisted: &[taffy::NodeId]) {
        let dst = &mut self.hoisted[ix(n)];
        for &c in hoisted {
            if let Some(id) = self.store.id_at(ix(c)) {
                dst.push(id);
            }
        }
    }
}

impl LayoutFlexboxContainer for LayoutView<'_> {
    type FlexboxContainerStyle<'a>
        = StyleView<'a>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = StyleView<'a>
    where
        Self: 'a;

    #[inline]
    fn get_flexbox_container_style(&self, n: taffy::NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.style(n)
    }

    #[inline]
    fn get_flexbox_child_style(&self, n: taffy::NodeId) -> Self::FlexboxItemStyle<'_> {
        self.style(n)
    }
}

impl LayoutGridContainer for LayoutView<'_> {
    type GridContainerStyle<'a>
        = StyleView<'a>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = StyleView<'a>
    where
        Self: 'a;

    #[inline]
    fn get_grid_container_style(&self, n: taffy::NodeId) -> Self::GridContainerStyle<'_> {
        self.style(n)
    }

    #[inline]
    fn get_grid_child_style(&self, n: taffy::NodeId) -> Self::GridItemStyle<'_> {
        self.style(n)
    }
}
