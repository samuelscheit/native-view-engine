//! Isolated dry-run scratch (`DryRunScratch`) for [`super::TaffyStoreLayout::measure_root`].
//!
//! Keeps dry-run Taffy caches, root-level result memoization, and fallback inheritance buffers
//! completely disjoint from committed layout state so `measure_root` never mutates committed
//! caches, boxes, rects, hoisted lists, `inherit_dirty`, or `layout_dirty`.

use nve_store::{NodeId, NodeStore};
use nve_tables::tables::{
    Direction as NveDirection, Layout as NveLayout, is_display_contents,
};
use taffy::tree::Cache;

use crate::{
    ResolvedTextStyle, SizeConstraints, SurfaceMeasure, TextMeasurer, resolve_inherited_text,
};

use super::contents::BoxChildren;
use super::layout_view::LayoutView;

const DRY_ROOT_CACHE_SLOTS: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
struct DryMeasureKey {
    root: NodeId,
    ideal: bool,
    min_w: u32,
    min_h: u32,
    max_w: u32,
    max_h: u32,
}

impl DryMeasureKey {
    #[inline]
    fn new(root: NodeId, c: SizeConstraints, ideal: bool) -> Self {
        Self {
            root,
            ideal,
            min_w: if ideal { 0 } else { c.min_w.to_bits() },
            min_h: if ideal { 0 } else { c.min_h.to_bits() },
            max_w: if ideal { 0 } else { c.max_w.to_bits() },
            max_h: if ideal { 0 } else { c.max_h.to_bits() },
        }
    }
}

#[inline]
pub(super) fn valid_size_constraints(c: SizeConstraints) -> bool {
    c.min_w.is_finite()
        && c.min_w >= 0.0
        && c.min_h.is_finite()
        && c.min_h >= 0.0
        && !c.max_w.is_nan()
        && c.max_w >= c.min_w
        && !c.max_h.is_nan()
        && c.max_h >= c.min_h
}

#[derive(Default)]
pub(super) struct DryRunScratch {
    caches: Vec<Cache>,
    cache_gen: Vec<u64>,
    pass_gen: u64,
    store_rev: Option<(u64, u64)>,
    root_results: [Option<(DryMeasureKey, SurfaceMeasure)>; DRY_ROOT_CACHE_SLOTS],
    root_next_slot: usize,
    resolved_rtl: Vec<bool>,
    resolved_text: Vec<ResolvedTextStyle>,
    box_children: BoxChildren,
    inherit_stack: Vec<(NodeId, bool, ResolvedTextStyle)>,
}

impl DryRunScratch {
    pub(super) fn invalidate(&mut self) {
        self.root_results = [None; DRY_ROOT_CACHE_SLOTS];
        self.root_next_slot = 0;
    }

    pub(super) fn reset(&mut self) {
        self.invalidate();
        self.caches.clear();
        self.cache_gen.clear();
        self.pass_gen = 0;
        self.store_rev = None;
        self.resolved_rtl.clear();
        self.resolved_text.clear();
        self.box_children.reset();
        self.inherit_stack.clear();
    }

    #[inline]
    fn next_pass_gen(&mut self) -> u64 {
        self.pass_gen = self.pass_gen.wrapping_add(1);
        if self.pass_gen == 0 {
            self.cache_gen.fill(0);
            self.pass_gen = 1;
        }
        self.pass_gen
    }

    fn ensure_caches(&mut self, cap: usize) {
        if self.caches.len() < cap {
            self.caches.resize_with(cap, Cache::new);
            self.cache_gen.resize(cap, 0);
        }
    }

    fn populate_inheritance(&mut self, store: &NodeStore, root: NodeId) {
        let cap = store.capacity();
        if self.resolved_rtl.len() < cap {
            self.resolved_rtl.resize(cap, false);
            self.resolved_text.resize(cap, ResolvedTextStyle::default());
        }
        self.box_children.ensure_len(cap);
        self.inherit_stack.clear();
        self.inherit_stack
            .push((root, false, ResolvedTextStyle::default()));
        while let Some((id, parent_rtl, parent_text)) = self.inherit_stack.pop() {
            if !store.is_live(id) {
                continue;
            }
            let idx = id.index();
            let rtl = if let Some(l) = store.try_row::<NveLayout>(id) {
                match l.direction() {
                    NveDirection::Inherit => parent_rtl,
                    NveDirection::Ltr => false,
                    NveDirection::Rtl => true,
                }
            } else {
                parent_rtl
            };
            let text = resolve_inherited_text(store, id, parent_text);
            self.resolved_rtl[idx] = rtl;
            self.resolved_text[idx] = text;
            if id != root && is_display_contents(store, id) {
                self.box_children.release(idx);
            } else {
                self.box_children.rebuild(store, id);
            }
            for &child in store.children(id).iter().rev() {
                self.inherit_stack.push((child, rtl, text));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn measure_root(
        &mut self,
        store: &NodeStore,
        root: NodeId,
        constraints: SizeConstraints,
        ideal: bool,
        committed_inherit: Option<(&[bool], &[ResolvedTextStyle], &BoxChildren)>,
        measurer: &dyn TextMeasurer,
    ) -> SurfaceMeasure {
        let empty = SurfaceMeasure {
            width: 0.0,
            height: 0.0,
            first_baseline: f32::NAN,
            last_baseline: f32::NAN,
        };
        if !store.is_live(root) || !valid_size_constraints(constraints) {
            return empty;
        }
        let rev = (store.instance_id(), store.frame());
        if self.store_rev != Some(rev) {
            self.invalidate();
            self.store_rev = Some(rev);
        }
        let key = DryMeasureKey::new(root, constraints, ideal);
        if let Some((_, hit)) = self
            .root_results
            .iter()
            .flatten()
            .find(|(k, _)| *k == key)
        {
            return *hit;
        }

        self.ensure_caches(store.capacity());
        if committed_inherit.is_none() {
            self.populate_inheritance(store, root);
        }
        let pass_gen = self.next_pass_gen();
        let (resolved_rtl, resolved_text, box_children) = if let Some(c) = committed_inherit {
            c
        } else {
            (
                self.resolved_rtl.as_slice(),
                self.resolved_text.as_slice(),
                &self.box_children,
            )
        };
        let mut view = LayoutView {
            store,
            root_index: root.index() as u32,
            root_fills_viewport: false,
            dry_run: true,
            viewport: (0.0, 0.0),
            resolved_rtl,
            resolved_text,
            caches: &mut self.caches,
            cache_gen: Some((&mut self.cache_gen, pass_gen)),
            boxes: &mut [],
            hoisted: None,
            box_children,
            measurer,
        };
        let measured = view.measure_root_dry_run(root, constraints, ideal);
        if let Some((_, final_gen)) = view.cache_gen {
            self.pass_gen = final_gen;
        }
        self.root_results[self.root_next_slot] = Some((key, measured));
        self.root_next_slot = (self.root_next_slot + 1) % DRY_ROOT_CACHE_SLOTS;
        measured
    }
}
