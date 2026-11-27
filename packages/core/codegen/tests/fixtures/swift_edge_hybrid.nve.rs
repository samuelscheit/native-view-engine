use nve_abi::NveValue;

/// Sparse discriminants: an unknown wire value must fail to decode.
#[nve_hybrid_enum]
#[repr(u32)]
pub enum Level {
    Low = 0,
    High = 7,
}

/// POD struct whose C fields are Swift member-access keywords (`raw.init` needs backticks).
#[nve_hybrid_struct]
pub struct Keywords {
    pub init: u32,
    pub deinit: f32,
    pub subscript: bool,
}

/// Sibling sequences (their codec scopes must not collide), nested bytes, and extreme scalars.
#[nve_hybrid_struct]
pub struct Bundle {
    pub ints: Vec<i32>,
    pub names: Vec<String>,
    pub blobs: Vec<Vec<u8>>,
    pub not_a_number: f64,
    pub min: i64,
    pub level: Level,
}

/// Swift client shapes that only compile when every nested scope returns, sibling sequences are
/// scoped, and C member names are escaped.
#[nve_hybrid_module(name = "Edge")]
pub trait EdgeSpec {
    fn mix(&self, a: &str, b: &[u8], meta: NveValue, extra: NveValue) -> String;
    fn pair(&self, xs: Vec<i32>, ys: Vec<String>) -> Bundle;
    fn echo_bundle(&self, bundle: Bundle) -> Bundle;
    fn init(&self, k: Keywords) -> Keywords;
    fn level(&self, raw: u32) -> Level;
    fn fail(&self, code: i32) -> Result<(), String>;
    fn junk(&self, kind: u32) -> Bundle;
}
