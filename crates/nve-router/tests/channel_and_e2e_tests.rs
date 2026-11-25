//! Binary `NVN1` fuzz tests, `NveChannelBus` bidirectional sync tests, and live Dioxus + `NVE1`
//! wire scene E2E tests for `nve-router`.

use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use dioxus_core::{Event, VirtualDom};
use nve_dioxus::elements::PressEventData;
use nve_dioxus::prelude::*;
use nve_dioxus::{DioxusSurface, NveMutationWriter};
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
    let mut bridge_a = RouterChannelBridge::attach(&mut bus, 10, false).unwrap();

    let tr = router_a.push(E2eRoute::Profile).unwrap();
    bridge_a.publish_transition(&mut bus, &tr).unwrap();
    assert!(bridge_a.drain_inbound().is_empty());

    let retained = bus.retained(TOPIC_NAVIGATION).expect("retained nav state");
    assert_eq!(retained.sender_surface, Some(10));
    assert_eq!(retained.encoding, PayloadEncoding::Binary);
    let retained_pkt = decode_nav_packet(retained.as_bytes()).unwrap();
    assert_eq!(retained_pkt.uri, "/settings/profile");
    assert!(retained_pkt.can_go_back);

    // Late joiner replays retained NVN1 state.
    let bridge_b = RouterChannelBridge::attach(&mut bus, 20, true).unwrap();
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
        Some(99),
        PayloadEncoding::Binary,
        ChannelPayload::from_vec(os_pkt.encode().unwrap()),
        true,
    )
    .unwrap();
    bus.publish_by_id(
        CHANNEL_ID_NAVIGATION,
        Some(99),
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
    let tab_bridge = RouterChannelBridge::attach(&mut bus, 30, false).unwrap();
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
        Some(99),
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
// 3. Live Dioxus + NVE1 CommandBuffer wire scene E2E tests
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct WireNode {
    kind: u16,
    text: Option<String>,
    raw_props: HashMap<String, String>,
    event_mask: u64,
    children: Vec<u32>,
}

struct WireScene {
    nodes: HashMap<u32, WireNode>,
}

const ROOT_NODE_ID: u32 = 1;

impl WireScene {
    fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(ROOT_NODE_ID, WireNode::default());
        Self { nodes }
    }

    fn apply(&mut self, bytes: &[u8]) {
        assert!(bytes.len() >= 24 && &bytes[0..4] == b"NVE1");
        let op_count = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let mut pos = 24usize;
        for _ in 0..op_count {
            let op = bytes[pos];
            pos += 1;
            match op {
                1 => {
                    let id = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
                    let kind = u16::from_le_bytes(bytes[pos + 4..pos + 6].try_into().unwrap());
                    let nlen = u16::from_le_bytes(bytes[pos + 6..pos + 8].try_into().unwrap()) as usize;
                    pos += 8 + nlen;
                    self.nodes.entry(id).or_default().kind = kind;
                }
                2 => {
                    let id = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
                    pos += 4;
                    self.nodes.remove(&id);
                }
                3 => {
                    let pid = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
                    let cnt = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
                    pos += 8;
                    let mut kids = Vec::with_capacity(cnt);
                    for _ in 0..cnt {
                        kids.push(u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()));
                        pos += 4;
                    }
                    self.nodes.entry(pid).or_default().children = kids;
                }
                4 => {
                    let id = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
                    let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
                    pos += 8;
                    let s = std::str::from_utf8(&bytes[pos..pos + len]).unwrap().to_owned();
                    pos += len;
                    self.nodes.entry(id).or_default().text = Some(s);
                }
                5 => pos += 81,
                6 => pos += 36,
                7 => {
                    let id = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
                    let klen = u16::from_le_bytes(bytes[pos + 4..pos + 6].try_into().unwrap()) as usize;
                    pos += 6;
                    let key = std::str::from_utf8(&bytes[pos..pos + klen]).unwrap().to_owned();
                    pos += klen;
                    let tag = bytes[pos];
                    pos += 1;
                    match tag {
                        0 => {}
                        1 => pos += 1,
                        2 | 3 => pos += 8,
                        4 => {
                            let vlen = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
                            pos += 4;
                            let val = std::str::from_utf8(&bytes[pos..pos + vlen]).unwrap().to_owned();
                            pos += vlen;
                            self.nodes.entry(id).or_default().raw_props.insert(key, val);
                        }
                        5 => {
                            let blen = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
                            pos += 4 + blen;
                        }
                        other => panic!("unexpected RawPropValue tag {other}"),
                    }
                }
                8 => {
                    let id = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
                    let mask = u64::from_le_bytes(bytes[pos + 4..pos + 12].try_into().unwrap());
                    pos += 12;
                    self.nodes.entry(id).or_default().event_mask = mask;
                }
                9 => {
                    let nlen = u16::from_le_bytes(bytes[pos + 4..pos + 6].try_into().unwrap()) as usize;
                    pos += 6 + nlen;
                    let plen = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
                    pos += 4 + plen;
                }
                10 => pos += 1,
                other => panic!("unexpected opcode {other}"),
            }
        }
        assert_eq!(pos, bytes.len());
    }

    fn collect_texts(&self) -> String {
        let mut out = Vec::new();
        self.walk_texts(ROOT_NODE_ID, &mut out);
        out.join(" | ")
    }

    fn walk_texts(&self, id: u32, out: &mut Vec<String>) {
        if let Some(node) = self.nodes.get(&id) {
            if let Some(t) = &node.text {
                out.push(t.clone());
            }
            for &kid in &node.children {
                self.walk_texts(kid, out);
            }
        }
    }

    fn find_by_test_id(&self, test_id: &str) -> Option<(u32, &WireNode)> {
        self.nodes
            .iter()
            .find(|(_, n)| n.raw_props.get("testID").map(String::as_str) == Some(test_id))
            .map(|(&id, n)| (id, n))
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

fn press_test_id(
    vdom: &mut VirtualDom,
    writer: &mut NveMutationWriter,
    scene: &mut WireScene,
    test_id: &str,
    frame_id: u64,
) {
    let (node_id, _) = scene
        .find_by_test_id(test_id)
        .unwrap_or_else(|| panic!("missing test_id {test_id}"));
    let el_id = writer.element_id_of(node_id).expect("element id");
    let data: Rc<dyn Any> = Rc::new(PressEventData { x: 10.0, y: 10.0 });
    vdom.runtime().handle_event("press", Event::new(data, true), el_id);
    writer.begin_frame(1, frame_id);
    vdom.render_immediate(writer);
    scene.apply(writer.finish_frame(true));
}

#[test]
fn dioxus_e2e_router_provider_link_outlet_press_navigation_and_modal() {
    let mut surface = DioxusSurface::new(1, RouterE2eApp);
    let (_, _, surface_frame) = surface.render_frame(1);
    assert_eq!(&surface_frame[0..4], b"NVE1");

    let mut vdom = VirtualDom::new(RouterE2eApp);
    let mut writer = NveMutationWriter::new(1);
    let mut scene = WireScene::new();

    writer.begin_frame(1, 1);
    vdom.rebuild(&mut writer);
    scene.apply(writer.finish_frame(true));
    assert!(scene.collect_texts().contains("Screen: Home (/)"));

    let (_, link_node) = scene.find_by_test_id("link-issue-42").expect("link node");
    assert_eq!(link_node.kind, 6);
    assert_eq!(
        link_node.raw_props.get("accessibilityRole").map(String::as_str),
        Some("link")
    );
    assert_ne!(link_node.event_mask, 0);

    press_test_id(&mut vdom, &mut writer, &mut scene, "link-issue-42", 2);
    let texts2 = scene.collect_texts();
    assert!(texts2.contains("Screen: Issue #42 (comments)"), "frame2: {texts2}");
    assert!(!texts2.contains("Screen: Home"));

    press_test_id(&mut vdom, &mut writer, &mut scene, "link-modal-archive", 3);
    let texts3 = scene.collect_texts();
    assert!(texts3.contains("Screen: Issue #42 (comments)"), "frame3: {texts3}");
    assert!(texts3.contains("Modal: Confirm archive"), "frame3: {texts3}");

    press_test_id(&mut vdom, &mut writer, &mut scene, "btn-dismiss-modal", 4);
    assert!(!scene.collect_texts().contains("Modal: Confirm"));

    press_test_id(&mut vdom, &mut writer, &mut scene, "btn-back", 5);
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
    let mut vdom = VirtualDom::new(TabRouterE2eApp);
    let mut writer = NveMutationWriter::new(1);
    let mut scene = WireScene::new();

    writer.begin_frame(1, 1);
    vdom.rebuild(&mut writer);
    scene.apply(writer.finish_frame(true));
    assert!(scene.collect_texts().contains("ActiveTab: Home"));
    assert!(scene.collect_texts().contains("Screen: Home (/)"));

    // Navigate deeper inside Home tab.
    press_test_id(&mut vdom, &mut writer, &mut scene, "link-issue-42", 2);
    assert!(scene.collect_texts().contains("Screen: Issue #42 (comments)"));

    // Switch to Docs tab.
    press_test_id(&mut vdom, &mut writer, &mut scene, "tab-docs", 3);
    assert!(scene.collect_texts().contains("ActiveTab: Docs"));
    assert!(scene.collect_texts().contains("Docs { slug: \"intro\" }"));

    // Switch back to Home tab -> Issue #42 preserved!
    press_test_id(&mut vdom, &mut writer, &mut scene, "tab-home", 4);
    assert!(scene.collect_texts().contains("ActiveTab: Home"));
    assert!(scene.collect_texts().contains("Screen: Issue #42 (comments)"));

    // Re-tap active Home tab -> pops back to Home root!
    press_test_id(&mut vdom, &mut writer, &mut scene, "tab-home", 5);
    assert!(scene.collect_texts().contains("Screen: Home (/)"));
}
