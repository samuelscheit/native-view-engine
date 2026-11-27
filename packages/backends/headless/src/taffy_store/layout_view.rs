//! Taffy tree adapter (`LayoutView`) over disjoint borrows of `NodeStore` and `TaffyStoreLayout`.

use nve_components::tables::{Display as NveDisplay, Layout as NveLayout, TextContent};
use nve_engine::TextMeasurer;
use nve_store::{NodeId, NodeStore};

use taffy::geometry::Size;
use taffy::style::AvailableSpace;
use taffy::tree::{Cache, Layout, LayoutInput, LayoutOutput, RunMode};
use taffy::util::ResolveOrZero;
use taffy::{
    CacheTree, CoreStyle, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutPartialTree, TraversePartialTree, TraverseTree, compute_cached_layout,
    compute_flexbox_layout, compute_grid_layout, compute_hidden_layout, compute_leaf_layout,
    compute_oof_layout,
};

use super::contents::BoxChildren;
use super::node_lists::NodeLists;
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
    pub(super) hoisted: &'s mut NodeLists,
    pub(super) box_children: &'s BoxChildren,
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
        self.store.id_at(ix(n)).unwrap_or(NodeId(0))
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

    #[inline(always)]
    fn box_children(&self, parent: taffy::NodeId) -> &[NodeId] {
        self.box_children.get(self.store, self.id_of(parent))
    }

    fn measure(
        &self,
        n: taffy::NodeId,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
        parent_width: Option<f32>,
    ) -> Size<f32> {
        let id = self.id_of(n);
        let Some(tc) = self.store.try_row::<TextContent>(id) else {
            return self.aspect_ratio_content_size(n, known, parent_width);
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

    /// Taffy 0.16 drops `aspect_ratio` when sizing a leaf under `SizingMode::ContentSize` (flex
    /// item hypothetical cross size from `flex-basis` / `flex-grow`, DioxusLabs/taffy#804), so an
    /// empty leaf collapses to 0. Transfer the single known border-box axis through the ratio and
    /// report the content box Taffy adds padding + border back onto.
    fn aspect_ratio_content_size(
        &self,
        n: taffy::NodeId,
        known: Size<Option<f32>>,
        parent_width: Option<f32>,
    ) -> Size<f32> {
        let style = self.style(n);
        let Some(ratio) = style.aspect_ratio() else {
            return Size::ZERO;
        };
        let (width, height) = match (known.width, known.height) {
            (Some(w), None) => (w, w / ratio),
            (None, Some(h)) => (h * ratio, h),
            _ => return Size::ZERO,
        };
        let no_calc = |_, _| 0.0;
        let pb = (style.padding().resolve_or_zero(parent_width, no_calc)
            + style.border().resolve_or_zero(parent_width, no_calc))
        .sum_axes();
        Size {
            width: (width - pb.width).max(0.0),
            height: (height - pb.height).max(0.0),
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
        self.box_children(parent)
            .iter()
            .copied()
            .map(tid_from_nve as fn(NodeId) -> taffy::NodeId)
    }

    #[inline]
    fn child_count(&self, parent: taffy::NodeId) -> usize {
        self.box_children(parent).len()
    }

    #[inline]
    fn get_child_id(&self, parent: taffy::NodeId, child_index: usize) -> taffy::NodeId {
        tid_from_nve(self.box_children(parent)[child_index])
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
            let has_children = !tree.box_children(n).is_empty();
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
                    compute_leaf_layout(
                        inputs,
                        &ro.style(n),
                        |_, _| 0.0,
                        |k, a| ro.measure(n, k, a, inputs.parent_size.width),
                    )
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
        self.hoisted.clear(ix(n));
    }

    #[inline]
    fn add_hoisted_children(&mut self, n: taffy::NodeId, hoisted: &[taffy::NodeId]) {
        let store = self.store;
        self.hoisted
            .extend(ix(n), hoisted.iter().filter_map(|&c| store.id_at(ix(c))));
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
