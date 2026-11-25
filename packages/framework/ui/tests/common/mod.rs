//! Shared headless integration test harness driving real `DioxusSurface` + `HeadlessBackend`.

use nve_abi::events::encode_keydown_bytes;
use nve_abi::{
    Command, CommandDecoder, EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue,
};
use nve_backend_headless::HeadlessBackend;
use nve_dioxus::elements::backend::{RenderBackend, Viewport};
use nve_dioxus::prelude::Element;
use nve_dioxus::DioxusSurface;

pub struct UiHarness {
    pub surface: DioxusSurface,
    pub backend: HeadlessBackend,
    frame_id: u64,
    timestamp_us: u64,
}

#[allow(dead_code)]
impl UiHarness {
    pub fn new(root: fn() -> Element) -> Self {
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(
            1,
            Viewport {
                width: 800.0,
                height: 600.0,
                scale_factor: 1.0,
            },
        );
        let mut h = Self {
            surface: DioxusSurface::new(1, root),
            backend,
            frame_id: 0,
            timestamp_us: 0,
        };
        let _ = h.render();
        h
    }

    pub fn render(&mut self) -> Vec<Command> {
        self.frame_id += 1;
        let (_, _, bytes) = self.surface.render_frame(self.frame_id);
        self.backend
            .submit_frame(&bytes)
            .expect("headless submit_frame succeeded");
        let (_, cmds) = CommandDecoder::new(&bytes)
            .expect("valid command buffer header")
            .decode_all()
            .expect("valid command buffer");
        cmds
    }

    pub fn find(&self, tid: &str) -> NodeId {
        self.maybe_find(tid)
            .unwrap_or_else(|| panic!("expected node with testID='{tid}'"))
    }

    pub fn maybe_find(&self, tid: &str) -> Option<NodeId> {
        self.backend.nodes().values().find_map(|n| {
            match n.raw_props.get("testID") {
                Some(RawPropValue::String(s)) if s == tid => Some(n.id),
                _ => None,
            }
        })
    }

    pub fn raw_str(&self, id: NodeId, key: &str) -> Option<&str> {
        match self.backend.node(id)?.raw_props.get(key) {
            Some(RawPropValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn raw_bool(&self, id: NodeId, key: &str) -> Option<bool> {
        match self.backend.node(id)?.raw_props.get(key) {
            Some(RawPropValue::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    pub fn text_of(&self, tid: &str) -> String {
        let id = self.find(tid);
        self.collect_text(id)
    }

    fn collect_text(&self, id: NodeId) -> String {
        let Some(node) = self.backend.node(id) else {
            return String::new();
        };
        let mut out = String::new();
        if let Some(t) = node.text.as_deref() {
            out.push_str(t);
        }
        for &cid in &node.children {
            out.push_str(&self.collect_text(cid));
        }
        out
    }

    fn next_ts(&mut self) -> u64 {
        self.timestamp_us += 16_000;
        self.timestamp_us
    }

    pub fn press(&mut self, tid: &str) -> Vec<Command> {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 4.0, y: 4.0 },
        }]));
        self.render()
    }

    pub fn keydown(&mut self, tid: &str, key: &str, shift: bool) -> Vec<Command> {
        let id = self.find(tid);
        let ts = self.next_ts();
        let data = encode_keydown_bytes(key, false, false, shift, false);
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "keydown".into(),
                data,
            },
        }]));
        self.render()
    }

    pub fn focus(&mut self, tid: &str) -> Vec<Command> {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Focus,
            payload: EventPayload::Empty,
        }]));
        self.render()
    }

    pub fn blur(&mut self, tid: &str) -> Vec<Command> {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Blur,
            payload: EventPayload::Empty,
        }]));
        self.render()
    }

    pub fn text_change(&mut self, tid: &str, value: &str) -> Vec<Command> {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(value.to_owned()),
        }]));
        self.render()
    }

    #[cfg(feature = "virtual-list")]
    pub fn scroll(&mut self, tid: &str, y: f32) -> Vec<Command> {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Scroll,
            payload: EventPayload::Scroll {
                x: 0.0,
                y,
                content_width: 800.0,
                content_height: 5_000_000.0,
            },
        }]));
        self.render()
    }
}
