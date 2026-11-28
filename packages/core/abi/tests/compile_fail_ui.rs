//! Snapshot-verified compile-fail UI tests (`trybuild`) for `#[derive(NveAbi)]` (vtables,
//! transparent handles) and `#[derive(ReprEnum)]` diagnostics on stable `rustc`.

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn macro_compile_fail_diagnostics() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
