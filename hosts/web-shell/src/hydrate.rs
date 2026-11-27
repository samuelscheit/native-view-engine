//! Cross-target Progressive & Island DOM Hydration Protocol (`HydrateTrigger`,
//! `IslandSpec`, `CapturedInteraction`, and `IslandHydrationScheduler`).
//!
//! Browser `wasm32` bindings live in `wasm_hydrate.rs`.

use nve_abi::hash::HashMap;

use nve_abi::NodeId;
use nve_store::NodeStore;
use nve_web_html::HtmlDataAttrs;

#[cfg(all(test, feature = "app-studio", feature = "app-kitchen-sink"))]
#[path = "hydrate_tests.rs"]
mod hydrate_tests;

/// Scheduling trigger controlling when an SSR `<Island>` hydrates its interactive runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HydrateTrigger {
    /// Hydrate immediately on initial page load.
    Load,
    /// Hydrate lazily when scrolled into the viewport via `IntersectionObserver`.
    Visible,
    /// Hydrate on first user interaction (`click`/`focusin`/`keydown`) and replay the event.
    Interaction,
    /// Hydrate during browser idle time (`requestIdleCallback`, with `setTimeout` fallback).
    Idle,
}

impl HydrateTrigger {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "load" | "eager" | "immediate" => Some(Self::Load),
            "visible" | "viewport" => Some(Self::Visible),
            "interaction" | "interact" | "event" => Some(Self::Interaction),
            "idle" => Some(Self::Idle),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Visible => "visible",
            Self::Interaction => "interaction",
            Self::Idle => "idle",
        }
    }
}

/// Metadata describing one SSR island discovered in the DOM or mirror tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslandSpec {
    pub island_id: String,
    pub node_id: NodeId,
    pub trigger: HydrateTrigger,
    pub hydrated: bool,
}

/// Captured user interaction recorded on an unhydrated `HydrateTrigger::Interaction` island
/// so it can be replayed cleanly immediately after hydration completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedInteraction {
    pub island_id: String,
    pub event_type: String,
    pub target_node_id: Option<NodeId>,
    pub client_x: i32,
    pub client_y: i32,
    pub key: Option<String>,
}

/// Cross-target progressive island hydration scheduler testable both in native host unit tests
/// and in browser WASM.
#[derive(Debug, Default)]
pub struct IslandHydrationScheduler {
    islands: Vec<IslandSpec>,
    by_id: HashMap<String, usize>,
    pending_replays: HashMap<String, Vec<CapturedInteraction>>,
}

impl IslandHydrationScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discovers every node stamped with `data-nve-island` in `store` (deterministic `NodeId`
    /// order), parsing `data-nve-hydrate` (defaulting to [`HydrateTrigger::Load`]).
    pub fn discover_from_store(store: &NodeStore) -> Self {
        let mut discovered: Vec<(NodeId, String, HydrateTrigger)> = Vec::new();
        for node_id in store.live_nodes() {
            let Some(attrs) = store.try_row::<HtmlDataAttrs>(node_id) else {
                continue;
            };
            let Some(list) = store.resource::<nve_web_html::DataAttrList>(attrs.entries()) else {
                continue;
            };
            let mut island_id: Option<&str> = None;
            let mut trigger = HydrateTrigger::Load;
            for (k, v) in list.0.iter() {
                match k.as_str() {
                    "data-nve-island" | "nve-island" => {
                        let trimmed = v.trim();
                        if !trimmed.is_empty() {
                            island_id = Some(trimmed);
                        }
                    }
                    "data-nve-hydrate" | "nve-hydrate" => {
                        if let Some(parsed) = HydrateTrigger::parse(v) {
                            trigger = parsed;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(id) = island_id {
                discovered.push((node_id.raw(), id.to_owned(), trigger));
            }
        }
        discovered.sort_unstable_by_key(|(id, _, _)| *id);

        let mut scheduler = Self::new();
        for (node_id, island_id, trigger) in discovered {
            scheduler.register(island_id, node_id, trigger);
        }
        scheduler
    }

    /// Registers an island with the scheduler. Duplicate IDs update trigger metadata in-place.
    pub fn register(
        &mut self,
        island_id: impl Into<String>,
        node_id: NodeId,
        trigger: HydrateTrigger,
    ) {
        let island_id = island_id.into();
        if let Some(&idx) = self.by_id.get(&island_id) {
            self.islands[idx].node_id = node_id;
            self.islands[idx].trigger = trigger;
            return;
        }
        let idx = self.islands.len();
        self.by_id.insert(island_id.clone(), idx);
        self.islands.push(IslandSpec {
            island_id,
            node_id,
            trigger,
            hydrated: false,
        });
    }

    pub fn islands(&self) -> &[IslandSpec] {
        &self.islands
    }

    pub fn is_hydrated(&self, island_id: &str) -> bool {
        self.by_id
            .get(island_id)
            .is_some_and(|&idx| self.islands[idx].hydrated)
    }

    pub fn hydrated_count(&self) -> usize {
        self.islands.iter().filter(|i| i.hydrated).count()
    }

    pub fn pending_count(&self) -> usize {
        self.islands.iter().filter(|i| !i.hydrated).count()
    }

    /// Hydrates all pending [`HydrateTrigger::Load`] islands and returns their IDs.
    pub fn on_load(&mut self) -> Vec<String> {
        let mut ready = Vec::new();
        for spec in &mut self.islands {
            if !spec.hydrated && spec.trigger == HydrateTrigger::Load {
                spec.hydrated = true;
                ready.push(spec.island_id.clone());
            }
        }
        ready
    }

    /// Marks a pending [`HydrateTrigger::Visible`] island hydrated when it intersects the viewport.
    pub fn on_visible(&mut self, island_id: &str) -> bool {
        let Some(&idx) = self.by_id.get(island_id) else {
            return false;
        };
        let spec = &mut self.islands[idx];
        if spec.hydrated || spec.trigger != HydrateTrigger::Visible {
            return false;
        }
        spec.hydrated = true;
        true
    }

    /// Queues an interaction event on an unhydrated island without hydrating it yet.
    pub fn queue_interaction(&mut self, interaction: CapturedInteraction) -> bool {
        let Some(&idx) = self.by_id.get(&interaction.island_id) else {
            return false;
        };
        if self.islands[idx].hydrated {
            return false;
        }
        self.pending_replays
            .entry(interaction.island_id.clone())
            .or_default()
            .push(interaction);
        true
    }

    /// Captures the first interaction on a pending [`HydrateTrigger::Interaction`] island, marks
    /// it hydrated, and returns all queued interactions (including this one) for immediate replay.
    pub fn on_interaction(
        &mut self,
        interaction: CapturedInteraction,
    ) -> Option<Vec<CapturedInteraction>> {
        let &idx = self.by_id.get(&interaction.island_id)?;
        let spec = &mut self.islands[idx];
        if spec.hydrated || spec.trigger != HydrateTrigger::Interaction {
            return None;
        }
        spec.hydrated = true;
        let mut queue = self
            .pending_replays
            .remove(&interaction.island_id)
            .unwrap_or_default();
        queue.push(interaction);
        Some(queue)
    }

    /// Hydrates all pending [`HydrateTrigger::Idle`] islands and returns their IDs.
    pub fn on_idle(&mut self) -> Vec<String> {
        let mut ready = Vec::new();
        for spec in &mut self.islands {
            if !spec.hydrated && spec.trigger == HydrateTrigger::Idle {
                spec.hydrated = true;
                ready.push(spec.island_id.clone());
            }
        }
        ready
    }

    /// Force-marks an island hydrated and drains any queued interactions for replay.
    pub fn mark_hydrated_and_take_replays(&mut self, island_id: &str) -> Vec<CapturedInteraction> {
        if let Some(&idx) = self.by_id.get(island_id) {
            self.islands[idx].hydrated = true;
        }
        self.pending_replays.remove(island_id).unwrap_or_default()
    }
}
