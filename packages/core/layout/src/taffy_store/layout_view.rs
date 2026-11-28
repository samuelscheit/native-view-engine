//! Taffy tree adapter (`LayoutView`) over disjoint borrows of `NodeStore` and `TaffyStoreLayout`.

use nve_tables::tables::{
    Display as NveDisplay, Layout as NveLayout, TextContent, is_node_display_none,
};
use nve_store::{NodeId, NodeStore};

use taffy::geometry::{Line, Size};
use taffy::style::{AvailableSpace, BoxSizing};
use taffy::tree::{Cache, Layout, LayoutInput, LayoutOutput, RequestedAxis, RunMode, SizingMode};
use taffy::util::ResolveOrZero;
use taffy::{
    CacheTree, CoreStyle, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutPartialTree, TraversePartialTree, TraverseTree, compute_cached_layout,
    compute_flexbox_layout, compute_grid_layout, compute_hidden_layout, compute_leaf_layout,
    compute_oof_layout,
};

use crate::{ResolvedTextStyle, SizeConstraints, SurfaceMeasure, TextMeasurer};

use super::contents::BoxChildren;
use super::node_lists::NodeLists;
use super::style_view::StyleView;
use super::UnroundedBox;

pub(super) struct LayoutView<'s> {
    pub(super) store: &'s NodeStore,
    pub(super) root_index: u32,
    pub(super) root_fills_viewport: bool,
    pub(super) dry_run: bool,
    pub(super) viewport: (f32, f32),
    pub(super) resolved_rtl: &'s [bool],
    pub(super) resolved_text: &'s [ResolvedTextStyle],
    pub(super) caches: &'s mut [Cache],
    pub(super) cache_gen: Option<(&'s mut [u64], u64)>,
    pub(super) boxes: &'s mut [UnroundedBox],
    pub(super) hoisted: Option<&'s mut NodeLists>,
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
        self.store.id_at(ix(n)).unwrap_or(NodeId::NULL)
    }

    #[inline(always)]
    fn style(&self, n: taffy::NodeId) -> StyleView<'s> {
        let i = ix(n);
        StyleView {
            store: self.store,
            id: self.id_of(n),
            is_root: i as u32 == self.root_index,
            root_fills_viewport: self.root_fills_viewport,
            viewport: self.viewport,
            rtl: self.resolved_rtl[i],
        }
    }

    #[inline]
    fn bump_dry_cache_gen(&mut self) {
        if let Some((gens, cur)) = self.cache_gen.as_mut() {
            *cur = cur.wrapping_add(1);
            if *cur == 0 {
                gens.fill(0);
                *cur = 1;
            }
        }
    }

    /// Pure dry-run measurement pass over `root` using `RunMode::PerformLayout` (so CSS flex/grid
    /// baselines are computed) while leaving committed boxes, rects, caches, and hoisted lists
    /// untouched. Re-measures at the clamped width/height whenever `0 < min < max` or an explicit
    /// root size exceeds `max` so wrapped height and baselines reflect the clamped axis.
    pub(super) fn measure_root_dry_run(
        &mut self,
        root: NodeId,
        constraints: SizeConstraints,
        ideal: bool,
    ) -> SurfaceMeasure {
        let min_w = constraints.min_w;
        let min_h = constraints.min_h;
        let max_w = constraints.max_w.is_finite().then_some(constraints.max_w);
        let max_h = constraints.max_h.is_finite().then_some(constraints.max_h);

        let (known_dimensions, available_space, parent_size) = if ideal {
            (
                Size::NONE,
                Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                Size::NONE,
            )
        } else {
            let axis = |min_v: f32, max_v: Option<f32>| match max_v {
                Some(m) if (m - min_v).abs() <= f32::EPSILON => {
                    (Some(m), AvailableSpace::Definite(m), Some(m))
                }
                Some(m) => (None, AvailableSpace::Definite(m), Some(m)),
                None => (None, AvailableSpace::MaxContent, None),
            };
            let (kw, aw, pw) = axis(min_w, max_w);
            let (kh, ah, ph) = axis(min_h, max_h);
            (
                Size { width: kw, height: kh },
                Size { width: aw, height: ah },
                Size { width: pw, height: ph },
            )
        };

        let make_input = |kd: Size<Option<f32>>, avail: Size<AvailableSpace>, ps: Size<Option<f32>>| LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: kd,
            known_dimensions_are_definite: Size {
                width: kd.width.is_some(),
                height: kd.height.is_some(),
            },
            parent_size: ps,
            available_space: avail,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let finite_non_neg = |v: f32| if v.is_finite() && v >= 0.0 { v } else { 0.0 };
        let clamp = |v: f32, min_v: f32, max_v: Option<f32>| {
            let f = finite_non_neg(v);
            if ideal { f } else { max_v.map_or(f, |m| f.min(m)).max(min_v) }
        };

        let mut out = self.compute_child_layout(
            tid_from_nve(root),
            make_input(known_dimensions, available_space, parent_size),
        );
        if !ideal {
            let raw_w = finite_non_neg(out.size.width);
            let clamped_w = clamp(raw_w, min_w, max_w);
            let need_w = known_dimensions.width.is_none() && (clamped_w - raw_w).abs() > f32::EPSILON;
            let (cur_kw, cur_aw, cur_pw) = if need_w {
                self.bump_dry_cache_gen();
                let kd = Size { width: Some(clamped_w), height: known_dimensions.height };
                let avail = Size { width: AvailableSpace::Definite(clamped_w), height: available_space.height };
                let ps = Size { width: Some(clamped_w), height: parent_size.height };
                out = self.compute_child_layout(tid_from_nve(root), make_input(kd, avail, ps));
                (Some(clamped_w), AvailableSpace::Definite(clamped_w), Some(clamped_w))
            } else {
                (known_dimensions.width, available_space.width, parent_size.width)
            };
            let raw_h = finite_non_neg(out.size.height);
            let clamped_h = clamp(raw_h, min_h, max_h);
            let need_h = known_dimensions.height.is_none() && (clamped_h - raw_h).abs() > f32::EPSILON;
            if need_h {
                self.bump_dry_cache_gen();
                let kd = Size { width: cur_kw, height: Some(clamped_h) };
                let avail = Size { width: cur_aw, height: AvailableSpace::Definite(clamped_h) };
                let ps = Size { width: cur_pw, height: Some(clamped_h) };
                out = self.compute_child_layout(tid_from_nve(root), make_input(kd, avail, ps));
            }
        }

        SurfaceMeasure {
            width: clamp(out.size.width, min_w, max_w),
            height: clamp(out.size.height, min_h, max_h),
            first_baseline: out.baselines.first.filter(|v| v.is_finite()).unwrap_or(f32::NAN),
            last_baseline: out.baselines.last.filter(|v| v.is_finite()).unwrap_or(f32::NAN),
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
            return self
                .aspect_ratio_content_size(n, known, parent_width)
                .unwrap_or(Size::ZERO);
        };
        let text = tc.text();
        if text.is_empty() {
            // An empty text node sizes like any other empty leaf (e.g. an `aspect-ratio` box).
            return self
                .aspect_ratio_content_size(n, known, parent_width)
                .unwrap_or(Size::ZERO);
        }
        let resolved = self.resolved_text[ix(n)];
        // `aspect-ratio` is a floor for text too (Taffy drops it here, see below): the ratio's
        // transferred size wins until wrapped text needs more. A width transferred from a known
        // height also bounds the line length, so text wraps inside the ratio box.
        let ratio = self.aspect_ratio_content_size(n, known, parent_width);
        let max_width = known
            .width
            // Without a known width a transfer comes from the known height; it bounds the lines
            // even when padding leaves it 0 wide.
            .or(ratio.map(|r| r.width))
            .or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                AvailableSpace::MinContent => Some(0.0),
                AvailableSpace::MaxContent => None,
            });
        let (mw, mh) = self
            .measurer
            .measure(text, &resolved.in_store(self.store), max_width);
        let safe_w = if mw.is_finite() && mw >= 0.0 { mw } else { 0.0 };
        let safe_h = if mh.is_finite() && mh >= 0.0 { mh } else { 0.0 };
        let floor = ratio.unwrap_or(Size::ZERO);
        Size {
            width: known.width.unwrap_or(safe_w.max(floor.width)),
            height: known.height.unwrap_or(safe_h.max(floor.height)),
        }
    }

    /// Taffy 0.16 drops `aspect_ratio` when sizing a leaf under `SizingMode::ContentSize` (flex
    /// item hypothetical cross size from `flex-basis` / `flex-grow`, DioxusLabs/taffy#804), so an
    /// empty leaf collapses to 0. Transfer the single known axis through the ratio and report the
    /// content box Taffy adds padding + border back onto.
    ///
    /// `known` is a border-box size. Like Taffy's own `InherentSize` path (`compute/leaf.rs`:
    /// ratio applied to the style size, then the `box-sizing` adjustment added), the ratio relates
    /// the `box-sizing` box: border-box transfers first and subtracts padding + border after;
    /// content-box subtracts first and transfers the content size.
    ///
    /// `None` when nothing transfers (no ratio, not exactly one known axis, or a transfer that
    /// overflows `f32`), as opposed to a transferred content size clamped to 0 by padding +
    /// border.
    fn aspect_ratio_content_size(
        &self,
        n: taffy::NodeId,
        known: Size<Option<f32>>,
        parent_width: Option<f32>,
    ) -> Option<Size<f32>> {
        let style = self.style(n);
        let ratio = style.aspect_ratio()?;
        let finite = |v: f32| if v.is_finite() && v >= 0.0 { v } else { 0.0 };
        let no_calc = |_, _| 0.0;
        let pb = (style.padding().resolve_or_zero(parent_width, no_calc)
            + style.border().resolve_or_zero(parent_width, no_calc))
        .sum_axes();
        let (width, height) = match (style.box_sizing(), known.width, known.height) {
            (BoxSizing::BorderBox, Some(w), None) => {
                let w = finite(w);
                (w - pb.width, w / ratio - pb.height)
            }
            (BoxSizing::BorderBox, None, Some(h)) => {
                let h = finite(h);
                (h * ratio - pb.width, h - pb.height)
            }
            (BoxSizing::ContentBox, Some(w), None) => {
                let cw = (finite(w) - pb.width).max(0.0);
                (cw, cw / ratio)
            }
            (BoxSizing::ContentBox, None, Some(h)) => {
                let ch = (finite(h) - pb.height).max(0.0);
                (ch * ratio, ch)
            }
            _ => return None,
        };
        // An extreme ratio overflows the transfer (`w / 1e-38`): no usable size, not a 0 one.
        // Finite negatives (padding + border above the transferred size) clamp to an empty box.
        (width.is_finite() && height.is_finite()).then(|| Size {
            width: width.max(0.0),
            height: height.max(0.0),
        })
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
        let i = ix(n);
        if let Some((gens, cur)) = self.cache_gen.as_ref()
            && gens[i] != *cur
        {
            return None;
        }
        self.caches[i].get(input)
    }

    #[inline]
    fn cache_store(&mut self, n: taffy::NodeId, input: &LayoutInput, output: LayoutOutput) {
        let i = ix(n);
        if let Some((gens, cur)) = self.cache_gen.as_mut()
            && gens[i] != *cur
        {
            self.caches[i].clear();
            gens[i] = *cur;
        }
        self.caches[i].store(input, output);
    }

    #[inline]
    fn cache_clear(&mut self, n: taffy::NodeId) {
        let i = ix(n);
        if let Some((gens, _)) = self.cache_gen.as_mut() {
            gens[i] = 0;
        } else {
            self.caches[i].clear();
        }
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
        if self.dry_run {
            return;
        }
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
            let display = if is_node_display_none(tree.store, id) {
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
            if inputs.run_mode == RunMode::PerformLayout && !tree.dry_run {
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
        if let Some(hoisted) = self.hoisted.as_deref_mut() {
            hoisted.clear(ix(n));
        }
    }

    #[inline]
    fn add_hoisted_children(&mut self, n: taffy::NodeId, hoisted: &[taffy::NodeId]) {
        if hoisted.is_empty() {
            return;
        }
        if let Some(lists) = self.hoisted.as_deref_mut() {
            let store = self.store;
            lists.extend(ix(n), hoisted.iter().filter_map(|&c| store.id_at(ix(c))));
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
