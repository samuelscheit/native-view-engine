//! Criterion micro-benchmarks for `nve-http-web` (`FetchConfig::build_url`,
//! `FetchConfig::build_url_into`, `CredentialsMode::as_js_str`, `BrowserClock::now_ms`).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_http_web::{BrowserClock, CredentialsMode, FetchConfig};
    use nve_query::Clock;

    fn bench_http_web(c: &mut Criterion) {
        let mut g = c.benchmark_group("http_web");

        let same_origin = FetchConfig::same_origin();
        let cross_origin = FetchConfig::with_base_url("https://api.example.com/")
            .with_rpc_prefix("/api/v2/rpc/");
        let mut scratch = String::with_capacity(128);
        let clock = BrowserClock::new();

        g.bench_function("build_url_same_origin_alloc", |b| {
            b.iter(|| {
                let url = black_box(&same_origin).build_url(black_box("issues.v1.Issues/List"));
                black_box(url.len())
            });
        });

        g.bench_function("build_url_cross_origin_alloc", |b| {
            b.iter(|| {
                let url = black_box(&cross_origin).build_url(black_box("/issues.v1.Issues/List"));
                black_box(url.len())
            });
        });

        g.bench_function("build_url_into_same_origin_warm", |b| {
            b.iter(|| {
                black_box(&same_origin)
                    .build_url_into(black_box("issues.v1.Issues/List"), &mut scratch);
                black_box(scratch.len())
            });
        });

        g.bench_function("build_url_into_cross_origin_warm", |b| {
            b.iter(|| {
                black_box(&cross_origin)
                    .build_url_into(black_box("/issues.v1.Issues/List"), &mut scratch);
                black_box(scratch.len())
            });
        });

        g.bench_function("credentials_mode_as_js_str", |b| {
            let modes = [
                CredentialsMode::SameOrigin,
                CredentialsMode::Include,
                CredentialsMode::Omit,
            ];
            b.iter(|| {
                let mut total = 0usize;
                for m in black_box(modes) {
                    total += m.as_js_str().len();
                }
                black_box(total)
            });
        });

        g.bench_function("browser_clock_now_ms", |b| {
            b.iter(|| black_box(clock.now_ms()));
        });

        g.finish();
    }

    criterion_group!(benches, bench_http_web);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
