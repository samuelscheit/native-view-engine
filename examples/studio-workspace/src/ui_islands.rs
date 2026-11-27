//! Live Multi-Surface Orchestration Sandbox view (`ActiveView::Islands`).
//!
//! Backed directly by `nve_interop::{SurfaceOrchestrator, NveChannelBus, GestureArbitrator}`
//! showcasing surface lifecycle transitions, binary `NveChannelBus` broadcasts, and
//! cross-surface `GestureArbitrator` claim / pass-through / defer-to-parent simulation.

use std::sync::{Arc, Mutex};

use nve_abi::SurfaceId;
use nve_interop::{
    ArbitrationDecision, ChannelPayload, ColorScheme, ContrastLevel, FrameworkKind,
    GestureArbitrator, GestureCapabilities, GestureDisposition, InteropError, LifecycleState,
    NveChannelBus, PointerPhase, PointerSample, ScrollEdgeState, SurfaceDescriptor,
    SurfaceEnvironment, SurfaceOrchestrator, TOPIC_THEME, ThemeContext,
};
use serde::{Deserialize, Serialize};

use crate::state::AccentTheme;

/// Custom zero-copy binary pub/sub topic shared across hosted islands.
pub const TOPIC_WORKSPACE_SYNC: &str = "workspace.sync";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IslandHostKind {
    ReactNative,
    Flutter,
    SwiftUI,
    JetpackCompose,
}

impl IslandHostKind {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::ReactNative => "rn",
            Self::Flutter => "flutter",
            Self::SwiftUI => "swiftui",
            Self::JetpackCompose => "compose",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ReactNative => "React Native",
            Self::Flutter => "Flutter",
            Self::SwiftUI => "SwiftUI",
            Self::JetpackCompose => "Compose",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GestureSimMode {
    Claim,
    PassThrough,
    DeferToParent,
}

impl GestureSimMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claim => "Claim",
            Self::PassThrough => "PassThrough",
            Self::DeferToParent => "DeferToParent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IslandCardState {
    pub slug: String,
    pub host: IslandHostKind,
    pub surface_id: SurfaceId,
    pub component_name: String,
    pub mount_policy: String,
    pub lifecycle: String,
    pub received_messages: u32,
}

/// Serializable state of the multi-host `<Island>` sandbox persisted across Time-Travel & hot-swap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IslandsSnapshot {
    pub root_surface_id: SurfaceId,
    pub rail_surface_id: SurfaceId,
    pub active_surface_count: usize,
    pub cards: Vec<IslandCardState>,
    pub bus_broadcast_count: u32,
    pub bus_total_deliveries: u32,
    pub last_bus_summary: String,
    pub last_gesture_disposition: String,
    pub last_gesture_winner: Option<SurfaceId>,
    pub last_gesture_reason: String,
    pub last_gesture_cancelled_count: usize,
}

impl Default for IslandsSnapshot {
    fn default() -> Self {
        IslandSandboxRuntime::boot(0, 0).initial_snapshot()
    }
}

type DeliveryLogEntry = (SurfaceId, String, u64, usize);

/// Multi-surface orchestration runtime coordinating `SurfaceOrchestrator`,
/// `NveChannelBus`, and `GestureArbitrator`.
pub struct IslandSandboxRuntime {
    pub orchestrator: SurfaceOrchestrator,
    pub bus: NveChannelBus,
    pub arbitrator: GestureArbitrator,
    pub root_id: SurfaceId,
    pub rail_id: SurfaceId,
    pub islands: Vec<(IslandHostKind, SurfaceId, &'static str)>,
    delivery_log: Arc<Mutex<Vec<DeliveryLogEntry>>>,
}

/// `ThemeContext::accent_rgba` (big-endian RGBA) of a workspace accent choice.
const fn accent_rgba(accent: AccentTheme) -> u32 {
    let c = accent.accent().srgb();
    u32::from_be_bytes([c.r, c.g, c.b, c.a])
}

impl IslandSandboxRuntime {
    /// Boots the orchestrator, bus and arbitrator, registering the root, rail and island surfaces.
    ///
    /// # Errors
    /// Propagates any orchestrator / bus registration failure.
    pub fn new() -> Result<Self, InteropError> {
        let mut orchestrator = SurfaceOrchestrator::new();
        let mut bus = NveChannelBus::new();
        let mut arbitrator = GestureArbitrator::new();
        let delivery_log = Arc::new(Mutex::new(Vec::<DeliveryLogEntry>::new()));

        let root_env = SurfaceEnvironment {
            theme: ThemeContext {
                color_scheme: Some(ColorScheme::Dark),
                contrast: ContrastLevel::Standard,
                accent_rgba: accent_rgba(AccentTheme::Cyan),
                tokens: Default::default(),
            },
            scale_factor: 2.0,
            ..SurfaceEnvironment::default()
        };

        let root_id =
            orchestrator.register_root(FrameworkKind::Nve, "AetherWorkspaceRoot", root_env)?;
        let rail_id = orchestrator.register_child(
            root_id,
            FrameworkKind::Nve,
            "AetherIslandHorizontalRail",
        )?;

        arbitrator.set_capabilities(
            root_id,
            GestureCapabilities::vertical_scroll(FrameworkKind::Nve),
        );
        arbitrator.set_capabilities(
            rail_id,
            GestureCapabilities::horizontal_scroll(FrameworkKind::Nve),
        );

        let specs: Vec<(
            IslandHostKind,
            FrameworkKind,
            &str,
            &'static str,
            GestureCapabilities,
        )> = vec![
            (
                IslandHostKind::ReactNative,
                FrameworkKind::ReactNativeFabric,
                "RNLiveMapView",
                "LazyOnVisible",
                GestureCapabilities::vertical_scroll(FrameworkKind::ReactNativeFabric),
            ),
            (
                IslandHostKind::Flutter,
                FrameworkKind::Flutter,
                "RevenueSparkChart",
                "LazyOnVisible",
                GestureCapabilities::horizontal_scroll(FrameworkKind::Flutter),
            ),
            (
                IslandHostKind::SwiftUI,
                FrameworkKind::SwiftUI,
                "ApplePayCheckoutCard",
                "OnFirstInteraction",
                GestureCapabilities {
                    framework: FrameworkKind::SwiftUI,
                    ..GestureCapabilities::default()
                },
            ),
            (
                IslandHostKind::JetpackCompose,
                FrameworkKind::JetpackCompose,
                "MediaPlayerSheet",
                "EagerOnCreate",
                GestureCapabilities::vertical_scroll(FrameworkKind::JetpackCompose),
            ),
        ];

        let _ = bus.ensure_channel(TOPIC_WORKSPACE_SYNC);

        let mut islands = Vec::with_capacity(specs.len());
        for (host, framework, name, policy, caps) in specs {
            let sid = orchestrator
                .register_surface(SurfaceDescriptor::new(framework, name).with_parent(rail_id))?;
            arbitrator.set_capabilities(sid, caps);

            for topic in [TOPIC_THEME, TOPIC_WORKSPACE_SYNC] {
                let log_ref = Arc::clone(&delivery_log);
                bus.subscribe(topic, Some(sid), false, move |msg| {
                    if let Ok(mut guard) = log_ref.lock() {
                        guard.push((sid, msg.topic.clone(), msg.seq, msg.payload.len()));
                    }
                })?;
            }

            islands.push((host, sid, policy));
        }

        // Bring root + Compose (EagerOnCreate) to Resumed at boot while lazy islands start Mounted.
        let _ = orchestrator.transition_state(root_id, LifecycleState::Resumed);
        let _ = orchestrator.transition_state(rail_id, LifecycleState::Resumed);
        if let Some((_, sid, _)) = islands
            .iter()
            .find(|(h, _, _)| *h == IslandHostKind::JetpackCompose)
        {
            let _ = orchestrator.transition_state(*sid, LifecycleState::Resumed);
        }

        Ok(Self {
            orchestrator,
            bus,
            arbitrator,
            root_id,
            rail_id,
            islands,
            delivery_log,
        })
    }

    /// [`Self::new`], or where it cannot boot a runtime with no registered surfaces anchored at
    /// `root_id` / `rail_id`: every lookup misses, so toggles are no-ops, broadcasts reach nobody,
    /// gestures find no surface, and the snapshot lists no cards.
    fn boot(root_id: SurfaceId, rail_id: SurfaceId) -> Self {
        Self::new().unwrap_or_else(|_| Self {
            orchestrator: SurfaceOrchestrator::new(),
            bus: NveChannelBus::new(),
            arbitrator: GestureArbitrator::new(),
            root_id,
            rail_id,
            islands: Vec::new(),
            delivery_log: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Reconstructs a live `IslandSandboxRuntime` hydrated to match a persisted [`IslandsSnapshot`].
    ///
    /// If the runtime cannot boot, every operation is a no-op (see [`Self::boot`]).
    pub fn from_snapshot(snap: &IslandsSnapshot) -> Self {
        let mut rt = Self::boot(snap.root_surface_id, snap.rail_surface_id);
        for card in &snap.cards {
            if let Some(sid) = rt.surface_id_for_slug(&card.slug) {
                let target = parse_lifecycle(&card.lifecycle);
                if rt
                    .orchestrator
                    .get(sid)
                    .is_some_and(|rec| rec.state != target)
                {
                    let _ = rt.orchestrator.transition_state(sid, target);
                }
            }
        }
        rt
    }

    pub fn surface_id_for_slug(&self, slug: &str) -> Option<SurfaceId> {
        self.islands
            .iter()
            .find(|(h, _, _)| h.slug() == slug)
            .map(|(_, sid, _)| *sid)
    }

    pub fn rn_surface_id(&self) -> Option<SurfaceId> {
        self.surface_id_for_slug(IslandHostKind::ReactNative.slug())
    }

    pub fn initial_snapshot(&self) -> IslandsSnapshot {
        let cards = self
            .islands
            .iter()
            .filter_map(|(host, sid, policy)| {
                let rec = self.orchestrator.get(*sid)?;
                Some(IslandCardState {
                    slug: host.slug().into(),
                    host: *host,
                    surface_id: *sid,
                    component_name: rec.component_name.clone(),
                    mount_policy: (*policy).into(),
                    lifecycle: format_lifecycle(rec.state).into(),
                    received_messages: 0,
                })
            })
            .collect();

        IslandsSnapshot {
            root_surface_id: self.root_id,
            rail_surface_id: self.rail_id,
            active_surface_count: self.orchestrator.active_count(),
            cards,
            bus_broadcast_count: 0,
            bus_total_deliveries: 0,
            last_bus_summary: "Bus idle (4 islands subscribed)".into(),
            last_gesture_disposition: "Idle".into(),
            last_gesture_winner: None,
            last_gesture_reason: "Tap Claim / PassThrough / Defer".into(),
            last_gesture_cancelled_count: 0,
        }
    }

    /// Toggles an island surface's lifecycle via real `SurfaceOrchestrator::transition_state`.
    pub fn toggle_island_lifecycle(&mut self, slug: &str) -> Option<(SurfaceId, LifecycleState)> {
        let sid = self.surface_id_for_slug(slug)?;
        let current = self.orchestrator.get(sid)?.state;
        let next = match current {
            LifecycleState::Mounted | LifecycleState::Paused => LifecycleState::Resumed,
            LifecycleState::Resumed => LifecycleState::Paused,
            LifecycleState::Destroyed => return None,
        };
        self.orchestrator.transition_state(sid, next).ok()?;
        Some((sid, next))
    }

    /// Broadcasts the active theme across all subscribed surfaces over `NveChannelBus` (`TOPIC_THEME`)
    /// using a compact binary payload (version 2: `scheme=dark`, accent label, big-endian RGBA).
    pub fn broadcast_theme(&mut self, accent: AccentTheme) -> usize {
        let label = accent.label().as_bytes();
        let mut bytes = Vec::with_capacity(7 + label.len());
        bytes.push(2u8); // version 2
        bytes.push(2u8); // dark scheme
        bytes.push(label.len() as u8);
        bytes.extend_from_slice(label);
        bytes.extend_from_slice(&accent_rgba(accent).to_be_bytes());
        self.bus
            .publish_bytes(
                TOPIC_THEME,
                Some(self.root_id),
                ChannelPayload::from_vec(bytes),
                true,
            )
            .unwrap_or(0)
    }

    /// Broadcasts a zero-copy binary workspace sync packet over `NveChannelBus` (`TOPIC_WORKSPACE_SYNC`).
    pub fn broadcast_sync(&mut self, frame_count: u64, task_count: usize) -> usize {
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&frame_count.to_le_bytes());
        bytes.extend_from_slice(&(task_count as u32).to_le_bytes());
        self.bus
            .publish_bytes(
                TOPIC_WORKSPACE_SYNC,
                Some(self.root_id),
                ChannelPayload::from_vec(bytes),
                false,
            )
            .unwrap_or(0)
    }

    /// Runs a real pointer arbitration sequence (`Down -> Move -> Up`) on the React Native
    /// island ancestor chain `[rn_island, horizontal_rail, root_nve]` via `GestureArbitrator`.
    pub fn simulate_gesture(&mut self, mode: GestureSimMode) -> ArbitrationDecision {
        // No React Native island (detached runtime) => empty chain => `NoEligibleSurface`.
        let rn_id = self.rn_surface_id();
        let chain = rn_id
            .and_then(|id| self.orchestrator.ancestor_chain(id).ok())
            .unwrap_or_default();

        let edges = match mode {
            GestureSimMode::DeferToParent => ScrollEdgeState {
                at_bottom: true,
                ..Default::default()
            },
            GestureSimMode::Claim | GestureSimMode::PassThrough => ScrollEdgeState::default(),
        };
        if let Some(rn_id) = rn_id {
            self.arbitrator.update_scroll_edges(rn_id, edges);
        }

        let pointer_id = match mode {
            GestureSimMode::Claim => 101,
            GestureSimMode::PassThrough => 102,
            GestureSimMode::DeferToParent => 103,
        };
        let (dx, dy) = match mode {
            GestureSimMode::Claim | GestureSimMode::DeferToParent => (0.0, -32.0),
            GestureSimMode::PassThrough => (-42.0, 2.0),
        };

        let _down = self.arbitrator.arbitrate(
            &chain,
            PointerSample {
                pointer_id,
                phase: PointerPhase::Down,
                x: 120.0,
                y: 220.0,
                delta_x: 0.0,
                delta_y: 0.0,
                timestamp_us: 1_000,
            },
        );

        let decision = self.arbitrator.arbitrate(
            &chain,
            PointerSample {
                pointer_id,
                phase: PointerPhase::Move,
                x: 120.0 + dx,
                y: 220.0 + dy,
                delta_x: dx,
                delta_y: dy,
                timestamp_us: 2_000,
            },
        );

        let _up = self.arbitrator.arbitrate(
            &chain,
            PointerSample {
                pointer_id,
                phase: PointerPhase::Up,
                x: 120.0 + dx,
                y: 220.0 + dy,
                delta_x: 0.0,
                delta_y: 0.0,
                timestamp_us: 3_000,
            },
        );

        decision
    }

    pub fn delivery_count_for(&self, sid: SurfaceId) -> usize {
        self.delivery_log
            .lock()
            .map(|g| g.iter().filter(|(id, _, _, _)| *id == sid).count())
            .unwrap_or(0)
    }
}

pub fn format_lifecycle(state: LifecycleState) -> &'static str {
    match state {
        LifecycleState::Mounted => "Mounted",
        LifecycleState::Resumed => "Resumed",
        LifecycleState::Paused => "Paused",
        LifecycleState::Destroyed => "Destroyed",
    }
}

fn parse_lifecycle(raw: &str) -> LifecycleState {
    match raw {
        "Resumed" => LifecycleState::Resumed,
        "Paused" => LifecycleState::Paused,
        "Destroyed" => LifecycleState::Destroyed,
        _ => LifecycleState::Mounted,
    }
}

pub fn format_disposition(d: GestureDisposition) -> &'static str {
    match d {
        GestureDisposition::Claim => "Claim",
        GestureDisposition::PassThrough => "PassThrough",
        GestureDisposition::DeferToParent => "DeferToParent",
    }
}
