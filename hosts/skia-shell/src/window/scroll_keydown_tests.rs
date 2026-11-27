use std::path::Path;

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EventMask, FlexDirection, FrameId,
    LayoutProps, NativeEventKind, StyleProps, SurfaceId,
    ROOT_NODE_ID,
};
use nve_engine::Viewport;

use super::input::format_keydown_shorthand;
use super::{DesktopApp, DesktopHostError, DesktopSession};

struct MockScrollAndKeyApp {
    scroll_events: Vec<(u32, f32, f32, f32, f32)>,
    keydown_payloads: Vec<(u32, String)>,
    palette_open: bool,
    frame_buf: Vec<u8>,
}

impl DesktopApp for MockScrollAndKeyApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let mut b = CommandBufferBuilder::new(1, frame_id);
        b.create_node(2, ComponentKind::ScrollView, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(200.0),
                height: Dimension::Points(100.0),
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
        );
        b.set_event_mask(2, EventMask::SCROLL | EventMask::KEY_DOWN | EventMask::CUSTOM);
        for (id, color) in [
            (10u32, ColorRgba::rgb(239, 68, 68)),
            (11u32, ColorRgba::rgb(16, 185, 129)),
            (12u32, ColorRgba::rgb(37, 99, 235)),
        ] {
            b.create_node(id, ComponentKind::View, None);
            b.set_layout(
                id,
                &LayoutProps {
                    width: Dimension::Points(200.0),
                    height: Dimension::Points(80.0),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
            );
            b.set_style(
                id,
                &StyleProps {
                    background: if self.palette_open {
                        ColorRgba::rgb(250, 204, 21)
                    } else {
                        color
                    },
                    ..Default::default()
                },
            );
        }
        b.set_children(2, &[10, 11, 12]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        self.frame_buf = b.into_bytes();
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
    let app = MockScrollAndKeyApp {
        scroll_events: Vec::new(),
        keydown_payloads: Vec::new(),
        palette_open: false,
        frame_buf: Vec::new(),
    };
    let mut session = DesktopSession::new(app, 1, viewport, ColorRgba::BLACK, false);
    session.render_and_rasterize().unwrap();

    assert_eq!(session.pixel_buffer().get_rgb(50, 20), Some((239, 68, 68)));
    assert_eq!(session.pixel_buffer().get_rgb(50, 120), Some((0, 0, 0)));
    assert!(!session.handle_scroll_logical(220.0, 140.0, 0.0, 60.0).unwrap());

    assert!(session.handle_scroll_logical(50.0, 50.0, 0.0, 100.0).unwrap());
    assert_eq!(session.app().scroll_events[0], (2, 0.0, 100.0, 200.0, 240.0));
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
