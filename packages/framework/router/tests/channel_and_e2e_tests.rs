//! Binary `NVN1` fuzz tests, `NveChannelBus` bidirectional sync tests, and live Dioxus + `NVE1`
//! wire scene E2E tests for `nve-router`.

use nve_abi::{EventPayload, FrameId, NativeEvent, NativeEventKind, SurfaceId};
use nve_components::tables::{Accessibility, AriaRole, DebugId, Interaction, TextContent};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_interop::{
    ChannelPayload, NveChannelBus, PayloadEncoding, CHANNEL_ID_NAVIGATION, TOPIC_NAVIGATION,
};
use nve_router::prelude::*;
use nve_router::{
    decode_nav_packet, decode_nav_packet_ref, encode_nav_packet, InboundApplyOutcome, NavPacketRef,
    Nvn1DecodeError, Nvn1EncodeError, NVN1_FLAG_CAN_GO_BACK, NVN1_FLAG_CAN_GO_FORWARD,
    NVN1_FLAG_IS_MODAL, NVN1_MAGIC, NVN1_VERSION,
};

define_routes! {
    pub enum E2eRoute {
        #[route("/", title = "Home")]
        Home,
        #[route("/settings/profile", title = "Profile")]
        Profile,
        #[route("/issues/:id", title = "Issue Detail")]
        Issue {
            id: u64,
            #[query]
            tab: Option<String>,
        },
        #[route("/docs/*slug", title = "Docs")]
        Docs { slug: String },
        #[route("/modal/confirm/:action", title = "Confirm")]
        ConfirmModal { action: String },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum E2eTab {
    Home,
    Docs,
}

// ---------------------------------------------------------------------------
// 1. Binary NVN1 exact wire layout, truncation, corruption & fuzzing
// ---------------------------------------------------------------------------

#[test]
fn nvn1_wire_layout_roundtrip_and_adversarial_fuzz() {
    let packet = NavPacket {
        action: NavigationAction::Push,
        can_go_back: true,
        can_go_forward: false,
        is_modal: true,
        stack_depth: 5,
        active_tab_index: 2,
        uri: "/issues/42?tab=comments".into(),
        tab_id: "issues".into(),
    };

    let bytes = packet.encode().unwrap();
    assert_eq!(&bytes[0..4], &NVN1_MAGIC);
    assert_eq!(bytes[4], NVN1_VERSION);
    assert_eq!(bytes[5], NavigationAction::Push as u8);
    assert_eq!(bytes[6], NVN1_FLAG_CAN_GO_BACK | NVN1_FLAG_IS_MODAL);
    assert_eq!(bytes[7], 0);
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 5);
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 2);
    assert_eq!(
        u16::from_le_bytes(bytes[16..18].try_into().unwrap()) as usize,
        packet.uri.len()
    );
    assert_eq!(decode_nav_packet_ref(&bytes).unwrap().to_owned(), packet);

    // Every truncation prefix returns Truncated cleanly.
    for cut in 0..bytes.len() {
        assert_eq!(decode_nav_packet(&bytes[..cut]), Err(Nvn1DecodeError::Truncated));
    }

    let mut trailing = bytes.clone();
    trailing.push(0x99);
    assert_eq!(decode_nav_packet(&trailing), Err(Nvn1DecodeError::TrailingBytes(1)));

    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert!(matches!(decode_nav_packet(&bad), Err(Nvn1DecodeError::InvalidMagic(_))));

    let mut bad = bytes.clone();
    bad[4] = 99;
    assert_eq!(decode_nav_packet(&bad), Err(Nvn1DecodeError::UnsupportedVersion(99)));

    for invalid_action in 9u8..=255 {
        let mut bad = bytes.clone();
        bad[5] = invalid_action;
        assert_eq!(
            decode_nav_packet(&bad),
            Err(Nvn1DecodeError::InvalidAction(invalid_action))
        );
    }

    let mut bad = bytes.clone();
    bad[6] = NVN1_FLAG_CAN_GO_FORWARD | 0x80;
    assert_eq!(decode_nav_packet(&bad), Err(Nvn1DecodeError::InvalidFlags(0x82)));

    let mut bad = bytes.clone();
    bad[7] = 1;
    assert_eq!(decode_nav_packet(&bad), Err(Nvn1DecodeError::NonZeroReserved(1)));

    let mut bad = bytes.clone();
    bad[18] = 0xFF;
    assert_eq!(decode_nav_packet(&bad), Err(Nvn1DecodeError::InvalidUtf8("uri")));

    let mut bad = bytes.clone();
    let last = bad.len() - 1;
    bad[last] = 0xFF;
    assert_eq!(decode_nav_packet(&bad), Err(Nvn1DecodeError::InvalidUtf8("tab_id")));

    let huge = "a".repeat(u16::MAX as usize + 1);
    assert_eq!(
        encode_nav_packet(NavPacketRef {
            action: NavigationAction::Push,
            can_go_back: false,
            can_go_forward: false,
            is_modal: false,
            stack_depth: 1,
            active_tab_index: 0,
            uri: &huge,
            tab_id: "",
        }),
        Err(Nvn1EncodeError::UriTooLong(u16::MAX as usize + 1))
    );

    // Deterministic xorshift fuzzing -> zero panics.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for iter in 0..512 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let len = (seed as usize) % 64;
        let mut buf = vec![0u8; len];
        for b in &mut buf {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 24) as u8;
        }
        if iter % 4 == 0 && buf.len() >= 4 {
            buf[0..4].copy_from_slice(&NVN1_MAGIC);
        }
        let _ = decode_nav_packet_ref(&buf);
    }
}

// ---------------------------------------------------------------------------
// 2. Live NveChannelBus bidirectional sync + loop prevention
// ---------------------------------------------------------------------------

#[test]
fn channel_bus_bidirectional_sync_retain_and_loop_prevention() {
    let mut bus = NveChannelBus::new();
    assert_eq!(bus.channel_id(TOPIC_NAVIGATION), Some(CHANNEL_ID_NAVIGATION));

    let mut router_a = RouterState::new(E2eRoute::Home);
    let mut bridge_a = RouterChannelBridge::attach(&mut bus, SurfaceId::new(10, 0), false).unwrap();

    let tr = router_a.push(E2eRoute::Profile).unwrap();
    bridge_a.publish_transition(&mut bus, &tr).unwrap();
    assert!(bridge_a.drain_inbound().is_empty());

    let retained = bus.retained(TOPIC_NAVIGATION).expect("retained nav state");
    assert_eq!(retained.sender_surface, Some(SurfaceId::new(10, 0)));
    assert_eq!(retained.encoding, PayloadEncoding::Binary);
    let retained_pkt = decode_nav_packet(retained.as_bytes()).unwrap();
    assert_eq!(retained_pkt.uri, "/settings/profile");
    assert!(retained_pkt.can_go_back);

    // Late joiner replays retained NVN1 state.
    let bridge_b = RouterChannelBridge::attach(&mut bus, SurfaceId::new(20, 0), true).unwrap();
    let mut router_b = RouterState::new(E2eRoute::Home);
    let outcomes = bridge_b.apply_inbound_to_router(&mut router_b, None);
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], InboundApplyOutcome::Applied(_)));
    assert_eq!(router_b.current_route(), &E2eRoute::Profile);

    // External OS deep-link command + malformed packet resilience.
    let dl_cfg = DeepLinkConfig::new()
        .with_scheme("nve")
        .with_base_prefix("/app");
    let os_pkt = NavPacket {
        action: NavigationAction::DeepLink,
        can_go_back: false,
        can_go_forward: false,
        is_modal: false,
        stack_depth: 1,
        active_tab_index: 0,
        uri: "nve://app/issues/77?tab=files".into(),
        tab_id: String::new(),
    };
    bus.publish_by_id(
        CHANNEL_ID_NAVIGATION,
        Some(SurfaceId::new(99, 0)),
        PayloadEncoding::Binary,
        ChannelPayload::from_vec(os_pkt.encode().unwrap()),
        true,
    )
    .unwrap();
    bus.publish_by_id(
        CHANNEL_ID_NAVIGATION,
        Some(SurfaceId::new(99, 0)),
        PayloadEncoding::Binary,
        ChannelPayload::from_slice(b"BAD!"),
        false,
    )
    .unwrap();
    assert_eq!(bridge_a.decode_error_count(), 1);

    let applied_a = bridge_a.apply_inbound_to_router(&mut router_a, Some(&dl_cfg));
    assert_eq!(applied_a.len(), 1);
    assert_eq!(
        router_a.current_route(),
        &E2eRoute::Issue {
            id: 77,
            tab: Some("files".into()),
        }
    );
    assert!(bridge_a.last_inbound_seq() >= 2);
    assert_eq!(bridge_a.published_count(), 1);

    // Tab router inbound SwitchTab via channel bus.
    let mut tab_router = TabRouterState::with_specs(
        vec![
            TabSpec::new(E2eTab::Home, "home", E2eRoute::Home),
            TabSpec::new(E2eTab::Docs, "docs", E2eRoute::Docs { slug: "api".into() }),
        ],
        E2eTab::Home,
    )
    .unwrap();
    let tab_bridge = RouterChannelBridge::attach(&mut bus, SurfaceId::new(30, 0), false).unwrap();
    let switch_pkt = NavPacket {
        action: NavigationAction::SwitchTab,
        can_go_back: false,
        can_go_forward: false,
        is_modal: false,
        stack_depth: 1,
        active_tab_index: 1,
        uri: "/docs/api".into(),
        tab_id: "docs".into(),
    };
    bus.publish_by_id(
        CHANNEL_ID_NAVIGATION,
        Some(SurfaceId::new(99, 0)),
        PayloadEncoding::Binary,
        ChannelPayload::from_vec(switch_pkt.encode().unwrap()),
        true,
    )
    .unwrap();
    let tab_outcomes = tab_bridge.apply_inbound_to_tabs(&mut tab_router, None);
    assert_eq!(tab_outcomes.len(), 1);
    assert_eq!(tab_router.active_tab(), E2eTab::Docs);
    bridge_a.detach(&mut bus).unwrap();
}

// ---------------------------------------------------------------------------
// 3. Live Dioxus + NVE2 NodeStore wire replica E2E tests
// ---------------------------------------------------------------------------

struct Nve2Replica {
    store: NodeStore,
    decoder: WireDecoder,
    changes: ChangeSet,
}

impl Nve2Replica {
    fn new() -> Self {
        Self {
            store: NodeStore::replica(),
            decoder: WireDecoder::default(),
            changes: ChangeSet::default(),
        }
    }

    fn apply(&mut self, bytes: &[u8]) {
        assert!(bytes.len() >= 24 && &bytes[0..4] == b"NVE2");
        let mut tx = self.store.tx();
        self.decoder.apply(&mut tx, bytes).expect("valid NVE2 frame");
        tx.commit_into(&mut self.changes);
    }

    fn collect_texts(&self) -> String {
        let mut out = Vec::new();
        self.walk_texts(NodeId::ROOT, &mut out);
        out.join(" | ")
    }

    fn walk_texts(&self, id: NodeId, out: &mut Vec<String>) {
        if let Some(tc) = self.store.try_row::<TextContent>(id) {
            let s = tc.text();
            if !s.is_empty() {
                out.push(s.to_owned());
            }
        }
        for &kid in self.store.children(id) {
            self.walk_texts(kid, out);
        }
    }

    fn find_by_test_id(&self, test_id: &str) -> Option<NodeId> {
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            if id != NodeId::ROOT
                && let Some(dbg) = self.store.try_row::<DebugId>(id)
                && dbg.test_id() == test_id
            {
                return Some(id);
            }
            stack.extend_from_slice(self.store.children(id));
        }
        None
    }
}

fn render_e2e_route(route: &E2eRoute) -> Element {
    let nav = use_navigator::<E2eRoute>();
    let current = use_route::<E2eRoute>();
    let path_str = current.to_path();
    match route {
        E2eRoute::Home => rsx! {
            view {
                text { "Screen: Home ({path_str})" }
                Link::<E2eRoute> {
                    to: E2eRoute::Issue {
                        id: 42,
                        tab: Some("comments".into()),
                    },
                    test_id: Some("link-issue-42".into()),
                    text { "Open Issue 42" }
                }
            }
        },
        E2eRoute::Issue { id, tab } => {
            let tab_label = tab.as_deref().unwrap_or("overview");
            rsx! {
                view {
                    text { "Screen: Issue #{id} ({tab_label})" }
                    Link::<E2eRoute> {
                        to: E2eRoute::ConfirmModal { action: "archive".into() },
                        modal: true,
                        test_id: Some("link-modal-archive".into()),
                        text { "Archive" }
                    }
                    pressable {
                        test_id: "btn-back",
                        onpress: move |_| { let _ = nav.pop(); },
                        text { "Go Back" }
                    }
                }
            }
        }
        E2eRoute::ConfirmModal { action } => rsx! {
            view {
                text { "Modal: Confirm {action}" }
                pressable {
                    test_id: "btn-dismiss-modal",
                    onpress: move |_| { let _ = nav.dismiss_modal(); },
                    text { "Close Modal" }
                }
            }
        },
        other => rsx! { view { text { "Screen: {other:?}" } } },
    }
}

#[component]
fn RouterE2eApp() -> Element {
    rsx! {
        RouterProvider::<E2eRoute> {
            initial: E2eRoute::Home,
            render: RouteRenderer::new(render_e2e_route),
            render_modal: Some(RouteRenderer::new(render_e2e_route)),
            view { Outlet {} }
        }
    }
}

fn press_test_id(surface: &mut DioxusSurface, scene: &mut Nve2Replica, test_id: &str, frame_id: u64) {
    let node_id = scene.find_by_test_id(test_id);
    assert!(node_id.is_some(), "missing test_id {test_id}");
    let node_id = node_id.expect("checked above");
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: SurfaceId::ROOT,
        node_id,
        timestamp_us: frame_id * 16_000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    }]));
    let mut wire = Vec::new();
    surface.render_nve2(FrameId(frame_id), &mut wire);
    scene.apply(&wire);
}

#[test]
fn dioxus_e2e_router_provider_link_outlet_press_navigation_and_modal() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, RouterE2eApp);
    let mut scene = Nve2Replica::new();
    let mut wire = Vec::new();
    surface.render_nve2(FrameId(1), &mut wire);
    scene.apply(&wire);
    assert!(scene.collect_texts().contains("Screen: Home (/)"));

    let link_id = scene.find_by_test_id("link-issue-42").expect("link node");
    assert_eq!(
        scene.store.try_row::<Accessibility>(link_id).and_then(|a| a.role()),
        Some(AriaRole::Link)
    );
    assert_ne!(
        scene
            .store
            .try_row::<Interaction>(link_id)
            .map_or(0, |i| i.event_mask()),
        0
    );

    press_test_id(&mut surface, &mut scene, "link-issue-42", 2);
    let texts2 = scene.collect_texts();
    assert!(texts2.contains("Screen: Issue #42 (comments)"), "frame2: {texts2}");
    assert!(!texts2.contains("Screen: Home"));

    press_test_id(&mut surface, &mut scene, "link-modal-archive", 3);
    let texts3 = scene.collect_texts();
    assert!(texts3.contains("Screen: Issue #42 (comments)"), "frame3: {texts3}");
    assert!(texts3.contains("Modal: Confirm archive"), "frame3: {texts3}");

    press_test_id(&mut surface, &mut scene, "btn-dismiss-modal", 4);
    assert!(!scene.collect_texts().contains("Modal: Confirm"));

    press_test_id(&mut surface, &mut scene, "btn-back", 5);
    assert!(scene.collect_texts().contains("Screen: Home (/)"));
}

#[component]
fn TabBarShell() -> Element {
    let tabs = use_tab_navigator::<E2eTab, E2eRoute>();
    let active = use_active_tab::<E2eTab, E2eRoute>();
    rsx! {
        view {
            text { "ActiveTab: {active:?}" }
            pressable {
                test_id: "tab-home",
                onpress: move |_| { let _ = tabs.select_tab(E2eTab::Home); },
                text { "HomeTab" }
            }
            pressable {
                test_id: "tab-docs",
                onpress: move |_| { let _ = tabs.select_tab(E2eTab::Docs); },
                text { "DocsTab" }
            }
            Outlet {}
        }
    }
}

#[component]
fn TabRouterE2eApp() -> Element {
    rsx! {
        TabRouterProvider::<E2eTab, E2eRoute> {
            tabs: vec![
                TabSpec::new(E2eTab::Home, "home", E2eRoute::Home),
                TabSpec::new(E2eTab::Docs, "docs", E2eRoute::Docs { slug: "intro".into() }),
            ],
            initial_tab: E2eTab::Home,
            render: RouteRenderer::new(render_e2e_route),
            TabBarShell {}
        }
    }
}

#[test]
fn dioxus_e2e_tab_router_provider_preserves_stack_and_retap_pops_to_root() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, TabRouterE2eApp);
    let mut scene = Nve2Replica::new();
    let mut wire = Vec::new();
    surface.render_nve2(FrameId(1), &mut wire);
    scene.apply(&wire);
    assert!(scene.collect_texts().contains("ActiveTab: Home"));
    assert!(scene.collect_texts().contains("Screen: Home (/)"));

    // Navigate deeper inside Home tab.
    press_test_id(&mut surface, &mut scene, "link-issue-42", 2);
    assert!(scene.collect_texts().contains("Screen: Issue #42 (comments)"));

    // Switch to Docs tab.
    press_test_id(&mut surface, &mut scene, "tab-docs", 3);
    assert!(scene.collect_texts().contains("ActiveTab: Docs"));
    assert!(scene.collect_texts().contains("Docs { slug: \"intro\" }"));

    // Switch back to Home tab -> Issue #42 preserved!
    press_test_id(&mut surface, &mut scene, "tab-home", 4);
    assert!(scene.collect_texts().contains("ActiveTab: Home"));
    assert!(scene.collect_texts().contains("Screen: Issue #42 (comments)"));

    // Re-tap active Home tab -> pops back to Home root!
    press_test_id(&mut surface, &mut scene, "tab-home", 5);
    assert!(scene.collect_texts().contains("Screen: Home (/)"));
}
