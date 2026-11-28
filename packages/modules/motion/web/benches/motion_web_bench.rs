//! Criterion micro-benchmarks for `nve-motion-web` CSS transition compilation (`cubic-bezier`
//! and closed-form spring `linear()` sampling) and `MotionWebSystem` memoized style resolution.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::{ResolvedNodeStyle, WebTableSystem};
    use nve_motion::{EasingCurve, Motion, SpringSpec, TransitionProperty, TransitionSpec, Transitions};
    use nve_motion_web::{compile_transitions, MotionWebSystem};
    use nve_store::{ChangeSet, NodeId, NodeStore};

    fn bench_motion_web(c: &mut Criterion) {
        let keyword_specs = [
            TransitionSpec::new(TransitionProperty::BackgroundColor, 180)
                .easing(EasingCurve::EaseOut),
            TransitionSpec::new(TransitionProperty::Opacity, 120)
                .delay_ms(20)
                .easing(EasingCurve::CubicBezier(0.22, 1.0, 0.36, 1.0)),
        ];
        c.bench_function("motion_web_compile_keyword_bezier", |b| {
            b.iter(|| black_box(compile_transitions(black_box(&keyword_specs).iter().copied())));
        });

        let spring_specs = [
            TransitionSpec::spring(TransitionProperty::Transform, SpringSpec::bouncy()),
            TransitionSpec::new(TransitionProperty::BackgroundColor, 200)
                .easing(EasingCurve::EaseInOut),
        ];
        c.bench_function("motion_web_compile_spring_linear", |b| {
            b.iter(|| black_box(compile_transitions(black_box(&spring_specs).iter().copied())));
        });

        let mut store = NodeStore::new();
        let node = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[n]);
            tx.edit::<Motion>(n)
                .transitions(Some(Transitions::new(spring_specs.to_vec())));
            let mut changes = ChangeSet::default();
            tx.commit_into(&mut changes);
            n
        };
        let system = MotionWebSystem::new();
        let mut warm_out = ResolvedNodeStyle::default();
        system.resolve_style(&store, node, &mut warm_out);

        c.bench_function("motion_web_resolve_style_cached", |b| {
            let mut out = ResolvedNodeStyle::default();
            b.iter(|| {
                out.classes.clear();
                out.class_rules.clear();
                out.media_rules.clear();
                system.resolve_style(black_box(&store), black_box(node), &mut out);
                black_box(out.class_rules.len())
            });
        });
    }

    criterion_group!(benches, bench_motion_web);
}

#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_main!(native::benches);
