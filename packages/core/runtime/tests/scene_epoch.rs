//! `NveRuntimeSceneView::scene_epoch`: hosts detect every scene reset (keyframe, detach) even when the new scene reuses the same node ids.

use nve_components::tables::TextContent;
use nve_abi::{FrameId, NveEventBatchRef};
use nve_runtime::{AppFrame, RuntimeApp, RuntimeBuilder, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, NodeId, NodeStore};

/// A fresh one-text keyframe (frame 1 of a new session: same node ids every time).
fn keyframe(text: &str) -> Vec<u8> {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let n = tx.create();
    tx.edit::<TextContent>(n).text(text);
    tx.set_children(NodeId::ROOT, &[n]);
    tx.commit_into(&mut changes);
    let mut out = Vec::new();
    WireEncoder::default().encode(&store, &changes, &mut out);
    out
}

/// Renders one keyframe, then stays clean.
struct OneFrameApp(Option<Vec<u8>>, Vec<u8>);

impl RuntimeApp for OneFrameApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        match self.0.take() {
            Some(frame) => {
                self.1 = frame;
                AppFrame::Wire(&self.1)
            }
            None => AppFrame::Clean,
        }
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }
}

fn epoch(rt: &SurfaceRuntime) -> (u32, u64) {
    let guard = rt.acquire_scene().expect("acquire");
    (guard.view().scene_epoch, guard.view().frame_id)
}

#[test]
fn every_scene_reset_bumps_the_epoch_and_plain_frames_do_not() {
    let mut rt = RuntimeBuilder::new().build();
    let (e0, _) = epoch(&rt);
    rt.submit_nve2(&keyframe("a")).expect("first keyframe");
    let (e1, f1) = epoch(&rt);
    assert_eq!(e1, e0, "the first session starts in the current epoch");

    rt.submit_nve2(&keyframe("b")).expect("second keyframe resets the replica");
    let (e2, f2) = epoch(&rt);
    assert_eq!(e2, e1.wrapping_add(1), "a new keyframe discards the old scene");
    assert!(f2 > f1, "frame ids stay monotonic across epochs");

    assert!(rt.submit_nve2(&[1, 2, 3, 4]).is_err(), "corrupt header");
    let (e3, _) = epoch(&rt);
    assert_eq!(e3, e2, "a frame rejected before applying keeps the scene and its epoch");

    rt.detach_app().expect("detach without an app");
    assert_eq!(epoch(&rt).0, e3, "detaching nothing keeps the scene");

    rt.attach_app(Box::new(OneFrameApp(Some(keyframe("app")), Vec::new()))).expect("attach");
    let _ = rt.tick(0.0);
    let (e4, _) = epoch(&rt);
    assert!(e4 != e3, "attaching an app discards the submitted scene");
    let _ = rt.tick(0.0);
    assert_eq!(epoch(&rt).0, e4, "ticking an unchanged surface keeps the epoch");
    rt.detach_app().expect("detach");
    assert!(epoch(&rt).0 != e4, "detaching the app discards its scene");
}
