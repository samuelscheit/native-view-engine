#![cfg_attr(doctest, doc = include_str!("../README.md"))]
//! Windows UI Automation (`accesskit_windows`) C-ABI bridge and `NveSceneView` mirror for
//! `hosts/windows-shell`.

pub mod action;
pub mod bridge;
pub mod ffi;
mod table_sync;

pub use action::{with_ffi_action, NveA11yWinAction, NveA11yWinActionKind};
pub use bridge::{NoopResources, ResourceReader, WinA11yBridge};
