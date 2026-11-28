//! Criterion micro-benchmarks for `nve-video`: `VideoPlayer` table + command dispatch/decode.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_video::{VideoPlayer, VideoPlayerCommand};

    fn bench_video_table_and_commands(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut nodes = Vec::with_capacity(32);
        for _ in 0..32 {
            let n = tx.create();
            tx.edit::<VideoPlayer>(n)
                .source_uri("https://cdn.nve.dev/demo.mp4")
                .autoplay(true)
                .muted(true);
            nodes.push(n);
        }
        tx.set_children(NodeId::ROOT, &nodes);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        c.bench_function("video_player_dispatch_and_decode_commands", |b| {
            let target = nodes[0];
            b.iter(|| {
                let mut tx = store.tx();
                VideoPlayerCommand::Play.dispatch(&mut tx, target);
                VideoPlayerCommand::SeekMs(black_box(42_500)).dispatch(&mut tx, target);
                VideoPlayerCommand::SetVolume(black_box(0.75)).dispatch(&mut tx, target);
                VideoPlayerCommand::Pause.dispatch(&mut tx, target);
                tx.commit_into(&mut changes);
                let mut decoded = 0usize;
                for cmd in &changes.commands {
                    if VideoPlayerCommand::decode(cmd.op, changes.command_payload(cmd)).is_some() {
                        decoded += 1;
                    }
                }
                black_box(decoded)
            });
        });
    }

    criterion_group!(benches, bench_video_table_and_commands);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
