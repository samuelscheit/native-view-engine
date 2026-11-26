#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

pub struct TempDirGuard(PathBuf);

impl TempDirGuard {
    pub fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "nve-engine-loader-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn dylib_ext() -> &'static str {
    if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    }
}

pub fn compile_test_cdylib(dir: &Path, name: &str, rust_source: &str) -> PathBuf {
    let src_path = dir.join(format!("{name}.rs"));
    let out_path = dir.join(format!("lib{name}.{}", dylib_ext()));
    std::fs::write(&src_path, rust_source).expect("write test cdylib source");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let output = ProcessCommand::new(rustc)
        .args([
            "--crate-type=cdylib",
            "--edition=2024",
            "-C",
            "opt-level=0",
            "-o",
        ])
        .arg(&out_path)
        .arg(&src_path)
        .output()
        .expect("invoke rustc for test cdylib");
    assert!(
        output.status.success(),
        "rustc failed for {name}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    out_path
}

pub const SHARED_ABI_DEFS: &str = r#"
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveBufferSlice { pub ptr: *const u8, pub len: usize }

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveStrSlice { pub ptr: *const u8, pub len: usize }

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveValue {
    pub tag: u32,
    pub _pad: u32,
    pub payload: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveMapEntry {
    pub key: NveStrSlice,
    pub value: NveValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveDirectEvent {
    pub surface_id: u32,
    pub node_id: u32,
    pub timestamp_us: u64,
    pub kind: u8,
    pub payload_tag: u8,
    pub modifiers: u8,
    pub event_flags: u8,
    pub key_code: u32,
    pub name: NveStrSlice,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text: NveStrSlice,
    pub value: NveValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveEventBatch {
    pub ptr: *const NveDirectEvent,
    pub count: u32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveSizeConstraints { pub min_w: f32, pub min_h: f32, pub max_w: f32, pub max_h: f32 }

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveIntrinsicSize { pub width: f32, pub height: f32 }

#[repr(C)]
pub struct NveSurfaceApi {
    pub create: unsafe extern "C" fn(u32) -> *mut c_void,
    pub destroy: unsafe extern "C" fn(*mut c_void),
    pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
    pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveEventBatch) -> u32,
}

#[repr(C)]
pub struct NveLayoutApi {
    pub measure: unsafe extern "C" fn(*mut c_void, NveSizeConstraints) -> NveIntrinsicSize,
    pub hit_test: unsafe extern "C" fn(*mut c_void, f32, f32, u8) -> u8,
}

#[repr(C)]
pub struct NveComponentApi {
    pub set_props: unsafe extern "C" fn(*mut c_void, *const NveMapEntry, u32) -> u32,
    pub drain_emitted_events: unsafe extern "C" fn(*mut c_void) -> NveEventBatch,
}

#[repr(C)]
pub struct NveDevApi {
    pub apply_hot_patch: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
    pub snapshot_state: unsafe extern "C" fn(*mut c_void) -> NveBufferSlice,
    pub restore_state: unsafe extern "C" fn(*mut c_void, NveBufferSlice),
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveHostCallbacks {
    pub host_ctx: *mut c_void,
    pub invoke: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u64, NveBufferSlice, *mut u8, usize, *mut usize) -> i32>,
    pub channel_publish: Option<unsafe extern "C" fn(*mut c_void, u32, u32, NveBufferSlice) -> i32>,
}

#[repr(C)]
pub struct NveHostApi {
    pub bind_host: unsafe extern "C" fn(*mut c_void, *const NveHostCallbacks),
    pub dispatch_host_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
}
"#;
