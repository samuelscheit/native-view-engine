//! Live Rust provider of `fixed_array_hybrid.nve.rs` for the compiled C++20 and Swift drivers:
//! every reply is a deterministic transform of its input, so the drivers can check each `[T; N]`
//! crossed both ways element by element.

use std::ffi::c_void;
use fixed_codec_hybrid::*;

#[derive(Default)]
struct LiveCodec;

impl FixedCodecHybrid for LiveCodec {
    fn echo(&mut self, mut bundle: FixedBundle) -> FixedBundle {
        bundle.label.push('!');
        bundle.coords.reverse();
        bundle.points.swap(0, 1);
        bundle.matrix = bundle.matrix.map(|[a, b]| [b, a]);
        bundle.tag = bundle.tag.map(|[a, b, c]| [c, b, a]);
        bundle
    }
    fn swap(&mut self, coords: [u32; 4]) -> [u32; 4] {
        [coords[3], coords[2], coords[1], coords[0]]
    }
    fn tag(&mut self, present: bool) -> Option<[u8; 3]> {
        present.then_some([7, 8, 255])
    }
    fn try_tag(&mut self, mode: u32) -> Result<Option<[u8; 3]>, String> {
        match mode {
            0 => Err("no tag".to_string()),
            1 => Ok(None),
            _ => Ok(Some([1, 2, 3])),
        }
    }
    fn retag(&mut self, tag: Option<[u8; 3]>) -> Option<[u8; 3]> {
        tag.map(|[a, b, c]| [c.wrapping_add(1), b, a])
    }
    fn mirror(&mut self, pod: FixedPod) -> FixedPod {
        FixedPod {
            v: pod.v.map(|x| -x),
            points: [pod.points[1], pod.points[0]],
            kinds: [pod.kinds[1], pod.kinds[0]],
            grid: pod.grid.map(|[a, b]| [a + 1, b + 1]),
            flags: pod.flags.map(|row| row.map(|f| !f)),
            one: [pod.one[0] * 2],
            cells: [pod.cells[1], pod.cells[0]],
        }
    }
    fn maybe_mirror(&mut self, pod: Option<FixedPod>) -> Option<FixedPod> {
        pod.map(|pod| self.mirror(pod))
    }
}

#[derive(Default)]
struct LiveView {
    props: FixedViewProps,
}

impl FixedViewHybrid for LiveView {
    fn apply_props(&mut self, props: FixedViewProps) {
        self.props = props;
    }
    fn summary(&mut self) -> FixedBundle {
        let pod = &self.props.pod;
        let flags = pod.flags[0].iter().chain(&pod.flags[1]).filter(|f| **f).count() as u32;
        FixedBundle {
            label: format!("view:{flags}"),
            coords: self.props.coords,
            points: pod.points,
            matrix: pod.grid,
            tag: self.props.tag,
        }
    }
}

static CODEC_VTABLE: NveHybridFixedCodecVTable = build_fixed_codec_vtable::<LiveCodec>();
static VIEW_VTABLE: NveHybridFixedViewVTable = build_fixed_view_vtable::<LiveView>();

#[unsafe(no_mangle)]
pub extern "C" fn nve_get_interface(interface_id: u32) -> *const c_void {
    match interface_id {
        NVE_HYBRID_FIXED_CODEC_INTERFACE_ID => (&CODEC_VTABLE as *const NveHybridFixedCodecVTable).cast(),
        NVE_HYBRID_FIXED_VIEW_INTERFACE_ID => (&VIEW_VTABLE as *const NveHybridFixedViewVTable).cast(),
        _ => std::ptr::null(),
    }
}
