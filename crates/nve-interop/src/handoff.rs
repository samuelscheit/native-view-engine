use serde::{Deserialize, Serialize};

use crate::error::InteropError;
use crate::surface::FrameworkKind;

/// Pixel format for zero-copy cross-framework GPU texture handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuTextureFormat {
    Bgra8Unorm,
    Rgba8Unorm,
    Rgba16Float,
    Nv12,
}

/// Platform-native GPU texture handle shared between NVE Skia and host compositors
/// (e.g. `FlutterTextureRegistry`, Metal `MTLTexture`, Vulkan/OpenGL `EGLImage`, Direct3D).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SharedGpuTextureKind {
    IoSurface {
        surface_id: u32,
        pixel_format: GpuTextureFormat,
    },
    CvPixelBuffer {
        ptr: usize,
        pixel_format: GpuTextureFormat,
    },
    AHardwareBuffer {
        ptr: usize,
        format: u32,
    },
    DxgiHandle {
        handle: usize,
        adapter_luid: u64,
    },
    DmaBuf {
        fd: i32,
        stride: u32,
        offset: u32,
        modifier: u64,
    },
}

/// Descriptor for a zero-copy GPU texture composited at display refresh rate (60/120Hz).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedGpuTexture {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    /// Optional host compositor texture registry ID (e.g. Flutter `textureId`).
    pub registry_texture_id: i64,
    pub handle: SharedGpuTextureKind,
}

/// Platform-native view/widget handle for direct native view tree embedding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NativeViewHandleKind {
    UiView { ptr: usize },
    NsView { ptr: usize },
    AndroidView { global_ref: usize },
    Win32Hwnd { hwnd: usize },
    GtkWidget { ptr: usize },
    DomElement { element_id: String },
}

/// Native view handle paired with the framework that owns it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NativeViewHandle {
    pub framework: FrameworkKind,
    pub handle: NativeViewHandleKind,
}

/// How pixels or native views cross a framework boundary for a given surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RenderHandoff {
    NativeView(NativeViewHandle),
    SharedGpuTexture(SharedGpuTexture),
    Headless,
}

/// Controls when a foreign framework island boots its runtime/engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IslandMountPolicy {
    /// Boot the foreign framework engine only when the island enters the viewport.
    #[default]
    LazyOnVisible,
    /// Boot immediately when the parent surface mounts.
    EagerOnCreate,
    /// Render a static snapshot/placeholder until the user interacts with the island.
    OnFirstInteraction,
}

/// Shared engine-group configuration for hosting multiple Flutter islands with minimal RAM
/// (~180KB per additional Flutter engine via `FlutterEngineGroup`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlutterEngineGroupConfig {
    pub group_id: String,
    pub entrypoint: String,
    pub initial_route: Option<String>,
    #[serde(default)]
    pub dart_entrypoint_args: Vec<String>,
    pub share_isolate_group: bool,
    pub warm_spare_engine: bool,
}

impl Default for FlutterEngineGroupConfig {
    fn default() -> Self {
        Self {
            group_id: "nve_default_flutter_group".to_owned(),
            entrypoint: "main".to_owned(),
            initial_route: None,
            dart_entrypoint_args: Vec::new(),
            share_isolate_group: true,
            warm_spare_engine: false,
        }
    }
}

/// Opt-in specification for mounting a React Native Fabric component as a lazy `<Island>` inside NVE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RnIslandSpec {
    pub module_name: String,
    pub component_name: String,
    pub initial_props_json: String,
    pub fabric_enabled: bool,
    pub hermes_bytecode_path: Option<String>,
    #[serde(default)]
    pub mount_policy: IslandMountPolicy,
}

/// Opt-in specification for mounting a Flutter widget as a lazy `<Island>` inside NVE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlutterIslandSpec {
    pub widget_name: String,
    #[serde(default)]
    pub engine_group: FlutterEngineGroupConfig,
    /// Prefer zero-copy `SharedGpuTexture` compositing over platform view embedding.
    pub prefer_texture_compositing: bool,
    pub initial_props_json: String,
    #[serde(default)]
    pub mount_policy: IslandMountPolicy,
}

/// Opt-in specification for mounting a native SwiftUI `View` as an `<Island>` inside NVE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwiftUIIslandSpec {
    pub view_name: String,
    pub module_name: String,
    pub initial_props_json: String,
    pub use_intrinsic_sizing: bool,
    #[serde(default)]
    pub mount_policy: IslandMountPolicy,
}

/// Opt-in specification for mounting a Jetpack Compose `@Composable` as an `<Island>` inside NVE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeIslandSpec {
    pub composable_fqn: String,
    pub initial_props_json: String,
    pub use_hardware_layer: bool,
    #[serde(default)]
    pub mount_policy: IslandMountPolicy,
}

/// Foreign-framework `<Island>` hosted inside an NVE surface tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "island_framework", rename_all = "snake_case")]
pub enum IslandSpec {
    ReactNative(RnIslandSpec),
    Flutter(FlutterIslandSpec),
    SwiftUI(SwiftUIIslandSpec),
    JetpackCompose(ComposeIslandSpec),
}

impl IslandSpec {
    pub fn framework(&self) -> FrameworkKind {
        match self {
            Self::ReactNative(_) => FrameworkKind::ReactNativeFabric,
            Self::Flutter(_) => FrameworkKind::Flutter,
            Self::SwiftUI(_) => FrameworkKind::SwiftUI,
            Self::JetpackCompose(_) => FrameworkKind::JetpackCompose,
        }
    }

    pub fn mount_policy(&self) -> IslandMountPolicy {
        match self {
            Self::ReactNative(s) => s.mount_policy,
            Self::Flutter(s) => s.mount_policy,
            Self::SwiftUI(s) => s.mount_policy,
            Self::JetpackCompose(s) => s.mount_policy,
        }
    }

    pub fn initial_props_json(&self) -> &str {
        match self {
            Self::ReactNative(s) => &s.initial_props_json,
            Self::Flutter(s) => &s.initial_props_json,
            Self::SwiftUI(s) => &s.initial_props_json,
            Self::JetpackCompose(s) => &s.initial_props_json,
        }
    }

    pub fn validate(&self) -> Result<(), InteropError> {
        let props_json = self.initial_props_json();
        if !props_json.trim().is_empty() {
            let _: serde_json::Value = serde_json::from_str(props_json).map_err(|e| {
                InteropError::InvalidIslandSpec(format!("invalid initial_props_json: {e}"))
            })?;
        }
        match self {
            Self::ReactNative(spec) => {
                if spec.module_name.trim().is_empty() || spec.component_name.trim().is_empty() {
                    return Err(InteropError::InvalidIslandSpec(
                        "RnIslandSpec requires non-empty module_name and component_name".into(),
                    ));
                }
            }
            Self::Flutter(spec) => {
                if spec.widget_name.trim().is_empty() {
                    return Err(InteropError::InvalidIslandSpec(
                        "FlutterIslandSpec requires non-empty widget_name".into(),
                    ));
                }
                if spec.engine_group.group_id.trim().is_empty()
                    || spec.engine_group.entrypoint.trim().is_empty()
                {
                    return Err(InteropError::InvalidIslandSpec(
                        "FlutterEngineGroupConfig requires non-empty group_id and entrypoint"
                            .into(),
                    ));
                }
            }
            Self::SwiftUI(spec) => {
                if spec.view_name.trim().is_empty() || spec.module_name.trim().is_empty() {
                    return Err(InteropError::InvalidIslandSpec(
                        "SwiftUIIslandSpec requires non-empty view_name and module_name".into(),
                    ));
                }
            }
            Self::JetpackCompose(spec) => {
                if spec.composable_fqn.trim().is_empty() {
                    return Err(InteropError::InvalidIslandSpec(
                        "ComposeIslandSpec requires non-empty composable_fqn".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}
