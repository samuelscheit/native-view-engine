//! Criterion micro-benchmarks for `nve-webview`: `WebView` table + command dispatch/decode.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_webview::{WebView, WebViewCommand};

    fn bench_webview_commands(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let target = tx.create();
        tx.set_children(NodeId::ROOT, &[target]);
        tx.edit::<WebView>(target).url("https://nve.dev");
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let nav = WebViewCommand::Navigate("https://nve.dev/docs/getting-started".into());
        let eval = WebViewCommand::EvaluateScript("window.__nvePing && window.__nvePing()".into());

        c.bench_function("webview_dispatch_and_decode_commands", |b| {
            b.iter(|| {
                let mut tx = store.tx();
                nav.dispatch(&mut tx, target);
                WebViewCommand::Reload.dispatch(&mut tx, target);
                eval.dispatch(&mut tx, target);
                tx.commit_into(&mut changes);
                let mut decoded = 0usize;
                for cmd in &changes.commands {
                    if WebViewCommand::decode(cmd.op, changes.command_payload(cmd)).is_some() {
                        decoded += 1;
                    }
                }
                black_box(decoded)
            });
        });
    }

    criterion_group!(benches, bench_webview_commands);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
