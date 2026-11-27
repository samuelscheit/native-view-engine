// Kotlin-only hybrid edge cases: a parameter named `handle`, and fields / parameters named after
// Kotlin hard keywords (`object`, `val`, `is`, `when`).

#[nve_hybrid_struct]
pub struct Keywords {
    pub object: u32,
    pub val: Option<String>,
    pub is: Vec<bool>,
}

#[nve_hybrid_module(name = "Edge")]
pub trait EdgeSpec {
    fn shift(&self, handle: u32, object: Option<i32>) -> u32;
    fn keywords(&self, when: Keywords) -> Keywords;
}
