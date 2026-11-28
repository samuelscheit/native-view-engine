//! Pairing URI validation, wake-latch reset, rebind, busy, and destroy lifecycle edge-case tests.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nve_abi::{NveRuntimeConfig, NveRuntimeStatus, NveSceneView, NveStrSlice};
use nve_engine::Viewport;
use nve_runtime::{RuntimeBuilder, RuntimeError};
use nve_wire::{NveWireServer, NveWireServerConfig};

use super::{
    encode_scene, get_api, read_text_slab, wait_until, DummyLocalApp, StatefulSceneServer,
};

#[test]
fn pair_disconnect_then_pair_connect_preserves_wakes_and_validates_uri_synchronously() {
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        })
        .build();

    // Synchronous URI / host-port / TLS cert-pin validation BEFORE spawning any worker.
    for bad in [
        "",
        "not-a-url",
        "http://127.0.0.1:8092",
        "ws://bad host:8092/nve-wire",
        "ws://127.0.0.1:0/nve-wire",
        "ws://http::8092/nve-wire",
        "wss://127.0.0.1:8092/nve-wire",
        "quic://127.0.0.1:8091",
        "https://127.0.0.1:8091/nve-wire",
        "127.0.0.1:8092",
    ] {
        let err = rt
            .pair_connect(bad, "tok")
            .expect_err(&format!("URI `{bad}` must be rejected synchronously"));
        assert!(matches!(err, RuntimeError::InvalidArg(_)), "bad={bad} err={err:?}");
    }

    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_cb = Arc::clone(&wakes);
    rt.set_wake_callback(Arc::new(move || {
        wakes_cb.fetch_add(1, Ordering::Release);
    }));

    // Session 1: receive a wake WITHOUT calling `tick()`, leaving WakeBridge.pending == true.
    let token = "wake-preserve-tok";
    let srv1 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv1.broadcast_scene_keyframe(1, &encode_scene("SESSION_ONE"));
    rt.pair_connect(&format!("ws://{}/nve-wire", srv1.wss_addr()), token)
        .unwrap();
    assert!(wait_until(Duration::from_secs(3), || {
        wakes.load(Ordering::Acquire) >= 1
    }));

    // Disconnect WITHOUT ever calling `tick()`, then connect to Session 2.
    rt.pair_disconnect().unwrap();
    let wakes_after_disconnect = wakes.load(Ordering::Acquire);

    let srv2 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv2.broadcast_scene_keyframe(1, &encode_scene("SESSION_TWO"));
    rt.pair_connect(&format!("ws://{}/nve-wire", srv2.wss_addr()), token)
        .unwrap();

    // Session 2 MUST fire a fresh wake (never silenced by Session 1's unconsumed wake latch!).
    assert!(
        wait_until(Duration::from_secs(3), || {
            wakes.load(Ordering::Acquire) > wakes_after_disconnect
        }),
        "stale pending wake from disconnected pair session silenced new pair session"
    );
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("SESSION_TWO")
    }));

    // Calling pair_connect with an invalid URI while ALREADY paired MUST fail synchronously
    // WITHOUT disconnecting or disturbing the live paired session!
    assert!(matches!(
        rt.pair_connect("not-a-url", "tok"),
        Err(RuntimeError::InvalidArg(_))
    ));
    assert!(rt.pair_is_connected());
    assert_eq!(
        srv2.broadcast_scene_keyframe(2, &encode_scene("STILL_LIVE_AFTER_BAD_URI"))
            .unwrap(),
        1
    );
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("STILL_LIVE_AFTER_BAD_URI")
    }));

    rt.pair_disconnect().unwrap();
    srv1.shutdown();
    srv2.shutdown();
}

#[test]
fn pair_lifecycle_edge_cases_busy_rebind_detach_null_non_owner_and_destroy() {
    let api = get_api();
    let token = "lifecycle-edge-tok";
    let srv1 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let (mut scene1, f1) = StatefulSceneServer::new("EDGE_SRV1_V1");
    let _ = srv1.broadcast_scene_keyframe(1, &f1);

    let srv2 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv2.broadcast_scene_keyframe(1, &encode_scene("EDGE_SRV2_V1"));

    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: 320.0,
        height: 240.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Valid config.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());

    // 1. Non-owner thread `pair_connect` fails with InvalidArg.
    let h_addr = h as usize;
    let uri1 = format!("ws://{}/nve-wire", srv1.wss_addr());
    let uri1_bg = uri1.clone();
    let bg_rc = std::thread::spawn(move || {
        // SAFETY: Handle stays alive until thread joins.
        unsafe {
            (api.pair_connect)(
                h_addr as *mut c_void,
                NveStrSlice::from_str(&uri1_bg),
                NveStrSlice::from_str(token),
            )
        }
    })
    .join()
    .unwrap();
    assert_eq!(bg_rc, NveRuntimeStatus::InvalidArg.as_i32());

    // 2. `pair_connect` while `scene_acquire` is held returns Busy.
    let mut view = NveSceneView::default();
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        assert_eq!(
            (api.pair_connect)(h, NveStrSlice::from_str(&uri1), NveStrSlice::from_str(token)),
            NveRuntimeStatus::Busy.as_i32()
        );
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 3. Connect to srv1 and receive initial frame.
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!(
            (api.pair_connect)(h, NveStrSlice::from_str(&uri1), NveStrSlice::from_str(token)),
            NveRuntimeStatus::Ok.as_i32()
        );
    }
    assert!(wait_until(Duration::from_secs(3), || {
        let mut ch = 0u32;
        // SAFETY: Owner thread.
        unsafe { (api.tick)(h, 0.016, &mut ch) == NveRuntimeStatus::Ok.as_i32() && ch == 1 }
    }));

    // 4. `attach_app_interface(h, NULL)` while paired MUST be a no-op (does NOT wipe paired replica!).
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!((api.attach_app_interface)(h, None), NveRuntimeStatus::Ok.as_i32());
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        assert!(read_text_slab(&view).contains("EDGE_SRV1_V1"));
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }
    // Subsequent delta on srv1 MUST still apply cleanly!
    let f2 = scene1.update_label_delta("EDGE_SRV1_DELTA2");
    assert_eq!(srv1.broadcast_command_delta(2, &f2).unwrap(), 1);
    assert!(wait_until(Duration::from_secs(2), || {
        let mut ch = 0u32;
        // SAFETY: Owner thread.
        let _ = unsafe { (api.tick)(h, 0.016, &mut ch) };
        if ch == 1 {
            // SAFETY: Valid handle.
            unsafe {
                assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
                let ok = read_text_slab(&view).contains("EDGE_SRV1_DELTA2");
                assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
                ok
            }
        } else {
            false
        }
    }));

    // 5. `pair_connect` while ALREADY paired cleanly replaces srv1 with srv2!
    let uri2 = format!("ws://{}/nve-wire", srv2.wss_addr());
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!(
            (api.pair_connect)(h, NveStrSlice::from_str(&uri2), NveStrSlice::from_str(token)),
            NveRuntimeStatus::Ok.as_i32()
        );
    }
    assert!(wait_until(Duration::from_secs(3), || {
        let mut ch = 0u32;
        // SAFETY: Owner thread.
        let _ = unsafe { (api.tick)(h, 0.016, &mut ch) };
        if ch == 1 {
            // SAFETY: Valid handle.
            unsafe {
                assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
                let ok = read_text_slab(&view).contains("EDGE_SRV2_V1");
                assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
                ok
            }
        } else {
            false
        }
    }));

    // 6. Install wake callback, destroy while paired, then broadcast frames -> ZERO wakes after destroy!
    static AFTER_DESTROY_WAKES: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn on_destroy_wake(ctx: *mut c_void) {
        // SAFETY: `ctx` points to static `AFTER_DESTROY_WAKES`.
        unsafe { (*ctx.cast::<AtomicUsize>()).fetch_add(1, Ordering::Release) };
    }
    AFTER_DESTROY_WAKES.store(0, Ordering::Release);
    // SAFETY: Valid handle & static callback.
    unsafe {
        assert_eq!(
            (api.set_wake_callback)(
                h,
                Some(on_destroy_wake),
                (&raw const AFTER_DESTROY_WAKES).cast_mut().cast::<c_void>(),
            ),
            NveRuntimeStatus::Ok.as_i32()
        );
        (api.destroy)(h);
    }
    let wakes_at_destroy = AFTER_DESTROY_WAKES.load(Ordering::Acquire);
    let _ = srv2.broadcast_scene_keyframe(9, &encode_scene("AFTER_DESTROY"));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        AFTER_DESTROY_WAKES.load(Ordering::Acquire),
        wakes_at_destroy,
        "wake callback fired after destroy!"
    );

    // 7. Rust API: attaching a local app while paired cleanly disconnects pairing.
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        })
        .build();
    rt.pair_connect(&uri2, token).unwrap();
    assert!(wait_until(Duration::from_secs(3), || rt.pair_is_connected()));
    rt.attach_app(Box::new(DummyLocalApp)).unwrap();
    assert!(!rt.pair_is_connected());

    srv1.shutdown();
    srv2.shutdown();
}
