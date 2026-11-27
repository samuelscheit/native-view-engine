//! Framework-agnostic host runtime for `native-view-engine`.
//!
//! Coordinates `NodeStore`, `LaidOutScene<DynLayoutEngine>`, `WakeBridge`, generic
//! [`FrameTickHook`]s, zero-copy [`nve_abi::NveRuntimeSceneView`] publication, and the single
//! versioned C ABI entry point [`nve_runtime_get_interface`].

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod app;
pub mod builder;
pub mod c_api;
mod component;
mod lock;
mod pair;
mod state;
pub mod surface;
pub use nve_engine::wake;

pub use app::{AppFrame, CAbiAppAdapter, FrameTickHook, NveAppAdapter, RuntimeApp};
pub use builder::{RuntimeBuilder, RuntimeError};
pub use c_api::nve_runtime_get_interface;
pub use surface::{SceneReadGuard, SurfaceRuntime};
pub use wake::{WakeBridge, WakeCallback, WakeToken};
