//! `dirty_indices` covers inherited typography: a `Typography` change, attach, or detach on an
//! ancestor dirties exactly the text / text-input descendants that inherit it (paint-only
//! changes move no rect), so retained hosts restyle `dirty_indices` without whole-tree walks.

use nve_components::tables::{Layout, TextContent, TextInput, Typography};
use nve_runtime::{RuntimeBuilder, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, Transaction};

struct Session {
    store: NodeStore,
    encoder: WireEncoder,
}

impl Session {
    fn frame(&mut self, edit: impl FnOnce(&mut Transaction<'_>)) -> Vec<u8> {
        let mut changes = ChangeSet::default();
        let mut tx = self.store.tx();
        edit(&mut tx);
        tx.commit_into(&mut changes);
        let mut out = Vec::new();
        self.encoder.encode(&self.store, &changes, &mut out);
        out
    }
}

/// `outer(Typography) > [inner(Typography) > [deep_text], mid_text, input, plain] + other_text`.
struct Tree {
    outer: NodeId,
    inner: NodeId,
    deep_text: NodeId,
    mid_text: NodeId,
    input: NodeId,
    plain: NodeId,
    other_text: NodeId,
}

fn fixed(tx: &mut Transaction<'_>, id: NodeId) {
    tx.edit::<Layout>(id).width(Length::points(50.0)).height(Length::points(20.0));
}

fn mount() -> (SurfaceRuntime, Session, Tree) {
    let mut s = Session { store: NodeStore::new(), encoder: WireEncoder::default() };
    let mut tree = None;
    let keyframe = s.frame(|tx| {
        let t = Tree {
            outer: tx.create(),
            inner: tx.create(),
            deep_text: tx.create(),
            mid_text: tx.create(),
            input: tx.create(),
            plain: tx.create(),
            other_text: tx.create(),
        };
        for id in [t.deep_text, t.mid_text, t.input, t.plain, t.other_text] {
            fixed(tx, id);
        }
        tx.edit::<Typography>(t.outer).color(Rgba::rgb(1, 2, 3));
        tx.edit::<Typography>(t.inner).font_weight(700);
        tx.edit::<TextContent>(t.deep_text).text("deep");
        tx.edit::<TextContent>(t.mid_text).text("mid");
        tx.edit::<TextInput>(t.input).value("draft");
        tx.edit::<TextContent>(t.other_text).text("other");
        tx.set_children(t.inner, &[t.deep_text]);
        tx.set_children(t.outer, &[t.inner, t.mid_text, t.input, t.plain]);
        tx.set_children(NodeId::ROOT, &[t.outer, t.other_text]);
        tree = Some(t);
    });
    let mut rt = RuntimeBuilder::new().build();
    rt.submit_nve2(&keyframe).expect("keyframe");
    (rt, s, tree.expect("tree"))
}

fn dirty(rt: &SurfaceRuntime) -> Vec<u32> {
    let guard = rt.acquire_scene().expect("acquire");
    let v = guard.view();
    if v.dirty_count == 0 {
        return Vec::new();
    }
    // SAFETY: the guard keeps the published slices alive.
    let mut out = unsafe { std::slice::from_raw_parts(v.dirty_indices, v.dirty_count as usize) }.to_vec();
    out.sort_unstable();
    out
}

fn indexes(ids: &[NodeId]) -> Vec<u32> {
    let mut out: Vec<u32> = ids.iter().map(|id| id.index() as u32).collect();
    out.sort_unstable();
    out
}

#[test]
fn an_ancestor_typography_edit_dirties_inheriting_text_descendants_only() {
    let (mut rt, mut s, t) = mount();
    let outer = t.outer;
    rt.submit_nve2(&s.frame(|tx| {
        tx.edit::<Typography>(outer).color(Rgba::rgb(9, 9, 9));
    }))
    .expect("paint-only edit");
    assert_eq!(
        dirty(&rt),
        indexes(&[t.outer, t.deep_text, t.mid_text, t.input]),
        "text / input descendants (through the inner typography node) are dirty; plain containers and outside text are not"
    );
    assert!(!dirty(&rt).contains(&(t.other_text.index() as u32)));
    assert!(!dirty(&rt).contains(&(t.plain.index() as u32)));
}

#[test]
fn nested_typography_changes_in_one_frame_list_each_descendant_once() {
    let (mut rt, mut s, t) = mount();
    let (outer, inner) = (t.outer, t.inner);
    rt.submit_nve2(&s.frame(|tx| {
        tx.edit::<Typography>(outer).color(Rgba::rgb(4, 4, 4));
        tx.edit::<Typography>(inner).color(Rgba::rgb(5, 5, 5));
    }))
    .expect("nested edits");
    let d = dirty(&rt);
    let mut unique = d.clone();
    unique.dedup();
    assert_eq!(d, unique, "no duplicates");
    assert_eq!(d, indexes(&[t.outer, t.inner, t.deep_text, t.mid_text, t.input]));
}

#[test]
fn detaching_an_ancestor_typography_dirties_its_inheriting_descendants() {
    let (mut rt, mut s, t) = mount();
    let inner = t.inner;
    rt.submit_nve2(&s.frame(|tx| tx.detach::<Typography>(inner))).expect("detach");
    let d = dirty(&rt);
    assert!(d.contains(&(t.deep_text.index() as u32)), "deep text lost its inherited weight: {d:?}");
    assert!(!d.contains(&(t.mid_text.index() as u32)), "siblings outside the detached subtree stay clean: {d:?}");
}

#[test]
fn edits_without_typography_changes_dirty_no_descendants() {
    let (mut rt, mut s, t) = mount();
    let mid = t.mid_text;
    rt.submit_nve2(&s.frame(|tx| {
        tx.edit::<TextContent>(mid).text("mud");
    }))
    .expect("text edit");
    assert_eq!(dirty(&rt), indexes(&[t.mid_text]));
}
