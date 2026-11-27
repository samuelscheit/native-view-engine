//! Cross-framework touch and nested-scroll arbitration across framework boundaries.

mod arbitrator;
#[cfg(test)]
mod gesture_tests;
mod types;

pub use arbitrator::GestureArbitrator;
pub use types::{
    ArbitrationDecision, ArbitrationReason, GestureCapabilities, GestureDisposition, PointerPhase,
    PointerSample, ScrollAxes, ScrollEdgeState,
};
