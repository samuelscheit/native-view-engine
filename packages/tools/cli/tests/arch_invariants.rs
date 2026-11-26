//! Mechanical Architectural Invariant Suite for `native-view-engine` (Track 1B-1c).
//!
//! Enforces crate DAG boundaries (via `cargo metadata --format-version 1 --offline --locked`),
//! generated-file & git hygiene, production file size ratchets, ABI/type invariants, legacy
//! regression bans, zero-boilerplate macro hygiene, hot-path performance rules, hashing/codegen
//! discipline, and platform purity using only `std`.
//!
//! Staged checks behind `ENFORCE_POST_*` run their real scanners on every test invocation and
//! are verified against synthetic positive/negative cases today. Flip the corresponding `const`
//! to `true` (or run with `NVE_STRICT_INVARIANTS=1` / `NVE_STRICT_INVARIANTS=nodestore,1b1d,1b1e`)
//! to make each stage a hard failure as each track lands.

#[path = "arch_invariants/content_purity.rs"]
mod content_purity;
#[path = "arch_invariants/json_meta.rs"]
mod json_meta;
#[path = "arch_invariants/rust_items.rs"]
mod rust_items;
#[path = "arch_invariants/rust_lex.rs"]
mod rust_lex;
#[path = "arch_invariants/support.rs"]
mod support;

/// Test modules grouped under `arch_inv::` so `scripts/check-all.sh` Suite 0 runs
/// `cargo test --locked -p nve-cli --test arch_invariants` once and Suite 1 skips duplicates
/// via `cargo test --workspace -- --skip arch_inv::`.
#[path = "arch_invariants"]
mod arch_inv {
    pub mod bench_and_alloc;
    pub mod crate_dag;
    pub mod dag_and_hygiene;
    pub mod enum_and_macro_hygiene;
    pub mod perf_hashing_codegen_purity;
    pub mod types_abi_and_regressions;
    pub mod unit_tests;
}

/// Flip to `true` when the `NodeStore` / `NVE2` / `nve-hash` branch lands on `main`.
pub const ENFORCE_POST_NODESTORE: bool = false;

/// Flip to `true` when Track 1B-1d (`NveEvent`, macro upgrades, core/motion purity) lands.
pub const ENFORCE_POST_1B1D: bool = false;

/// Flip to `true` when Track 1B-1e (codegen templates + static foreign files) lands.
pub const ENFORCE_POST_1B1E: bool = false;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    PostNodeStore,
    Post1B1D,
    Post1B1E,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::PostNodeStore => "post-NodeStore (ENFORCE_POST_NODESTORE)",
            Self::Post1B1D => "post-1B-1d (ENFORCE_POST_1B1D)",
            Self::Post1B1E => "post-1B-1e (ENFORCE_POST_1B1E)",
        }
    }

    pub fn is_enforced(self) -> bool {
        let compiled = match self {
            Self::PostNodeStore => ENFORCE_POST_NODESTORE,
            Self::Post1B1D => ENFORCE_POST_1B1D,
            Self::Post1B1E => ENFORCE_POST_1B1E,
        };
        if compiled {
            return true;
        }
        let Ok(raw) = std::env::var("NVE_STRICT_INVARIANTS") else {
            return false;
        };
        let norm = raw.trim().to_ascii_lowercase();
        if matches!(norm.as_str(), "1" | "true" | "all" | "yes") {
            return true;
        }
        let needle = match self {
            Self::PostNodeStore => "nodestore",
            Self::Post1B1D => "1b1d",
            Self::Post1B1E => "1b1e",
        };
        norm.split(',').any(|tok| tok.trim() == needle)
    }
}
