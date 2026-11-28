//! `dirty_indices` covers inherited typography: a `Typography` change, attach, or detach on an
//! ancestor dirties exactly the text / text-input descendants that inherit it (paint-only
//! changes move no rect), so retained hosts restyle `dirty_indices` without whole-tree walks.

use nve_components::tables::{Layout, TextContent, TextInput, Typography};
use std::sync::Arc;

use nve_engine::{LayoutRect, Viewport};
use nve_layout::{StoreLayoutEngine, TextMeasurer};
use nve_runtime::{RuntimeBuilder, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, Transaction};

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::count_allocs;

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
    mount_with(RuntimeBuilder::new())
}

fn mount_with(builder: RuntimeBuilder) -> (SurfaceRuntime, Session, Tree) {
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
    let mut rt = builder.build();
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

/// Moves `inner` (`Typography` weight 700, holding `deep_text`) between `outer` and the root.
fn move_inner(tx: &mut Transaction<'_>, t: &Tree, under_root: bool) {
    if under_root {
        tx.set_children(t.outer, &[t.mid_text, t.input, t.plain]);
        tx.set_children(NodeId::ROOT, &[t.outer, t.other_text, t.inner]);
    } else {
        tx.set_children(NodeId::ROOT, &[t.outer, t.other_text]);
        tx.set_children(t.outer, &[t.inner, t.mid_text, t.input, t.plain]);
    }
}

#[test]
fn a_reparented_subtree_dirties_its_inheriting_text_descendants() {
    let (mut rt, mut s, t) = mount();
    // `plain` (a leaf container without text) moves too: only text / input descendants join.
    let wrapper_text = {
        let mut created = None;
        rt.submit_nve2(&s.frame(|tx| {
            let text = tx.create();
            fixed(tx, text);
            tx.edit::<TextContent>(text).text("wrapped");
            tx.set_children(t.plain, &[text]);
            created = Some(text);
        }))
        .expect("wrap");
        created.expect("text")
    };
    rt.submit_nve2(&s.frame(|tx| {
        tx.set_children(t.outer, &[t.inner, t.mid_text, t.input]);
        tx.set_children(t.inner, &[t.deep_text, t.plain]);
    }))
    .expect("move plain under inner");
    let d = dirty(&rt);
    assert!(d.contains(&(wrapper_text.index() as u32)), "text inside the moved container: {d:?}");
    assert!(!d.contains(&(t.mid_text.index() as u32)), "staying siblings stay clean: {d:?}");
    assert!(!d.contains(&(t.other_text.index() as u32)), "{d:?}");

    let tree = &t;
    rt.submit_nve2(&s.frame(|tx| move_inner(tx, tree, true))).expect("inner to root");
    let d = dirty(&rt);
    for text in [t.deep_text, wrapper_text] {
        assert!(d.contains(&(text.index() as u32)), "{text:?} inherits from a new ancestor: {d:?}");
    }
    assert!(!d.contains(&(t.mid_text.index() as u32)), "{d:?}");
    let mut unique = d.clone();
    unique.dedup();
    assert_eq!(d, unique, "a node reached from several roots is listed once");
}

/// Never lays out (Taffy's flexbox pass allocates per run): isolates the runtime's own frame
/// path — decode, commit, inherited-typography walk, delta, publish.
struct NoLayout;

impl StoreLayoutEngine for NoLayout {
    fn name(&self) -> &'static str {
        "none"
    }
    fn sync_changes(&mut self, _store: &NodeStore, _changes: &ChangeSet) {}
    fn compute_layout(&mut self, _store: &NodeStore, _root: NodeId, _viewport: Viewport) -> bool {
        false
    }
    fn layout_of(&self, _id: NodeId) -> Option<LayoutRect> {
        None
    }
    fn set_text_measurer(&mut self, _measurer: Arc<dyn TextMeasurer>) {}
    fn reset(&mut self) {}
}

#[test]
fn steady_state_typography_and_reparent_frames_are_zero_alloc() {
    let (mut rt, mut s, t) = mount_with(RuntimeBuilder::new().layout(Box::new(NoLayout)));
    let frames: Vec<Vec<u8>> = (0..24u8)
        .map(|i| {
            let tree = &t;
            s.frame(|tx| {
                move_inner(tx, tree, i % 2 == 0);
                tx.edit::<Typography>(tree.outer).color(Rgba::rgb(i, 0, 0));
            })
        })
        .collect();
    let (warm, measured) = frames.split_at(8);
    for f in warm {
        rt.submit_nve2(f).expect("warm-up frame");
    }
    let (results, allocs) = count_allocs(|| {
        measured.iter().map(|f| rt.submit_nve2(f).is_ok()).fold(true, |a, ok| a & ok)
    });
    assert!(results, "every frame applies");
    assert_eq!(allocs, 0, "typography + reparent frames allocate nothing once warm");
    assert!(dirty(&rt).contains(&(t.deep_text.index() as u32)));
}
