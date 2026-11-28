//! Criterion micro-benchmarks for `nve-camera`: 40-byte `CameraFrameWireLayout` encode/validate
//! and `CameraPreviewTable` store reads/updates.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_camera::{
        CameraFacing, CameraFrameDescriptor, CameraFrameWireLayout, CameraPixelFormat,
        CameraPreviewTable, CameraStabilization, CameraTorch, CAMERA_FRAME_DESCRIPTOR_SIZE,
    };
    use nve_store::{ChangeSet, NodeId, NodeStore};

    fn bench_camera_frame_and_table(c: &mut Criterion) {
        let desc = CameraFrameDescriptor {
            width: 1920,
            height: 1080,
            stride_bytes: 1920 * 4,
            pixel_format: CameraPixelFormat::Rgba8888,
            rotation_deg: 90,
            timestamp_ns: 1_700_000_000_000,
            native_handle_id: 0xDEAD_BEEF_CAFE_0001,
        };

        c.bench_function("camera_frame_wire_layout_encode_roundtrip", |b| {
            let mut buf = [0u8; CAMERA_FRAME_DESCRIPTOR_SIZE];
            b.iter(|| {
                let wire = CameraFrameWireLayout::from_descriptor(black_box(&desc))
                    .expect("valid camera descriptor");
                wire.write_into(&mut buf).expect("fixed 40-byte frame buffer");
                let back = wire.to_descriptor().expect("valid camera wire layout");
                black_box((buf[0], back.timestamp_ns))
            });
        });

        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut nodes = Vec::with_capacity(32);
        for i in 0..32u16 {
            let n = tx.create();
            tx.edit::<CameraPreviewTable>(n)
                .device_id("cam-wide-0")
                .facing(CameraFacing::Back)
                .torch(CameraTorch::Auto)
                .stabilization(CameraStabilization::Cinematic)
                .fps(60 + (i % 2) * 60)
                .zoom(1.5);
            nodes.push(n);
        }
        tx.set_children(NodeId::ROOT, &nodes);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        c.bench_function("camera_preview_table_read_32_rows", |b| {
            b.iter(|| {
                let mut total_fps = 0u64;
                for &n in &nodes {
                    if let Some(row) = store.try_row::<CameraPreviewTable>(black_box(n)) {
                        total_fps += u64::from(row.fps()) + row.device_id().len() as u64;
                    }
                }
                black_box(total_fps)
            });
        });
    }

    criterion_group!(benches, bench_camera_frame_and_table);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
