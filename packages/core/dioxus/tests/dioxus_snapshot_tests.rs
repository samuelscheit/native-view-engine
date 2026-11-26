//! State snapshot & restore edge-case tests through `NveApp` + `DioxusSurface`.

use std::cell::RefCell;
use std::collections::HashMap;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, FrameId, NodeId, NveDirectEvent, NveEventBatchRef,
    RawPropValue, SurfaceId, ROOT_NODE_ID,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::NveApp;

#[derive(Default)]
struct SimpleTree {
    text: HashMap<NodeId, String>,
    test_ids: HashMap<String, NodeId>,
}

impl SimpleTree {
    fn apply(&mut self, bytes: &[u8]) {
        let (_, cmds) = CommandDecoder::new(bytes).unwrap().decode_all().unwrap();
        for cmd in cmds {
            match cmd {
                Command::CreateNode { node_id, kind: ComponentKind::Text, .. } => {
                    self.text.entry(node_id).or_default();
                }
                Command::SetText { node_id, text } => {
                    self.text.insert(node_id, text);
                }
                Command::SetRawProp {
                    node_id,
                    key,
                    value: RawPropValue::String(v),
                } if key == "testID" => {
                    self.test_ids.insert(v, node_id);
                }
                Command::RemoveNode { node_id } => {
                    self.text.remove(&node_id);
                    self.test_ids.retain(|_, v| *v != node_id);
                }
                _ => {}
            }
        }
    }

    fn has_text(&self, needle: &str) -> bool {
        self.text.values().any(|t| t == needle)
    }

    fn find_by_test_id(&self, tid: &str) -> NodeId {
        *self
            .test_ids
            .get(tid)
            .unwrap_or_else(|| panic!("missing testID={tid} (root={ROOT_NODE_ID})"))
    }
}

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

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        self.surface.dispatch_direct_events(events)
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
    let mut tree1 = SimpleTree::default();
    tree1.apply(app1.render(1));
    assert!(tree1.has_text("SnapCount:0"));
    assert!(tree1.has_text("SnapNote:init"));

    let inc_id = tree1.find_by_test_id("snap-inc");
    let note_id = tree1.find_by_test_id("snap-note");
    let evs = [
        NveDirectEvent::press(1, inc_id, 100, 1.0, 1.0),
        NveDirectEvent::press(1, inc_id, 200, 1.0, 1.0),
        NveDirectEvent::text_change(1, note_id, 300, "persisted"),
    ];
    assert!(app1.dispatch_events(unsafe { NveEventBatchRef::from_slice(&evs) }));
    tree1.apply(app1.render(2));
    assert!(tree1.has_text("SnapCount:2"));
    assert!(tree1.has_text("SnapNote:persisted"));

    let snapshot = app1.snapshot_state().to_vec();

    SNAPSHOT_SEED.with(|s| *s.borrow_mut() = (0, "reset".into()));
    SNAPSHOT_LIVE.with(|s| *s.borrow_mut() = (0, "reset".into()));

    let mut app_bad = SnapshotTestApp::create(1);
    app_bad.restore_state(&[]);
    app_bad.restore_state(b"SNAP\x01\x02");
    app_bad.restore_state(b"NOPE\x05\x00\x00\x00\x00\x00\x00\x00");
    app_bad.restore_state(b"SNAP\x05\x00\x00\x00\x02\x00\x00\x00\xFF\xFE");
    let mut tree_bad = SimpleTree::default();
    tree_bad.apply(app_bad.render(1));
    assert!(tree_bad.has_text("SnapCount:0"));
    assert!(tree_bad.has_text("SnapNote:reset"));

    let mut app2 = SnapshotTestApp::create(1);
    app2.restore_state(&snapshot);
    let mut tree2 = SimpleTree::default();
    tree2.apply(app2.render(1));
    assert!(tree2.has_text("SnapCount:2"));
    assert!(tree2.has_text("SnapNote:persisted"));

    let inc_id2 = tree2.find_by_test_id("snap-inc");
    let ev3 = [NveDirectEvent::press(1, inc_id2, 400, 1.0, 1.0)];
    assert!(app2.dispatch_events(unsafe { NveEventBatchRef::from_slice(&ev3) }));
    tree2.apply(app2.render(2));
    assert!(tree2.has_text("SnapCount:3"));

    app2.restore_state(&snapshot);
    let mut tree3 = SimpleTree::default();
    tree3.apply(app2.render(3));
    assert!(tree3.has_text("SnapCount:2"));
    assert!(tree3.has_text("SnapNote:persisted"));
}
