use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nve_abi::{ColorRgba, EventMask, FrameId, NativeEventKind, NodeId, SurfaceId};
use nve_components::tables::{register_core_tables, FlexDirection, Interaction, Layout, Visual};
use nve_engine::Viewport;
use nve_interop::{SurfacePresentation, WindowDescriptor, WindowError, WindowRect};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore, Rgba};

use super::multi_window::HeadlessMultiWindowHost;
use super::{DesktopApp, DesktopHostError, DesktopSession};

struct SurfaceCounterApp {
    surface_id: SurfaceId,
    presses: u32,
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    initialized: bool,
    frame_buf: Vec<u8>,
}

impl SurfaceCounterApp {
    fn new(surface_id: SurfaceId) -> Self {
        register_core_tables();
        Self {
            surface_id,
            presses: 0,
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            initialized: false,
            frame_buf: Vec::new(),
        }
    }
}

impl DesktopApp for SurfaceCounterApp {
    fn generation(&self) -> u64 { 1 }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        let mut tx = self.store.tx();
        if !self.initialized {
            let _ = tx.create_at(NodeId::ROOT);
            let _ = tx.create_at(NodeId::new(2, 0));
            let _ = tx.create_at(NodeId::new(4, 0));
            tx.set(NodeId::new(2, 0), Layout::WIDTH, Length::percent(100.0));
            tx.set(NodeId::new(2, 0), Layout::HEIGHT, Length::percent(100.0));
            let p = Length::points(12.0);
            tx.set(NodeId::new(2, 0), Layout::PADDING_TOP, p);
            tx.set(NodeId::new(2, 0), Layout::PADDING_RIGHT, p);
            tx.set(NodeId::new(2, 0), Layout::PADDING_BOTTOM, p);
            tx.set(NodeId::new(2, 0), Layout::PADDING_LEFT, p);
            tx.set(NodeId::new(2, 0), Layout::FLEX_DIRECTION, FlexDirection::Column);
            tx.set(NodeId::new(4, 0), Layout::WIDTH, Length::points(140.0));
            tx.set(NodeId::new(4, 0), Layout::HEIGHT, Length::points(48.0));
            tx.set(NodeId::new(4, 0), Interaction::EVENT_MASK, EventMask::PRESS as u32);
            tx.set_children(NodeId::new(2, 0), &[NodeId::new(4, 0)]);
            tx.set_children(NodeId::ROOT, &[NodeId::new(2, 0)]);
            self.initialized = true;
        }
        let bg = if self.presses > 0 {
            Rgba::rgb(16, 185, 129)
        } else {
            Rgba::rgb(37, 99, 235)
        };
        tx.set(NodeId::new(4, 0), Visual::BACKGROUND, bg);
        tx.commit_into(&mut self.changes);
        self.frame_buf.clear();
        self.encoder
            .encode(&self.store, &self.changes, &mut self.frame_buf);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            if ev.surface_id() == self.surface_id
                && ev.node_id() == NodeId::new(4, 0)
                && ev.kind() == Some(NativeEventKind::Press)
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
            SurfaceId::ROOT,
            SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Primary", 400.0, 300.0)),
            make_session(SurfaceId::ROOT, 400.0, 300.0),
        )
        .unwrap();
    let inspector_h = host
        .open_window(
            200,
            SurfaceId::new(2, 0),
            SurfacePresentation::SecondaryWindow {
                parent: Some(SurfaceId::ROOT),
                modal: false,
                descriptor: WindowDescriptor::new("Inspector", 320.0, 260.0),
            },
            make_session(SurfaceId::new(2, 0), 320.0, 260.0),
        )
        .unwrap();

    assert_eq!(host.window_count(), 2);
    assert_ne!(primary_h, inspector_h);

    // Click inside window 200 (surface 2) affects ONLY surface 2, never surface 1.
    let btn2 = host.window(200).unwrap().session.layout_of(NodeId::new(4, 0)).unwrap();
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
            SurfaceId::new(3, 0),
            SurfacePresentation::SecondaryWindow {
                parent: Some(SurfaceId::ROOT),
                modal: true,
                descriptor: WindowDescriptor::new("Confirm", 280.0, 180.0),
            },
            make_session(SurfaceId::new(3, 0), 280.0, 180.0),
        )
        .unwrap();
    assert_eq!(
        host.route_focus(100),
        Err(WindowError::BlockedByModalChild { target: primary_h, modal_child: modal_h })
    );
    let btn1 = host.window(100).unwrap().session.layout_of(NodeId::new(4, 0)).unwrap();
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
        if sid == SurfaceId::ROOT { allow_flag.load(Ordering::SeqCst) } else { true }
    });

    let primary_h = host
        .open_window(
            10,
            SurfaceId::ROOT,
            SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Primary", 400.0, 300.0)),
            make_session(SurfaceId::ROOT, 400.0, 300.0),
        )
        .unwrap();
    let sheet_h = host
        .open_window(
            20,
            SurfaceId::new(2, 0),
            SurfacePresentation::Sheet {
                parent: SurfaceId::ROOT,
                descriptor: WindowDescriptor::new("Sheet", 300.0, 200.0),
            },
            make_session(SurfaceId::new(2, 0), 300.0, 200.0),
        )
        .unwrap();
    let popover_h = host
        .open_window(
            30,
            SurfaceId::new(3, 0),
            SurfacePresentation::Popover {
                anchor_surface: SurfaceId::new(2, 0),
                anchor_rect: WindowRect::new(10.0, 10.0, 80.0, 28.0),
                descriptor: WindowDescriptor::new("Popover", 180.0, 140.0),
            },
            make_session(SurfaceId::new(3, 0), 180.0, 140.0),
        )
        .unwrap();
    let detached_h = host
        .open_window(
            40,
            SurfaceId::new(4, 0),
            SurfacePresentation::SecondaryWindow {
                parent: None,
                modal: false,
                descriptor: WindowDescriptor::new("Detached", 300.0, 200.0),
            },
            make_session(SurfaceId::new(4, 0), 300.0, 200.0),
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
    assert!(host.window_by_surface(SurfaceId::new(4, 0)).is_some());

    // Closing the final remaining window signals loop exit.
    let closed_last = host.handle_close_requested(40).unwrap();
    assert_eq!(closed_last.closed_handles, vec![detached_h]);
    assert!(closed_last.should_exit_loop);
    assert_eq!(host.window_count(), 0);
}
