use nve_abi::NveValue;

/// Supported cryptographic hash algorithms.
#[nve_hybrid_enum]
#[repr(u32)]
pub enum HashAlgorithm {
    Blake3 = 0,
    Sha256 = 1,
    Sha512 = 2,
}

/// Video playback quality tier.
#[nve_hybrid_enum]
#[repr(u32)]
pub enum VideoQuality {
    Auto = 0,
    Sd480,
    Hd1080 = 10,
    Uhd4k,
}

/// Scalar-only POD point struct (passed directly by value across C ABI).
#[nve_hybrid_struct]
pub struct FramePoint {
    pub x: f32,
    pub y: f32,
}

/// Nested POD struct embedding another POD struct and an enum.
#[nve_hybrid_struct]
pub struct DigestHeader {
    pub algorithm: HashAlgorithm,
    pub byte_len: u32,
    pub verified: bool,
    pub anchor: FramePoint,
}

/// Non-POD owned record containing strings, bytes, options, vectors, and nested structs.
#[nve_hybrid_struct]
pub struct SignedPayload {
    pub header: DigestHeader,
    pub label: String,
    pub digest: Vec<u8>,
    pub tags: Vec<String>,
    pub nonce: Option<u64>,
}

/// A record whose field names shadow the generated codec's parameters and locals (`w`, `mark`,
/// `r`, `count`), with a nested sequence.
#[nve_hybrid_struct]
pub struct Cursor {
    pub mark: u32,
    pub w: Vec<u8>,
    pub r: Option<String>,
    pub count: u32,
    pub grid: Vec<Vec<i32>>,
}

/// High-performance binary native crypto module.
#[nve_hybrid_module(name = "Crypto")]
pub trait CryptoSpec {
    fn version(&self) -> String;
    fn add(&self, a: i64, b: i64) -> i64;
    fn blake3(&self, data: &[u8]) -> Vec<u8>;
    fn hash_with(&self, algo: HashAlgorithm, data: &[u8]) -> DigestHeader;
    fn verify_payload(&self, payload: SignedPayload) -> Result<bool, String>;
    fn find_nonce(&self, label: &str, hint: Option<u64>) -> Option<u64>;
    fn lookup_payload(&self, id: u32) -> Option<SignedPayload>;
    fn batch_sign(
        &self,
        inputs: Vec<String>,
        algo: HashAlgorithm,
    ) -> Result<Vec<SignedPayload>, String>;
    fn inspect_dynamic(&self, meta: NveValue) -> NveValue;
    fn transpose(&self, cursor: Cursor) -> Cursor;
    fn reset_cache(&mut self) -> Result<(), String>;
}

/// Native hardware-accelerated video surface view.
#[nve_hybrid_view(name = "VideoSurface")]
pub trait VideoSurfaceSpec {
    #[prop]
    fn source_url(&self) -> String;

    #[prop(default = false)]
    fn muted(&self) -> bool;

    #[prop(default = 1.0)]
    fn playback_rate(&self) -> f32;

    #[prop(default = VideoQuality::Auto)]
    fn quality(&self) -> VideoQuality;

    #[prop(default = None)]
    fn poster_url(&self) -> Option<String>;

    #[prop(default = None)]
    fn focus_point(&self) -> Option<FramePoint>;

    #[event]
    fn on_progress(&self, position_ms: u64, duration_ms: u64);

    #[event]
    fn on_error(&self, code: i32, message: String);

    #[method]
    fn seek(&mut self, position_ms: u64);

    #[method]
    fn capture_frame(&self, at_point: FramePoint) -> Result<Vec<u8>, String>;
}
