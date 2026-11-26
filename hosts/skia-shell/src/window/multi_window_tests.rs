use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
    FlexDirection, FrameId, LayoutProps, NativeEventKind, NveDirectEvent, StyleProps, SurfaceId,
    ROOT_NODE_ID,
};
use nve_engine::{RenderBackend, Viewport};
use nve_interop::{SurfacePresentation, WindowDescriptor, WindowError, WindowRect};

use super::multi_window::HeadlessMultiWindowHost;
use super::{DesktopApp, DesktopHostError, DesktopSession};

struct SurfaceCounterApp {
    surface_id: SurfaceId,
    presses: u32,
    frame_buf: Vec<u8>,
}

impl SurfaceCounterApp {
    fn new(surface_id: SurfaceId) -> Self {
        Self { surface_id, presses: 0, frame_buf: Vec::new() }
    }
}

impl DesktopApp for SurfaceCounterApp {
    fn generation(&self) -> u64 { 1 }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let mut b = CommandBufferBuilder::new(self.surface_id, frame_id);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Percent(100.0),
                padding: EdgeInsets::all(12.0),
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
        );
        b.create_node(4, ComponentKind::Pressable, None);
        b.set_layout(
            4,
            &LayoutProps {
                width: Dimension::Points(140.0),
                height: Dimension::Points(48.0),
                ..Default::default()
            },
        );
        b.set_style(
            4,
            &StyleProps {
                background: if self.presses > 0 {
                    ColorRgba::rgb(16, 185, 129)
                } else {
                    ColorRgba::rgb(37, 99, 235)
                },
                ..Default::default()
            },
        );
        b.set_event_mask(4, EventMask::PRESS);
        b.set_children(2, &[4]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        self.frame_buf = b.into_bytes();
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: &[NveDirectEvent]) -> bool {
        let mut handled = false;
        for ev in events {
            if ev.surface_id == self.surface_id
                && ev.node_id == 4
                && ev.kind == NativeEventKind::Press as u8
            {
                self.presses += 1;
                handled = true;
            }
        }
        handled
    }

    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool { false }
    fn reload(&mut self, _path: &Path, _surface_id: SurfaceId) -> Result<u64, DesktopHostError> {
        Ok(1)
    }
}

fn make_session(surface_id: SurfaceId, w: f32, h: f32) -> DesktopSession<SurfaceCounterApp> {
    DesktopSession::new(
        SurfaceCounterApp::new(surface_id),
        surface_id,
        Viewport { width: w, height: h, scale_factor: 1.0 },
        ColorRgba::BLACK,
        false,
    )
}

#[test]
fn multi_window_host_routes_events_independently_and_enforces_modal_blocking() {
    let mut host = HeadlessMultiWindowHost::<u64, SurfaceCounterApp>::new();
    let primary_h = host
        .open_window(
            100,
            1,
            SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Primary", 400.0, 300.0)),
            make_session(1, 400.0, 300.0),
        )
        .unwrap();
    let inspector_h = host
        .open_window(
            200,
            2,
            SurfacePresentation::SecondaryWindow {
                parent: Some(1),
                modal: false,
                descriptor: WindowDescriptor::new("Inspector", 320.0, 260.0),
            },
            make_session(2, 320.0, 260.0),
        )
        .unwrap();

    assert_eq!(host.window_count(), 2);
    assert_ne!(primary_h, inspector_h);

    // Click inside window 200 (surface 2) affects ONLY surface 2, never surface 1.
    let btn2 = host.window(200).unwrap().session.backend().layout_of(4).unwrap();
    assert!(host
        .route_click(200, btn2.x + btn2.width * 0.5, btn2.y + btn2.height * 0.5)
        .unwrap());
    assert_eq!(host.window(200).unwrap().session.app().presses, 1);
    assert_eq!(host.window(100).unwrap().session.app().presses, 0);

    // Independent resize on window 100 does not resize window 200.
    assert!(host.route_resize(100, 800, 600, 2.0).unwrap());
    assert_eq!(host.window(100).unwrap().session.viewport().width, 400.0);
    assert_eq!(host.window(100).unwrap().session.pixel_buffer().width(), 800);
    assert_eq!(host.window(200).unwrap().session.pixel_buffer().width(), 320);

    // Open a modal dialog parented to surface 1 -> focus & clicks on primary are blocked!
    let modal_h = host
        .open_window(
            300,
            3,
            SurfacePresentation::SecondaryWindow {
                parent: Some(1),
                modal: true,
                descriptor: WindowDescriptor::new("Confirm", 280.0, 180.0),
            },
            make_session(3, 280.0, 180.0),
        )
        .unwrap();
    assert_eq!(
        host.route_focus(100),
        Err(WindowError::BlockedByModalChild { target: primary_h, modal_child: modal_h })
    );
    let btn1 = host.window(100).unwrap().session.backend().layout_of(4).unwrap();
    assert!(!host
        .route_click(100, btn1.x + btn1.width * 0.5, btn1.y + btn1.height * 0.5)
        .unwrap());
    assert_eq!(host.window(100).unwrap().session.app().presses, 0);

    // Closing modal unblocks primary clicks.
    let close_modal = host.handle_close_requested(300).unwrap();
    assert!(!close_modal.vetoed);
    assert!(!close_modal.should_exit_loop);
    assert_eq!(close_modal.closed_handles, vec![modal_h]);
    assert!(host.route_focus(100).is_ok());
    assert!(host
        .route_click(100, btn1.x + btn1.width * 0.5, btn1.y + btn1.height * 0.5)
        .unwrap());
    assert_eq!(host.window(100).unwrap().session.app().presses, 1);
}

#[test]
fn close_veto_and_cascading_child_cleanup_and_exit_policy() {
    let mut host = HeadlessMultiWindowHost::<u64, SurfaceCounterApp>::new();
    host.set_exit_on_primary_close(false);

    let allow_primary = Arc::new(AtomicBool::new(false));
    let allow_flag = Arc::clone(&allow_primary);
    host.set_close_veto_handler(move |_handle, sid| {
        if sid == 1 { allow_flag.load(Ordering::SeqCst) } else { true }
    });

    let primary_h = host
        .open_window(
            10,
            1,
            SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Primary", 400.0, 300.0)),
            make_session(1, 400.0, 300.0),
        )
        .unwrap();
    let sheet_h = host
        .open_window(
            20,
            2,
            SurfacePresentation::Sheet {
                parent: 1,
                descriptor: WindowDescriptor::new("Sheet", 300.0, 200.0),
            },
            make_session(2, 300.0, 200.0),
        )
        .unwrap();
    let popover_h = host
        .open_window(
            30,
            3,
            SurfacePresentation::Popover {
                anchor_surface: 2,
                anchor_rect: WindowRect::new(10.0, 10.0, 80.0, 28.0),
                descriptor: WindowDescriptor::new("Popover", 180.0, 140.0),
            },
            make_session(3, 180.0, 140.0),
        )
        .unwrap();
    let detached_h = host
        .open_window(
            40,
            4,
            SurfacePresentation::SecondaryWindow {
                parent: None,
                modal: false,
                descriptor: WindowDescriptor::new("Detached", 300.0, 200.0),
            },
            make_session(4, 300.0, 200.0),
        )
        .unwrap();

    // First close attempt on primary is vetoed -> all 4 windows remain open.
    let vetoed = host.handle_close_requested(10).unwrap();
    assert!(vetoed.vetoed);
    assert!(!vetoed.should_exit_loop);
    assert_eq!(host.window_count(), 4);

    // Allow close on primary -> cascades popover + sheet + primary, leaving detached open!
    allow_primary.store(true, Ordering::SeqCst);
    let closed_primary = host.handle_close_requested(10).unwrap();
    assert!(!closed_primary.vetoed);
    assert_eq!(closed_primary.closed_handles, vec![popover_h, sheet_h, primary_h]);
    assert!(!closed_primary.should_exit_loop);
    assert_eq!(host.window_count(), 1);
    assert!(host.window_by_surface(4).is_some());

    // Closing the final remaining window signals loop exit.
    let closed_last = host.handle_close_requested(40).unwrap();
    assert_eq!(closed_last.closed_handles, vec![detached_h]);
    assert!(closed_last.should_exit_loop);
    assert_eq!(host.window_count(), 0);
}
