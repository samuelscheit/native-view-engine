// Live Rust provider of `overlay_hybrid.nve.rs` (`hybrid_kotlin_runtime_tests`), built as a staticlib.

use std::ffi::c_void;
use overlay_view_hybrid::*;

#[derive(Default)]
struct LiveOverlay {
    props: OverlayViewProps,
}

impl OverlayViewHybrid for LiveOverlay {
    fn apply_props(&mut self, props: OverlayViewProps) { self.props = props; }
    fn current_overlay(&mut self) -> Option<Overlay> { self.props.overlay.clone() }
    fn current_overlays(&mut self) -> Vec<Overlay> { self.props.overlays.clone() }
    fn current_extra(&mut self) -> NveHybridValue { self.props.extra.clone() }
    fn summary(&mut self) -> String {
        let p = &self.props;
        format!(
            "{}|{:?}|{}|{}|{}|{}|{}",
            p.title, p.kind, p.overlay.is_some(), p.overlays.len(), p.cues.len(), p.limit, p.big
        )
    }
    fn shift(&mut self, base: u32, object: Option<i32>) -> u32 {
        base.wrapping_add(object.unwrap_or(0) as u32)
    }
    fn echo(&mut self, overlay: Overlay) -> Result<Overlay, String> {
        if overlay.caption.as_deref() == Some("fail") {
            return Err("bad \u{1F600}".to_string());
        }
        Ok(Overlay {
            kind: OverlayKind::Caption,
            cues: overlay.cues.iter().rev().copied().collect(),
            caption: overlay.caption.map(|c| c.to_uppercase()),
            extra: overlay.extra,
            labels: overlay.labels.iter().map(|l| l.to_uppercase()).collect(),
            when: overlay.when ^ 0x8000_0000,
        })
    }
}

static VTABLE: NveHybridOverlayViewVTable = build_overlay_view_vtable::<LiveOverlay>();

#[unsafe(no_mangle)]
pub extern "C" fn nve_get_interface(interface_id: u32) -> *const c_void {
    if interface_id == NVE_HYBRID_OVERLAY_VIEW_INTERFACE_ID {
        (&VTABLE as *const NveHybridOverlayViewVTable).cast()
    } else {
        std::ptr::null()
    }
}
