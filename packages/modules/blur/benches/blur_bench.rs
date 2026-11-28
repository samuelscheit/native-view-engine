//! Criterion micro-benchmarks for `nve-blur`: `BlurSurface` sparse-table writes and reads.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_blur::{BlurMaterial, BlurSurface};
    use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

    const MATERIALS: [BlurMaterial; 6] = [
        BlurMaterial::UltraThin,
        BlurMaterial::Thin,
        BlurMaterial::Regular,
        BlurMaterial::Thick,
        BlurMaterial::Chrome,
        BlurMaterial::SystemMaterial,
    ];

    fn bench_blur_table(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut nodes = Vec::with_capacity(64);
        for i in 0..64usize {
            let n = tx.create();
            tx.edit::<BlurSurface>(n)
                .material(MATERIALS[i % MATERIALS.len()])
                .intensity(0.75)
                .tint(Some(Rgba::new(240, 245, 255, 64)));
            nodes.push(n);
        }
        tx.set_children(NodeId::ROOT, &nodes);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        c.bench_function("blur_surface_read_64_rows", |b| {
            b.iter(|| {
                let mut sum = 0.0f32;
                for &n in &nodes {
                    if let Some(row) = store.try_row::<BlurSurface>(black_box(n)) {
                        sum += row.material().css_blur_px() * row.intensity();
                    }
                }
                black_box(sum)
            });
        });

        c.bench_function("blur_surface_update_64_rows", |b| {
            let mut step = 0usize;
            b.iter(|| {
                step = step.wrapping_add(1);
                let mut tx = store.tx();
                for (i, &n) in nodes.iter().enumerate() {
                    tx.edit::<BlurSurface>(n)
                        .material(MATERIALS[(i + step) % MATERIALS.len()])
                        .intensity(0.5 + ((i + step) & 3) as f32 * 0.125);
                }
                tx.commit_into(&mut changes);
                black_box(changes.touched.len())
            });
        });
    }

    criterion_group!(benches, bench_blur_table);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
