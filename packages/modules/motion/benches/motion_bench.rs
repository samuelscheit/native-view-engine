//! Criterion micro-benchmarks for `nve-motion` hot paths: closed-form spring steps, the
//! `MotionClock` slab tick, and spring easing sampling.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, BatchSize, Criterion};
    use nve_abi::NodeId;
    use nve_motion::{
        EasingCurve, MotionClock, MotionPreference, SpringSpec, SpringState, TransitionProperty,
        TransitionSpec,
    };

    fn bench_motion(c: &mut Criterion) {
        let spec = SpringSpec::default();
        c.bench_function("motion_spring_60_steps", |b| {
            b.iter(|| {
                let mut state = SpringState::new(0.0, 1.0);
                for _ in 0..60 {
                    state.step(black_box(&spec), black_box(1.0 / 60.0), MotionPreference::Full);
                }
                black_box(state.value)
            });
        });

        let mut clock = MotionClock::new();
        for i in 1..=128u32 {
            let node = NodeId::new(i, 0);
            clock.animate_spring((node, TransitionProperty::Opacity), spec, 0.0, 1.0);
            clock.animate_transition(
                (node, TransitionProperty::BackgroundColor),
                TransitionSpec::all(10_000),
                0.0,
                1.0,
            );
        }
        // The clone is setup (not measured): each iteration ticks a fresh copy of 256 tracks.
        c.bench_function("motion_clock_tick_256_tracks", |b| {
            b.iter_batched_ref(
                || clock.clone(),
                |warm| {
                    warm.tick_with(black_box(1.0 / 120.0), |ev| {
                        black_box(ev);
                    });
                    black_box(warm.active_count())
                },
                BatchSize::SmallInput,
            );
        });

        let easing = EasingCurve::Spring(SpringSpec::bouncy());
        c.bench_function("motion_spring_easing_sample", |b| {
            b.iter(|| black_box(easing).sample(black_box(0.37)));
        });
    }

    criterion_group!(benches, bench_motion);
}

#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_main!(native::benches);
