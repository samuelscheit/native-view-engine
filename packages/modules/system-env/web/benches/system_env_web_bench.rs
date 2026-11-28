//! Criterion micro-benchmark for `nve-system-env-web` theme query dispatch.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_host_services::{CompletionSink, HostServiceHandler};
    use nve_system_env::METHOD_GET_THEME;
    use nve_system_env_web::{theme_from_media_flags, WebSystemEnv};

    fn bench_system_env_web(c: &mut Criterion) {
        let env = WebSystemEnv::new();
        let sink = CompletionSink::new();
        c.bench_function("system_env_web_get_theme_inline", |b| {
            b.iter(|| {
                let reply = env
                    .handle(black_box(METHOD_GET_THEME), 1, black_box(&[]), &sink)
                    .expect("get_theme succeeds");
                black_box(reply)
            });
        });

        c.bench_function("system_env_web_theme_from_flags", |b| {
            b.iter(|| {
                black_box(theme_from_media_flags(
                    black_box(true),
                    black_box(false),
                    black_box(true),
                    black_box(false),
                    black_box(2.0),
                ))
            });
        });
    }

    criterion_group!(benches, bench_system_env_web);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
