//! Algebraic property tests, HLC skew/monotonicity tests, RGA concurrency tests,
//! and adversarial `NveCrdtDelta` (`XXH64`) binary wire fuzz tests for `nve_interop::crdt`.

use nve_interop::crdt::{
    CrdtRecord, CrdtValue, CrdtWireError, GCounter, HLC_ENCODED_LEN, HlcClock, HlcTimestamp,
    LwwRegister, MAX_WIRE_BYTES, NVE_CRDT_DELTA_MAGIC, NVE_CRDT_DELTA_VERSION, NveCrdtDelta, OpId,
    OrSet, PeerId, PnCounter, RgaList, VersionVector, crdt_xxh64,
};

/// Deterministic xorshift64* PRNG (zero external crate dependencies).
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn range(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.range(i + 1);
            slice.swap(i, j);
        }
    }
}

fn frame_with_valid_checksum(mut body_without_cksum: Vec<u8>) -> Vec<u8> {
    let cksum = crdt_xxh64(&body_without_cksum);
    body_without_cksum.extend_from_slice(&cksum.to_le_bytes());
    body_without_cksum
}

#[test]
fn hlc_clock_monotonicity_skew_overflow_and_wire_roundtrip() {
    let mut c1 = HlcClock::new(PeerId(1));
    let mut c2 = HlcClock::new(PeerId(2));

    // Rapid same-ms ticks + backward clock jumps stay strictly monotonic.
    let mut prev = HlcTimestamp::ZERO;
    for &now_ms in &[1000u64, 1000, 1000, 500, 0, 999, 1000, 1001, 1001, 200] {
        let ts = c1.tick(now_ms);
        assert!(ts > prev, "tick must be strictly monotonic: {ts:?} <= {prev:?}");
        let enc = nve_codec::NveEncode::encode_to_vec(&ts).unwrap();
        assert_eq!(enc.len(), HLC_ENCODED_LEN);
        assert_eq!(<HlcTimestamp as nve_codec::NveDecode>::decode_exact(&enc).unwrap(), ts);
        assert_eq!(HlcTimestamp::decode_slice(&enc), Some(ts));
        prev = ts;
    }

    // Remote clock far ahead in the future pulls receiver forward strictly past remote.
    let future_remote = HlcTimestamp::new(9_000_000, 42, 99);
    let recv1 = c2.receive(1000, future_remote);
    assert!(recv1 > future_remote);
    assert_eq!(recv1.physical_ms, 9_000_000);
    assert_eq!(recv1.logical, 43);
    assert_eq!(recv1.peer_id, 2);

    // Receiving a remote at u16::MAX logical overflows cleanly into physical_ms + 1.
    let max_logical = HlcTimestamp::new(9_000_000, u16::MAX, 5);
    let recv2 = c2.receive(1000, max_logical);
    assert!(recv2 > max_logical);
    assert!(recv2 > recv1);
    assert_eq!(recv2.physical_ms, 9_000_001);
    assert_eq!(recv2.logical, 0);

    // Total ordering matches (physical_ms, logical, peer_id).
    let a = HlcTimestamp::new(100, 2, 1);
    let b = HlcTimestamp::new(100, 2, 2);
    let c = HlcTimestamp::new(100, 3, 0);
    let d = HlcTimestamp::new(101, 0, 0);
    assert!(a < b && b < c && c < d);
    assert_eq!(HlcTimestamp::decode_slice(&[0u8; 17]), None);
}

#[test]
fn lww_register_commutativity_associativity_idempotence() {
    let mut rng = Rng::new(0x1010_2020_3030_4040);
    let mut clocks = [
        HlcClock::new(PeerId(1)),
        HlcClock::new(PeerId(2)),
        HlcClock::new(PeerId(3)),
    ];

    for round in 0..64 {
        let mut regs = [
            LwwRegister::new("init".to_owned(), HlcTimestamp::ZERO),
            LwwRegister::new("init".to_owned(), HlcTimestamp::ZERO),
            LwwRegister::new("init".to_owned(), HlcTimestamp::ZERO),
        ];
        for step in 0..12 {
            let p = rng.range(3);
            let wall = 1000 + (rng.next_u64() % 20);
            let ts = clocks[p].tick(wall);
            regs[p].set(format!("r{round}-p{p}-s{step}"), ts);
        }

        // Commutativity: merge(a, b) == merge(b, a)
        let mut ab = regs[0].clone();
        ab.merge(&regs[1]);
        let mut ba = regs[1].clone();
        ba.merge(&regs[0]);
        assert_eq!(ab, ba);

        // Associativity: merge(merge(a, b), c) == merge(a, merge(b, c))
        let mut ab_c = ab.clone();
        ab_c.merge(&regs[2]);
        let mut bc = regs[1].clone();
        bc.merge(&regs[2]);
        let mut a_bc = regs[0].clone();
        a_bc.merge(&bc);
        assert_eq!(ab_c, a_bc);

        // Idempotence: merge(x, x) == x and returns false
        let snapshot = ab_c.clone();
        assert!(!ab_c.merge(&snapshot));
        assert_eq!(ab_c, snapshot);
    }
}

#[test]
fn counters_commutativity_associativity_idempotence() {
    let mut rng = Rng::new(0xDEAD_BEEF_CAFE_0001);

    for _ in 0..64 {
        let mut g = [GCounter::new(), GCounter::new(), GCounter::new()];
        let mut pn = [PnCounter::new(), PnCounter::new(), PnCounter::new()];

        for _ in 0..20 {
            let idx = rng.range(3);
            let peer = PeerId((idx + 1) as u64);
            let delta = (rng.next_u64() % 7) + 1;
            g[idx].increment(peer, delta);
            if rng.range(2) == 0 {
                pn[idx].increment(peer, delta);
            } else {
                pn[idx].decrement(peer, delta);
            }
        }

        // GCounter laws
        let mut g_ab = g[0].clone();
        g_ab.merge(&g[1]);
        let mut g_ba = g[1].clone();
        g_ba.merge(&g[0]);
        assert_eq!(g_ab, g_ba);

        let mut g_ab_c = g_ab.clone();
        g_ab_c.merge(&g[2]);
        let mut g_bc = g[1].clone();
        g_bc.merge(&g[2]);
        let mut g_a_bc = g[0].clone();
        g_a_bc.merge(&g_bc);
        assert_eq!(g_ab_c, g_a_bc);
        assert!(!g_ab_c.merge(&g_a_bc));
        assert_eq!(
            g_ab_c.value(),
            g[0].value() + g[1].value() + g[2].value()
        );

        // PnCounter laws
        let mut pn_ab = pn[0].clone();
        pn_ab.merge(&pn[1]);
        let mut pn_ba = pn[1].clone();
        pn_ba.merge(&pn[0]);
        assert_eq!(pn_ab, pn_ba);

        let mut pn_ab_c = pn_ab.clone();
        pn_ab_c.merge(&pn[2]);
        let mut pn_bc = pn[1].clone();
        pn_bc.merge(&pn[2]);
        let mut pn_a_bc = pn[0].clone();
        pn_a_bc.merge(&pn_bc);
        assert_eq!(pn_ab_c, pn_a_bc);
        assert!(!pn_ab_c.merge(&pn_a_bc));
        assert_eq!(
            pn_ab_c.value(),
            pn[0].value() + pn[1].value() + pn[2].value()
        );
    }
}

#[test]
fn orset_add_wins_laws_and_delta_permutation_convergence() {
    // Explicit add-wins test: A adds k (op1), syncs to B; then A removes k while B concurrently adds k (op2).
    let mut a = OrSet::new();
    let mut b = OrSet::new();
    let op1 = OpId::new(PeerId(1), 1);
    let op2 = OpId::new(PeerId(2), 1);
    a.add("task-1", op1);
    b.merge(&a);
    assert!(b.contains(&"task-1"));

    a.remove_with_op(&"task-1", OpId::new(PeerId(1), 2));
    b.add("task-1", op2);
    assert!(!a.contains(&"task-1"));
    assert!(b.contains(&"task-1"));

    let mut ab = a.clone();
    ab.merge(&b);
    let mut ba = b.clone();
    ba.merge(&a);
    assert_eq!(ab, ba);
    assert!(ab.contains(&"task-1"), "concurrent add must win over remove");

    // Randomized 4-peer property test: commutativity, associativity, idempotence, and delta permutations.
    let mut rng = Rng::new(0x5555_AAAA_1234_9876);
    for _ in 0..32 {
        let mut replicas = vec![OrSet::<u64>::new(); 4];
        let mut vvs = vec![VersionVector::new(); 4];
        let mut seqs = [0u64; 4];

        for _ in 0..24 {
            let p = rng.range(4);
            let key = rng.next_u64() % 8;
            seqs[p] += 1;
            let op = OpId::new(PeerId((p + 1) as u64), seqs[p]);
            vvs[p].observe(op);
            if rng.range(3) != 0 {
                replicas[p].add(key, op);
            } else {
                replicas[p].remove_with_op(&key, op);
            }
            // Occasional mid-flight sync so removes observe remote tags.
            if rng.range(4) == 0 {
                let target = rng.range(4);
                let delta = replicas[p].delta_since(&vvs[target]);
                replicas[target].merge_delta(&delta);
                let src_vv = vvs[p].clone();
                vvs[target].merge(&src_vv);
            }
        }

        // Verify all 6 permutations of merging replicas[0..3] match full merge.
        let perms = [
            [0, 1, 2, 3],
            [3, 2, 1, 0],
            [1, 3, 0, 2],
            [2, 0, 3, 1],
        ];
        let mut expected: Option<OrSet<u64>> = None;
        for perm in perms {
            let mut acc = OrSet::new();
            for &i in &perm {
                acc.merge(&replicas[i]);
            }
            // Idempotence check on full state
            let before = acc.clone();
            assert!(!acc.merge(&before));
            assert_eq!(acc, before);
            if let Some(ref exp) = expected {
                assert_eq!(&acc, exp);
            } else {
                expected = Some(acc);
            }
        }

        // Verify delta-based convergence from empty VV matches full merge.
        let mut via_deltas = OrSet::new();
        let mut delta_vv = VersionVector::new();
        let mut order = [0usize, 1, 2, 3];
        rng.shuffle(&mut order);
        for &i in &order {
            let d = replicas[i].delta_since(&delta_vv);
            via_deltas.merge_delta(&d);
            delta_vv.merge(&vvs[i]);
        }
        assert_eq!(via_deltas, expected.unwrap());
    }
}

#[test]
fn rga_list_concurrent_same_anchor_out_of_order_cycles_and_laws() {
    // Concurrent same-anchor inserts: descending OpId tie-break.
    let mut a = RgaList::new();
    let mut b = RgaList::new();
    let root = OpId::new(PeerId(1), 1);
    a.insert_after(None, root, "root");
    b.merge(&a);

    let p1_ins = OpId::new(PeerId(1), 2);
    let p2_ins = OpId::new(PeerId(2), 2);
    let p3_ins = OpId::new(PeerId(3), 2);
    a.insert_after(Some(root), p1_ins, "from-p1");
    b.insert_after(Some(root), p2_ins, "from-p2");

    let mut c = a.clone();
    c.insert_after(Some(root), p3_ins, "from-p3");

    let mut m1 = a.clone();
    m1.merge(&b);
    m1.merge(&c);

    let mut m2 = c.clone();
    m2.merge(&b);
    m2.merge(&a);

    assert_eq!(m1, m2);
    // p3 > p2 > p1 (same seq=2, higher peer wins descending order).
    assert_eq!(m1.to_vec(), vec!["root", "from-p3", "from-p2", "from-p1"]);

    // Out-of-order arrival: child arrives before parent, delete arrives before insert.
    let mut ooo = RgaList::new();
    let n1 = OpId::new(PeerId(1), 1);
    let n2 = OpId::new(PeerId(1), 2);
    let n3 = OpId::new(PeerId(2), 3);
    ooo.delete(n2); // delete before insert!
    ooo.insert_after(Some(n2), n3, "third"); // child before parent!
    ooo.insert_after(Some(n1), n2, "second-deleted");
    ooo.insert_after(None, n1, "first");
    assert_eq!(ooo.to_vec(), vec!["first", "third"]);

    // Adversarial cycle safety: must never hang or stack-overflow.
    let mut cyc = RgaList::new();
    let c1 = OpId::new(PeerId(1), 1);
    let c2 = OpId::new(PeerId(2), 2);
    cyc.insert_after(Some(c2), c1, "c1");
    cyc.insert_after(Some(c1), c2, "c2");
    cyc.insert_after(Some(c1), c1, "self-ignored");
    let vec_cyc = cyc.to_vec();
    assert_eq!(vec_cyc.len(), 2);

    // Randomized multi-peer RGA convergence across delivery permutations.
    let mut rng = Rng::new(0x9876_5432_1111_2222);
    for round in 0..24 {
        let mut peers = vec![RgaList::<u32>::new(); 4];
        let mut vvs = vec![VersionVector::new(); 4];
        let mut seqs = [0u64; 4];

        // Shared seed elements
        let s1 = OpId::new(PeerId::SEED, 1);
        let s2 = OpId::new(PeerId::SEED, 2);
        for (p, vv) in peers.iter_mut().zip(vvs.iter_mut()) {
            p.insert_after(None, s1, 100);
            p.insert_after(Some(s1), s2, 200);
            vv.observe(s2);
        }

        for step in 0..20 {
            let p = rng.range(4);
            seqs[p] += 1;
            let op = OpId::new(PeerId((p + 1) as u64), seqs[p]);
            vvs[p].observe(op);
            let visible: Vec<OpId> = peers[p].visible_nodes().iter().map(|n| n.id).collect();
            if visible.is_empty() || rng.range(4) != 0 {
                let anchor = if visible.is_empty() || rng.range(4) == 0 {
                    None
                } else {
                    Some(visible[rng.range(visible.len())])
                };
                peers[p].insert_after(anchor, op, (round * 100 + step) as u32);
            } else {
                let victim = visible[rng.range(visible.len())];
                peers[p].delete_with_op(victim, op);
            }

            // Occasional delta gossip
            if rng.range(3) == 0 {
                let dst = rng.range(4);
                let delta = peers[p].delta_since(&vvs[dst]);
                peers[dst].merge_delta(&delta);
                let src_vv = vvs[p].clone();
                vvs[dst].merge(&src_vv);
            }
        }

        let mut canonical = RgaList::new();
        for p in &peers {
            canonical.merge(p);
        }
        let expected_vec = canonical.to_vec();

        // Check shuffled full merges & delta merges converge identically.
        for _ in 0..6 {
            let mut order = [0usize, 1, 2, 3];
            rng.shuffle(&mut order);
            let mut merged = RgaList::new();
            for &i in &order {
                merged.merge(&peers[i]);
            }
            assert_eq!(merged, canonical);
            assert_eq!(merged.to_vec(), expected_vec);
            assert!(!merged.merge(&canonical));
        }
    }
}

#[test]
fn wire_crdt_delta_roundtrip_and_adversarial_fuzzing() {
    let mut vv = VersionVector::new();
    vv.set_max(PeerId(1), 17);
    vv.set_max(PeerId(2), 42);
    let packet = NveCrdtDelta::new(
        PeerId(1),
        vv,
        vec![
            CrdtRecord::LwwSet {
                namespace: 2,
                entity_id: 9,
                field_id: 1,
                op_id: OpId::new(PeerId(1), 10),
                timestamp: HlcTimestamp::new(1_700_000_000_000, 3, 1),
                value: CrdtValue::Utf8("Binary CRDT 🚀".into()),
            },
            CrdtRecord::GCounterSet {
                namespace: 3,
                entity_id: 7,
                peer: PeerId(1),
                op_id: OpId::new(PeerId(1), 11),
                count: 55,
            },
            CrdtRecord::PnCounterSet {
                namespace: 4,
                entity_id: 8,
                peer: PeerId(2),
                op_id: OpId::new(PeerId(2), 12),
                pos: 10,
                neg: 3,
            },
            CrdtRecord::OrSetAddU64 {
                namespace: 1,
                key: 99,
                op_id: OpId::new(PeerId(1), 13),
            },
            CrdtRecord::OrSetRemove {
                namespace: 1,
                removed_tag: OpId::new(PeerId(2), 4),
                remove_op: OpId::new(PeerId(1), 14),
            },
            CrdtRecord::RgaInsert {
                namespace: 9,
                list_id: 1,
                id: OpId::new(PeerId(1), 15),
                after_id: Some(OpId::new(PeerId::SEED, 1)),
                value: CrdtValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]),
            },
            CrdtRecord::RgaDelete {
                namespace: 9,
                list_id: 1,
                target_id: OpId::new(PeerId(2), 5),
                remove_op: OpId::new(PeerId(1), 16),
            },
            CrdtRecord::OrSetAddString {
                namespace: 12,
                key: "tag:crdt".into(),
                op_id: OpId::new(PeerId(1), 17),
            },
        ],
    );

    let encoded = packet.encode().expect("encode valid packet");
    assert_eq!(&encoded[0..8], &NVE_CRDT_DELTA_MAGIC);
    assert_eq!(
        u16::from_le_bytes([encoded[8], encoded[9]]),
        NVE_CRDT_DELTA_VERSION
    );
    let decoded = NveCrdtDelta::decode(&encoded).expect("decode valid packet");
    assert_eq!(decoded, packet);

    // 1. Every truncation length must return Err cleanly (zero panics).
    for len in 0..encoded.len() {
        assert!(
            NveCrdtDelta::decode(&encoded[..len]).is_err(),
            "truncated len {len} must fail"
        );
    }

    // 2. Every single bit flip across the entire wire frame must fail cleanly.
    for byte_idx in 0..encoded.len() {
        for bit in 0..8u8 {
            let mut mutated = encoded.clone();
            mutated[byte_idx] ^= 1 << bit;
            assert!(
                NveCrdtDelta::decode(&mutated).is_err(),
                "bit flip at byte {byte_idx} bit {bit} must be rejected"
            );
        }
    }

    // 3. Bad magic & unsupported version.
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert!(matches!(
        NveCrdtDelta::decode(&bad_magic),
        Err(CrdtWireError::InvalidMagic(_))
    ));

    let mut bad_ver = encoded.clone();
    bad_ver[8..10].copy_from_slice(&99u16.to_le_bytes());
    assert!(matches!(
        NveCrdtDelta::decode(&bad_ver),
        Err(CrdtWireError::UnsupportedVersion(99))
    ));

    // 4. Valid-checksum adversarial payloads: bogus op tag, bogus lengths, invalid UTF-8, trailing bytes.
    // 4a. Huge vv_len with valid checksum.
    let mut huge_vv = Vec::new();
    huge_vv.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
    huge_vv.extend_from_slice(&NVE_CRDT_DELTA_VERSION.to_le_bytes());
    huge_vv.extend_from_slice(&1u64.to_le_bytes()); // sender
    huge_vv.extend_from_slice(&u32::MAX.to_le_bytes()); // vv_len
    huge_vv.extend_from_slice(&0u32.to_le_bytes()); // record_count
    assert!(matches!(
        NveCrdtDelta::decode(&frame_with_valid_checksum(huge_vv)),
        Err(CrdtWireError::LimitExceeded { .. })
    ));

    // 4b. Huge record_count with valid checksum.
    let mut huge_recs = Vec::new();
    huge_recs.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
    huge_recs.extend_from_slice(&NVE_CRDT_DELTA_VERSION.to_le_bytes());
    huge_recs.extend_from_slice(&1u64.to_le_bytes());
    huge_recs.extend_from_slice(&0u32.to_le_bytes());
    huge_recs.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        NveCrdtDelta::decode(&frame_with_valid_checksum(huge_recs)),
        Err(CrdtWireError::LimitExceeded { .. })
    ));

    // 4c. Huge record_len with valid checksum.
    let mut huge_rec_len = Vec::new();
    huge_rec_len.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
    huge_rec_len.extend_from_slice(&NVE_CRDT_DELTA_VERSION.to_le_bytes());
    huge_rec_len.extend_from_slice(&1u64.to_le_bytes());
    huge_rec_len.extend_from_slice(&0u32.to_le_bytes());
    huge_rec_len.extend_from_slice(&1u32.to_le_bytes()); // 1 record
    huge_rec_len.extend_from_slice(&0x0FFF_FFFFu32.to_le_bytes()); // huge record_len
    huge_rec_len.push(1);
    assert!(matches!(
        NveCrdtDelta::decode(&frame_with_valid_checksum(huge_rec_len)),
        Err(CrdtWireError::LimitExceeded { .. })
    ));

    // 4d. Bogus op tag (0xEE) with valid checksum.
    let mut bogus_tag = Vec::new();
    bogus_tag.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
    bogus_tag.extend_from_slice(&NVE_CRDT_DELTA_VERSION.to_le_bytes());
    bogus_tag.extend_from_slice(&1u64.to_le_bytes());
    bogus_tag.extend_from_slice(&0u32.to_le_bytes());
    bogus_tag.extend_from_slice(&1u32.to_le_bytes());
    bogus_tag.extend_from_slice(&1u32.to_le_bytes()); // record_len = 1
    bogus_tag.push(0xEE);
    assert!(matches!(
        NveCrdtDelta::decode(&frame_with_valid_checksum(bogus_tag)),
        Err(CrdtWireError::UnknownOpTag(0xEE))
    ));

    // 4e. Invalid UTF-8 string inside OrSetAddString with valid checksum.
    let mut bad_utf8 = Vec::new();
    bad_utf8.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
    bad_utf8.extend_from_slice(&NVE_CRDT_DELTA_VERSION.to_le_bytes());
    bad_utf8.extend_from_slice(&1u64.to_le_bytes());
    bad_utf8.extend_from_slice(&0u32.to_le_bytes());
    bad_utf8.extend_from_slice(&1u32.to_le_bytes());
    // record: tag(1) + ns(2) + str_len(4) + bad_bytes(2) + op_id(16) = 25
    bad_utf8.extend_from_slice(&25u32.to_le_bytes());
    bad_utf8.push(8); // TAG_ORSET_ADD_STR
    bad_utf8.extend_from_slice(&1u16.to_le_bytes());
    bad_utf8.extend_from_slice(&2u32.to_le_bytes());
    bad_utf8.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
    bad_utf8.extend_from_slice(&nve_codec::NveEncode::encode_to_vec(&OpId::new(PeerId(1), 1)).unwrap());
    assert!(matches!(
        NveCrdtDelta::decode(&frame_with_valid_checksum(bad_utf8)),
        Err(CrdtWireError::InvalidUtf8)
    ));

    // 4f. Trailing garbage byte after records with valid checksum.
    let mut trailing = encoded[..encoded.len() - 8].to_vec();
    trailing.push(0xAA);
    assert!(matches!(
        NveCrdtDelta::decode(&frame_with_valid_checksum(trailing)),
        Err(CrdtWireError::TrailingBytes(1))
    ));

    // 4g. Oversized frame exceeds MAX_WIRE_BYTES cleanly.
    let oversized = vec![0u8; MAX_WIRE_BYTES + 1];
    assert!(matches!(
        NveCrdtDelta::decode(&oversized),
        Err(CrdtWireError::LimitExceeded { kind: "wire_bytes", .. })
    ));

    // 5. Randomized fuzz loop with valid checksums: never panics.
    let mut rng = Rng::new(0xF002_BAAD_C0DE_7777);
    for _ in 0..512 {
        let body_len = rng.range(128);
        let mut raw = Vec::with_capacity(10 + body_len + 8);
        raw.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
        raw.extend_from_slice(&NVE_CRDT_DELTA_VERSION.to_le_bytes());
        for _ in 0..body_len {
            raw.push((rng.next_u64() & 0xFF) as u8);
        }
        let frame = frame_with_valid_checksum(raw);
        let _ = NveCrdtDelta::decode(&frame);
    }
}
