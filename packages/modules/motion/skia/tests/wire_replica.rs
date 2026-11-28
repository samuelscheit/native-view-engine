//! A wire replica whose process never touched `Motion` before registering `MotionDriver` must
//! still animate transitions: the frames come from a child process (its own table registry),
//! exactly like a hot-swapped cdylib host.

use std::process::Command;

use nve_abi::{ColorRgba, SurfaceId};
use nve_backend_skia::SkiaBackend;
use nve_engine::{StoreBackend, StoreWireReplica, Viewport};
use nve_motion::{EasingCurve, Motion, TransitionProperty, TransitionSpec, Transitions};
use nve_motion_skia::MotionDriver;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{core_realization_order, Layout, Visual};

const OUT_ENV: &str = "NVE_MOTION_WIRE_FRAMES_OUT";

/// Child side: encodes an initial frame (button with a background transition) and a second
/// frame changing the background. A no-op unless spawned by the parent test.
#[test]
fn child_encodes_frames() {
    let Ok(out) = std::env::var(OUT_ENV) else {
        return;
    };
    let mut store = NodeStore::new();
    let mut enc = WireEncoder::default();
    let mut frames: Vec<u8> = Vec::new();
    let mut push_frame = |store: &NodeStore, changes: &ChangeSet, enc: &mut WireEncoder| {
        let mut bytes = Vec::new();
        enc.encode(store, changes, &mut bytes);
        frames.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        frames.extend_from_slice(&bytes);
    };

    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let btn = tx.create();
    tx.set_children(NodeId::ROOT, &[btn]);
    tx.edit::<Layout>(btn).width(Length::points(100.0)).height(Length::points(40.0));
    tx.edit::<Visual>(btn).background(Rgba::rgb(0, 0, 0));
    tx.edit::<Motion>(btn).transitions(Some(Transitions::new([TransitionSpec::new(
        TransitionProperty::BackgroundColor,
        200,
    )
    .easing(EasingCurve::Linear)])));
    tx.commit_into(&mut changes);
    push_frame(&store, &changes, &mut enc);

    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    tx.edit::<Visual>(btn).background(Rgba::rgb(200, 200, 200));
    tx.commit_into(&mut changes);
    push_frame(&store, &changes, &mut enc);

    std::fs::write(out, frames).expect("write frames");
}

#[test]
fn replica_without_prior_motion_registration_animates_wire_transitions() {
    if std::env::var(OUT_ENV).is_ok() {
        return;
    }
    let out = std::env::temp_dir().join(format!("nve-motion-wire-{}.bin", std::process::id()));
    let status = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "child_encodes_frames", "--test-threads", "1"])
        .env(OUT_ENV, &out)
        .status()
        .expect("spawn child");
    assert!(status.success());
    let frames = std::fs::read(&out).expect("frames");
    let _ = std::fs::remove_file(&out);

    // This process registers `Motion` only through `register_frame_hook`.
    let mut skia = SkiaBackend::new();
    skia.register_frame_hook(MotionDriver::new());
    skia.set_viewport(SurfaceId::ROOT, Viewport { width: 200.0, height: 100.0, scale_factor: 1.0 });
    let mut replica = StoreWireReplica::new(skia, core_realization_order());
    let mut rest = frames.as_slice();
    while let Some((len, tail)) = rest.split_first_chunk::<4>() {
        let len = u32::from_le_bytes(*len) as usize;
        replica.submit_nve2_frame(&tail[..len]).expect("apply frame");
        rest = &tail[len..];
    }

    let btn = replica.store().children(NodeId::ROOT)[0];
    assert!(replica.store().has::<Motion>(btn), "Motion resolved to its static slot");
    let (store, skia) = replica.store_and_backend_mut();
    assert!(skia.needs_next_frame(), "background change animates over the wire");
    assert!(skia.advance_animations(store, 0.1));
    let mid = skia.animated_style(store, btn).expect("live").background;
    assert!(mid.r > 0 && mid.r < 200, "linear midpoint, got {mid:?}");
    assert!(!skia.advance_animations(store, 0.2));
    assert_eq!(skia.animated_style(store, btn).expect("live").background, ColorRgba::rgb(200, 200, 200));
}
