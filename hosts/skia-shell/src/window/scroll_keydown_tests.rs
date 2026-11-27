use std::path::Path;

use nve_abi::{ColorRgba, EventMask, FrameId, NativeEventKind, NodeId, SurfaceId};
use nve_components::tables::{
    register_core_tables, FlexDirection, Interaction, Layout, Overflow, Scroll, Visual,
};
use nve_engine::Viewport;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore, Rgba};

use super::input::format_keydown_shorthand;
use super::{DesktopApp, DesktopHostError, DesktopSession};

struct MockScrollAndKeyApp {
    scroll_events: Vec<(NodeId, f32, f32, f32, f32)>,
    keydown_payloads: Vec<(NodeId, String)>,
    palette_open: bool,
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    initialized: bool,
    frame_buf: Vec<u8>,
}

impl MockScrollAndKeyApp {
    fn new() -> Self {
        register_core_tables();
        Self {
            scroll_events: Vec::new(),
            keydown_payloads: Vec::new(),
            palette_open: false,
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            initialized: false,
            frame_buf: Vec::new(),
        }
    }
}

impl DesktopApp for MockScrollAndKeyApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        let mut tx = self.store.tx();
        if !self.initialized {
            let _ = tx.create_at(NodeId::ROOT);
            let _ = tx.create_at(NodeId::new(2, 0));
            tx.attach::<Scroll>(NodeId::new(2, 0));
            tx.set(NodeId::new(2, 0), Layout::WIDTH, Length::points(200.0));
            tx.set(NodeId::new(2, 0), Layout::HEIGHT, Length::points(100.0));
            tx.set(NodeId::new(2, 0), Layout::FLEX_DIRECTION, FlexDirection::Column);
            tx.set(NodeId::new(2, 0), Visual::OVERFLOW_Y, Overflow::Scroll);
            tx.set(
                NodeId::new(2, 0),
                Interaction::EVENT_MASK,
                (EventMask::SCROLL | EventMask::KEY_DOWN | EventMask::CUSTOM) as u32,
            );
            for id in [10u32, 11, 12] {
                let nid = NodeId::new(id, 0);
                let _ = tx.create_at(nid);
                tx.set(nid, Layout::WIDTH, Length::points(200.0));
                tx.set(nid, Layout::HEIGHT, Length::points(80.0));
                tx.set(nid, Layout::FLEX_SHRINK, 0.0);
            }
            tx.set_children(
                NodeId::new(2, 0),
                &[NodeId::new(10, 0), NodeId::new(11, 0), NodeId::new(12, 0)],
            );
            tx.set_children(NodeId::ROOT, &[NodeId::new(2, 0)]);
            self.initialized = true;
        }
        for (id, color) in [
            (10u32, Rgba::rgb(239, 68, 68)),
            (11u32, Rgba::rgb(16, 185, 129)),
            (12u32, Rgba::rgb(37, 99, 235)),
        ] {
            let bg = if self.palette_open {
                Rgba::rgb(250, 204, 21)
            } else {
                color
            };
            tx.set(NodeId::new(id, 0), Visual::BACKGROUND, bg);
        }
        tx.commit_into(&mut self.changes);
        self.frame_buf.clear();
        self.encoder
            .encode(&self.store, &self.changes, &mut self.frame_buf);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            match (ev.kind(), ev.payload_tag()) {
                (Some(NativeEventKind::Scroll), 2) => {
                    self.scroll_events.push((
                        ev.node_id(),
                        ev.x(),
                        ev.y(),
                        ev.width(),
                        ev.height(),
                    ));
                    handled = true;
                }
                (Some(NativeEventKind::KeyDown), 6) => {
                    let shorthand = format_keydown_shorthand(
                        ev.key(),
                        ev.meta(),
                        ev.ctrl(),
                        ev.shift(),
                        ev.alt(),
                    );
                    if ev.meta() && ev.key() == "k" {
                        self.palette_open = !self.palette_open;
                    } else if ev.key() == "Escape" {
                        self.palette_open = false;
                    }
                    self.keydown_payloads.push((ev.node_id(), shorthand));
                    handled = true;
                }
                _ => {}
            }
        }
        handled
    }

    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
        false
    }

    fn reload(
        &mut self,
        _new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        Ok(1)
    }
}

#[test]
fn desktop_session_forwards_wheel_scroll_and_typed_keydown_events() {
    let viewport = Viewport {
        width: 240.0,
        height: 160.0,
        scale_factor: 1.0,
    };
    let app = MockScrollAndKeyApp::new();
    let mut session = DesktopSession::new(app, SurfaceId::ROOT, viewport, ColorRgba::BLACK, false);
    session.render_and_rasterize().unwrap();

    assert_eq!(session.pixel_buffer().get_rgb(50, 20), Some((239, 68, 68)));
    assert_eq!(session.pixel_buffer().get_rgb(50, 120), Some((0, 0, 0)));
    assert!(!session.handle_scroll_logical(220.0, 140.0, 0.0, 60.0).unwrap());

    assert!(session.handle_scroll_logical(50.0, 50.0, 0.0, 100.0).unwrap());
    assert_eq!(session.app().scroll_events[0], (NodeId::new(2, 0), 0.0, 100.0, 200.0, 240.0));
    assert_eq!(session.pixel_buffer().get_rgb(50, 20), Some((16, 185, 129)));
    assert_eq!(session.pixel_buffer().get_rgb(50, 80), Some((37, 99, 235)));
    assert_eq!(session.pixel_buffer().get_rgb(50, 120), Some((0, 0, 0)));
    assert!(session.handle_scroll_logical(50.0, 50.0, 0.0, 500.0).unwrap());
    assert_eq!(session.app().scroll_events[1].2, 140.0);

    assert!(session.handle_keydown("k", true, false, false, false).unwrap());
    assert!(session.app().palette_open);
    assert_eq!(session.pixel_buffer().get_rgb(50, 20), Some((250, 204, 21)));
    assert!(session.handle_keydown("ArrowDown", false, false, false, false).unwrap());
    assert!(session.handle_keydown("ArrowUp", false, false, false, false).unwrap());
    assert!(session.handle_keydown("Enter", false, true, true, false).unwrap());
    assert!(session.handle_keydown("Escape", false, false, false, false).unwrap());
    assert!(!session.app().palette_open);

    let payloads: Vec<&str> = session
        .app()
        .keydown_payloads
        .iter()
        .map(|(_, s)| s.as_str())
        .collect();
    assert_eq!(
        payloads,
        vec!["Cmd+k", "ArrowDown", "ArrowUp", "Ctrl+Shift+Enter", "Escape"]
    );
}
