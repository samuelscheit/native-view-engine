//! Declarative single-source specification for NVE's Core C ABI (`nve_abi.generated.h`,
//! `nve_core_abi.generated.dart`, `generated/NveCoreAbi.java`, `nve_core_abi.generated.ts`).
//!
//! Dogfoods NVE's own Nitro-style Rust-first Hybrid FFI parser & generator
//! (`crates/nve-codegen/src/hybrid/`).

#[nve_hybrid_namespace(name = "NveCoreAbi")]

/// Wire magic bytes (`"NVE1"` little-endian).
#[nve_hybrid_const]
pub const NVE_ABI_MAGIC: u32 = 0x3145564E;

/// Binary `CommandBuffer` ABI version.
#[nve_hybrid_const]
pub const NVE_ABI_VERSION: u32 = 1;

/// Reserved root node ID for every surface.
#[nve_hybrid_const]
pub const NVE_ROOT_NODE_ID: u32 = 1;

/// Maximum permitted container nesting depth for `NveBinValue`.
#[nve_hybrid_const]
pub const NVE_BIN_VALUE_MAX_DEPTH: u32 = 64;

/// Binary `NveHostEventBatch` wire magic (`"NVH1"` little-endian).
#[nve_hybrid_const]
pub const NVE_HOST_EVENT_MAGIC: u32 = 0x3148564E;

/// Binary `NveHostEventBatch` wire version.
#[nve_hybrid_const]
pub const NVE_HOST_EVENT_VERSION: u32 = 1;

/// Flag bit on `channel_publish` / `ChannelMessage` requesting retained channel state.
#[nve_hybrid_const]
pub const NVE_CHANNEL_FLAG_RETAIN: u32 = 1;

/// Opaque handle to an instantiated NVE app/component.
#[nve_hybrid_alias]
pub type NveAppHandle = *mut c_void;

/// Opaque handle to a C++20 `nve-fabric-bridge` host instance.
#[nve_hybrid_alias]
pub type NveFabricHostHandle = *mut c_void;

/// Host capability invocation callback function pointer (`NveHostCallbacks::invoke`).
#[nve_hybrid_alias]
pub type NveHostInvokeFn = *const c_void;

/// Host channel-bus publish callback function pointer (`NveHostCallbacks::channel_publish`).
#[nve_hybrid_alias]
pub type NveHostChannelPublishFn = *const c_void;

/// Binary CommandBuffer wire opcodes.
#[nve_hybrid_enum(c_prefix = "NVE_OP_")]
#[repr(u8)]
pub enum NveOpcode {
    CreateNode = 1,
    RemoveNode = 2,
    SetChildren = 3,
    SetText = 4,
    SetLayout = 5,
    SetStyle = 6,
    SetRawProp = 7,
    SetEventMask = 8,
    DispatchCommand = 9,
    Commit = 10,
}

/// Built-in and custom component discriminants.
#[nve_hybrid_enum(c_prefix = "NVE_COMP_")]
#[repr(u16)]
pub enum NveComponentKind {
    Root = 0,
    View = 1,
    Text = 2,
    ScrollView = 3,
    Image = 4,
    TextInput = 5,
    Pressable = 6,
    SkiaCanvas = 7,
    Custom = 0xFFFF,
}

/// Gesture arbitration disposition returned by `NveLayoutApi::hit_test`.
#[nve_hybrid_enum(c_prefix = "NVE_GESTURE_", dart_fallback = "PassThrough")]
#[repr(u8)]
pub enum NveGestureDisposition {
    Claim = 1,
    PassThrough = 2,
    DeferToParent = 3,
}

/// Discriminant tags shared between `NveValue::tag` and binary `NveBinValue`.
#[nve_hybrid_enum(c_prefix = "NVE_VAL_")]
#[repr(u32)]
pub enum NveValueTag {
    Null = 0,
    Bool = 1,
    I64 = 2,
    F64 = 3,
    Str = 4,
    Bytes = 5,
    Array = 6,
    Map = 7,
}

/// Native event discriminator matching `NativeEventKind` in `crates/nve-abi/src/events.rs`.
#[nve_hybrid_enum(c_prefix = "NVE_EVENT_", dart_fallback = "Custom")]
#[repr(u8)]
pub enum NveEventKind {
    Press = 1,
    LongPress = 2,
    Scroll = 3,
    TextChange = 4,
    Focus = 5,
    Blur = 6,
    Layout = 7,
    Custom = 8,
}

/// Event payload wire tag matching `EventPayload` in `crates/nve-abi/src/events.rs`.
#[nve_hybrid_enum(c_prefix = "NVE_EVENT_PAYLOAD_")]
#[repr(u8)]
pub enum NveEventPayloadTag {
    Empty = 0,
    Point = 1,
    Scroll = 2,
    Text = 3,
    Layout = 4,
    Custom = 5,
}

/// Borrowed immutable byte slice across the C ABI.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 16)]
pub struct NveBufferSlice {
    pub ptr: *const u8,
    pub len: usize,
}

/// Borrowed UTF-8 string slice across the C ABI.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 16)]
pub struct NveStrSlice {
    pub ptr: *const u8,
    pub len: usize,
}

/// Contiguous slice of `NveValue` items.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 16)]
pub struct NveValueArray {
    pub ptr: *const NveValue,
    pub len: usize,
}

/// Contiguous slice of `NveMapEntry` key/value pairs.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 16)]
pub struct NveValueMap {
    pub ptr: *const NveMapEntry,
    pub len: usize,
}

/// Untagged union payload inside `NveValue`.
#[repr(C)]
#[nve_hybrid_union(size_64 = 16)]
pub union NveValueData {
    pub bool_val: u8,
    pub i64_val: i64,
    pub f64_val: f64,
    pub str_val: NveStrSlice,
    pub bytes_val: NveBufferSlice,
    pub array_val: NveValueArray,
    pub map_val: NveValueMap,
}

/// Zero-copy tagged value passed directly across C/C++/Rust/Swift/Dart FFI.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 24)]
pub struct NveValue {
    /// `NveValueTag` discriminant.
    pub tag: u32,
    pub data: NveValueData,
}

/// Key/value entry inside `NveValueMap` or `NveComponentApi::set_props_direct`.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 40)]
pub struct NveMapEntry {
    pub key: NveStrSlice,
    pub value: NveValue,
}

/// Zero-copy direct native event record matching `crates/nve-abi/src/value.rs`.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 96)]
pub struct NveDirectEvent {
    #[offset(0)]
    pub surface_id: u32,
    #[offset(4)]
    pub node_id: u32,
    #[offset(8)]
    pub timestamp_us: u64,
    /// `NativeEventKind` (1..8).
    #[offset(16)]
    pub kind: u8,
    /// `NveEventPayloadTag` (0..5).
    #[offset(17)]
    pub payload_tag: u8,
    #[offset64(24)]
    pub name: NveStrSlice,
    #[offset64(40)]
    pub x: f32,
    #[offset64(44)]
    pub y: f32,
    #[offset64(48)]
    pub width: f32,
    #[offset64(52)]
    pub height: f32,
    #[offset64(56)]
    pub text: NveStrSlice,
    #[offset64(72)]
    pub value: NveValue,
}

/// Borrowed batch of direct events passed to `NveSurfaceApi::dispatch_events_direct`.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 16)]
pub struct NveEventBatch {
    pub ptr: *const NveDirectEvent,
    pub len: u32,
}

/// Surface viewport configuration passed to `nve_fabric_host_create`.
#[repr(C)]
#[nve_hybrid_struct(size = 16)]
pub struct NveSurfaceConfig {
    #[offset(0)]
    pub surface_id: u32,
    #[offset(4)]
    pub width: f32,
    #[offset(8)]
    pub height: f32,
    #[offset(12)]
    pub scale_factor: f32,
}

/// Per-frame timing and mutation counters populated by `nve_fabric_host_submit_frame`.
#[repr(C)]
#[nve_hybrid_struct(size = 40)]
pub struct NveFrameStats {
    #[offset(0)]
    pub op_count: u32,
    #[offset(4)]
    pub mutation_count: u32,
    #[offset(8)]
    pub decode_us: u64,
    #[offset(16)]
    pub layout_us: u64,
    #[offset(24)]
    pub diff_us: u64,
    #[offset(32)]
    pub mount_us: u64,
}

/// Box layout constraints passed to `NveLayoutApi::measure`.
#[repr(C)]
#[nve_hybrid_struct(size = 16)]
pub struct NveSizeConstraints {
    #[offset(0)]
    pub min_w: f32,
    #[offset(4)]
    pub min_h: f32,
    #[offset(8)]
    pub max_w: f32,
    #[offset(12)]
    pub max_h: f32,
}

/// Measured intrinsic dimensions returned by `NveLayoutApi::measure`.
#[repr(C)]
#[nve_hybrid_struct(size = 8)]
pub struct NveIntrinsicSize {
    #[offset(0)]
    pub width: f32,
    #[offset(4)]
    pub height: f32,
}

/// Host-provided zero-JSON callback table bound via `NveHostApi::bind_host`.
#[repr(C)]
#[nve_hybrid_struct(size_64 = 24)]
pub struct NveHostCallbacks {
    #[offset64(0)]
    pub host_ctx: *mut c_void,
    #[offset64(8)]
    pub invoke: NveHostInvokeFn,
    #[offset64(16)]
    pub channel_publish: NveHostChannelPublishFn,
}

/// Required core surface interface (`NVE_IFACE_SURFACE = 1`).
#[nve_hybrid_module(name = "Surface", interface_id = 1, c_name = "NveSurfaceApi")]
pub trait NveSurfaceApiSpec {
    fn create(surface_id: u32) -> NveAppHandle;
    fn destroy(&mut self);
    fn render(&mut self, frame_id: u64) -> NveBufferSlice;
    fn dispatch_events(&mut self, events: NveBufferSlice) -> u32;
    /// Optional zero-copy direct event dispatch (may be NULL).
    #[optional]
    fn dispatch_events_direct(&mut self, batch: NveEventBatch) -> u32;
}

/// Optional layout & gesture interface (`NVE_IFACE_LAYOUT = 2`). Zero-serialization register calls.
#[nve_hybrid_module(name = "Layout", interface_id = 2, c_name = "NveLayoutApi")]
pub trait NveLayoutApiSpec {
    fn measure(&self, constraints: NveSizeConstraints) -> NveIntrinsicSize;
    fn hit_test(&self, x: f32, y: f32, pointer_phase: u8) -> u8;
}

/// Optional embedded component props & outbound event interface (`NVE_IFACE_COMPONENT = 3`).
#[nve_hybrid_module(name = "Component", interface_id = 3, c_name = "NveComponentApi")]
pub trait NveComponentApiSpec {
    fn set_props(&mut self, props: NveBufferSlice) -> u32;
    fn drain_emitted_events(&mut self) -> NveBufferSlice;
    /// Optional zero-copy structured props update (may be NULL).
    #[optional]
    fn set_props_direct(&mut self, entries: *const NveMapEntry, count: u32) -> u32;
}

/// Optional dev / 2-tier hot-reload interface (`NVE_IFACE_DEV = 4`, may be NULL in release builds).
#[nve_hybrid_module(name = "Dev", interface_id = 4, c_name = "NveDevApi")]
pub trait NveDevApiSpec {
    fn apply_hot_patch(&mut self, patch_json: NveBufferSlice) -> u32;
    fn snapshot_state(&self) -> NveBufferSlice;
    fn restore_state(&mut self, state: NveBufferSlice);
}

/// Optional zero-JSON host-capability & channel-bus interface (`NVE_IFACE_HOST = 5`).
#[nve_hybrid_module(name = "Host", interface_id = 5, c_name = "NveHostApi")]
pub trait NveHostApiSpec {
    fn bind_host(&mut self, callbacks: *const NveHostCallbacks);
    fn dispatch_host_events(&mut self, batch: NveBufferSlice) -> u32;
}

/// C API implemented by `cxx/nve-fabric-bridge` for iOS, macOS, Android, and headless Fabric tests.
#[nve_hybrid_c_fn]
pub extern "C" fn nve_fabric_host_create(config: NveSurfaceConfig) -> NveFabricHostHandle;

#[nve_hybrid_c_fn]
pub extern "C" fn nve_fabric_host_destroy(host: NveFabricHostHandle);

#[nve_hybrid_c_fn]
pub extern "C" fn nve_fabric_host_submit_frame(
    host: NveFabricHostHandle,
    command_buffer: NveBufferSlice,
    out_stats: *mut NveFrameStats,
) -> i32;

#[nve_hybrid_c_fn]
pub extern "C" fn nve_fabric_host_drain_events(host: NveFabricHostHandle) -> NveBufferSlice;

#[nve_hybrid_c_fn]
pub extern "C" fn nve_fabric_host_node_count(host: NveFabricHostHandle) -> u32;
