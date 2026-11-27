use nve_abi::NveValue;

/// Overlay category with sparse discriminants, one above `i32::MAX`.
#[nve_hybrid_enum]
#[repr(u32)]
pub enum OverlayKind {
    Caption = 0,
    Chapter = 3,
    Ad = 7,
    Max = 4294967295,
}

/// Non-POD record: a `Vec`, an `Option`, a dynamic `NveValue`, and an enum, plus a `u32` field
/// named after a Kotlin hard keyword (`when`).
#[nve_hybrid_struct]
pub struct Overlay {
    pub kind: OverlayKind,
    pub cues: Vec<i32>,
    pub caption: Option<String>,
    pub extra: NveValue,
    pub labels: Vec<String>,
    pub when: u32,
}

/// Props covering every crossing: scalars, encoded `Struct` / `Vec`, and `NveValue`.
#[nve_hybrid_view(name = "OverlayView")]
pub trait OverlayViewSpec {
    #[prop]
    fn title(&self) -> String;
    #[prop(default = OverlayKind::Chapter)]
    fn kind(&self) -> OverlayKind;
    #[prop(default = None)]
    fn overlay(&self) -> Option<Overlay>;
    #[prop]
    fn overlays(&self) -> Vec<Overlay>;
    #[prop]
    fn cues(&self) -> Vec<i32>;
    #[prop]
    fn extra(&self) -> NveValue;
    #[prop(default = 4294967295)]
    fn limit(&self) -> u32;
    #[prop(default = 18446744073709551615)]
    fn big(&self) -> u64;
    #[method]
    fn shift(&self, base: u32, object: Option<i32>) -> u32;
    #[method]
    fn current_overlay(&self) -> Option<Overlay>;
    #[method]
    fn current_overlays(&self) -> Vec<Overlay>;
    #[method]
    fn current_extra(&self) -> NveValue;
    #[method]
    fn summary(&self) -> String;
    #[method]
    fn echo(&self, overlay: Overlay) -> Result<Overlay, String>;
    #[event]
    fn on_cue(&self, index: i32, overlay: Overlay);
}
