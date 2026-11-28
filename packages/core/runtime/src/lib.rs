//! Framework-agnostic host runtime for `native-view-engine`.
//!
//! Coordinates `NodeStore`, `LaidOutScene<DynLayoutEngine>`, `WakeBridge`, generic
//! [`FrameTickHook`]s, zero-copy [`nve_abi::NveSceneView`] publication, and the single
//! versioned C ABI entry point [`nve_runtime_get_interface`].

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod app;
mod app_files;
pub mod builder;
pub mod c_api;
mod delta;
mod hooks;
pub mod host_router;
mod lock;
mod pair;
mod publish;
mod state;
pub mod surface;
pub use nve_engine::wake;

pub use app::{AppFrame, CAbiAppAdapter, FrameTickHook, NveAppAdapter, RuntimeApp};
#[cfg(not(target_arch = "wasm32"))]
pub use app_files::MODULE_STAGING_DIR;
pub use builder::{RuntimeBuilder, RuntimeError};
pub use c_api::nve_runtime_get_interface;
pub use hooks::IntrinsicSizeCallback;
pub use host_router::RuntimeHostRouter;
#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub use pair::PairReplicaClient;
pub use surface::{SceneReadGuard, SurfaceRuntime};
pub use wake::{WakeBridge, WakeCallback, WakeToken};
