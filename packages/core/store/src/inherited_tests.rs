//! Thorough unit, effect-tracking, and `NVE2` wire tests for `#[nve_table(inherited)]` +
//! [`ResolvedRow`].

use crate::wire::{WireDecoder, WireEncoder};
use crate::{
    effect, ChangeSet, FlatResource, InheritedTable, Length, NodeId, NodeStore, NveTable, Res,
    ResolvedRow, Rgba, Str, WordEnum,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, WordEnum)]
#[repr(u8)]
enum TestAlign {
    #[default]
    Start = 0,
    Center = 1,
    End = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestBlob(u32);

impl FlatResource for TestBlob {
    const KIND: nve_hash::Key64 = nve_hash::Key64::of("test.inherited.blob");
    type View<'a> = u32;
    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        out.push(self.0);
        true
    }
    fn validate(words: &[u32]) -> bool {
        words.len() == 1
    }
    fn view(words: &[u32]) -> u32 {
        words[0]
    }
}

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.inherited.full", storage = "sparse", flags = "publish", inherited)]
struct TestInherited {
    #[effect(paint)]
    #[default(Rgba::BLACK)]
    color: Rgba,
    #[effect(measure)]
    #[default(16.0)]
    font_size: f32,
    #[effect(measure)]
    #[default(Length::AUTO)]
    line_height: Length,
    #[effect(measure)]
    font_family: Str,
    #[effect(measure)]
    max_lines: Option<u32>,
    #[effect(paint)]
    #[default(TestAlign::Start)]
    align: TestAlign,
    #[effect(paint)]
    #[default(false)]
    underline: bool,
    #[effect(measure)]
    #[default(400u16)]
    weight: u16,
    #[effect(paint)]
    #[default(0u64)]
    tag64: u64,
    #[effect(paint)]
    blob: Res<TestBlob>,
}

#[test]
fn inherited_table_word0_explicit_layout_and_schema_hash() {
    const { assert!(TestInherited::DESC.inherited) };
    assert_eq!(TestInherited::DESC.word_effects[0], 0);
    assert_eq!(TestInherited::DESC.defaults[0], 0);
    // Every declared field must live at word >= 1 (word 0 is hidden `__explicit`).
    for f in TestInherited::DESC.fields {
        assert!(f.word >= 1, "field `{}` unexpectedly placed in word 0", f.name);
    }
    assert_eq!(
        <TestInherited as InheritedTable>::default_words().as_ref(),
        TestInherited::DESC.defaults
    );
}

#[test]
fn multi_level_inheritance_explicit_default_override_and_clear() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();

    let (grandparent, parent, child, unstyled_leaf) = {
        let mut tx = store.tx();
        let gp = tx.create();
        let p = tx.create();
        let c = tx.create();
        let leaf = tx.create();
        tx.set_children(NodeId::ROOT, &[gp]);
        tx.set_children(gp, &[p]);
        tx.set_children(p, &[c]);
        tx.set_children(c, &[leaf]);

        tx.edit::<TestInherited>(gp)
            .color(Rgba::WHITE)
            .font_size(24.0)
            .line_height(Length::points(30.0))
            .font_family("Inter")
            .max_lines(Some(3))
            .align(TestAlign::Center)
            .underline(true)
            .weight(700)
            .tag64(0x1122_3344_5566_7788)
            .blob(Some(TestBlob(99)));

        // Parent explicitly overrides only `font_size` via `tx.set` and `align` via `NodeMut`.
        tx.set(p, TestInherited::FONT_SIZE, 20.0);
        tx.node_mut(p).unwrap().set(TestInherited::ALIGN, TestAlign::End);

        // Child explicitly resets `color`, `underline`, `max_lines`, `font_family`, and `weight`
        // to the table's DEFAULT values — must override grandparent instead of inheriting!
        tx.edit::<TestInherited>(c)
            .color(Rgba::BLACK)
            .underline(false)
            .max_lines(None)
            .font_family("")
            .weight(400)
            .tag64(0)
            .blob(None);

        tx.commit_into(&mut changes);
        (gp, p, c, leaf)
    };

    {
        let root_r = ResolvedRow::<TestInherited>::root(&store);
        let gp_r = ResolvedRow::<TestInherited>::resolve(&store, grandparent, &root_r);
        let p_r = ResolvedRow::<TestInherited>::resolve(&store, parent, &gp_r);
        let c_r = ResolvedRow::<TestInherited>::resolve(&store, child, &p_r);
        let leaf_r = ResolvedRow::<TestInherited>::resolve(&store, unstyled_leaf, &c_r);

        // Check explicit masks on stored rows.
        let p_own = store.try_row::<TestInherited>(parent).unwrap();
        assert!(!p_own.is_color_explicit());
        assert!(p_own.is_font_size_explicit());
        assert!(p_own.is_align_explicit());
        assert!(!p_own.is_underline_explicit());

        // Parent inherits everything except font_size and align.
        let pt = p_r.get();
        assert_eq!(pt.color(), Rgba::WHITE);
        assert_eq!(pt.font_size(), 20.0);
        assert_eq!(pt.line_height(), Length::points(30.0));
        assert_eq!(pt.font_family(), "Inter");
        assert_eq!(pt.max_lines(), Some(3));
        assert_eq!(pt.align(), TestAlign::End);
        assert!(pt.underline());
        assert_eq!(pt.weight(), 700);
        assert_eq!(pt.tag64(), 0x1122_3344_5566_7788);
        assert_eq!(store.resource::<TestBlob>(pt.blob()), Some(99));

        // Child's explicit table defaults override grandparent/parent, while unset fields inherit.
        let ct = c_r.get();
        assert_eq!(ct.color(), Rgba::BLACK);
        assert_eq!(ct.font_size(), 20.0);
        assert_eq!(ct.line_height(), Length::points(30.0));
        assert_eq!(ct.font_family(), "");
        assert_eq!(ct.max_lines(), None);
        assert_eq!(ct.align(), TestAlign::End);
        assert!(!ct.underline());
        assert_eq!(ct.weight(), 400);
        assert_eq!(ct.tag64(), 0);
        assert!(ct.blob().is_none());

        // Unstyled leaf inherits resolved child state verbatim.
        let lt = leaf_r.get();
        assert_eq!(lt.color(), Rgba::BLACK);
        assert_eq!(lt.font_size(), 20.0);
        assert_eq!(lt.align(), TestAlign::End);
        assert_eq!(lt.weight(), 400);
    }

    // Clear child's explicit `color` (which currently holds the table default `BLACK`!):
    // Must dirty word 0 AND emit strictly `effect::PAINT` (NOT `effect::MEASURE`)!
    {
        let mut tx = store.tx();
        tx.edit::<TestInherited>(child).clear_color();
        tx.commit_into(&mut changes);
    }
    let idx = changes
        .touched
        .iter()
        .position(|&id| id == child)
        .expect("clearing explicit default must touch child");
    assert_eq!(
        changes.effects[idx],
        effect::PAINT,
        "clearing paint-only color must emit strictly effect::PAINT"
    );
    {
        let root_r = ResolvedRow::<TestInherited>::root(&store);
        let gp_r = ResolvedRow::<TestInherited>::resolve(&store, grandparent, &root_r);
        let p_r = ResolvedRow::<TestInherited>::resolve(&store, parent, &gp_r);
        let c_after = ResolvedRow::<TestInherited>::resolve(&store, child, &p_r);
        assert_eq!(
            c_after.get().color(),
            Rgba::WHITE,
            "cleared color must revert to parent WHITE"
        );
        assert!(!store.try_row::<TestInherited>(child).unwrap().is_color_explicit());
    }

    // Re-setting child's `color` back to default `Rgba::BLACK` must ALSO dirty word 0 and
    // emit strictly `effect::PAINT` even though the color word already held `BLACK`!
    {
        let mut tx = store.tx();
        tx.edit::<TestInherited>(child).color(Rgba::BLACK);
        tx.commit_into(&mut changes);
    }
    let idx = changes
        .touched
        .iter()
        .position(|&id| id == child)
        .expect("setting explicit table default must touch child");
    assert_eq!(changes.effects[idx], effect::PAINT);
    {
        let root_r = ResolvedRow::<TestInherited>::root(&store);
        let gp_r = ResolvedRow::<TestInherited>::resolve(&store, grandparent, &root_r);
        let p_r = ResolvedRow::<TestInherited>::resolve(&store, parent, &gp_r);
        let c_reoverride = ResolvedRow::<TestInherited>::resolve(&store, child, &p_r);
        assert_eq!(c_reoverride.get().color(), Rgba::BLACK);
    }

    // Clearing already-cleared field is a complete no-op (zero dirty, zero touched).
    {
        let mut tx = store.tx();
        tx.edit::<TestInherited>(parent).clear_color();
        tx.commit_into(&mut changes);
    }
    assert!(changes.touched.is_empty());
    assert!(changes.table(TestInherited::slot()).is_none());

    // Test `set_str` and `set_resource` setting explicit bits on inherited table.
    {
        let mut tx = store.tx();
        tx.set_str(parent, TestInherited::FONT_FAMILY, "JetBrains Mono");
        tx.set_resource(parent, TestInherited::BLOB, Some(TestBlob(7)));
        tx.commit_into(&mut changes);
    }
    {
        let p_own2 = store.try_row::<TestInherited>(parent).unwrap();
        assert!(p_own2.is_font_family_explicit());
        assert!(p_own2.is_blob_explicit());
        let root_r = ResolvedRow::<TestInherited>::root(&store);
        let gp_r = ResolvedRow::<TestInherited>::resolve(&store, grandparent, &root_r);
        let p_r2 = ResolvedRow::<TestInherited>::resolve(&store, parent, &gp_r);
        assert_eq!(p_r2.get().font_family(), "JetBrains Mono");
        assert_eq!(store.resource::<TestBlob>(p_r2.get().blob()), Some(7));
    }

    // Clear string + resource reverts them back to grandparent.
    {
        let mut tx = store.tx();
        tx.edit::<TestInherited>(parent).clear_font_family().clear_blob();
        tx.commit_into(&mut changes);
    }
    {
        let root_r = ResolvedRow::<TestInherited>::root(&store);
        let gp_r = ResolvedRow::<TestInherited>::resolve(&store, grandparent, &root_r);
        let p_r3 = ResolvedRow::<TestInherited>::resolve(&store, parent, &gp_r);
        assert_eq!(p_r3.get().font_family(), "Inter");
        assert_eq!(store.resource::<TestBlob>(p_r3.get().blob()), Some(99));
    }
}

#[test]
fn inherited_table_nve2_wire_roundtrip_propagates_word0_effects() {
    let mut src = NodeStore::new();
    let mut dst = NodeStore::replica();
    let mut changes = ChangeSet::default();
    let mut dst_changes = ChangeSet::default();
    let mut enc = WireEncoder::default();
    let mut dec = WireDecoder::default();
    let mut wire = Vec::new();

    let (parent, child) = {
        let mut tx = src.tx();
        let p = tx.create();
        let c = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[c]);
        tx.edit::<TestInherited>(p).color(Rgba::WHITE).font_size(22.0);
        tx.edit::<TestInherited>(c).font_size(14.0);
        tx.commit_into(&mut changes);
        (p, c)
    };
    enc.encode(&src, &changes, &mut wire);
    {
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).expect("frame 1 decode");
        rtx.commit_into(&mut dst_changes);
    }

    // Frame 2: child explicitly writes table-default `Rgba::BLACK` — ONLY word 0 changes!
    {
        let mut tx = src.tx();
        tx.edit::<TestInherited>(child).color(Rgba::BLACK);
        tx.commit_into(&mut changes);
    }
    wire.clear();
    enc.encode(&src, &changes, &mut wire);
    {
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).expect("frame 2 decode");
        rtx.commit_into(&mut dst_changes);
    }

    let idx = dst_changes
        .touched
        .iter()
        .position(|&id| id == child)
        .expect("replica must mark child touched when word-0 explicit bit flips");
    assert_eq!(
        dst_changes.effects[idx],
        effect::PAINT,
        "replica word-0 paint-field flip must emit strictly effect::PAINT"
    );

    let root_r = ResolvedRow::<TestInherited>::root(&dst);
    let p_r = ResolvedRow::<TestInherited>::resolve(&dst, parent, &root_r);
    let c_r = ResolvedRow::<TestInherited>::resolve(&dst, child, &p_r);
    assert_eq!(c_r.get().color(), Rgba::BLACK);
    assert_eq!(c_r.get().font_size(), 14.0);
}
