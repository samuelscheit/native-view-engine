//! Snapshot-verified compile-fail UI tests (`trybuild`) for `#[derive(NveTable)]` and
//! `#[derive(WordEnum)]` diagnostics on stable `rustc` (exact error codes & messages).

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn macro_compile_fail_diagnostics() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
