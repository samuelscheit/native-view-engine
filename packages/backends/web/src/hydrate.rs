//! Zero-flicker top-down structural (positional) SSR DOM hydration (`WebBackend::hydrate_into` on
//! `wasm32`, `WebBackend::hydrate_from_ssr` & `WebBackend::hydrate_from_streaming_html` on native),
//! atomic Declarative Shadow DOM promotion, structured [`HydrationMismatch`] reporting, 3-way
//! `StateReconciler` dirty-control preservation, and ID-remapped pre-hydration event replay.

use nve_abi::hash::HashMap;
use nve_abi::{
    EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue, ROOT_NODE_ID,
};
use nve_components::reconcile::{reconcile_three_way, ConflictPolicy, ThreeWayEntry};

use crate::dom::is_truthy_raw_prop;
use crate::event_replay::{
    drain_valid_pre_hydration_events, pre_hydration_event_to_native, CapturedPreHydrationEvent,
};
#[cfg(target_arch = "wasm32")]
use crate::hydrate_wasm::write_dom_control_value;
use crate::WebBackend;

/// Structured mismatch recorded when an SSR DOM element diverges structurally from the hydrated tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationMismatch {
    pub node_id: NodeId,
    pub expected_tag: &'static str,
    pub actual_tag: String,
}

/// Telemetry & diagnostics produced by zero-flicker hydration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HydrationReport {
    pub hydrated: bool,
    pub ssr_hash: Option<String>,
    pub seeded_rule_count: usize,
    pub adopted_nodes: usize,
    pub replaced_nodes: usize,
    pub mismatches: Vec<HydrationMismatch>,
    pub replayed_events: usize,
    pub reconciled_dirty_controls: usize,
}

/// Captured SSR baseline (`base`) and live DOM value (`local`) of an adopted form control
/// (`<input>`, `<textarea>`, `<select>`) awaiting 3-way reconciliation in `finalize_hydration`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HydratedControlState {
    pub(crate) base: String,
    pub(crate) local: String,
}

#[cfg(any(target_arch = "wasm32", feature = "ssr"))]
#[inline]
pub(crate) fn is_form_control_tag(tag: &str) -> bool {
    matches!(tag, "input" | "textarea" | "select")
}

fn remap_pre_hydration_event(
    ev: CapturedPreHydrationEvent,
    remap: &HashMap<NodeId, NodeId>,
) -> Option<CapturedPreHydrationEvent> {
    let old_id = ev.node_id();
    let new_id = match remap.get(&old_id).copied() {
        Some(mapped) => mapped,
        None if old_id == ROOT_NODE_ID => ROOT_NODE_ID,
        None => return None,
    };
    Some(match ev {
        CapturedPreHydrationEvent::Press { x, y, age_ms, .. } => {
            CapturedPreHydrationEvent::Press {
                node_id: new_id,
                x,
                y,
                age_ms,
            }
        }
        CapturedPreHydrationEvent::KeyDown {
            key,
            meta,
            ctrl,
            shift,
            alt,
            age_ms,
            ..
        } => CapturedPreHydrationEvent::KeyDown {
            node_id: new_id,
            key,
            meta,
            ctrl,
            shift,
            alt,
            age_ms,
        },
        CapturedPreHydrationEvent::Submit { value, age_ms, .. } => {
            CapturedPreHydrationEvent::Submit {
                node_id: new_id,
                value,
                age_ms,
            }
        }
    })
}

impl WebBackend {
    /// Returns the structured hydration report populated by `WebBackend::hydrate_into` (`wasm32`),
    /// `WebBackend::hydrate_from_ssr`, or `WebBackend::hydrate_from_streaming_html` (native).
    #[inline]
    pub fn hydration_report(&self) -> &HydrationReport {
        &self.hydration_report
    }

    /// Queues captured pre-hydration events to be remapped, filtered, and replayed at the end of
    /// Frame 0 hydration (used directly by native tests; on `wasm32`, `hydrate_into` drains
    /// `window.__nveReplayDrain()` automatically).
    pub fn queue_pre_hydration_events(&mut self, events: Vec<CapturedPreHydrationEvent>) {
        self.pending_pre_hydration_events.extend(events);
    }

    fn resolve_press_replay_target(&self, start_id: NodeId) -> Option<NodeId> {
        let mut cur = Some(start_id);
        let mut depth = 0usize;
        while let Some(id) = cur {
            if depth > 256 {
                break;
            }
            depth += 1;
            if let Some(el) = self.nodes.get(&id)
                && (el.event_mask & EventMask::PRESS) != 0
            {
                if el.raw_props.get("disabled").is_some_and(is_truthy_raw_prop) {
                    return None;
                }
                return Some(id);
            }
            cur = self.parent_of.get(&id).copied();
        }
        let el = self.nodes.get(&start_id)?;
        if el.raw_props.get("disabled").is_some_and(is_truthy_raw_prop) {
            return None;
        }
        Some(start_id)
    }

    /// Completes Frame 0 hydration via top-down positional structural matching, selective subtree
    /// replacement, 3-way `StateReconciler` control reconciliation, and ID-remapped event replay.
    pub(crate) fn finalize_hydration(&mut self, dirty_styles: &mut Vec<NodeId>) {
        #[allow(unused_mut)]
        let mut ssr_id_remap: HashMap<NodeId, NodeId> = HashMap::default();
        let mut captured_controls: Vec<(NodeId, HydratedControlState)> = Vec::new();

        #[cfg(target_arch = "wasm32")]
        self.hydrate_walk_wasm(&mut ssr_id_remap, &mut captured_controls, dirty_styles);
        #[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
        self.hydrate_walk_native(&mut ssr_id_remap, &mut captured_controls, dirty_styles);
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "ssr")))]
        let _ = dirty_styles;

        self.hydrating = false;

        #[cfg(target_arch = "wasm32")]
        let ts_us = crate::events::now_us();
        #[cfg(not(target_arch = "wasm32"))]
        let ts_us = 0u64;
        let surface_id = self.surface_id.get();
        let mut hydrated_events: Vec<NativeEvent> = Vec::new();

        // 3-way StateReconciler for adopted form controls (enqueued BEFORE replayed actions so
        // typed input syncs into Rust state before a pre-hydration Submit/Press runs).
        captured_controls.sort_unstable_by_key(|(nid, _)| *nid);
        for (nid, ctrl) in captured_controls {
            let Some(incoming) = self.nodes.get(&nid).map(|el| match el.raw_props.get("value") {
                Some(RawPropValue::String(v)) => v.clone(),
                _ => el.text.clone().unwrap_or_default(),
            }) else {
                continue;
            };
            let decision = reconcile_three_way(
                ThreeWayEntry {
                    key: nid,
                    base: ctrl.base,
                    local: ctrl.local,
                    incoming,
                    local_clock: None,
                    incoming_clock: None,
                },
                ConflictPolicy::PreferLocal,
            );
            if decision.emit_upstream {
                if let Some(el) = self.nodes.get_mut(&nid) {
                    el.text = Some(decision.resolved.clone());
                    if el.raw_props.contains_key("value") {
                        el.raw_props.insert(
                            "value".into(),
                            RawPropValue::String(decision.resolved.clone()),
                        );
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(node) = self.native_adopted_nodes.get_mut(&nid) {
                    node.control_value = Some(decision.resolved.clone());
                }
                self.hydration_report.reconciled_dirty_controls += 1;
                hydrated_events.push(NativeEvent {
                    surface_id,
                    node_id: nid,
                    timestamp_us: ts_us,
                    kind: NativeEventKind::TextChange,
                    payload: EventPayload::Text(decision.resolved),
                });
            } else {
                #[cfg(target_arch = "wasm32")]
                if decision.apply_local
                    && let Some(dom_el) = self.dom_nodes.get(&nid)
                {
                    write_dom_control_value(dom_el, &decision.resolved);
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(node) = self.native_adopted_nodes.get_mut(&nid) {
                    node.control_value = Some(decision.resolved);
                }
            }
        }

        // Remap all pre-hydration events from SSR IDs to adopted live mirror IDs BEFORE filtering.
        let raw_replay = std::mem::take(&mut self.pending_pre_hydration_events);
        if !raw_replay.is_empty() {
            let remapped: Vec<CapturedPreHydrationEvent> = raw_replay
                .into_iter()
                .filter_map(|ev| remap_pre_hydration_event(ev, &ssr_id_remap))
                .collect();
            let valid =
                drain_valid_pre_hydration_events(remapped, |id| self.nodes.contains_key(&id));
            for ev in valid {
                let target_id = match &ev {
                    CapturedPreHydrationEvent::Press { node_id, .. } => {
                        let Some(resolved) = self.resolve_press_replay_target(*node_id) else {
                            continue;
                        };
                        resolved
                    }
                    _ => ev.node_id(),
                };
                hydrated_events.push(pre_hydration_event_to_native(
                    ev,
                    surface_id,
                    ts_us,
                    target_id,
                ));
                self.hydration_report.replayed_events += 1;
            }
        }

        if !hydrated_events.is_empty() {
            self.events.borrow_mut().extend(hydrated_events);
            if let Some(cb) = self.on_event.borrow().clone() {
                cb();
            }
        }
    }
}
