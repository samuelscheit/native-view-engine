//! Workload B: Per-frame UI tree & prop allocation (`1k`, `10k`, `50k` nodes).
//!
//! Benchmarks production [`nve_engine::FrameArena`] (64-byte-aligned contiguous slab bump
//! allocator with O(1) borrow-checked `reset()`) against standard `Vec` + `String` heap churn.

use std::mem::{align_of, offset_of};

pub use nve_engine::{FrameArena, FRAME_ARENA_BASE_ALIGN};

#[inline]
fn write_u64_into(dst: &mut [u8], pos: &mut usize, mut v: u64) {
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let slice = &tmp[i..];
    dst[*pos..*pos + slice.len()].copy_from_slice(slice);
    *pos += slice.len();
}

#[inline]
fn format_node_label_buf(buf: &mut [u8; 40], idx: usize, frame: u32) -> usize {
    let mut pos = 0;
    buf[pos..pos + 5].copy_from_slice(b"node-");
    pos += 5;
    write_u64_into(buf, &mut pos, idx as u64);
    buf[pos..pos + 2].copy_from_slice(b"-f");
    pos += 2;
    write_u64_into(buf, &mut pos, frame as u64);
    pos
}

#[inline]
fn format_prop_str_buf(buf: &mut [u8; 40], idx: usize, frame: u32) -> usize {
    let mut pos = 0;
    buf[pos..pos + 4].copy_from_slice(b"val-");
    pos += 4;
    write_u64_into(buf, &mut pos, idx as u64);
    buf[pos] = b'-';
    pos += 1;
    write_u64_into(buf, &mut pos, frame as u64);
    pos
}

#[inline]
fn alloc_node_label(arena: &FrameArena, idx: usize, frame: u32) -> &str {
    let mut stack = [0u8; 40];
    let len = format_node_label_buf(&mut stack, idx, frame);
    // SAFETY: `format_node_label_buf` writes pure ASCII bytes.
    let s = unsafe { std::str::from_utf8_unchecked(&stack[..len]) };
    arena.alloc_str(s)
}

#[inline]
fn alloc_prop_str(arena: &FrameArena, idx: usize, frame: u32) -> &str {
    let mut stack = [0u8; 40];
    let len = format_prop_str_buf(&mut stack, idx, frame);
    // SAFETY: `format_prop_str_buf` writes pure ASCII bytes.
    let s = unsafe { std::str::from_utf8_unchecked(&stack[..len]) };
    arena.alloc_str(s)
}

// ============================================================================
// Candidate UI Tree Types (`#[repr(C)]`, zero heap allocs per frame)
// ============================================================================

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ArenaPropValue<'a> {
    Float(f32),
    Color(u32),
    Text(&'a str),
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ArenaPropEntry<'a> {
    pub key: &'a str,
    pub value: ArenaPropValue<'a>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ArenaRenderNode<'a> {
    pub id: u64,
    pub parent: u32,
    pub kind: u16,
    pub flags: u16,
    pub layout: [f32; 4],
    pub style_rgba: u32,
    pub z_index: i32,
    pub label: &'a str,
    pub props: &'a [ArenaPropEntry<'a>],
    pub children: &'a [u32],
}

const _: () = {
    assert!(align_of::<ArenaRenderNode<'static>>() == 8);
    assert!(offset_of!(ArenaRenderNode<'static>, id) == 0);
    assert!(offset_of!(ArenaRenderNode<'static>, parent) == 8);
    assert!(offset_of!(ArenaRenderNode<'static>, kind) == 12);
    assert!(offset_of!(ArenaRenderNode<'static>, flags) == 14);
    assert!(offset_of!(ArenaRenderNode<'static>, layout) == 16);
    assert!(offset_of!(ArenaRenderNode<'static>, style_rgba) == 32);
    assert!(offset_of!(ArenaRenderNode<'static>, z_index) == 36);
};

/// Returns a generous slab capacity in bytes sufficient for `node_count` nodes + props + strings.
pub const fn recommended_frame_arena_capacity(node_count: usize) -> usize {
    node_count * 384 + 65_536
}

/// Builds a `node_count`-node UI tree in `arena`, traverses every node/prop/string, and resets
/// the arena in O(1).
pub fn run_arena_frame(arena: &mut FrameArena, node_count: usize, frame: u32) -> u64 {
    let acc = {
        let a: &FrameArena = arena;
        let nodes = a.alloc_slice_fill_with(node_count, |i| {
            let parent = if i == 0 { 0 } else { ((i - 1) / 4) as u32 };
            let kind = (i % 4) as u16;
            let flags = ((i & 7) as u16) | ((frame as u16 & 1) << 8);
            let x = ((i % 32) as f32) * 12.0;
            let y = ((i / 32) as f32) * 44.0 + (frame as f32 * 0.5);
            let label = alloc_node_label(a, i, frame);
            let prop_val_str = alloc_prop_str(a, i, frame);

            let props = a.alloc_slice_copy(&[
                ArenaPropEntry {
                    key: a.alloc_str("opacity"),
                    value: ArenaPropValue::Float(0.25 + ((i % 4) as f32) * 0.25),
                },
                ArenaPropEntry {
                    key: a.alloc_str("tint"),
                    value: ArenaPropValue::Color(0xFF20_3040u32.wrapping_add(i as u32)),
                },
                ArenaPropEntry {
                    key: a.alloc_str("accessibilityLabel"),
                    value: ArenaPropValue::Text(prop_val_str),
                },
            ]);

            let first_child = i * 4 + 1;
            let child_count = if first_child < node_count {
                (node_count - first_child).min(4)
            } else {
                0
            };
            let mut child_buf = [0u32; 4];
            for (c, slot) in child_buf.iter_mut().take(child_count).enumerate() {
                *slot = (first_child + c) as u32;
            }
            let children = a.alloc_slice_copy(&child_buf[..child_count]);

            ArenaRenderNode {
                id: 1 + i as u64,
                parent,
                kind,
                flags,
                layout: [x, y, 320.0, 40.0],
                style_rgba: 0xFF1E_293Bu32.wrapping_add((i as u32) * 3),
                z_index: (i % 16) as i32,
                label,
                props,
                children,
            }
        });

        let mut acc = 0u64;
        for n in nodes.iter() {
            acc = fold_node(
                acc,
                n.id,
                n.parent,
                n.layout[0],
                n.layout[1],
                n.style_rgba,
                n.label,
                n.children.len(),
            );
            for p in n.props {
                acc = acc.wrapping_add(p.key.len() as u64);
                match p.value {
                    ArenaPropValue::Float(v) => acc ^= v.to_bits() as u64,
                    ArenaPropValue::Color(c) => acc = acc.wrapping_add(c as u64),
                    ArenaPropValue::Text(s) => {
                        acc = acc.wrapping_add(s.len() as u64);
                        if let Some(&b) = s.as_bytes().last() {
                            acc ^= b as u64;
                        }
                    }
                }
            }
        }
        acc
    };
    arena.reset();
    acc
}

/// Runs `frames` consecutive UI frames on a single reused [`FrameArena`].
pub fn run_arena_frames(arena: &mut FrameArena, node_count: usize, frames: u32) -> u64 {
    let mut acc = 0u64;
    for frame in 0..frames {
        acc ^= run_arena_frame(arena, node_count, frame);
    }
    acc
}

// ============================================================================
// Baseline UI Tree Types (standard `Vec` + `String` heap allocations per frame)
// ============================================================================

#[derive(Debug)]
pub enum HeapPropValue {
    Float(f32),
    Color(u32),
    Text(String),
}

#[derive(Debug)]
pub struct HeapPropEntry {
    pub key: String,
    pub value: HeapPropValue,
}

#[derive(Debug)]
pub struct HeapRenderNode {
    pub id: u64,
    pub parent: u32,
    pub kind: u16,
    pub flags: u16,
    pub layout: [f32; 4],
    pub style_rgba: u32,
    pub z_index: i32,
    pub label: String,
    pub props: Vec<HeapPropEntry>,
    pub children: Vec<u32>,
}

/// Builds the exact same `node_count`-node UI tree using standard `Vec` + `String` heap
/// allocations, traverses it, and drops everything at frame end.
pub fn run_heap_frame(node_count: usize, frame: u32) -> u64 {
    let mut nodes = Vec::with_capacity(node_count);
    let mut label_buf = [0u8; 40];
    let mut prop_buf = [0u8; 40];

    for i in 0..node_count {
        let parent = if i == 0 { 0 } else { ((i - 1) / 4) as u32 };
        let kind = (i % 4) as u16;
        let flags = ((i & 7) as u16) | ((frame as u16 & 1) << 8);
        let x = ((i % 32) as f32) * 12.0;
        let y = ((i / 32) as f32) * 44.0 + (frame as f32 * 0.5);

        let l_len = format_node_label_buf(&mut label_buf, i, frame);
        let label = std::str::from_utf8(&label_buf[..l_len])
            .expect("format_node_label_buf emits ASCII")
            .to_owned();

        let p_len = format_prop_str_buf(&mut prop_buf, i, frame);
        let prop_text = std::str::from_utf8(&prop_buf[..p_len])
            .expect("format_prop_str_buf emits ASCII")
            .to_owned();

        let props = vec![
            HeapPropEntry {
                key: "opacity".to_owned(),
                value: HeapPropValue::Float(0.25 + ((i % 4) as f32) * 0.25),
            },
            HeapPropEntry {
                key: "tint".to_owned(),
                value: HeapPropValue::Color(0xFF20_3040u32.wrapping_add(i as u32)),
            },
            HeapPropEntry {
                key: "accessibilityLabel".to_owned(),
                value: HeapPropValue::Text(prop_text),
            },
        ];

        let first_child = i * 4 + 1;
        let child_count = if first_child < node_count {
            (node_count - first_child).min(4)
        } else {
            0
        };
        let mut children = Vec::with_capacity(child_count);
        for c in 0..child_count {
            children.push((first_child + c) as u32);
        }

        nodes.push(HeapRenderNode {
            id: 1 + i as u64,
            parent,
            kind,
            flags,
            layout: [x, y, 320.0, 40.0],
            style_rgba: 0xFF1E_293Bu32.wrapping_add((i as u32) * 3),
            z_index: (i % 16) as i32,
            label,
            props,
            children,
        });
    }

    let mut acc = 0u64;
    for n in &nodes {
        acc = fold_node(
            acc,
            n.id,
            n.parent,
            n.layout[0],
            n.layout[1],
            n.style_rgba,
            &n.label,
            n.children.len(),
        );
        for p in &n.props {
            acc = acc.wrapping_add(p.key.len() as u64);
            match &p.value {
                HeapPropValue::Float(v) => acc ^= v.to_bits() as u64,
                HeapPropValue::Color(c) => acc = acc.wrapping_add(*c as u64),
                HeapPropValue::Text(s) => {
                    acc = acc.wrapping_add(s.len() as u64);
                    if let Some(&b) = s.as_bytes().last() {
                        acc ^= b as u64;
                    }
                }
            }
        }
    }
    acc
}

/// Runs `frames` consecutive UI frames using standard heap allocation/drop churn.
pub fn run_heap_frames(node_count: usize, frames: u32) -> u64 {
    let mut acc = 0u64;
    for frame in 0..frames {
        acc ^= run_heap_frame(node_count, frame);
    }
    acc
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn fold_node(
    acc: u64,
    id: u64,
    parent: u32,
    x: f32,
    y: f32,
    rgba: u32,
    label: &str,
    child_count: usize,
) -> u64 {
    let mut h = acc.wrapping_mul(0x9E37_79B1_85EB_CA87).wrapping_add(id);
    h ^= ((parent as u64) << 32) | (rgba as u64);
    h = h.wrapping_add(x.to_bits() as u64 ^ ((y.to_bits() as u64) << 16));
    h = h.wrapping_add(label.len() as u64 + child_count as u64);
    if let Some(&b) = label.as_bytes().last() {
        h ^= b as u64;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_and_heap_ui_trees_produce_identical_traversals() {
        for &nodes in &[1usize, 64, 1_000, 10_000] {
            let mut arena = FrameArena::with_capacity(recommended_frame_arena_capacity(nodes));
            let arena_hash = run_arena_frames(&mut arena, nodes, 5);
            let heap_hash = run_heap_frames(nodes, 5);
            assert_eq!(
                arena_hash, heap_hash,
                "traversal parity failed for {nodes} nodes"
            );
            assert_eq!(arena.used_bytes(), 0, "arena must reset after each frame");
            assert!(arena.high_water_bytes() <= arena.capacity_bytes());
        }
    }
}
