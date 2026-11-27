//! Native headless SSR & streaming Declarative Shadow DOM hydration simulation
//! (`WebBackend::hydrate_from_ssr`, `WebBackend::hydrate_from_streaming_html`): parses SSR HTML into
//! an element arena and runs the shared positional pairing (`crate::hydrate::pair_ssr_dom`) against
//! the first committed `NodeStore`, tracking simulated DOM identities and form control values.

use nve_abi::hash::HashMap;
use nve_abi::NodeId;
use nve_components::tables::TextInput;
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore};

use crate::hydrate::{pair_ssr_dom, HydratedControlState, HydrationPass, HydrationReport, SsrDomTree};
use crate::hydrate_native_parse::{parse_ssr_html_tree, NativeSsrTree};
use crate::ssr::SsrOutput;
use crate::WebBackend;

/// Simulated DOM element adopted from SSR or created for a client node.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeDomNode {
    instance_id: u64,
    control_value: Option<String>,
}

/// Simulated native DOM: the parsed SSR tree awaiting the first hydrated commit, then client-keyed
/// element identities.
#[derive(Debug, Default)]
pub(crate) struct NativeDom {
    pending: Option<NativeSsrTree>,
    nodes: HashMap<NodeId, NativeDomNode>,
    next_instance_id: u64,
}

impl SsrDomTree for NativeSsrTree {
    type El = usize;

    fn tag(&self, el: &usize) -> String {
        self.arena.get(*el).map(|n| n.tag.clone()).unwrap_or_default()
    }

    fn ssr_id(&self, el: &usize) -> Option<NodeId> {
        self.arena.get(*el).and_then(|n| n.ssr_id)
    }

    fn ui_children(&self, el: &usize) -> Vec<usize> {
        self.ui_children_of(*el)
    }

    fn control_state(&self, el: &usize, _tag: &str) -> HydratedControlState {
        let node = self.arena.get(*el);
        let base = node
            .and_then(|n| n.base_control_value.clone())
            .unwrap_or_default();
        let local = node
            .and_then(|n| n.live_control_value.clone())
            .unwrap_or_else(|| base.clone());
        HydratedControlState { base, local }
    }
}

impl WebBackend {
    fn from_native_ssr_tree(tree: NativeSsrTree, seeded_hashes: &[String]) -> Self {
        let mut backend = Self::new();
        let before = backend.stylesheet.rule_count();
        backend
            .stylesheet
            .seed_ssr_hashes(seeded_hashes.iter().map(String::as_str));
        let seeded = backend.stylesheet.rule_count().saturating_sub(before);
        backend.hydration_report = HydrationReport::begin(seeded, tree.root_hash.as_deref());
        backend.native_dom = Some(NativeDom {
            next_instance_id: tree.next_instance_id.max(1),
            pending: Some(tree),
            nodes: HashMap::default(),
        });
        backend
    }

    /// Headless native hydration simulator from an [`SsrOutput`]: seeds `StyleSheetCache` from
    /// `ssr.rule_hashes` and parses the SSR HTML so the next `commit()` runs positional hydration.
    pub fn hydrate_from_ssr(ssr: &SsrOutput) -> Self {
        let mut backend = Self::from_native_ssr_tree(parse_ssr_html_tree(&ssr.html), &ssr.rule_hashes);
        backend.hydration_report.expected_root_hash = Some(ssr.root_hash);
        backend
    }

    /// Headless native hydration simulator from a full or mid-stream partial DSD streaming HTML
    /// document: promotes Declarative Shadow DOM slots (last-wins complete Light-DOM chunks,
    /// fallback unwrapping for still-pending slots), seeds ALL `<style data-nve-ssr>` and
    /// `<style data-nve-ssr-chunk>` hashes, and runs positional hydration on the next `commit()`.
    pub fn hydrate_from_streaming_html(html: &str) -> Self {
        let tree = parse_ssr_html_tree(html);
        let hashes = tree.seeded_hashes.clone();
        Self::from_native_ssr_tree(tree, &hashes)
    }

    fn native_pending_idx(&self, node_id: NodeId) -> Option<(&NativeSsrTree, usize)> {
        let tree = self.native_dom.as_ref()?.pending.as_ref()?;
        Some((tree, tree.find_by_ssr_id(node_id)?))
    }

    /// Simulates a user editing an SSR form control before hydration (looked up by SSR
    /// `data-nve-id` until the first hydrated commit, by client `NodeId` afterwards).
    pub fn set_hydrated_dom_control_value(&mut self, node_id: NodeId, value: impl Into<String>) {
        let value = value.into();
        let Some(dom) = self.native_dom.as_mut() else {
            return;
        };
        if let Some(tree) = dom.pending.as_mut() {
            if let Some(n) = tree
                .find_by_ssr_id(node_id)
                .and_then(|idx| tree.arena.get_mut(idx))
            {
                n.live_control_value = Some(value);
            }
        } else if let Some(n) = dom.nodes.get_mut(&node_id) {
            n.control_value = Some(value);
        }
    }

    /// Returns the simulated DOM control `.value` (SSR `data-nve-id` before hydration, client
    /// `NodeId` after).
    pub fn native_dom_control_value(&self, node_id: NodeId) -> Option<&str> {
        if let Some((tree, idx)) = self.native_pending_idx(node_id) {
            return tree.arena.get(idx)?.live_control_value.as_deref();
        }
        self.native_dom
            .as_ref()?
            .nodes
            .get(&node_id)?
            .control_value
            .as_deref()
    }

    /// Returns the simulated DOM object identity (SSR `data-nve-id` before hydration, client
    /// `NodeId` after) so tests can prove SSR elements were adopted rather than recreated.
    pub fn native_dom_instance_id(&self, node_id: NodeId) -> Option<u64> {
        if let Some((tree, idx)) = self.native_pending_idx(node_id) {
            return tree.arena.get(idx).map(|n| n.instance_id);
        }
        self.native_dom
            .as_ref()?
            .nodes
            .get(&node_id)
            .map(|n| n.instance_id)
    }

    /// Pairs the pending parsed SSR tree against `store` (first hydrated commit only).
    pub(crate) fn pair_pending_hydration(&mut self, store: &NodeStore) -> Option<HydrationPass> {
        let tree = self.native_dom.as_mut()?.pending.take()?;
        let root = tree.root_idx?;
        let pairing = pair_ssr_dom(
            &tree,
            root,
            store,
            self.renderer.registry(),
            &mut self.hydration_report,
        );
        let dom = self.native_dom.as_mut()?;
        for (node, idx) in pairing.adopted {
            if let Some(el) = tree.arena.get(idx) {
                dom.nodes.insert(
                    node.raw(),
                    NativeDomNode {
                        instance_id: el.instance_id,
                        control_value: el.live_control_value.clone(),
                    },
                );
            }
        }
        Some(pairing.pass)
    }

    /// Mirrors live DOM commit effects onto the simulated DOM: drops removed nodes, creates fresh
    /// identities for non-adopted nodes, and syncs control values (except controls awaiting
    /// 3-way reconciliation in this pass).
    pub(crate) fn sync_native_dom(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        pass: Option<&HydrationPass>,
    ) {
        let Some(dom) = self.native_dom.as_mut() else {
            return;
        };
        if dom.pending.is_some() {
            return;
        }
        for removed in &changes.removed {
            dom.nodes.remove(&removed.raw());
        }
        let deferred = |id: StoreNodeId| pass.is_some_and(|p| p.controls.iter().any(|(n, _)| *n == id));
        for &id in changes.created.iter().chain(changes.touched.iter()) {
            if !store.is_live(id) {
                continue;
            }
            let value = store.try_row::<TextInput>(id).map(|t| t.value().to_owned());
            let next_id = &mut dom.next_instance_id;
            let node = dom.nodes.entry(id.raw()).or_insert_with(|| {
                let instance_id = *next_id;
                *next_id += 1;
                NativeDomNode {
                    instance_id,
                    control_value: None,
                }
            });
            if value.is_some() && !deferred(id) {
                node.control_value = value;
            }
        }
    }

    pub(crate) fn write_hydrated_control(&mut self, node: StoreNodeId, value: &str) {
        if let Some(n) = self
            .native_dom
            .as_mut()
            .and_then(|d| d.nodes.get_mut(&node.raw()))
        {
            n.control_value = Some(value.to_owned());
        }
    }
}
