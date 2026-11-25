//! Live `<Island>` Multi-Host Sandbox view (`ActiveView::Islands`).
//!
//! Backed directly by real `nve_interop::{SurfaceOrchestrator, IslandSpec, NveChannelBus,
//! GestureArbitrator}` showcasing interactive `ReactNative`, `Flutter`, `SwiftUI`, and
//! `JetpackCompose` island cards with live zero-copy `NveChannelBus` broadcasts and
//! cross-framework `GestureArbitrator` claim / pass-through / defer-to-parent simulation.

use std::sync::{Arc, Mutex};

use nve_abi::SurfaceId;
use nve_interop::{
    ArbitrationDecision, ChannelPayload, ColorScheme, ComposeIslandSpec, FlutterEngineGroupConfig,
    FlutterIslandSpec, FrameworkKind, GestureArbitrator, GestureCapabilities, GestureDisposition,
    IslandMountPolicy, IslandSpec, LifecycleState, NveChannelBus, PointerPhase, PointerSample,
    RnIslandSpec, ScrollEdgeState, SurfaceDescriptor, SurfaceEnvironment, SurfaceOrchestrator,
    SwiftUIIslandSpec, ThemeContext, TOPIC_THEME,
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

    pub const fn badge_bg(self) -> &'static str {
        match self {
            Self::ReactNative => "#0c4a6e",
            Self::Flutter => "#172554",
            Self::SwiftUI => "#2e1065",
            Self::JetpackCompose => "#052e16",
        }
    }

    pub const fn badge_fg(self) -> &'static str {
        match self {
            Self::ReactNative => "#7dd3fc",
            Self::Flutter => "#93c5fd",
            Self::SwiftUI => "#c4b5fd",
            Self::JetpackCompose => "#86efac",
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
        let rt = IslandSandboxRuntime::new();
        rt.initial_snapshot()
    }
}

/// Real `nve_interop` multi-host runtime coordinating `SurfaceOrchestrator`, `IslandSpec`,
/// `NveChannelBus`, and `GestureArbitrator`.
pub struct IslandSandboxRuntime {
    pub orchestrator: SurfaceOrchestrator,
    pub bus: NveChannelBus,
    pub arbitrator: GestureArbitrator,
    pub root_id: SurfaceId,
    pub rail_id: SurfaceId,
    pub islands: Vec<(IslandHostKind, SurfaceId, IslandSpec)>,
    delivery_log: Arc<Mutex<Vec<(SurfaceId, String, u64, usize)>>>,
}

impl Default for IslandSandboxRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl IslandSandboxRuntime {
    pub fn new() -> Self {
        let mut orchestrator = SurfaceOrchestrator::new();
        let mut bus = NveChannelBus::new();
        let mut arbitrator = GestureArbitrator::new();
        let delivery_log = Arc::new(Mutex::new(Vec::<(SurfaceId, String, u64, usize)>::new()));

        let root_env = SurfaceEnvironment {
            theme: ThemeContext {
                color_scheme: ColorScheme::Dark,
                accent_rgba: 0x38BD_F8FF,
                tokens: Default::default(),
            },
            scale_factor: 2.0,
            ..SurfaceEnvironment::default()
        };

        let root_id = orchestrator
            .register_root(FrameworkKind::Nve, "AetherWorkspaceRoot", root_env)
            .expect("register root NVE surface");
        let rail_id = orchestrator
            .register_child(root_id, FrameworkKind::Nve, "AetherIslandHorizontalRail")
            .expect("register middle NVE horizontal rail");

        arbitrator.set_capabilities(root_id, GestureCapabilities::vertical_scroll(FrameworkKind::Nve));
        arbitrator.set_capabilities(
            rail_id,
            GestureCapabilities::horizontal_scroll(FrameworkKind::Nve),
        );

        let specs: Vec<(IslandHostKind, &str, IslandSpec, GestureCapabilities)> = vec![
            (
                IslandHostKind::ReactNative,
                "RNLiveMapView",
                IslandSpec::ReactNative(RnIslandSpec {
                    module_name: "MapsModule".into(),
                    component_name: "RNLiveMapView".into(),
                    initial_props_json: r#"{"zoom":14,"dark":true}"#.into(),
                    fabric_enabled: true,
                    hermes_bytecode_path: None,
                    mount_policy: IslandMountPolicy::LazyOnVisible,
                }),
                GestureCapabilities::vertical_scroll(FrameworkKind::ReactNativeFabric),
            ),
            (
                IslandHostKind::Flutter,
                "RevenueSparkChart",
                IslandSpec::Flutter(FlutterIslandSpec {
                    widget_name: "RevenueSparkChart".into(),
                    engine_group: FlutterEngineGroupConfig {
                        group_id: "aether_flutter_pool".into(),
                        entrypoint: "telemetryMain".into(),
                        initial_route: Some("/spark".into()),
                        dart_entrypoint_args: vec!["--120hz".into()],
                        share_isolate_group: true,
                        warm_spare_engine: true,
                    },
                    prefer_texture_compositing: true,
                    initial_props_json: r#"{"window":"7d"}"#.into(),
                    mount_policy: IslandMountPolicy::LazyOnVisible,
                }),
                GestureCapabilities::horizontal_scroll(FrameworkKind::Flutter),
            ),
            (
                IslandHostKind::SwiftUI,
                "ApplePayCheckoutCard",
                IslandSpec::SwiftUI(SwiftUIIslandSpec {
                    view_name: "ApplePayCheckoutCard".into(),
                    module_name: "PayKit".into(),
                    initial_props_json: r#"{"currency":"USD"}"#.into(),
                    use_intrinsic_sizing: true,
                    mount_policy: IslandMountPolicy::OnFirstInteraction,
                }),
                GestureCapabilities {
                    framework: FrameworkKind::SwiftUI,
                    ..GestureCapabilities::default()
                },
            ),
            (
                IslandHostKind::JetpackCompose,
                "MediaPlayerSheet",
                IslandSpec::JetpackCompose(ComposeIslandSpec {
                    composable_fqn: "dev.aether.compose.MediaPlayerSheet".into(),
                    initial_props_json: r#"{"hardwareLayer":true}"#.into(),
                    use_hardware_layer: true,
                    mount_policy: IslandMountPolicy::EagerOnCreate,
                }),
                GestureCapabilities::vertical_scroll(FrameworkKind::JetpackCompose),
            ),
        ];

        let _ = bus.ensure_channel(TOPIC_WORKSPACE_SYNC);

        let mut islands = Vec::with_capacity(specs.len());
        for (host, name, spec, caps) in specs {
            spec.validate().expect("built-in island spec valid");
            let framework = spec.framework();
            let sid = orchestrator
                .register_surface(
                    SurfaceDescriptor::new(framework, name)
                        .with_parent(rail_id)
                        .with_island(spec.clone()),
                )
                .expect("register island surface");
            arbitrator.set_capabilities(sid, caps);

            for topic in [TOPIC_THEME, TOPIC_WORKSPACE_SYNC] {
                let log_ref = Arc::clone(&delivery_log);
                bus.subscribe(topic, Some(sid), false, move |msg| {
                    if let Ok(mut guard) = log_ref.lock() {
                        guard.push((sid, msg.topic.clone(), msg.seq, msg.payload.len()));
                    }
                })
                .expect("subscribe island to NveChannelBus");
            }

            islands.push((host, sid, spec));
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

        Self {
            orchestrator,
            bus,
            arbitrator,
            root_id,
            rail_id,
            islands,
            delivery_log,
        }
    }

    /// Reconstructs a live `IslandSandboxRuntime` hydrated to match a persisted [`IslandsSnapshot`].
    pub fn from_snapshot(snap: &IslandsSnapshot) -> Self {
        let mut rt = Self::new();
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

    pub fn rn_surface_id(&self) -> SurfaceId {
        self.surface_id_for_slug(IslandHostKind::ReactNative.slug())
            .expect("RN island registered")
    }

    pub fn initial_snapshot(&self) -> IslandsSnapshot {
        let cards = self
            .islands
            .iter()
            .map(|(host, sid, spec)| {
                let rec = self.orchestrator.get(*sid).expect("island record exists");
                IslandCardState {
                    slug: host.slug().into(),
                    host: *host,
                    surface_id: *sid,
                    component_name: rec.component_name.clone(),
                    mount_policy: format_mount_policy(spec.mount_policy()).into(),
                    lifecycle: format_lifecycle(rec.state).into(),
                    received_messages: 0,
                }
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

    /// Broadcasts the active theme across all subscribed islands over `NveChannelBus` (`TOPIC_THEME`).
    pub fn broadcast_theme(&mut self, accent: AccentTheme) -> usize {
        let payload = serde_json::json!({
            "scheme": "dark",
            "accent": accent.label(),
            "hex": accent.hex(),
        });
        self.bus
            .publish_json(TOPIC_THEME, Some(self.root_id), &payload, true)
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
        let rn_id = self.rn_surface_id();
        let chain = self
            .orchestrator
            .ancestor_chain(rn_id)
            .expect("valid island ancestor chain");

        let edges = match mode {
            GestureSimMode::DeferToParent => ScrollEdgeState {
                at_bottom: true,
                ..Default::default()
            },
            GestureSimMode::Claim | GestureSimMode::PassThrough => ScrollEdgeState::default(),
        };
        self.arbitrator.update_scroll_edges(rn_id, edges);

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

fn format_mount_policy(policy: IslandMountPolicy) -> &'static str {
    match policy {
        IslandMountPolicy::LazyOnVisible => "LazyOnVisible",
        IslandMountPolicy::EagerOnCreate => "EagerOnCreate",
        IslandMountPolicy::OnFirstInteraction => "OnFirstInteraction",
    }
}

pub fn format_disposition(d: GestureDisposition) -> &'static str {
    match d {
        GestureDisposition::Claim => "Claim",
        GestureDisposition::PassThrough => "PassThrough",
        GestureDisposition::DeferToParent => "DeferToParent",
    }
}
