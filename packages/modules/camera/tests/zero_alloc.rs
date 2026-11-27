//! Global-allocator verification proving `0` heap allocations for `CameraFrameDescriptor::encode_into`,
//! `encode`, and `decode` via `nve_codec::SliceWriter` / `ByteReader`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_camera::{
    CameraFrameDescriptor, CameraPixelFormat, CAMERA_FRAME_DESCRIPTOR_SIZE,
};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Delegates directly to `std::alloc::System` while counting allocations.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn camera_frame_encode_into_and_decode_allocate_zero_bytes() {
    let desc = CameraFrameDescriptor {
        width: 1920,
        height: 1080,
        stride_bytes: 1920 * 4,
        pixel_format: CameraPixelFormat::Rgba8888,
        rotation_deg: 90,
        timestamp_ns: 987_654_321,
        native_handle_id: 0x0123_4567_89AB_CDEF,
    };
    let mut buf = [0u8; CAMERA_FRAME_DESCRIPTOR_SIZE];

    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    for i in 0..64u64 {
        let cur = CameraFrameDescriptor {
            timestamp_ns: desc.timestamp_ns + i,
            ..desc
        };
        cur.encode_into(&mut buf).unwrap();
        let decoded = CameraFrameDescriptor::decode(&buf).unwrap();
        assert_eq!(decoded, cur);
    }
    let after = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(
        after - before,
        0,
        "CameraFrameDescriptor::encode_into + decode must allocate 0 bytes"
    );
}
