//! Zero-flicker top-down positional (structural) SSR hydration over `NodeStore`.
//!
//! One pairing algorithm ([`pair_ssr_dom`]) walks the client's first committed `NodeStore` tree and
//! an SSR element tree in lockstep — the live browser DOM after atomic Declarative Shadow DOM
//! promotion (`hydrate_wasm`) or a parsed native HTML simulation (`hydrate_native`). Matching tags
//! adopt the SSR element under the client's (possibly shifted) `NodeId`; diverging subtrees are
//! reported as structured [`HydrationMismatch`]es and rebuilt. Adopted form controls go through 3-way
//! `PreferLocal` reconciliation, and pre-hydration events are remapped from SSR to client IDs.

use std::rc::Rc;

use nve_abi::hash::HashMap;
use nve_abi::{InteractionEvent, NodeId, TextInputEvent, TimestampUs};
use nve_interop::{reconcile_three_way, ConflictPolicy, ThreeWayEntry};
use nve_tables::tables::{Interaction, TextInput};
use nve_store::NodeStore;

use crate::event_replay::{
    drain_valid_pre_hydration_events, push_pre_hydration_event, CapturedPreHydrationEvent,
};
use crate::events::WebEventState;
use crate::table_system::WebTableRegistry;
use crate::WebBackend;

/// Structured divergence between an SSR element and the client's committed tree. A missing SSR
/// element reports `actual_tag == "<missing>"`; an excess SSR element reports `expected_tag == ""`
/// under its parent's `node_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationMismatch {
    pub node_id: NodeId,
    pub expected_tag: String,
    pub actual_tag: String,
}

/// Telemetry report emitted during SSR hydration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HydrationReport {
    pub hydrated: bool,
    pub adopted_nodes: u32,
    pub replaced_nodes: u32,
    pub reused_css_rules: u32,
    pub replayed_events: u32,
    pub reconciled_dirty_controls: u32,
    pub expected_root_hash: Option<u64>,
    pub mismatches: Vec<HydrationMismatch>,
}

impl HydrationReport {
    #[inline]
    pub fn is_clean(&self) -> bool {
        self.replaced_nodes == 0 && self.mismatches.is_empty()
    }

    /// Starts a hydration report from seeded SSR CSS hashes and the SSR root's
    /// `data-nve-ssr-hash` (16 hex digits).
    pub(crate) fn begin(seeded_rules: usize, root_hash_hex: Option<&str>) -> Self {
        Self {
            hydrated: true,
            reused_css_rules: u32::try_from(seeded_rules).unwrap_or(u32::MAX),
            expected_root_hash: root_hash_hex.and_then(|h| u64::from_str_radix(h, 16).ok()),
            ..Self::default()
        }
    }
}

/// SSR baseline (`base`) and pre-hydration DOM value (`local`) of an adopted form control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HydratedControlState {
    pub(crate) base: String,
    pub(crate) local: String,
}

/// One `<option>` in a `<select>`'s list of options (a direct child, or a child of a direct
/// `<optgroup>` child), as authored in SSR markup.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SsrSelectOption {
    /// `[selected]` (`defaultSelected`).
    pub(crate) selected: bool,
    /// `[disabled]` on the option or on its parent `<optgroup>`.
    pub(crate) disabled: bool,
    /// `value` attribute, else the option's text content.
    pub(crate) value: Option<String>,
    pub(crate) text: String,
}

/// HTML "rules for parsing non-negative integers" as browsers apply them to `<select size>`:
/// leading ASCII whitespace and one `+` / `-` sign are skipped, leading digits are read and
/// trailing garbage (`3px`, `3 x`, `1.9`) is ignored. No digits, a negative value, or `u32`
/// overflow is a parse error (Chrome: `size="4294967296"` renders a drop-down).
fn parse_non_negative_u32(s: &str) -> Option<u32> {
    let s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let mut n: u32 = 0;
    for b in rest[..digits].bytes() {
        n = n.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
    }
    (!negative || n == 0).then_some(n)
}

/// Whether `<select size>` implies display size 1 (`size` absent, invalid, `0`, or `1`).
#[inline]
pub(crate) fn select_display_size_is_one(size: Option<&str>) -> bool {
    size.and_then(parse_non_negative_u32).is_none_or(|n| n <= 1)
}

/// Strips leading/trailing ASCII whitespace and collapses internal runs to a single space
/// (`HTMLOptionElement.value` text fallback) without intermediate `Vec` allocations.
pub(crate) fn collapse_ascii_whitespace(s: &str) -> String {
    let trimmed = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(trimmed.len());
    for word in trimmed.split_ascii_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// `select.value` implied by SSR markup alone (the HTML selectedness setting algorithm): the
/// first `[selected]` option for `multiple`; otherwise the last `[selected]` one, else the first
/// enabled option when the display size is 1 (`size` absent, invalid, `0`, or `1`).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ssr_select_base_value(
    options: &[SsrSelectOption],
    multiple: bool,
    size: Option<&str>,
) -> String {
    let display_size_one = select_display_size_is_one(size);
    let chosen = if multiple {
        options.iter().find(|o| o.selected)
    } else {
        options.iter().rev().find(|o| o.selected).or_else(|| {
            display_size_one
                .then(|| options.iter().find(|o| !o.disabled))
                .flatten()
        })
    };
    chosen.map_or_else(String::new, |o| match &o.value {
        Some(v) => v.clone(),
        // `option.value` falls back to the text with ASCII whitespace stripped and collapsed.
        None => collapse_ascii_whitespace(&o.text),
    })
}

/// Read-only view of an SSR-rendered element tree (live browser DOM or parsed native HTML).
pub(crate) trait SsrDomTree {
    type El: Clone;
    /// Lowercase tag name.
    fn tag(&self, el: &Self::El) -> String;
    /// Parsed `data-nve-id` assigned by the server.
    fn ssr_id(&self, el: &Self::El) -> Option<NodeId>;
    /// Ordered UI element children (excluding `<style>`, `<script>`, `<template>`, DSD hosts).
    fn ui_children(&self, el: &Self::El) -> Vec<Self::El>;
    /// SSR baseline + current value of a form control element.
    fn control_state(&self, el: &Self::El, tag: &str) -> HydratedControlState;
}

/// Result of pairing one SSR element tree against the first hydrated commit.
pub(crate) struct HydrationPairing<El> {
    /// `(client node, SSR element)` pairs adopted in place.
    pub(crate) adopted: Vec<(NodeId, El)>,
    /// Stale SSR elements (excess siblings and tag-mismatched subtrees) to discard.
    pub(crate) stale: Vec<El>,
    pub(crate) pass: HydrationPass,
}

/// Per-commit hydration outcome consumed by [`WebBackend::finish_hydration`].
#[derive(Default)]
pub(crate) struct HydrationPass {
    /// SSR `data-nve-id` -> client `NodeId`.
    pub(crate) remap: HashMap<NodeId, NodeId>,
    pub(crate) controls: Vec<(NodeId, HydratedControlState)>,
}

/// Top-down positional pairing of `root` against `store` starting at `NodeId::ROOT`.
pub(crate) fn pair_ssr_dom<T: SsrDomTree>(
    tree: &T,
    root: T::El,
    store: &NodeStore,
    registry: &WebTableRegistry,
    report: &mut HydrationReport,
) -> HydrationPairing<T::El> {
    let mut out = HydrationPairing {
        adopted: Vec::new(),
        stale: Vec::new(),
        pass: HydrationPass::default(),
    };
    let mut stack = vec![(NodeId::ROOT, root)];
    while let Some((node, el)) = stack.pop() {
        if !store.is_live(node) {
            continue;
        }
        let (expected, _) = registry.resolve_tag(store, node);
        let actual = tree.tag(&el);
        if !actual.eq_ignore_ascii_case(expected) {
            report.replaced_nodes += 1;
            report.mismatches.push(HydrationMismatch {
                node_id: node,
                expected_tag: expected.to_owned(),
                actual_tag: actual,
            });
            out.stale.push(el);
            continue;
        }
        report.adopted_nodes += 1;
        if let Some(ssr_id) = tree.ssr_id(&el) {
            out.pass.remap.insert(ssr_id, node);
        }
        if store.has::<TextInput>(node) {
            out.pass.controls.push((node, tree.control_state(&el, expected)));
        }
        let kids = store.children(node);
        // A leaf's element children without `data-nve-id` are plugin-owned inner content.
        let dom_kids: Vec<T::El> = tree
            .ui_children(&el)
            .into_iter()
            .filter(|c| !kids.is_empty() || tree.ssr_id(c).is_some())
            .collect();
        let paired = kids.len().min(dom_kids.len());
        for &missing in &kids[paired..] {
            report.replaced_nodes += 1;
            report.mismatches.push(HydrationMismatch {
                node_id: missing,
                expected_tag: registry.resolve_tag(store, missing).0.to_owned(),
                actual_tag: "<missing>".to_owned(),
            });
        }
        let mut dom_kids = dom_kids.into_iter();
        let pairs: Vec<(NodeId, T::El)> =
            kids.iter().copied().zip(dom_kids.by_ref()).collect();
        for extra in dom_kids {
            report.mismatches.push(HydrationMismatch {
                node_id: node,
                expected_tag: String::new(),
                actual_tag: tree.tag(&extra),
            });
            out.stale.push(extra);
        }
        stack.extend(pairs.into_iter().rev());
        out.adopted.push((node, el));
    }
    out
}

fn remap_pre_hydration_event(
    ev: CapturedPreHydrationEvent,
    remap: &HashMap<NodeId, NodeId>,
) -> Option<CapturedPreHydrationEvent> {
    let old = ev.node_id();
    let new_id = match remap.get(&old) {
        Some(&mapped) => mapped,
        None if old == NodeId::ROOT => old,
        None => return None,
    };
    Some(ev.with_node_id(new_id))
}

impl WebBackend {
    #[inline]
    pub fn hydration_report(&self) -> &HydrationReport {
        &self.hydration_report
    }

    /// Queues captured pre-hydration events, remapped from SSR to client IDs and replayed after
    /// the next commit (on `wasm32`, `hydrate_into` drains `window.__nveReplayDrain()` itself).
    pub fn queue_pre_hydration_events(&mut self, events: Vec<CapturedPreHydrationEvent>) {
        self.pre_hydration_events.extend(events);
    }

    /// Completes a commit: 3-way `PreferLocal` reconciliation of adopted form controls (their
    /// `TextChange` events are enqueued BEFORE replayed actions so typed input reaches app state
    /// first), then ID-remapped pre-hydration event replay.
    pub(crate) fn finish_hydration(&mut self, store: &NodeStore, pass: Option<HydrationPass>) {
        let pending = Rc::clone(&self.pending_events);
        let mut events = pending.borrow_mut();
        let surface_id = self.surface_id.get();
        let remap = pass.as_ref().map(|p| &p.remap);
        for (node, ctrl) in pass.as_ref().map_or(&[][..], |p| &p.controls[..]) {
            let Some(incoming) = store.try_row::<TextInput>(*node).map(|t| t.value().to_owned())
            else {
                continue;
            };
            let decision = reconcile_three_way(
                ThreeWayEntry {
                    key: node.raw(),
                    base: ctrl.base.clone(),
                    local: ctrl.local.clone(),
                    incoming,
                    local_clock: None,
                    incoming_clock: None,
                },
                ConflictPolicy::PreferLocal,
            );
            self.write_hydrated_control(*node, &decision.resolved);
            self.input_sync
                .borrow_mut()
                .on_backend_value_sync(*node, &decision.resolved);
            if decision.emit_upstream {
                self.hydration_report.reconciled_dirty_controls += 1;
                events.push_text_change(surface_id, *node, TimestampUs(0), &decision.resolved);
            }
        }

        if !self.pre_hydration_events.is_empty() {
            let drained = std::mem::take(&mut self.pre_hydration_events);
            // Captured events carry SSR `data-nve-id`s: without this commit's pairing remap
            // (no SSR root, or hydration already completed) they would hit unrelated client
            // nodes, so they are discarded instead of replayed raw.
            let remapped: Vec<CapturedPreHydrationEvent> = remap.map_or_else(Vec::new, |map| {
                drained
                    .into_iter()
                    .filter_map(|ev| remap_pre_hydration_event(ev, map))
                    .collect()
            });
            let valid = drain_valid_pre_hydration_events(remapped, |id| store.is_live(id));
            let state = self.event_state.borrow();
            for captured in &valid {
                let Some(target_id) = resolve_replay_target(store, &state, captured) else {
                    continue;
                };
                push_pre_hydration_event(
                    captured,
                    surface_id,
                    TimestampUs(0),
                    target_id,
                    &mut events,
                );
                self.hydration_report.replayed_events += 1;
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.deferred_controls.clear();
    }

    /// Seeds additional SSR structural class hashes (comma-separated, e.g. from a late mid-stream
    /// `<style data-nve-ssr-chunk data-nve-hashes="...">` moved into `document.head`) so subsequent
    /// client renders never duplicate already-present CSS rules. Returns the newly seeded count.
    pub fn seed_ssr_hashes(&mut self, hashes_csv: &str) -> usize {
        let before = self.stylesheet.rule_count();
        self.stylesheet.seed_ssr_hashes(hashes_csv.split(','));
        self.stylesheet.rule_count().saturating_sub(before)
    }
}

/// Bubbles a replayed event from its (remapped) target to the nearest ancestor subscribed to its
/// event (`Press`, `KeyDown`, `Submit`/text input), falling back to the original target.
/// `Press`/`Submit` are dropped (`None`) when any node on that path is `Interaction::disabled`,
/// matching the browser, which never dispatches activation on disabled controls.
pub(crate) fn resolve_replay_target(
    store: &NodeStore,
    state: &WebEventState,
    ev: &CapturedPreHydrationEvent,
) -> Option<NodeId> {
    let activation = matches!(
        ev,
        CapturedPreHydrationEvent::Press { .. } | CapturedPreHydrationEvent::Submit { .. }
    );
    let mut cur = Some(ev.node_id());
    while let Some(nid) = cur {
        if activation && store.try_row::<Interaction>(nid).is_some_and(|i| i.disabled()) {
            return None;
        }
        let matched = match ev {
            CapturedPreHydrationEvent::Press { .. } => {
                state.listens::<Interaction>(nid, InteractionEvent::Press)
            }
            CapturedPreHydrationEvent::KeyDown { .. } => {
                state.listens::<Interaction>(nid, InteractionEvent::KeyDown)
            }
            CapturedPreHydrationEvent::Submit { .. } => {
                state.is_input(nid) || state.listens::<TextInput>(nid, TextInputEvent::Submit)
            }
        };
        if matched {
            return Some(nid);
        }
        cur = store.parent(nid);
    }
    Some(ev.node_id())
}
