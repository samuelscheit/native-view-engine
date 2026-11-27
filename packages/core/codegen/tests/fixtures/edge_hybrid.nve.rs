// Kotlin-only hybrid edge cases: a parameter named `handle` (the Rust target cannot compile it yet;
// it moves to the shared fixtures with commit-5a-rust), fields / parameters named after Kotlin hard
// keywords (`object`, `val`, `is`, `when`), and boundary integer prop defaults.

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

/// Boundary integer defaults: each must be a Kotlin `const`-able literal with the exact bits.
#[nve_hybrid_view(name = "EdgeView")]
pub trait EdgeViewSpec {
    #[prop(default = -9223372036854775808)]
    fn min_long(&self) -> i64;
    #[prop(default = -2147483648)]
    fn min_int(&self) -> i32;
    #[prop(default = 7)]
    fn size(&self) -> usize;
    #[prop(default = 4294967295)]
    fn max_u32(&self) -> u32;
}
