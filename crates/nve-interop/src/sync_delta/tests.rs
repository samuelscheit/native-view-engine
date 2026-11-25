use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, FlexDirection,
    LayoutProps, StyleProps, ROOT_NODE_ID,
};

use super::wire::write_varint;
use super::*;

fn make_studio_like_command_buffer(frame_id: u64, accent: ColorRgba, status_text: &str) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, frame_id);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            flex_direction: FlexDirection::Row,
            padding: EdgeInsets::all(12.0),
            gap: 8.0,
            ..Default::default()
        },
    );
    let mut children = Vec::new();
    for i in 0..40u32 {
        let nid = 10 + i;
        children.push(nid);
        b.create_node(nid, ComponentKind::View, None);
        b.set_layout(
            nid,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Points(44.0),
                ..Default::default()
            },
        );
        b.set_style(
            nid,
            &StyleProps {
                background: if i == 0 {
                    accent
                } else {
                    ColorRgba::rgb(30, 41, 59)
                },
                border_radius: 12.0,
                font_size: 12.0,
                ..Default::default()
            },
        );
    }
    b.create_node(99, ComponentKind::Text, None);
    b.set_text(99, status_text);
    children.push(99);
    b.set_children(2, &children);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    b.into_bytes()
}

#[test]
fn incremental_studio_edits_stay_under_256_bytes_and_under_10_kbps() {
    let mut sender = SyncDeltaSession::default();
    let mut receiver = SyncDeltaSession::default();

    // Frame 1: Full Studio tree keyframe (~6 KB raw).
    let f1 = make_studio_like_command_buffer(1, ColorRgba::rgb(56, 189, 248), "Todo");
    assert!(f1.len() > 4_000, "expected realistic multi-KB Studio frame");

    let p1 = sender.encode_frame(&f1, 0);
    let r1 = receiver.receive_packet(&p1).unwrap();
    assert_eq!(r1.applied_frames.len(), 1);
    assert_eq!(r1.applied_frames[0].1, f1);
    sender.handle_ack(r1.ack_packet.as_ref().unwrap()).unwrap();
    assert_eq!(sender.in_flight_count(), 0);

    // Frame 2: Incremental Studio edit (task status text changed + accent toggled!).
    // Even with variable-length string shift ("Todo" -> "In Progress" shifts all subsequent bytes!),
    // CommandOpDelta encodes ONLY the 2 changed ops + CopyRun tokens!
    let f2 = make_studio_like_command_buffer(2, ColorRgba::rgb(167, 139, 250), "In Progress");
    let p2 = sender.encode_frame(&f2, 16);
    let parsed_p2 = DeltaPacketRef::parse(&p2).unwrap();
    assert_eq!(parsed_p2.header().kind, PacketKind::CommandOpDelta);
    assert!(
        p2.len() < 256,
        "incremental Studio edit wire packet must be < 256B, got {} B (raw {} B)",
        p2.len(),
        f2.len()
    );

    let kbps_30 = SyncMetrics::estimated_kbps(p2.len(), 30.0);
    let kbps_60 = SyncMetrics::estimated_kbps(p2.len(), 60.0);
    assert!(
        kbps_60 < 10.0,
        "expected < 10 KB/s at 60 edits/sec, got {kbps_60:.2} KB/s ({} B/frame)",
        p2.len()
    );

    let r2 = receiver.receive_packet(&p2).unwrap();
    assert_eq!(r2.applied_frames.len(), 1);
    assert_eq!(r2.applied_frames[0].1, f2);

    // Frame 3: Single-field status cycle ("In Progress" -> "Done").
    let f3 = make_studio_like_command_buffer(3, ColorRgba::rgb(167, 139, 250), "Done");
    let p3 = sender.encode_frame(&f3, 33);
    assert!(
        p3.len() < 160,
        "single-field edit should be < 160B, got {} B",
        p3.len()
    );
    let r3 = receiver.receive_packet(&p3).unwrap();
    assert_eq!(r3.applied_frames[0].1, f3);

    eprintln!(
        "[sync_delta] raw={} B | accent+status delta={} B ({:.2} KB/s @30fps, {:.2} KB/s @60fps) | status-only delta={} B",
        f2.len(),
        p2.len(),
        kbps_30,
        kbps_60,
        p3.len()
    );
}

#[test]
fn xor_rle_handles_growth_shrink_identical_and_arbitrary_binary_blobs() {
    let base = vec![0xABu8; 8192];

    // 1. Identical payload -> tiny delta (target_len varint + 1 extended zero-run)
    let d_same = encode_xor_rle_delta(&base, &base);
    assert!(d_same.len() <= 8, "identical 8KB delta was {}B", d_same.len());
    assert_eq!(decode_xor_rle_delta(&base, &d_same).unwrap(), base);

    // 2. Mutate 4 bytes in the middle of 8KB
    let mut modified = base.clone();
    modified[4000] = 0x11;
    modified[4001] = 0x22;
    modified[4002] = 0x33;
    modified[4003] = 0x44;
    let d_mod = encode_xor_rle_delta(&base, &modified);
    assert!(d_mod.len() < 24, "4-byte delta in 8KB was {}B", d_mod.len());
    assert_eq!(decode_xor_rle_delta(&base, &d_mod).unwrap(), modified);

    // 3. Growing buffer
    let mut grown = base.clone();
    grown.extend_from_slice(b"appended-tail-data");
    let d_grow = encode_xor_rle_delta(&base, &grown);
    assert_eq!(decode_xor_rle_delta(&base, &d_grow).unwrap(), grown);

    // 4. Shrinking buffer to 0 bytes
    let d_empty = encode_xor_rle_delta(&base, &[]);
    assert_eq!(decode_xor_rle_delta(&base, &d_empty).unwrap(), Vec::<u8>::new());
}

#[test]
fn out_of_order_delivery_nack_retransmit_and_rto_window_recovery() {
    let mut sender = SyncDeltaSession::new(8);
    let mut receiver = SyncDeltaSession::new(8);

    let f1 = make_studio_like_command_buffer(1, ColorRgba::rgb(56, 189, 248), "S1");
    let f2 = make_studio_like_command_buffer(2, ColorRgba::rgb(56, 189, 248), "S2");
    let f3 = make_studio_like_command_buffer(3, ColorRgba::rgb(56, 189, 248), "S3");

    let p1 = sender.encode_frame(&f1, 100);
    let _p2 = sender.encode_frame(&f2, 116);
    let p3 = sender.encode_frame(&f3, 132);
    assert_eq!(sender.in_flight_count(), 3);

    // Deliver p1, drop p2, deliver p3 out of order!
    let r1 = receiver.receive_packet(&p1).unwrap();
    assert_eq!(r1.applied_frames.len(), 1);
    sender.handle_ack(r1.ack_packet.as_ref().unwrap()).unwrap();
    assert_eq!(sender.in_flight_count(), 2);

    let r3 = receiver.receive_packet(&p3).unwrap();
    assert!(r3.applied_frames.is_empty(), "p3 must wait for missing p2");
    assert_eq!(r3.nack_packets.len(), 1, "expected NACK for seq 2");

    // Selective ACK for p3 evicts p3 from sender's window while keeping p2!
    sender.handle_ack(r3.ack_packet.as_ref().unwrap()).unwrap();
    assert_eq!(sender.in_flight_count(), 1);

    // Sender handles NACK for seq 2 -> retransmits with FLAG_RETRANSMIT set.
    let retx2 = sender
        .handle_nack(&r3.nack_packets[0], 150)
        .unwrap()
        .expect("expected retransmit for seq 2");
    let parsed_retx = DeltaPacketRef::parse(&retx2).unwrap();
    assert_eq!(parsed_retx.header().seq, 2);
    assert_ne!(parsed_retx.header().flags & FLAG_RETRANSMIT, 0);

    // Delivering retx2 applies BOTH seq 2 AND buffered seq 3 in strict order!
    let r2 = receiver.receive_packet(&retx2).unwrap();
    assert_eq!(r2.applied_frames.len(), 2);
    assert_eq!(r2.applied_frames[0], (2, f2));
    assert_eq!(r2.applied_frames[1], (3, f3));
    assert_eq!(receiver.last_applied_seq(), 3);
    sender.handle_ack(r2.ack_packet.as_ref().unwrap()).unwrap();
    assert_eq!(sender.in_flight_count(), 0);

    // RTO poll triggers retransmit when unacked packet ages past RTO.
    let f4 = make_studio_like_command_buffer(4, ColorRgba::rgb(34, 197, 94), "S4");
    let _p4 = sender.encode_frame(&f4, 200);
    assert!(sender.poll_retransmits(220, 50).is_empty());
    let timed_out = sender.poll_retransmits(260, 50);
    assert!(!timed_out.is_empty());
    assert!(sender.metrics().retransmits_sent >= 2);
}

#[test]
fn fuzz_and_corruption_guards_reject_bitflips_truncation_and_oom_bombs() {
    let mut sender = SyncDeltaSession::default();
    let f1 = make_studio_like_command_buffer(1, ColorRgba::rgb(56, 189, 248), "Alpha");
    let f2 = make_studio_like_command_buffer(2, ColorRgba::rgb(245, 158, 11), "Beta");
    let _ = sender.encode_frame(&f1, 0);
    let p2 = sender.encode_frame(&f2, 16);

    // 1. Every single bit flip in the packet must fail validation cleanly.
    for byte_idx in 0..p2.len() {
        let mut mutated = p2.clone();
        mutated[byte_idx] ^= 0x01;
        assert!(
            DeltaPacketRef::parse(&mutated).is_err(),
            "bit flip at byte {byte_idx} was not detected!"
        );
    }

    // 2. Every prefix truncation must fail cleanly.
    for cut in 0..p2.len() {
        assert!(DeltaPacketRef::parse(&p2[..cut]).is_err());
    }

    // 3. Trailing byte appended must fail cleanly.
    let mut extended = p2.clone();
    extended.push(0x00);
    assert!(matches!(
        DeltaPacketRef::parse(&extended),
        Err(SyncDeltaError::LengthMismatch { .. })
    ));

    // 4. Forged valid-checksum OOM-bomb XOR+RLE payload declaring 1 GiB target_len.
    let mut bomb_payload = Vec::new();
    write_varint(1024 * 1024 * 1024, &mut bomb_payload);
    let bomb_pkt = encode_packet(PacketKind::XorRleDelta, 0, 2, 1, 0, &bomb_payload);
    let parsed = DeltaPacketRef::parse(&bomb_pkt).unwrap();
    assert!(matches!(
        decode_xor_rle_delta(&f1, parsed.payload()),
        Err(SyncDeltaError::FrameTooLarge(_))
    ));

    // 5. Deterministic pseudo-random fuzz corpus against decoders (zero panics!).
    let mut seed: u64 = 0xDEAD_BEEF_CAFE_BABEu64;
    for _ in 0..256 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let len = ((seed >> 48) as usize) % 96;
        let mut buf = vec![0u8; len];
        for b in &mut buf {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 56) as u8;
        }
        let _ = DeltaPacketRef::parse(&buf);
        let _ = decode_xor_rle_delta(&f1, &buf);
        let _ = decode_command_op_delta(&f1, &buf);
    }
}
