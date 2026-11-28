//! Criterion micro-benchmark for `nve-haptics-web` vibration pattern resolution.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_haptics::{ImpactStyle, NotificationHaptic};
    use nve_haptics_web::{impact_pattern_ms, notification_pattern_ms};

    fn bench_haptics_web(c: &mut Criterion) {
        let styles = [
            ImpactStyle::Light,
            ImpactStyle::Medium,
            ImpactStyle::Heavy,
            ImpactStyle::Soft,
            ImpactStyle::Rigid,
        ];
        let notices = [
            NotificationHaptic::Success,
            NotificationHaptic::Warning,
            NotificationHaptic::Error,
        ];
        c.bench_function("haptics_web_pattern_lookup", |b| {
            b.iter(|| {
                let mut sum = 0u32;
                for &s in black_box(&styles) {
                    sum = sum.wrapping_add(impact_pattern_ms(s)[0]);
                }
                for &n in black_box(&notices) {
                    sum = sum.wrapping_add(notification_pattern_ms(n).len() as u32);
                }
                black_box(sum)
            });
        });
    }

    criterion_group!(benches, bench_haptics_web);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
