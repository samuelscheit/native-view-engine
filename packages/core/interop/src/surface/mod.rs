//! Cross-framework surface hierarchy, environment/theme cascading, and lifecycle orchestration.

mod orchestrator;
mod types;

pub use orchestrator::SurfaceOrchestrator;
pub use types::{
    ColorScheme, ContrastLevel, EnvironmentOverrides, FrameworkKind, LifecycleState,
    SafeAreaInsets, SurfaceDescriptor, SurfaceEnvironment, SurfaceEvent, SurfaceRecord,
    TeardownRecord, ThemeContext,
};
#[cfg(feature = "window")]
pub use types::SurfacePresentation;
