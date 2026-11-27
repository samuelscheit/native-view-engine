use crate::tests::{Align, Shadows, Text, Visual};
use crate::wire::{WireDecoder, WireEncoder, WireError, MAGIC};
use crate::{ChangeSet, NodeId, NodeStore, NveTable, Rgba, Transaction};

struct Pair {
    src: NodeStore,
    dst: NodeStore,
    enc: WireEncoder,
    dec: WireDecoder,
    bytes: Vec<u8>,
    dst_changes: ChangeSet,
}

impl Pair {
    fn new() -> Self {
        Self {
            src: NodeStore::new(),
            dst: NodeStore::replica(),
            enc: WireEncoder::default(),
            dec: WireDecoder::default(),
            bytes: Vec::new(),
            dst_changes: ChangeSet::default(),
        }
    }

    fn frame(&mut self, f: impl FnOnce(&mut Transaction<'_>)) -> ChangeSet {
        let mut tx = self.src.tx();
        f(&mut tx);
        let mut c = ChangeSet::default();
        tx.commit_into(&mut c);
        self.bytes.clear();
        self.enc.encode(&self.src, &c, &mut self.bytes);
        assert_eq!(self.bytes.len() % 4, 0);
        let mut tx = self.dst.tx();
        let frame = self.dec.apply(&mut tx, &self.bytes).expect("apply");
        tx.commit_into(&mut self.dst_changes);
        assert_eq!(frame, c.frame);
        self.assert_mirrored(&c);
        c
    }

    fn assert_mirrored(&self, c: &ChangeSet) {
        let live: Vec<NodeId> = self.src.live_nodes().collect();
        assert_eq!(live, self.dst.live_nodes().collect::<Vec<_>>());
        for &n in &live {
            assert_eq!(self.src.children(n), self.dst.children(n), "children of {n:?}");
            assert_eq!(self.src.node(n).unwrap().tables, self.dst.node(n).unwrap().tables);
            if let (Some(a), Some(b)) = (self.src.try_row::<Visual>(n), self.dst.try_row::<Visual>(n)) {
                assert_eq!(a.bg(), b.bg());
                assert_eq!(a.opacity().to_bits(), b.opacity().to_bits());
                assert_eq!(a.align(), b.align());
                assert_eq!(a.width(), b.width());
                assert_eq!(
                    self.src.resource::<Shadows>(a.shadows()),
                    self.dst.resource::<Shadows>(b.shadows())
                );
                assert_eq!(a.label(), b.label());
            }
            if let (Some(a), Some(b)) = (self.src.try_row::<Text>(n), self.dst.try_row::<Text>(n)) {
                assert_eq!(a.text(), b.text());
                assert_eq!(a.color(), b.color());
            }
        }
        // The replica's change set reports the same structural changes.
        let mut a = c.created.clone();
        let mut b = self.dst_changes.created.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        assert_eq!(c.commands.len(), self.dst_changes.commands.len());
        let s_stats = self.src.stats();
        let d_stats = self.dst.stats();
        assert_eq!(s_stats.live_nodes, d_stats.live_nodes);
        assert_eq!(s_stats.text_live, d_stats.text_live);
        assert_eq!(s_stats.children_live, d_stats.children_live);
        assert_eq!(s_stats.resources_live, d_stats.resources_live);
    }
}

#[test]
fn nve2_roundtrip_mirrors_store_across_frames() {
    let mut p = Pair::new();
    let (mut a, mut b, mut c) = (NodeId::NULL, NodeId::NULL, NodeId::NULL);
    p.frame(|tx| {
        a = tx.create();
        b = tx.create();
        c = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b]);
        tx.append_child(a, c);
        tx.edit::<Visual>(a)
            .bg(Rgba::rgb(9, 8, 7))
            .label("héllo")
            .align(Align::Center)
            .shadows(Some(Shadows(vec![1.0, 2.5])));
        tx.edit::<Visual>(b).shadows(Some(Shadows(vec![1.0, 2.5])));
        tx.edit::<Text>(c).text("leaf");
        tx.command::<Text>(c, 7, b"focus");
    });
    let ch = p.frame(|tx| {
        tx.edit::<Visual>(a).opacity(0.25).label("bye");
        tx.edit::<Text>(c).text("leaf2");
        tx.detach::<Text>(b);
    });
    assert_eq!(ch.tables().len(), 2);
    p.frame(|tx| {
        tx.set_children(b, &[c]);
        tx.remove(a);
        tx.edit::<Visual>(b).shadows_ephemeral(Shadows(vec![3.0]));
    });
    p.frame(|tx| {
        tx.edit::<Visual>(b).shadows_ephemeral(Shadows(vec![4.0]));
        let d = tx.create();
        tx.append_child(NodeId::ROOT, d);
        tx.edit::<Text>(d).text("recycled");
    });
    p.frame(|tx| tx.remove(b));
    assert_eq!(p.dst.stats().resources_live, 0);
    assert_eq!(p.dst.stats().text_live, "recycled".len());
}

#[test]
fn nve2_declares_each_table_once_per_session() {
    let mut p = Pair::new();
    let mut n = NodeId::NULL;
    p.frame(|tx| {
        n = tx.create();
        tx.edit::<Text>(n).text("a");
    });
    let first = p.bytes.len();
    p.frame(|tx| {
        tx.edit::<Text>(n).text("b");
    });
    assert!(p.bytes.len() < first - 16, "second frame must not redeclare the table");
}

#[test]
fn nve2_rejects_corrupt_frames_without_panicking() {
    let mut p = Pair::new();
    p.frame(|tx| {
        let n = tx.create();
        tx.edit::<Visual>(n).label("x").shadows(Some(Shadows(vec![1.0])));
    });
    let good = p.bytes.clone();
    let apply = |bytes: &[u8]| {
        let mut s = NodeStore::replica();
        let mut d = WireDecoder::default();
        let mut tx = s.tx();
        d.apply(&mut tx, bytes)
    };
    assert_eq!(apply(&good[..good.len() - 4]), Err(WireError::Truncated));
    assert_eq!(apply(&good[..good.len() - 1]), Err(WireError::Misaligned));
    let mut bad = good.clone();
    bad[0] ^= 1;
    assert_eq!(apply(&bad), Err(WireError::BadMagic(MAGIC ^ 1)));
    // Every single-word corruption either applies or errors; never panics.
    for i in 6..good.len() / 4 {
        for flip in [1u32, 0x8000_0000, 0xFFFF_FFFF] {
            let mut b = good.clone();
            let w = u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()) ^ flip;
            b[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
            let _ = apply(&b);
        }
    }
    // Patch for a table never declared in this session.
    let mut s = NodeStore::new();
    let mut tx = s.tx();
    let n = tx.create();
    tx.edit::<Text>(n).text("z");
    let mut c = ChangeSet::default();
    tx.commit_into(&mut c);
    let mut enc = WireEncoder::default();
    let mut bytes = Vec::new();
    enc.encode(&s, &c, &mut bytes);
    let mut dst = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut tx = dst.tx();
    dec.apply(&mut tx, &bytes).unwrap();
    tx.commit_into(&mut c);
    // A fresh decoder (receiver restart) must refuse the undeclared slot.
    let mut tx2 = s.tx();
    tx2.edit::<Text>(n).text("w");
    tx2.commit_into(&mut c);
    bytes.clear();
    enc.encode(&s, &c, &mut bytes);
    let mut fresh = WireDecoder::default();
    let mut tx = dst.tx();
    assert_eq!(fresh.apply(&mut tx, &bytes), Err(WireError::UndeclaredTable(Text::slot().0)));
}

#[test]
fn wire_rejects_dos_and_structural_corruption_vectors() {
    use crate::wire::Op;

    let build_frame = |flags: u16, ops: &[(u8, u8, u32, &[u8])]| -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC.to_le_bytes());
        out.extend_from_slice(&crate::wire::VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&1u64.to_le_bytes());
        out.extend_from_slice(&(ops.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for &(tag, slot, count, body) in ops {
            let inline = count.min(0xFFFF);
            let head = (tag as u32) | ((slot as u32) << 8) | (inline << 16);
            let extra = usize::from(count >= 0xFFFF) * 4;
            let padded_body = (body.len() + extra).div_ceil(4) * 4;
            out.extend_from_slice(&head.to_le_bytes());
            out.extend_from_slice(&((padded_body / 4) as u32).to_le_bytes());
            if count >= 0xFFFF {
                out.extend_from_slice(&count.to_le_bytes());
            }
            out.extend_from_slice(body);
            while out.len() % 4 != 0 {
                out.push(0);
            }
        }
        let words = (out.len() / 4) as u32;
        out[20..24].copy_from_slice(&words.to_le_bytes());
        out
    };

    let apply = |bytes: &[u8]| {
        let mut s = NodeStore::replica();
        let mut d = WireDecoder::default();
        let mut tx = s.tx();
        d.apply(&mut tx, bytes)
    };

    // 1. Non-zero flags rejected.
    let bad_flags = build_frame(1, &[]);
    assert_eq!(apply(&bad_flags), Err(WireError::UnsupportedFlags(1)));

    // 2. Trailing unconsumed bytes after op_count ops rejected.
    let mut trailing = build_frame(0, &[]);
    trailing.extend_from_slice(&0u32.to_le_bytes());
    let tw = (trailing.len() / 4) as u32;
    trailing[20..24].copy_from_slice(&tw.to_le_bytes());
    assert_eq!(apply(&trailing), Err(WireError::TrailingBytes));

    // 3. 40-byte DefineResources DoS packet claiming 1_000_000_000 resources rejected immediately.
    let dos_frame = build_frame(0, &[(Op::DefineResources as u8, 0, 1_000_000_000, &[])]);
    assert_eq!(apply(&dos_frame), Err(WireError::Truncated));

    let build_decl = |key: u64,
                      schema_hash: u64,
                      wire_slot: u8,
                      storage: u8,
                      flags: u8,
                      words: u8,
                      str_words: u32,
                      res_words: u32,
                      defaults: &[u32],
                      effects: &[u8],
                      name: &str|
     -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&(key as u32).to_le_bytes());
        b.extend_from_slice(&((key >> 32) as u32).to_le_bytes());
        b.extend_from_slice(&(schema_hash as u32).to_le_bytes());
        b.extend_from_slice(&((schema_hash >> 32) as u32).to_le_bytes());
        let packed = (wire_slot as u32)
            | ((storage as u32) << 8)
            | ((flags as u32) << 16)
            | ((words as u32) << 24);
        b.extend_from_slice(&packed.to_le_bytes());
        b.extend_from_slice(&str_words.to_le_bytes());
        b.extend_from_slice(&res_words.to_le_bytes());
        for &d in defaults {
            b.extend_from_slice(&d.to_le_bytes());
        }
        b.extend_from_slice(effects);
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b.extend_from_slice(&(name.len() as u32).to_le_bytes());
        b.extend_from_slice(name.as_bytes());
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b
    };

    // 4. DeclareTables with words > 32 (`words = 33`) rejected with exact `BadMask`.
    let bad_words_decl = build_decl(
        0x1234_5678_9ABC_DEF0,
        0xDEAD_BEEF,
        1,
        0,
        0,
        33,
        0,
        0,
        &[0u32; 4],
        &[1u8; 4],
        "bad.words",
    );
    let wrap_frame = build_frame(0, &[(Op::DeclareTables as u8, 0, 1, &bad_words_decl)]);
    assert_eq!(
        apply(&wrap_frame),
        Err(WireError::BadMask { mask: 0, words: 33 })
    );

    // 5. DeclareTables with out-of-bounds str_words bit (bit 1 on a 2-word table needs word 2)
    //    rejected with exact `SchemaMismatch`.
    let bad_str_key = 0xAAAA_BBBB_CCCC_DDDDu64;
    let bad_str_decl = build_decl(
        bad_str_key,
        0x1111_2222_3333_4444,
        2,
        0,
        0,
        2,
        1u32 << 1, // OOB: string at word 1 requires word 2
        0,
        &[0, 0],
        &[1, 1],
        "bad.str",
    );
    let oob_str_frame = build_frame(0, &[(Op::DeclareTables as u8, 0, 1, &bad_str_decl)]);
    assert_eq!(
        apply(&oob_str_frame),
        Err(WireError::SchemaMismatch { key: bad_str_key })
    );

    // 5b. Peer attempting to shadow static `Text` table with `str_words = 0` (even with matching
    //     key and schema hash!) is rejected with `SchemaMismatch`.
    let _ = Text::slot();
    let td = &<Text as NveTable>::DESC;
    let shadow_decl = build_decl(
        td.key.0,
        td.schema_hash,
        0,
        td.storage as u8,
        td.flags,
        td.words() as u8,
        0, // spoofed str_words = 0 instead of td.str_words!
        td.resource_words,
        td.defaults,
        td.word_effects,
        td.name,
    );
    let shadow_frame = build_frame(0, &[(Op::DeclareTables as u8, 0, 1, &shadow_decl)]);
    assert_eq!(
        apply(&shadow_frame),
        Err(WireError::SchemaMismatch { key: td.key.0 })
    );

    // 5c. Multi-op `+1024` amplification rejected: two CreateNodes ops in one tiny frame cannot
    //     push node index bound past a single per-frame cap.
    let n1 = NodeId::new(1000, 0).raw().to_le_bytes();
    let n2 = NodeId::new(1800, 0).raw().to_le_bytes();
    let amp_frame = build_frame(
        0,
        &[
            (Op::CreateNodes as u8, 0, 1, &n1),
            (Op::CreateNodes as u8, 0, 1, &n2),
        ],
    );
    assert_eq!(
        apply(&amp_frame),
        Err(WireError::NodeExists(NodeId::new(1800, 0).raw()))
    );

    // 6. SetChildren with duplicate or self-cycle rejected with WireError::InvalidChildren.
    let mut create_body = Vec::new();
    create_body.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
    let child_id = NodeId::new(2, 0);
    create_body.extend_from_slice(&child_id.raw().to_le_bytes());

    let mut dup_children = Vec::new();
    dup_children.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
    dup_children.extend_from_slice(&2u32.to_le_bytes()); // 2 kids
    dup_children.extend_from_slice(&child_id.raw().to_le_bytes());
    dup_children.extend_from_slice(&child_id.raw().to_le_bytes()); // duplicate!
    let dup_frame = build_frame(
        0,
        &[
            (Op::CreateNodes as u8, 0, 2, &create_body),
            (Op::SetChildren as u8, 0, 1, &dup_children),
        ],
    );
    assert_eq!(apply(&dup_frame), Err(WireError::InvalidChildren));

    let mut cycle_children = Vec::new();
    cycle_children.extend_from_slice(&child_id.raw().to_le_bytes());
    cycle_children.extend_from_slice(&1u32.to_le_bytes());
    cycle_children.extend_from_slice(&child_id.raw().to_le_bytes()); // self cycle!
    let cycle_frame = build_frame(
        0,
        &[
            (Op::CreateNodes as u8, 0, 2, &create_body),
            (Op::SetChildren as u8, 0, 1, &cycle_children),
        ],
    );
    assert_eq!(apply(&cycle_frame), Err(WireError::InvalidChildren));

    // 7. Same-frame command on a removed node must NOT poison the wire stream with DeadNode.
    let mut p = Pair::new();
    let mut target = NodeId::NULL;
    p.frame(|tx| {
        target = tx.create();
        tx.set_children(NodeId::ROOT, &[target]);
    });
    p.frame(|tx| {
        tx.command::<Visual>(target, 7, &[1, 2, 3]);
        tx.remove(target);
    });
}
