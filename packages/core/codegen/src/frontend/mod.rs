//! Input specification parsers: React Native specs (`rn_spec`) and declarative hybrid Rust specs
//! (`hybrid_spec`, on `syn` 2).

pub mod hybrid_spec;
pub mod rn_spec;

pub use hybrid_spec::{parse_hybrid_rust_spec, parse_hybrid_rust_spec_file};
pub use rn_spec::{parse_rn_spec, parse_rn_spec_file};
