//! Latency benches for the TV input hot paths: warm spatial moves over 500 and 2 000 focusables
//! along a walk that takes explicit overrides, wrap-around, and shelf group memory; the
//! just-in-time `replace_nodes` + move of the DOM controller; and one web `keydown`
//! normalization across vendor tables.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nve_tv::{
    FocusDirection, NodeId, SpatialFocusEngine, SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
    TextInputContext, WebKeyInput, WebTvPlatform, map_web_tv_key,
};

/// `cols x rows` card grid, one shelf (group) per 5 rows, a row-end Right override back to the
/// row start, and wrap-around.
fn grid(cols: u32, rows: u32) -> SpatialFocusEngine {
    let mut engine = SpatialFocusEngine::new().with_wrap_axis(true);
    for row in 0..rows {
        for col in 0..cols {
            let index = row * cols + col + 1;
            let rect = SpatialRect::new(col as f32 * 120.0, row as f32 * 90.0, 110.0, 80.0);
            let mut node = SpatialFocusNode::new(NodeId::new(index, 1), rect)
                .with_group(SpatialFocusGroupId(row / 5));
            if col == cols - 1 {
                node = node.with_next(FocusDirection::Right, NodeId::new(row * cols + 1, 1));
            }
            engine.register(node);
        }
    }
    engine
}

/// A full row of Rights (ending on the override), a full column of Downs (crossing every shelf
/// boundary and wrapping at the bottom), then a short Left / Up tail.
fn walk(cols: u32, rows: u32) -> Vec<FocusDirection> {
    let mut path = vec![FocusDirection::Right; cols as usize];
    path.extend(std::iter::repeat_n(FocusDirection::Down, rows as usize));
    path.extend([FocusDirection::Left; 3]);
    path.extend([FocusDirection::Up; 2]);
    path
}

fn bench_walk(c: &mut Criterion, name: &str, cols: u32, rows: u32) {
    let mut engine = grid(cols, rows);
    let path = walk(cols, rows);
    for &direction in &path {
        let _ = engine.move_focus(direction);
    }
    let mut step = 0usize;
    c.bench_function(name, |b| {
        b.iter(|| {
            step = (step + 1) % path.len();
            black_box(engine.move_focus(black_box(path[step])))
        })
    });
}

fn spatial_move(c: &mut Criterion) {
    bench_walk(c, "spatial_move_500_focusables", 25, 20);
    bench_walk(c, "spatial_move_2000_focusables_stress", 50, 40);
}

fn jit_snapshot_move(c: &mut Criterion) {
    let mut engine = grid(50, 40);
    let snapshot: Vec<SpatialFocusNode> = engine.nodes().to_vec();
    let path = walk(50, 40);
    let mut step = 0usize;
    c.bench_function("replace_nodes_then_move_2000_focusables", |b| {
        b.iter(|| {
            step = (step + 1) % path.len();
            engine.replace_nodes(black_box(&snapshot).iter().copied());
            black_box(engine.move_focus(path[step]))
        })
    });
}

fn web_key_mapping(c: &mut Criterion) {
    let key = |key, code, key_code| WebKeyInput {
        key,
        code,
        key_code,
        ..WebKeyInput::default()
    };
    let cases: [(WebTvPlatform, WebKeyInput<'static>); 8] = [
        (WebTvPlatform::Standard, key("ArrowDown", "ArrowDown", 40)),
        (WebTvPlatform::Tizen, key("XF86Back", "", 10009)),
        (WebTvPlatform::Tizen, key("Unidentified", "", 10252)),
        (WebTvPlatform::WebOs, key("PageUp", "PageUp", 33)),
        (WebTvPlatform::AndroidTv, key("Unidentified", "", 19)),
        (WebTvPlatform::Xbox, key("GamepadA", "", 195)),
        (WebTvPlatform::PlayStation, key("Enter", "Enter", 13)),
        (
            WebTvPlatform::Standard,
            WebKeyInput {
                text_input: TextInputContext::SingleLine,
                ..key("a", "KeyA", 65)
            },
        ),
    ];
    let mut i = 0usize;
    c.bench_function("map_web_tv_key", |b| {
        b.iter(|| {
            i = (i + 1) % cases.len();
            let (platform, input) = &cases[i];
            black_box(map_web_tv_key(black_box(*platform), black_box(input)))
        })
    });
}

criterion_group!(benches, spatial_move, jit_snapshot_move, web_key_mapping);
criterion_main!(benches);
