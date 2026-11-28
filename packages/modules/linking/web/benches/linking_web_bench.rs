//! Criterion micro-benchmark for `nve-linking-web` URL allowlist validation.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_linking_web::{is_allowed_web_url_scheme, validate_web_url};

    fn bench_linking_web(c: &mut Criterion) {
        let urls = [
            "https://nve.dev/docs",
            "http://localhost:8080",
            "mailto:dev@nve.dev",
            "tel:+15551234567",
            "javascript:alert(1)",
            "data:text/html,hi",
            "file:///etc/passwd",
        ];
        c.bench_function("linking_web_allowlist_batch", |b| {
            b.iter(|| {
                let mut ok = 0usize;
                for &u in black_box(&urls) {
                    if is_allowed_web_url_scheme(u) && validate_web_url(u).is_ok() {
                        ok += 1;
                    }
                }
                black_box(ok)
            });
        });
    }

    criterion_group!(benches, bench_linking_web);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
