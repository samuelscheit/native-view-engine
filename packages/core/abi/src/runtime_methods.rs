//! Single-source method table of the [`NveRuntimeApi`](crate::NveRuntimeApi) vtable: one C
//! declaration per slot, consumed by the runtime schema hash and every generated foreign mirror.

/// One [`NveRuntimeApi`] slot as its C declaration: the single source for the schema hash and
/// every generated foreign mirror (field order == slot order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveRuntimeMethod {
    pub name: &'static str,
    /// C return type.
    pub ret: &'static str,
    /// C parameter list (without parentheses).
    pub params: &'static str,
}

const fn rt_method(name: &'static str, ret: &'static str, params: &'static str) -> NveRuntimeMethod {
    NveRuntimeMethod { name, ret, params }
}

/// Every [`NveRuntimeApi`] function slot in field order.
pub const NVE_RUNTIME_API_METHODS: &[NveRuntimeMethod] = &[
    rt_method("create", "NveRuntimeHandle", "const NveRuntimeConfig* config"),
    rt_method("destroy", "void", "NveRuntimeHandle handle"),
    rt_method("set_wake_callback", "int32_t", "NveRuntimeHandle handle, NveRuntimeWakeFn cb, void* ctx"),
    rt_method("set_viewport", "int32_t", "NveRuntimeHandle handle, float width, float height, float scale_factor"),
    rt_method("attach_app_interface", "int32_t", "NveRuntimeHandle handle, NveGetInterfaceFn get_iface"),
    rt_method("submit_nve2", "int32_t", "NveRuntimeHandle handle, NveBufferSlice frame"),
    rt_method("tick", "int32_t", "NveRuntimeHandle handle, float dt_s, uint32_t* out_changed"),
    rt_method("dispatch_events", "int32_t", "NveRuntimeHandle handle, NveEventBatch batch, uint32_t* out_changed"),
    rt_method("hit_test", "int32_t", "NveRuntimeHandle handle, float x, float y, uint32_t* out_node_id"),
    rt_method(
        "scroll_by",
        "int32_t",
        "NveRuntimeHandle handle, uint32_t node_id, float dx, float dy, float* out_x, float* out_y",
    ),
    rt_method("scene_acquire", "int32_t", "NveRuntimeHandle handle, NveRuntimeSceneView* out_view"),
    rt_method("scene_release", "int32_t", "NveRuntimeHandle handle"),
    rt_method("get_metrics", "int32_t", "NveRuntimeHandle handle, NveRuntimeMetrics* out_metrics"),
    rt_method("pair_connect", "int32_t", "NveRuntimeHandle handle, NveStrSlice uri, NveStrSlice token"),
    rt_method("pair_disconnect", "int32_t", "NveRuntimeHandle handle"),
    rt_method("pair_is_connected", "uint32_t", "NveRuntimeHandle handle"),
    rt_method(
        "set_props",
        "int32_t",
        "NveRuntimeHandle handle, const struct NveMapEntry* entries, uint32_t count, uint32_t* out_changed",
    ),
    rt_method("drain_emitted_events", "int32_t", "NveRuntimeHandle handle, NveEventBatch* out_batch"),
    rt_method(
        "measure",
        "int32_t",
        "NveRuntimeHandle handle, NveSizeConstraints constraints, NveIntrinsicSize* out_size",
    ),
    rt_method(
        "apply_hot_patch",
        "int32_t",
        "NveRuntimeHandle handle, NveBufferSlice patch_nvp1, uint32_t* out_changed",
    ),
    rt_method("snapshot_state", "int32_t", "NveRuntimeHandle handle, NveBufferSlice* out_state"),
    rt_method("restore_state", "int32_t", "NveRuntimeHandle handle, NveBufferSlice state"),
    rt_method("set_text_measurer", "int32_t", "NveRuntimeHandle handle, NveTextMeasureFn cb, void* ctx"),
    rt_method(
        "resource_bytes",
        "int32_t",
        "NveRuntimeHandle handle, uint32_t resource_id, uint8_t* out_buf, size_t cap, size_t* out_len",
    ),
    rt_method("last_error", "size_t", "uint8_t* out_buf, size_t cap"),
];
