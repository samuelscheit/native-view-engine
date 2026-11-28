//! Real React Native Yoga C++20 [`StoreLayoutEngine`] implementation for `native-view-engine`.
//!
//! When built with `RN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon`,
//! `build.rs` compiles React Native's `yogacore` C++20 sources and links [`YogaLayoutEngine`]
//! directly to Yoga with full `YGNodeSetMeasureFunc` -> Rust [`TextMeasurer`] callbacks,
//! reading directly from `NodeStore` word tables (`Layout`, `TextContent`, `Typography`).
//!
//! When built without `RN_REACT_COMMON_DIR`, the crate still compiles cleanly on every machine
//! with [`YogaLayoutEngine::is_available()`] returning `false` and [`YogaLayoutEngine::new()`]
//! returning [`YogaUnavailableError`].

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

#[cfg(nve_has_rn_yoga)]
mod ffi;
#[cfg(nve_has_rn_yoga)]
mod measure;
#[cfg(nve_has_rn_yoga)]
mod measure_root;
#[cfg(nve_has_rn_yoga)]
mod real;

use std::fmt;

/// Error returned by [`YogaLayoutEngine::new`] when `nve-layout-yoga` was compiled without
/// `RN_REACT_COMMON_DIR` pointing to a React Native `ReactCommon/yoga` checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YogaUnavailableError {
    message: &'static str,
}

impl YogaUnavailableError {
    pub const fn new() -> Self {
        Self {
            message: "React Native Yoga was not compiled into nve-layout-yoga; set RN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon and rebuild",
        }
    }

    #[cfg(nve_has_rn_yoga)]
    pub(crate) const fn with_message(message: &'static str) -> Self {
        Self { message }
    }

    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl Default for YogaUnavailableError {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for YogaUnavailableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for YogaUnavailableError {}

#[cfg(not(nve_has_rn_yoga))]
mod unavailable_impl {
    use std::sync::Arc;

    use nve_engine::{LayoutRect, Viewport};
    use nve_layout::{SizeConstraints, StoreLayoutEngine, SurfaceMeasure, TextMeasurer};
    use nve_store::{ChangeSet, NodeId, NodeStore};

    use crate::YogaUnavailableError;

    /// Stub handle compiled when `RN_REACT_COMMON_DIR` was not set at build time.
    pub struct YogaLayoutEngine {
        _uninhabited: std::convert::Infallible,
    }

    impl YogaLayoutEngine {
        pub const fn is_available() -> bool {
            false
        }

        pub fn new() -> Result<Self, YogaUnavailableError> {
            Err(YogaUnavailableError::new())
        }

        pub fn with_text_measurer(
            _text_measurer: Arc<dyn TextMeasurer>,
        ) -> Result<Self, YogaUnavailableError> {
            Err(YogaUnavailableError::new())
        }
    }

    impl StoreLayoutEngine for YogaLayoutEngine {
        fn name(&self) -> &'static str {
            match self._uninhabited {}
        }

        fn sync_changes(&mut self, _store: &NodeStore, _changes: &ChangeSet) {
            match self._uninhabited {}
        }

        fn compute_layout(
            &mut self,
            _store: &NodeStore,
            _root: NodeId,
            _viewport: Viewport,
        ) -> bool {
            match self._uninhabited {}
        }

        fn measure_root(
            &mut self,
            _store: &NodeStore,
            _root: NodeId,
            _constraints: SizeConstraints,
            _ideal: bool,
        ) -> SurfaceMeasure {
            match self._uninhabited {}
        }

        fn layout_of(&self, _id: NodeId) -> Option<LayoutRect> {
            match self._uninhabited {}
        }

        fn set_text_measurer(&mut self, _measurer: Arc<dyn TextMeasurer>) {
            match self._uninhabited {}
        }

        fn reset(&mut self) {
            match self._uninhabited {}
        }
    }
}

#[cfg(nve_has_rn_yoga)]
pub use real::YogaLayoutEngine;
#[cfg(not(nve_has_rn_yoga))]
pub use unavailable_impl::YogaLayoutEngine;

#[cfg(test)]
mod tests {
    use super::*;
    use nve_layout::StoreLayoutEngine;

    #[test]
    fn availability_matches_constructor() {
        if YogaLayoutEngine::is_available() {
            let engine = YogaLayoutEngine::new().expect("real RN Yoga should initialize");
            assert_eq!(engine.name(), "yoga");
        } else {
            let err = YogaLayoutEngine::new()
                .err()
                .expect("expected YogaUnavailableError");
            assert!(err.to_string().contains("RN_REACT_COMMON_DIR"));
        }
    }
}
