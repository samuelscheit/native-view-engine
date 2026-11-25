//! Deterministic, non-cryptographic `FxHash` maps shared by every engine crate.
//!
//! std's default `RandomState` links SipHash-1-3, the per-thread random key cache and its lazy
//! init (getrandom on wasm32) into every binary. Engine maps are keyed by trusted dense ids
//! (`NodeId`, `ElementId`, `SurfaceId`, op hashes), so HashDoS resistance buys nothing. Construct
//! with `HashMap::default()` / `with_capacity_and_hasher(n, Default::default())`.
pub use rustc_hash::FxBuildHasher;

pub type HashMap<K, V> = std::collections::HashMap<K, V, FxBuildHasher>;
pub type HashSet<K> = std::collections::HashSet<K, FxBuildHasher>;
