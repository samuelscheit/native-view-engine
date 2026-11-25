//! Deep unit & edge-case tests for `nve-dioxus`:
//! - Multi-level `Text` style inheritance (explicit vs unstyled, nested overrides, non-Text parents, dynamic updates)
//! - Incremental tree diffing (insert, remove, reorder, text updates, conditional mount/unmount)
//! - Event dispatch (`Press`, `TextChange`, unknown nodes, mismatched payloads)
//! - Tier-1 `TemplatePatch` persistence across frames and non-template rejection
//! - State snapshot/restore edge cases through `NveApp` + `DioxusSurface`

use std::cell::RefCell;
use std::collections::HashMap;

use nve_abi::{
    encode_events, ColorRgba, Command, CommandDecoder, ComponentKind, EventMask, EventPayload,
    FrameId, NativeEvent, NativeEventKind, NodeId, RawPropValue, StyleProps, SurfaceId,
    ROOT_NODE_ID,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};

#[derive(Debug, Clone)]
struct DecodedNode {
    kind: ComponentKind,
    text: Option<String>,
    style: StyleProps,
    event_mask: u64,
    children: Vec<NodeId>,
    raw_props: HashMap<String, RawPropValue>,
}

/// Minimal state tracker that applies decoded `nve-abi` `CommandBuffer` frames so tests can assert
/// both exact wire commands and resulting retained tree state.
struct WireTree {
    nodes: HashMap<NodeId, DecodedNode>,
}

impl WireTree {
    fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            ROOT_NODE_ID,
            DecodedNode {
                kind: ComponentKind::Root,
                text: None,
                style: StyleProps::default(),
                event_mask: 0,
                children: Vec::new(),
                raw_props: HashMap::new(),
            },
        );
        Self { nodes }
    }

    fn apply(&mut self, bytes: &[u8]) -> Vec<Command> {
        let (_, cmds) = CommandDecoder::new(bytes)
            .expect("valid header")
            .decode_all()
            .expect("valid commands");
        for cmd in &cmds {
            match cmd {
                Command::CreateNode { node_id, kind, .. } => {
                    self.nodes.insert(
                        *node_id,
                        DecodedNode {
                            kind: *kind,
                            text: None,
                            style: StyleProps::default(),
                            event_mask: 0,
                            children: Vec::new(),
                            raw_props: HashMap::new(),
                        },
                    );
                }
                Command::RemoveNode { node_id } => {
                    self.nodes.remove(node_id);
                }
                Command::SetChildren {
                    parent_id,
                    children,
                } => {
                    if let Some(p) = self.nodes.get_mut(parent_id) {
                        p.children = children.clone();
                    }
                }
                Command::SetText { node_id, text } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.text = Some(text.clone());
                    }
                }
                Command::SetStyle { node_id, style } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.style = *style;
                    }
                }
                Command::SetRawProp {
                    node_id,
                    key,
                    value,
                } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.raw_props.insert(key.clone(), value.clone());
                    }
                }
                Command::SetEventMask { node_id, mask } => {
                    if let Some(n) = self.nodes.get_mut(node_id) {
                        n.event_mask = *mask;
                    }
                }
                Command::SetLayout { .. }
                | Command::DispatchCommand { .. }
                | Command::Commit { .. } => {}
            }
        }
        cmds
    }

    fn find_by_text(&self, needle: &str) -> Option<(NodeId, &DecodedNode)> {
        self.nodes
            .iter()
            .find(|(_, n)| n.text.as_deref() == Some(needle))
            .map(|(&id, n)| (id, n))
    }

    fn find_by_test_id(&self, test_id: &str) -> Option<(NodeId, &DecodedNode)> {
        self.nodes
            .iter()
            .find(|(_, n)| {
                matches!(
                    n.raw_props.get("testID"),
                    Some(RawPropValue::String(s)) if s == test_id
                )
            })
            .map(|(&id, n)| (id, n))
    }

    fn collect_texts_in_order(&self, node_id: NodeId, out: &mut Vec<String>) {
        if let Some(node) = self.nodes.get(&node_id) {
            if let Some(t) = &node.text {
                out.push(t.clone());
            }
            for &cid in &node.children {
                self.collect_texts_in_order(cid, out);
            }
        }
    }
}

#[component]
fn TextInheritanceApp() -> Element {
    let mut toggled = use_signal(|| false);
    let outer_style = if toggled() {
        "color: #eab308; font-size: 30px;"
    } else {
        "color: #38bdf8; font-size: 24px;"
    };

    rsx! {
        view {
            // Non-Text parent with explicit color/font-size must NOT propagate into child Text.
            style: "color: #ef4444; font-size: 36px;",
            text {
                "UnderViewDirect"
            }
            // 2-level + 3-level unstyled inheritance + explicit override branch.
            text {
                style: "{outer_style}",
                "Level2Leaf"
                text {
                    "Level3Inherited"
                }
                text {
                    style: "color: #22c55e; font-size: 13px;",
                    "ExplicitChildLeaf"
                    text {
                        "NestedUnderOverride"
                    }
                }
            }
            pressable {
                test_id: "toggle-style",
                onpress: move |_| toggled.toggle(),
                text { "Toggle" }
            }
        }
    }
}

#[test]
fn text_style_inheritance_multilevel_overrides_non_text_parent_and_dynamic_updates() {
    let mut surface = DioxusSurface::new(1, TextInheritanceApp);
    let mut tree = WireTree::new();

    let (_, _, frame1) = surface.render_frame(1);
    tree.apply(&frame1);

    let cyan = ColorRgba::rgb(0x38, 0xbd, 0xf8);
    let green = ColorRgba::rgb(0x22, 0xc5, 0x5e);
    let yellow = ColorRgba::rgb(0xea, 0xb3, 0x08);

    // Non-Text parent (`view`) does NOT propagate text_color / font_size to child `text`.
    let (_, under_view) = tree
        .find_by_text("UnderViewDirect")
        .expect("UnderViewDirect leaf");
    assert_eq!(under_view.style.text_color, StyleProps::default().text_color);
    assert_eq!(under_view.style.font_size, StyleProps::default().font_size);

    // 2-level inheritance
    let (_, lvl2) = tree.find_by_text("Level2Leaf").expect("Level2Leaf");
    assert_eq!(lvl2.style.text_color, cyan);
    assert_eq!(lvl2.style.font_size, 24.0);

    // 3-level inheritance through unstyled intermediate Text
    let (_, lvl3) = tree
        .find_by_text("Level3Inherited")
        .expect("Level3Inherited");
    assert_eq!(lvl3.style.text_color, cyan);
    assert_eq!(lvl3.style.font_size, 24.0);

    // Explicit nested override keeps its own color/font_size AND propagates to its own unstyled child.
    let (_, explicit_leaf) = tree
        .find_by_text("ExplicitChildLeaf")
        .expect("ExplicitChildLeaf");
    assert_eq!(explicit_leaf.style.text_color, green);
    assert_eq!(explicit_leaf.style.font_size, 13.0);

    let (_, under_override) = tree
        .find_by_text("NestedUnderOverride")
        .expect("NestedUnderOverride");
    assert_eq!(under_override.style.text_color, green);
    assert_eq!(under_override.style.font_size, 13.0);

    // Trigger dynamic parent Text style update -> propagates to unstyled descendants, preserves override.
    let (toggle_id, _) = tree.find_by_test_id("toggle-style").expect("toggle button");
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: toggle_id,
        timestamp_us: 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]));

    let (_, _, frame2) = surface.render_frame(2);
    tree.apply(&frame2);

    assert_eq!(tree.find_by_text("Level2Leaf").unwrap().1.style.text_color, yellow);
    assert_eq!(tree.find_by_text("Level2Leaf").unwrap().1.style.font_size, 30.0);
    assert_eq!(tree.find_by_text("Level3Inherited").unwrap().1.style.text_color, yellow);
    assert_eq!(tree.find_by_text("Level3Inherited").unwrap().1.style.font_size, 30.0);

    // Explicit override branch remains untouched by grandparent style change.
    assert_eq!(tree.find_by_text("ExplicitChildLeaf").unwrap().1.style.text_color, green);
    assert_eq!(tree.find_by_text("ExplicitChildLeaf").unwrap().1.style.font_size, 13.0);
    assert_eq!(tree.find_by_text("NestedUnderOverride").unwrap().1.style.text_color, green);
    assert_eq!(tree.find_by_text("NestedUnderOverride").unwrap().1.style.font_size, 13.0);
}

#[component]
fn DiffingListApp() -> Element {
    let mut items = use_signal(|| {
        vec![
            (1u32, "Alpha".to_string()),
            (2u32, "Beta".to_string()),
            (3u32, "Gamma".to_string()),
        ]
    });
    let mut show_banner = use_signal(|| true);

    rsx! {
        view {
            view {
                test_id: "list-container",
                for (id, label) in items.read().iter() {
                    text { key: "{id}", "{label}" }
                }
            }
            if show_banner() {
                view {
                    test_id: "banner",
                    text { "LiveBanner" }
                }
            }
            pressable {
                test_id: "btn-edit-text",
                onpress: move |_| {
                    items.write()[1].1 = "Beta-v2".into();
                },
            }
            pressable {
                test_id: "btn-insert",
                onpress: move |_| {
                    items.write().insert(0, (0, "Zero".into()));
                    items.write().push((4, "Delta".into()));
                },
            }
            pressable {
                test_id: "btn-reorder",
                onpress: move |_| {
                    items.write().reverse();
                },
            }
            pressable {
                test_id: "btn-remove",
                onpress: move |_| {
                    items.write().retain(|(id, _)| *id != 2);
                },
            }
            pressable {
                test_id: "btn-toggle-banner",
                onpress: move |_| show_banner.toggle(),
            }
        }
    }
}

fn press_test_id(surface: &mut DioxusSurface, tree: &mut WireTree, test_id: &str, frame_id: u64) -> Vec<Command> {
    let (node_id, _) = tree
        .find_by_test_id(test_id)
        .unwrap_or_else(|| panic!("missing testID {test_id}"));
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id,
        timestamp_us: frame_id * 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 5.0, y: 5.0 },
    }]));
    let (_, _, bytes) = surface.render_frame(frame_id);
    tree.apply(&bytes)
}

#[test]
fn incremental_tree_diffing_insert_remove_reorder_text_update_and_conditional() {
    let mut surface = DioxusSurface::new(1, DiffingListApp);
    let mut tree = WireTree::new();

    let (_, _, f1) = surface.render_frame(1);
    tree.apply(&f1);

    let list_id = tree.find_by_test_id("list-container").unwrap().0;
    let mut texts = Vec::new();
    tree.collect_texts_in_order(list_id, &mut texts);
    assert_eq!(texts, vec!["Alpha", "Beta", "Gamma"]);
    assert!(tree.find_by_text("LiveBanner").is_some());

    // 1. In-place text update emits targeted SetText with zero CreateNode/RemoveNode.
    let cmds_text = press_test_id(&mut surface, &mut tree, "btn-edit-text", 2);
    assert!(
        cmds_text
            .iter()
            .any(|c| matches!(c, Command::SetText { text, .. } if text == "Beta-v2")),
        "expected SetText(Beta-v2), got {cmds_text:?}"
    );
    assert!(
        !cmds_text
            .iter()
            .any(|c| matches!(c, Command::CreateNode { .. } | Command::RemoveNode { .. })),
        "text update should not create/remove nodes: {cmds_text:?}"
    );
    texts.clear();
    tree.collect_texts_in_order(list_id, &mut texts);
    assert_eq!(texts, vec!["Alpha", "Beta-v2", "Gamma"]);

    // 2. Prepend + append insertion creates new nodes and updates list order.
    let cmds_insert = press_test_id(&mut surface, &mut tree, "btn-insert", 3);
    assert!(cmds_insert
        .iter()
        .any(|c| matches!(c, Command::CreateNode { .. })));
    assert!(cmds_insert
        .iter()
        .any(|c| matches!(c, Command::SetChildren { parent_id, .. } if *parent_id == list_id)));
    texts.clear();
    tree.collect_texts_in_order(list_id, &mut texts);
    assert_eq!(texts, vec!["Zero", "Alpha", "Beta-v2", "Gamma", "Delta"]);

    // 3. Reorder (reverse) updates child ordering cleanly.
    let cmds_reorder = press_test_id(&mut surface, &mut tree, "btn-reorder", 4);
    assert!(cmds_reorder
        .iter()
        .any(|c| matches!(c, Command::SetChildren { parent_id, .. } if *parent_id == list_id)));
    texts.clear();
    tree.collect_texts_in_order(list_id, &mut texts);
    assert_eq!(texts, vec!["Delta", "Gamma", "Beta-v2", "Alpha", "Zero"]);

    // 4. Remove emits RemoveNode and detaches the removed item.
    let cmds_remove = press_test_id(&mut surface, &mut tree, "btn-remove", 5);
    assert!(cmds_remove
        .iter()
        .any(|c| matches!(c, Command::RemoveNode { .. })));
    assert!(tree.find_by_text("Beta-v2").is_none());
    texts.clear();
    tree.collect_texts_in_order(list_id, &mut texts);
    assert_eq!(texts, vec!["Delta", "Gamma", "Alpha", "Zero"]);

    // 5. Conditional unmount then remount
    let cmds_hide = press_test_id(&mut surface, &mut tree, "btn-toggle-banner", 6);
    assert!(cmds_hide
        .iter()
        .any(|c| matches!(c, Command::RemoveNode { .. })));
    assert!(tree.find_by_text("LiveBanner").is_none());

    let cmds_show = press_test_id(&mut surface, &mut tree, "btn-toggle-banner", 7);
    assert!(cmds_show
        .iter()
        .any(|c| matches!(c, Command::CreateNode { .. })));
    assert!(tree.find_by_text("LiveBanner").is_some());
}

#[component]
fn EventDispatchApp() -> Element {
    let mut last_press = use_signal(|| "none".to_string());
    let mut last_text = use_signal(|| "empty".to_string());

    rsx! {
        view {
            pressable {
                test_id: "press-target",
                onpress: move |ev| {
                    last_press.set(format!("{:.1},{:.2}", ev.data().x, ev.data().y));
                },
            }
            textinput {
                test_id: "input-target",
                onchange: move |ev| {
                    last_text.set(ev.data().value.clone());
                },
            }
            text { "Press:{last_press}" }
            text { "Input:{last_text}" }
        }
    }
}

#[test]
fn event_dispatch_handles_press_text_change_unknown_node_and_mismatched_payloads() {
    let mut surface = DioxusSurface::new(1, EventDispatchApp);
    let mut tree = WireTree::new();

    let (_, _, f1) = surface.render_frame(1);
    tree.apply(&f1);

    let (press_id, press_node) = tree.find_by_test_id("press-target").unwrap();
    let (input_id, input_node) = tree.find_by_test_id("input-target").unwrap();
    assert_eq!(press_node.kind, ComponentKind::Pressable);
    assert_eq!(input_node.kind, ComponentKind::TextInput);
    assert_ne!(press_node.event_mask & EventMask::PRESS, 0);
    assert_ne!(input_node.event_mask & EventMask::TEXT_CHANGE, 0);

    // Unknown node returns false without panicking.
    assert!(!surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: 999_999,
        timestamp_us: 10,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 0.0, y: 0.0 },
    }]));

    // Mismatched payloads and unhandled event kinds return false.
    assert!(!surface.dispatch_events(&[
        NativeEvent {
            surface_id: 1,
            node_id: press_id,
            timestamp_us: 20,
            kind: NativeEventKind::Press,
            payload: EventPayload::Empty,
        },
        NativeEvent {
            surface_id: 1,
            node_id: input_id,
            timestamp_us: 30,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Point { x: 1.0, y: 2.0 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: press_id,
            timestamp_us: 40,
            kind: NativeEventKind::Focus,
            payload: EventPayload::Empty,
        },
    ]));

    // Valid Press + TextChange in a single batch both dispatch cleanly.
    let handled = surface.dispatch_events(&[
        NativeEvent {
            surface_id: 1,
            node_id: press_id,
            timestamp_us: 50,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 42.5, y: 88.25 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: input_id,
            timestamp_us: 60,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("hello dioxus 🚀".into()),
        },
    ]);
    assert!(handled);

    let (_, _, f2) = surface.render_frame(2);
    tree.apply(&f2);

    assert!(tree.find_by_text("Press:42.5,88.25").is_some());
    assert!(tree.find_by_text("Input:hello dioxus 🚀").is_some());
}

#[component]
fn HotPatchPersistenceApp() -> Element {
    let mut step = use_signal(|| 0u32);
    let bg = if step() % 2 == 0 { "#0f172a" } else { "#1e293b" };

    rsx! {
        view {
            test_id: "card",
            style: "background-color: {bg};",
            text { "Original Title" }
            pressable {
                test_id: "step-btn",
                onpress: move |_| step += 1,
                text { "Step: {step}" }
            }
        }
    }
}

#[test]
fn tier1_template_patch_persists_across_frames_and_ignores_non_template_messages() {
    let mut surface = DioxusSurface::new(1, HotPatchPersistenceApp);
    let mut tree = WireTree::new();

    // Non-TemplatePatch messages must be rejected cleanly.
    assert!(!surface.apply_hot_patch(&HotPatchMsg::DylibReload {
        generation: 2,
        dylib_path: "libapp.dylib".into(),
        build_ms: 120,
    }));
    assert!(!surface.apply_hot_patch(&HotPatchMsg::DevStatus {
        message: "ok".into(),
        latency_ms: 4,
    }));

    // Queue a patch BEFORE initial render_frame(1): must apply during frame 1.
    assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "card".into(),
        attr_updates: vec![("border-radius".into(), "14px".into())],
        text_updates: vec![],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    }));

    let (_, _, f1) = surface.render_frame(1);
    tree.apply(&f1);
    let (card_id, card1) = tree.find_by_test_id("card").unwrap();
    assert_eq!(card1.style.border_radius, 14.0);

    // Frame 2: apply opacity + leaf text update + out-of-bounds text index (must not panic).
    assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
        generation: 2,
        template_id: "card".into(),
        attr_updates: vec![("opacity".into(), "0.5".into())],
        text_updates: vec![
            (0, "Patched Title".into()),
            (999, "OutOfBoundsIgnored".into()),
        ],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    }));
    let (_, _, f2) = surface.render_frame(2);
    tree.apply(&f2);

    assert_eq!(tree.nodes[&card_id].style.opacity, 0.5);
    assert_eq!(tree.nodes[&card_id].style.border_radius, 14.0);
    assert!(tree.find_by_text("Patched Title").is_some());

    // Frame 3: trigger a Dioxus state update that changes `background-color` on `card`.
    // Because NveMutationWriter retains patched style state, `opacity` (0.5) & `border_radius` (14.0)
    // MUST persist inside the newly emitted SetStyle command on frame 3!
    let cmds3 = press_test_id(&mut surface, &mut tree, "step-btn", 3);
    assert!(
        cmds3.iter().any(|c| matches!(
            c,
            Command::SetStyle { node_id, style }
                if *node_id == card_id
                    && style.background == ColorRgba::rgb(0x1e, 0x29, 0x3b)
                    && (style.opacity - 0.5).abs() < f32::EPSILON
                    && (style.border_radius - 14.0).abs() < f32::EPSILON
        )),
        "expected persisted opacity=0.5 & border_radius=14.0 on frame 3 SetStyle, got {cmds3:?}"
    );
    assert!(tree.find_by_text("Step: 1").is_some());
}

// Thread-local state bridge for parallel-safe NveApp + DioxusSurface snapshot/restore tests.
thread_local! {
    static SNAPSHOT_SEED: RefCell<(i32, String)> = const { RefCell::new((0, String::new())) };
    static SNAPSHOT_LIVE: RefCell<(i32, String)> = const { RefCell::new((0, String::new())) };
}

#[component]
fn SnapshotCounterApp() -> Element {
    let (init_count, init_note) = SNAPSHOT_SEED.with(|s| s.borrow().clone());
    let mut count = use_signal(|| init_count);
    let mut note = use_signal(|| init_note);
    SNAPSHOT_LIVE.with(|s| {
        *s.borrow_mut() = (count(), note());
    });

    rsx! {
        view {
            text { "SnapCount:{count}" }
            text { "SnapNote:{note}" }
            pressable {
                test_id: "snap-inc",
                onpress: move |_| count += 1,
            }
            textinput {
                test_id: "snap-note",
                onchange: move |ev| note.set(ev.data().value.clone()),
            }
        }
    }
}

struct SnapshotTestApp {
    surface_id: SurfaceId,
    surface: DioxusSurface,
    frame_buf: Vec<u8>,
    state_buf: Vec<u8>,
}

impl SnapshotTestApp {
    fn encode_state(count: i32, note: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"SNAP");
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&(note.len() as u32).to_le_bytes());
        out.extend_from_slice(note.as_bytes());
        out
    }

    fn decode_state(bytes: &[u8]) -> Option<(i32, String)> {
        if bytes.len() < 12 || &bytes[0..4] != b"SNAP" {
            return None;
        }
        let count = i32::from_le_bytes(bytes[4..8].try_into().ok()?);
        let len = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        if bytes.len() != 12 + len {
            return None;
        }
        let note = std::str::from_utf8(&bytes[12..]).ok()?.to_owned();
        Some((count, note))
    }
}

impl NveApp for SnapshotTestApp {
    fn create(surface_id: SurfaceId) -> Self {
        Self {
            surface_id,
            surface: DioxusSurface::new(surface_id, SnapshotCounterApp),
            frame_buf: Vec::new(),
            state_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let (_, _, bytes) = self.surface.render_frame(frame_id);
        self.frame_buf = bytes;
        &self.frame_buf
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        let Ok(events) = nve_abi::decode_events(encoded_events) else {
            return false;
        };
        self.surface.dispatch_events(&events)
    }

    fn snapshot_state(&mut self) -> &[u8] {
        let (count, note) = SNAPSHOT_LIVE.with(|s| s.borrow().clone());
        self.state_buf = Self::encode_state(count, &note);
        &self.state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        if let Some((count, note)) = Self::decode_state(state) {
            SNAPSHOT_SEED.with(|s| *s.borrow_mut() = (count, note.clone()));
            SNAPSHOT_LIVE.with(|s| *s.borrow_mut() = (count, note));
            self.surface = DioxusSurface::new(self.surface_id, SnapshotCounterApp);
        }
    }
}

#[test]
fn state_snapshot_and_restore_edge_cases() {
    SNAPSHOT_SEED.with(|s| *s.borrow_mut() = (0, "init".into()));
    SNAPSHOT_LIVE.with(|s| *s.borrow_mut() = (0, "init".into()));

    let mut app1 = SnapshotTestApp::create(1);
    let mut tree1 = WireTree::new();
    tree1.apply(app1.render(1));
    assert!(tree1.find_by_text("SnapCount:0").is_some());
    assert!(tree1.find_by_text("SnapNote:init").is_some());

    // Mutate state via encoded EventRing (2 presses + 1 text change).
    let inc_id = tree1.find_by_test_id("snap-inc").unwrap().0;
    let note_id = tree1.find_by_test_id("snap-note").unwrap().0;
    let evs = encode_events(&[
        NativeEvent {
            surface_id: 1,
            node_id: inc_id,
            timestamp_us: 100,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 1.0, y: 1.0 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: inc_id,
            timestamp_us: 200,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 1.0, y: 1.0 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: note_id,
            timestamp_us: 300,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("persisted".into()),
        },
    ]);
    assert!(app1.dispatch_events(&evs));
    tree1.apply(app1.render(2));
    assert!(tree1.find_by_text("SnapCount:2").is_some());
    assert!(tree1.find_by_text("SnapNote:persisted").is_some());

    let snapshot = app1.snapshot_state().to_vec();

    // Reset seed to verify restore actually comes from snapshot bytes.
    SNAPSHOT_SEED.with(|s| *s.borrow_mut() = (0, "reset".into()));
    SNAPSHOT_LIVE.with(|s| *s.borrow_mut() = (0, "reset".into()));

    // Edge cases: empty, truncated, bad-magic, invalid-UTF8 snapshots must not panic or corrupt defaults.
    let mut app_bad = SnapshotTestApp::create(1);
    app_bad.restore_state(&[]);
    app_bad.restore_state(b"SNAP\x01\x02");
    app_bad.restore_state(b"NOPE\x05\x00\x00\x00\x00\x00\x00\x00");
    app_bad.restore_state(b"SNAP\x05\x00\x00\x00\x02\x00\x00\x00\xFF\xFE");
    let mut tree_bad = WireTree::new();
    tree_bad.apply(app_bad.render(1));
    assert!(tree_bad.find_by_text("SnapCount:0").is_some());
    assert!(tree_bad.find_by_text("SnapNote:reset").is_some());

    // Valid restore BEFORE first render.
    let mut app2 = SnapshotTestApp::create(1);
    app2.restore_state(&snapshot);
    let mut tree2 = WireTree::new();
    tree2.apply(app2.render(1));
    assert!(tree2.find_by_text("SnapCount:2").is_some());
    assert!(tree2.find_by_text("SnapNote:persisted").is_some());

    // Subsequent event after restore increments from restored baseline (2 -> 3).
    let inc_id2 = tree2.find_by_test_id("snap-inc").unwrap().0;
    let ev3 = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: inc_id2,
        timestamp_us: 400,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]);
    assert!(app2.dispatch_events(&ev3));
    tree2.apply(app2.render(2));
    assert!(tree2.find_by_text("SnapCount:3").is_some());

    // Valid restore AFTER initial render replaces state cleanly.
    app2.restore_state(&snapshot);
    let mut tree3 = WireTree::new();
    tree3.apply(app2.render(3));
    assert!(tree3.find_by_text("SnapCount:2").is_some());
    assert!(tree3.find_by_text("SnapNote:persisted").is_some());
}
