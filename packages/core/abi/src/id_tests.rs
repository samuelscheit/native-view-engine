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
fn index_zero_with_nonzero_generation_canonicalizes_to_null_at_all_ingresses() {
    let bad_node = NodeId::new(0, 1);
    assert_eq!(bad_node, NodeId::NULL);
    assert!(bad_node.is_null());
    assert_eq!(bad_node.index_u32(), 0);
    assert_eq!(bad_node.generation(), 0);
    assert_eq!(bad_node.to_string(), "node(null)");
    assert_eq!(NodeId::new(0, 255), NodeId::NULL);

    let bad_surface = SurfaceId::new(0, 1);
    assert_eq!(bad_surface, SurfaceId::NULL);
    assert!(bad_surface.is_null());
    assert_eq!(bad_surface.index_u32(), 0);
    assert_eq!(bad_surface.generation(), 0);
    assert_eq!(bad_surface.to_string(), "surface(null)");
    assert_eq!(SurfaceId::new(0, 255), SurfaceId::NULL);

    // NveCodec ingress seals `0x0100_0000` -> canonical `NULL` and preserves live handles.
    let bad_bytes = 0x0100_0000u32.to_le_bytes();
    assert_eq!(NodeId::decode_exact(&bad_bytes).unwrap(), NodeId::NULL);
    assert_eq!(SurfaceId::decode_exact(&bad_bytes).unwrap(), SurfaceId::NULL);

    let live_node = NodeId::new(42, 7);
    let live_surf = SurfaceId::new(19, 3);
    assert_eq!(
        NodeId::decode_exact(&live_node.encode_to_vec().unwrap()).unwrap(),
        live_node
    );
    assert_eq!(
        SurfaceId::decode_exact(&live_surf.encode_to_vec().unwrap()).unwrap(),
        live_surf
    );

    // Serde ingress seals `0x0100_0000` -> canonical `NULL` and preserves live handles.
    #[cfg(feature = "serde")]
    {
        use serde::de::value::{Error as DeError, U32Deserializer};
        use serde::Deserialize;

        let de_bad = U32Deserializer::<DeError>::new(0x0100_0000);
        assert_eq!(NodeId::deserialize(de_bad).unwrap(), NodeId::NULL);
        let de_bad_surf = U32Deserializer::<DeError>::new(0x0100_0000);
        assert_eq!(SurfaceId::deserialize(de_bad_surf).unwrap(), SurfaceId::NULL);

        let de_live_node = U32Deserializer::<DeError>::new(live_node.raw());
        assert_eq!(NodeId::deserialize(de_live_node).unwrap(), live_node);
        let de_live_surf = U32Deserializer::<DeError>::new(live_surf.raw());
        assert_eq!(SurfaceId::deserialize(de_live_surf).unwrap(), live_surf);
    }
}

#[test]
fn display_formats_cleanly_for_live_and_null_handles() {
    assert_eq!(NodeId::NULL.to_string(), "node(null)");
    assert_eq!(NodeId::ROOT.to_string(), "node(1,0)");
    assert_eq!(NodeId::new(42, 7).to_string(), "node(42,7)");

    assert_eq!(SurfaceId::NULL.to_string(), "surface(null)");
    assert_eq!(SurfaceId::ROOT.to_string(), "surface(1,0)");
    assert_eq!(SurfaceId::new(7, 3).to_string(), "surface(7,3)");

    assert_eq!(InterfaceId(5).to_string(), "5");
    assert_eq!(PropFieldId(12).to_string(), "12");
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
