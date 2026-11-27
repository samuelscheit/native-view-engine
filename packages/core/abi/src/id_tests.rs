use super::*;
use crate::NVE_PRIMARY_SLOT_NONE;

#[test]
fn node_and_surface_id_try_new_and_from_raw_reject_null_and_overflow() {
    assert_eq!(NodeId::try_new(0, 0), None);
    assert_eq!(NodeId::try_new(0, 1), None);
    assert_eq!(NodeId::try_new(NodeId::MAX_INDEX + 1, 0), None);
    assert_eq!(NodeId::try_new(u32::MAX, 0), None);

    let max_node = NodeId::try_new(NodeId::MAX_INDEX, 255).unwrap();
    assert_eq!(max_node.index_u32(), NodeId::MAX_INDEX);
    assert_eq!(max_node.generation(), 255);
    assert!(!max_node.is_null());

    assert_eq!(NodeId::from_raw(0), None);
    assert_eq!(NodeId::from_raw(1u32 << NodeId::INDEX_BITS), None);
    assert_eq!(NodeId::from_raw(0xFF00_0000), None);
    assert_eq!(NodeId::from_raw(max_node.raw()), Some(max_node));
    assert_eq!(NodeId::from_raw_or_null(0), NodeId::NULL);
    assert_eq!(NodeId::from_raw_or_null(1u32 << NodeId::INDEX_BITS), NodeId::NULL);
    assert_eq!(NodeId::from_raw_or_null(max_node.raw()), max_node);

    assert_eq!(SurfaceId::try_new(0, 0), None);
    assert_eq!(SurfaceId::try_new(0, 1), None);
    assert_eq!(SurfaceId::try_new(SurfaceId::MAX_INDEX + 1, 0), None);
    assert_eq!(SurfaceId::try_new(u32::MAX, 0), None);

    let max_surf = SurfaceId::try_new(SurfaceId::MAX_INDEX, 255).unwrap();
    assert_eq!(max_surf.index_u32(), SurfaceId::MAX_INDEX);
    assert_eq!(max_surf.generation(), 255);
    assert!(!max_surf.is_null());

    assert_eq!(SurfaceId::from_raw(0), None);
    assert_eq!(SurfaceId::from_raw(1u32 << SurfaceId::INDEX_BITS), None);
    assert_eq!(SurfaceId::from_raw(0xFF00_0000), None);
    assert_eq!(SurfaceId::from_raw(max_surf.raw()), Some(max_surf));
    assert_eq!(SurfaceId::from_raw_or_null(0), SurfaceId::NULL);
    assert_eq!(SurfaceId::from_raw_or_null(1u32 << SurfaceId::INDEX_BITS), SurfaceId::NULL);
    assert_eq!(SurfaceId::from_raw_or_null(max_surf.raw()), max_surf);
}

#[test]
fn index_zero_with_nonzero_generation_is_treated_as_null() {
    let bad_node = NodeId::new(0, 1);
    assert!(bad_node.is_null());
    assert_eq!(bad_node.index_u32(), 0);
    assert_eq!(bad_node.generation(), 1);
    assert_eq!(bad_node.to_string(), "node(null)");

    let bad_surface = SurfaceId::new(0, 1);
    assert!(bad_surface.is_null());
    assert_eq!(bad_surface.index_u32(), 0);
    assert_eq!(bad_surface.generation(), 1);
    assert_eq!(bad_surface.to_string(), "surface(null)");
}

#[test]
fn display_formats_cleanly_for_live_and_null_handles() {
    assert_eq!(NodeId::NULL.to_string(), "node(null)");
    assert_eq!(NodeId::ROOT.to_string(), "node(1,0)");
    assert_eq!(NodeId::new(42, 7).to_string(), "node(42,7)");

    assert_eq!(SurfaceId::NULL.to_string(), "surface(null)");
    assert_eq!(SurfaceId::ROOT.to_string(), "surface(1,0)");
    assert_eq!(SurfaceId::new(7, 3).to_string(), "surface(7,3)");
}

#[test]
fn frame_id_next_skips_zero_on_wraparound() {
    assert_eq!(FrameId::ZERO.next(), FrameId(1));
    assert_eq!(FrameId(1).next(), FrameId(2));
    assert_eq!(FrameId(u64::MAX).next(), FrameId(1));
    assert_ne!(FrameId(u64::MAX).next(), FrameId::ZERO);
}

#[test]
fn primary_slot_none_matches_scene_abi_constant() {
    assert_eq!(PrimarySlot::NONE.raw(), NVE_PRIMARY_SLOT_NONE);
    assert!(PrimarySlot::NONE.is_none());
    assert_eq!(PrimarySlot::NONE.slot(), None);
    assert_eq!(PrimarySlot(3).slot(), Some(3));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "NodeId index exceeds 24-bit MAX_INDEX")]
fn node_id_new_debug_asserts_index_bounds() {
    let _ = NodeId::new(1 << NodeId::INDEX_BITS, 0);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "SurfaceId index exceeds 24-bit MAX_INDEX")]
fn surface_id_new_debug_asserts_index_bounds() {
    let _ = SurfaceId::new(1 << SurfaceId::INDEX_BITS, 0);
}
