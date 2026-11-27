//! Latency benches for the TV input hot paths: one warm spatial move over 500 focusables and
//! one web `keydown` normalization across vendor tables.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nve_tv::{
    FocusDirection, NodeId, SpatialFocusEngine, SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
    TextInputContext, WebKeyInput, WebTvPlatform, map_web_tv_key,
};

/// 25 x 20 card grid split into 4 shelves (groups), one explicit override per row.
fn grid_500() -> SpatialFocusEngine {
    let mut engine = SpatialFocusEngine::new().with_wrap_axis(true);
    for row in 0..20u32 {
        for col in 0..25u32 {
            let index = row * 25 + col + 1;
            let rect = SpatialRect::new(col as f32 * 120.0, row as f32 * 90.0, 110.0, 80.0);
            let mut node = SpatialFocusNode::new(NodeId::new(index, 1), rect)
                .with_group(SpatialFocusGroupId(row / 5));
            if col == 24 {
                node = node.with_next(FocusDirection::Right, NodeId::new(row * 25 + 1, 1));
            }
            engine.register(node);
        }
    }
    engine
}

fn spatial_move(c: &mut Criterion) {
    let mut engine = grid_500();
    let _ = engine.move_focus(FocusDirection::Right);
    let path = [
        FocusDirection::Right,
        FocusDirection::Down,
        FocusDirection::Left,
        FocusDirection::Up,
    ];
    let mut step = 0usize;
    c.bench_function("spatial_move_500_focusables", |b| {
        b.iter(|| {
            step = (step + 1) & 3;
            black_box(engine.move_focus(black_box(path[step])))
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

criterion_group!(benches, spatial_move, web_key_mapping);
criterion_main!(benches);
