# `nve-dioxus-macros`

Zero-dependency proc-macro wrapper around `dioxus_core_macro::rsx!` that stamps compile-time
template coordinates (`"__nve_tpl": "<rel_file>#<ordinal>@<path>"`) onto native RSX elements in
debug builds (`#[cfg(debug_assertions)]`) so Tier-1 binary `RowPatch` hot reload can address any
un-tagged (`test_id: None`) element or `for`-loop instance with zero runtime overhead in release
builds.

## Design Invariants

- Zero external dependencies (`proc_macro` + `std` only), keeping compile times minimal and
  satisfying `[INV:core-zero-dioxus]`.
- Emits `#[cfg(debug_assertions)]` / `#[cfg(not(debug_assertions))]` in the expanded tokens so the
  target crate's build profile controls instrumentation (release builds receive the untouched input).
- Template ordinal numbering mirrors `nve-cli`'s `rsx_diff` parser 1-to-1 across `rsx!`, `if`,
  `else if`, `else`, and `for` bodies.
