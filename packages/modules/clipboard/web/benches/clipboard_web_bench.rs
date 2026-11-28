//! Criterion micro-benchmark for `nve-clipboard-web` shadow buffer dispatch.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_clipboard::METHOD_HAS_TEXT;
    use nve_clipboard_web::WebClipboard;
    use nve_host_services::{CompletionSink, HostServiceHandler};

    fn bench_web_clipboard(c: &mut Criterion) {
        let clip = WebClipboard::headless();
        clip.set_shadow_text(Some("benchmark-payload".into()));
        let sink = CompletionSink::new();
        c.bench_function("clipboard_web_has_text_inline", |b| {
            b.iter(|| {
                let reply = clip
                    .handle(black_box(METHOD_HAS_TEXT), 1, black_box(&[]), &sink)
                    .expect("has_text succeeds");
                black_box(reply)
            });
        });
    }

    criterion_group!(benches, bench_web_clipboard);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
