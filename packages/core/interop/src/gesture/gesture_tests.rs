use nve_abi::SurfaceId;

use super::types::{FastKeyMap, SurfaceChain, INLINE_CHAIN_CAP};
use super::*;
use crate::surface::FrameworkKind;

#[test]
fn surface_chain_inline_and_heap_spill_preserve_order_and_retain() {
    let small: Vec<SurfaceId> = (1..=INLINE_CHAIN_CAP as u32).map(SurfaceId).collect();
    let mut inline_chain = SurfaceChain::from_slice(&small);
    assert!(matches!(inline_chain, SurfaceChain::Inline { .. }));
    assert_eq!(inline_chain.as_slice(), small.as_slice());
    inline_chain.retain(|id| id.raw() % 2 == 0);
    let expected_small: Vec<SurfaceId> = small.into_iter().filter(|id| id.raw() % 2 == 0).collect();
    assert_eq!(inline_chain.as_slice(), expected_small.as_slice());

    let deep: Vec<SurfaceId> = (1..=(INLINE_CHAIN_CAP as u32 + 5)).map(SurfaceId).collect();
    let mut heap_chain = SurfaceChain::from_slice(&deep);
    assert!(matches!(heap_chain, SurfaceChain::Heap(_)));
    assert_eq!(heap_chain.as_slice(), deep.as_slice());
    heap_chain.retain(|id| id.raw() > 10);
    let expected_deep: Vec<SurfaceId> = deep.into_iter().filter(|&id| id.raw() > 10).collect();
    assert_eq!(heap_chain.as_slice(), expected_deep.as_slice());
}

#[test]
fn fast_u32_map_inline_spill_remove_and_retain() {
    let mut map: FastKeyMap<u32, u32, 4> = FastKeyMap::default();
    assert!(map.is_empty());

    for k in 1..=4u32 {
        assert_eq!(map.insert(k, k * 10), None);
    }
    assert!(matches!(map, FastKeyMap::Inline { .. }));
    assert_eq!(map.insert(2, 200), Some(20));
    assert_eq!(map.get(2), Some(&200));
    assert_eq!(map.remove(3), Some(30));
    assert_eq!(map.len(), 3);
    assert_eq!(map.get(3), None);

    // Refill and trigger spill to FxHashMap.
    assert_eq!(map.insert(3, 30), None);
    assert_eq!(map.insert(5, 50), None);
    assert!(matches!(map, FastKeyMap::Spill(_)));
    assert_eq!(map.len(), 5);
    assert_eq!(map.get(1), Some(&10));
    assert_eq!(map.get(2), Some(&200));
    assert_eq!(map.get(5), Some(&50));

    map.retain(|k, v| {
        *v += 1;
        k % 2 == 1
    });
    assert_eq!(map.len(), 3);
    assert_eq!(map.get(1), Some(&11));
    assert_eq!(map.get(3), Some(&31));
    assert_eq!(map.get(5), Some(&51));
    assert_eq!(map.get(2), None);
}

#[test]
fn gesture_arbitrator_handles_deep_chains_and_many_surfaces_across_spill() {
    let mut arb = GestureArbitrator::new();
    // Register 24 surfaces (> inline cap 16) and 12 active pointers (> inline cap 8).
    let chain: Vec<SurfaceId> = (1..=24u32).map(SurfaceId).collect();
    for &sid in &chain {
        let mut caps = GestureCapabilities::vertical_scroll(FrameworkKind::Nve);
        if sid.raw() < 20 {
            caps.scroll_edges.at_bottom = true;
        }
        arb.set_capabilities(sid, caps);
    }

    for ptr in 1..=12u32 {
        let down = arb.arbitrate(
            &chain,
            PointerSample {
                pointer_id: ptr,
                phase: PointerPhase::Down,
                x: 10.0,
                y: 100.0,
                delta_x: 0.0,
                delta_y: 0.0,
                timestamp_us: u64::from(ptr) * 1000,
            },
        );
        assert_eq!(down.winner, Some(SurfaceId(1)));

        let mv = arb.arbitrate(
            &chain,
            PointerSample {
                pointer_id: ptr,
                phase: PointerPhase::Move,
                x: 10.0,
                y: 60.0,
                delta_x: 0.0,
                delta_y: -40.0,
                timestamp_us: u64::from(ptr) * 1000 + 500,
            },
        );
        assert_eq!(mv.winner, Some(SurfaceId(20)));
        assert_eq!(mv.innermost_disposition, GestureDisposition::DeferToParent);
        assert_eq!(mv.cancelled_surfaces, vec![SurfaceId(1)]);
    }
    assert_eq!(arb.active_pointer_count(), 12);

    // Unregistering winner clears it from every active pointer.
    arb.unregister_surface(SurfaceId(20));
    for ptr in 1..=12u32 {
        let end = arb.arbitrate(
            &chain,
            PointerSample {
                pointer_id: ptr,
                phase: PointerPhase::Up,
                x: 10.0,
                y: 60.0,
                delta_x: 0.0,
                delta_y: 0.0,
                timestamp_us: 99_000,
            },
        );
        assert_eq!(end.winner, None);
        assert_eq!(end.reason, ArbitrationReason::PointerEnded);
    }
    assert_eq!(arb.active_pointer_count(), 0);
}
