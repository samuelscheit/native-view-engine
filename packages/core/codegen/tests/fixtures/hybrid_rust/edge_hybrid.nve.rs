// Every codec shape (POD with fixed-size arrays, nested `Option` / `Vec`, `Vec<Bytes>`, `Value`,
// narrow integers, enums, arrays of non-`Copy` elements and of more than 32 elements), POD structs
// that cross as raw carriers (an enum without a 0 variant, `bool`), plus parameter / field / event
// / method names that shadow the trampolines' and emitters' own bindings (`handle`, `out_ok`,
// `out_err`, `instance`, `entries`, `props`, `surface_id`, `sink`, `value`, `fields`, `m`,
// `default`).
use nve_abi::NveValue;

#[nve_hybrid_enum]
#[repr(u32)]
pub enum Tone {
    Low = 0,
    High = 9,
}

/// No 0 variant: a zeroed C value is not a valid `Phase`.
#[nve_hybrid_enum]
#[repr(u32)]
pub enum Phase {
    Active = 1,
    Paused = 2,
}

#[nve_hybrid_struct]
pub struct Rgba {
    pub channels: [f32; 4],
    pub tone: Tone,
}

#[nve_hybrid_struct]
pub struct Pulse {
    pub phase: Phase,
    pub live: bool,
    pub level: u8,
}

#[nve_hybrid_struct]
pub struct Frame {
    pub status: Pulse,
    pub history: [Phase; 2],
    pub flags: [bool; 3],
    pub pad: [u8; 40],
    pub grid: [[bool; 33]; 2],
}

#[nve_hybrid_struct]
pub struct Node {
    pub name: String,
    pub depth: u16,
    pub small: u8,
    pub size: usize,
    pub weight: f64,
    pub color: Rgba,
    pub accent: Option<Rgba>,
    pub children: Vec<Option<Vec<i32>>>,
    pub blobs: Vec<Vec<u8>>,
    pub extra: NveValue,
    pub m: Option<u64>,
    pub fields: Vec<Tone>,
    pub pair: [String; 2],
    pub digest: [u8; 64],
}

#[nve_hybrid_module(name = "Shadow")]
pub trait ShadowSpec {
    fn shadow(&self, handle: i64, out_err: &str, out_ok: &[u8], instance: Node, entries: u32) -> Result<Node, String>;
    fn pick(&self, tone: Option<Tone>, color: Option<Rgba>) -> Option<Tone>;
    fn echo_value(&self, value: NveValue) -> Result<NveValue, String>;
    fn tones(&self, tones: Vec<Tone>) -> Vec<Tone>;
    fn strict(&self, tone: Tone) -> u32;
    fn fail_always(&self, nodes: Vec<Node>) -> Result<(), String>;
    fn default(&self) -> u32;
    fn check(&self, status: Pulse, maybe: Option<Pulse>) -> Result<Option<Pulse>, String>;
    fn advance(&self, frame: Frame) -> Frame;
    fn toggle(&self, on: bool) -> Result<bool, String>;
}

#[nve_hybrid_view(name = "Canvas")]
pub trait CanvasSpec {
    #[prop]
    fn node(&self) -> Node;
    #[prop(default = None)]
    fn label(&self) -> Option<String>;
    #[prop]
    fn layers(&self) -> Vec<Node>;
    #[prop]
    fn extra(&self) -> NveValue;
    #[prop]
    fn seed(&self) -> [u8; 48];
    #[event]
    fn on_change(&self, surface_id: u32, sink: Node, values: Vec<Option<String>>, extra: NveValue, maybe: Option<Rgba>, tone: Tone);
    #[event]
    fn on_ping(&self);
    #[event]
    fn on_blob(&self, digest: [u8; 33], size: usize);
    #[method]
    fn snapshot(&self, props: u32) -> Node;
}

#[nve_hybrid_view(name = "Blank")]
pub trait BlankSpec {
    #[event]
    fn on_tap(&self, x: f32);
}
