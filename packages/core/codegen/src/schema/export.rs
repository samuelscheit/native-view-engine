//! Shared Intermediate Representation (IR) for bi-directional React Native <-> NVE codegen.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CodegenError;

/// Typed value representation shared by RN Fabric props, event payloads, commands, and TurboModules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PropType {
    Bool,
    Int32,
    Float,
    Double,
    String,
    Color,
    ImageSource,
    Point,
    EdgeInsets,
    Dimension,
    StringEnum { variants: Vec<String> },
    IntEnum { variants: Vec<i32> },
    Array { item: Box<PropType> },
    Object { fields: Vec<ObjectField> },
    Mixed,
    Void,
    Promise { inner: Box<PropType> },
    Callback { params: Vec<ParamSpec> },
}

/// Named field inside a structured object prop or event payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectField {
    pub name: String,
    pub field_type: PropType,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub nullable: bool,
}

/// Typed literal default value extracted from `WithDefault<T, default>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DefaultValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

/// Component prop specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropSpec {
    pub name: String,
    pub prop_type: PropType,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<DefaultValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// React Native Fabric event dispatch semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventBubbling {
    #[default]
    Direct,
    Bubbling,
}

/// Component event handler specification (`DirectEventHandler` / `BubblingEventHandler`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSpec {
    pub name: String,
    #[serde(default)]
    pub bubbling: EventBubbling,
    #[serde(default = "default_true")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_top_level_name: Option<String>,
    #[serde(default)]
    pub payload: Vec<ObjectField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_type_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Parameter for a native command or TurboModule method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub name: String,
    pub param_type: PropType,
    #[serde(default)]
    pub optional: bool,
}

/// Imperative native command declared via `codegenNativeCommands`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub name: String,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
}

/// Optional second-argument metadata passed to `codegenNativeComponent(name, options)`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ComponentOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_component_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_platforms: Vec<String>,
    #[serde(default)]
    pub interface_only: bool,
}

/// Parsed React Native Fabric `*NativeComponent.{ts,js}` specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RnComponentSpec {
    pub component_name: String,
    pub props_type_name: String,
    pub extends_view_props: bool,
    pub props: Vec<PropSpec>,
    pub events: Vec<EventSpec>,
    pub commands: Vec<CommandSpec>,
    pub options: ComponentOptions,
}

/// Parsed method on a React Native `TurboModule` `Spec` interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurboModuleMethod {
    pub name: String,
    pub params: Vec<ParamSpec>,
    pub return_type: PropType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Parsed React Native `Native*.{ts,js}` TurboModule specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurboModuleSpec {
    pub module_name: String,
    pub enforcing: bool,
    pub methods: Vec<TurboModuleMethod>,
}

/// Result of parsing any React Native Codegen spec file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "spec_kind", rename_all = "snake_case")]
pub enum ParsedRnSpec {
    Component(RnComponentSpec),
    TurboModule(TurboModuleSpec),
}

impl ParsedRnSpec {
    pub fn name(&self) -> &str {
        match self {
            Self::Component(c) => &c.component_name,
            Self::TurboModule(m) => &m.module_name,
        }
    }
}

/// Rendering backend used inside an exported NVE component embedded in React Native.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExportBackend {
    #[default]
    Fabric,
    Skia,
}

impl ExportBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fabric => "fabric",
            Self::Skia => "skia",
        }
    }
}

/// Specification for Direction 2 (`export_rn`): exporting an NVE Rust component
/// as a drop-in React Native Fabric NativeComponent package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NveExportSpec {
    pub name: String,
    #[serde(default)]
    pub backend: ExportBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_vtable_symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_package: Option<String>,
    #[serde(default)]
    pub props: Vec<PropSpec>,
    #[serde(default)]
    pub events: Vec<EventSpec>,
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

impl NveExportSpec {
    pub fn new(name: impl Into<String>, backend: ExportBackend) -> Self {
        Self {
            name: name.into(),
            backend,
            rust_vtable_symbol: None,
            android_package: None,
            props: Vec::new(),
            events: Vec::new(),
            commands: Vec::new(),
            doc: None,
        }
    }

    pub fn from_json(json: &str) -> Result<Self, CodegenError> {
        let spec: Self =
            serde_json::from_str(json).map_err(|e| CodegenError::Json(e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn from_json_file(path: &Path) -> Result<Self, CodegenError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| CodegenError::io(path.display().to_string(), e))?;
        Self::from_json(&content)
    }

    pub fn to_json_pretty(&self) -> Result<String, CodegenError> {
        serde_json::to_string_pretty(self).map_err(|e| CodegenError::Json(e.to_string()))
    }

    pub fn vtable_symbol(&self) -> &str {
        self.rust_vtable_symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("nve_get_interface")
    }

    pub fn android_package_name(&self) -> &str {
        self.android_package
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("dev.nve.components")
    }

    pub fn validate(&self) -> Result<(), CodegenError> {
        if self.name.trim().is_empty() {
            return Err(CodegenError::validation(
                "NveExportSpec.name must not be empty",
            ));
        }
        if !self
            .name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            return Err(CodegenError::validation(format!(
                "NveExportSpec.name '{}' must start with an ASCII letter",
                self.name
            )));
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(CodegenError::validation(format!(
                "NveExportSpec.name '{}' must contain only ASCII alphanumeric characters or '_'",
                self.name
            )));
        }
        for prop in &self.props {
            if prop.name.trim().is_empty() {
                return Err(CodegenError::validation("prop name must not be empty"));
            }
        }
        for ev in &self.events {
            if ev.name.trim().is_empty() {
                return Err(CodegenError::validation("event name must not be empty"));
            }
        }
        Ok(())
    }
}

/// A single generated artifact with its relative output path and full UTF-8 source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedFile {
    pub relative_path: String,
    pub content: String,
}

/// Output bundle for Direction 1 (`import_rn`: RN -> NVE).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportRnOutput {
    pub spec: ParsedRnSpec,
    pub rust_module: GeneratedFile,
    pub cpp_adapter: GeneratedFile,
}

impl ImportRnOutput {
    pub fn files(&self) -> [&GeneratedFile; 2] {
        [&self.rust_module, &self.cpp_adapter]
    }

    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        write_generated_files(out_dir, &self.files())
    }
}

/// Target platform for multi-target export from a single `NveExportSpec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportTarget {
    ReactNative,
    Flutter,
    SwiftUi,
    Compose,
    WebComponent,
}

impl ExportTarget {
    pub const ALL: &'static [ExportTarget] = &[
        ExportTarget::ReactNative,
        ExportTarget::Flutter,
        ExportTarget::SwiftUi,
        ExportTarget::Compose,
        ExportTarget::WebComponent,
    ];

    pub fn dir_name(self) -> &'static str {
        match self {
            Self::ReactNative => "react-native",
            Self::Flutter => "flutter",
            Self::SwiftUi => "swiftui",
            Self::Compose => "compose",
            Self::WebComponent => "web-component",
        }
    }

    pub fn from_name(raw: &str) -> Result<Self, CodegenError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "rn" | "react-native" | "react_native" | "fabric" => Ok(Self::ReactNative),
            "flutter" | "dart" => Ok(Self::Flutter),
            "swiftui" | "swift-ui" | "swift" | "ios" => Ok(Self::SwiftUi),
            "compose" | "jetpack-compose" | "kotlin" | "android" => Ok(Self::Compose),
            "web" | "web-component" | "web_component" | "wasm" | "custom-element" => {
                Ok(Self::WebComponent)
            }
            other => Err(CodegenError::validation(format!(
                "unknown export target '{other}'; expected one of: rn, flutter, swiftui, compose, web-component"
            ))),
        }
    }
}

/// Output bundle for Direction 2 (`export_rn`: NVE -> RN Fabric NativeComponent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportRnOutput {
    pub spec: NveExportSpec,
    pub native_component_ts: GeneratedFile,
    pub cpp_bridge_hpp: GeneratedFile,
    pub cpp_bridge_cpp: GeneratedFile,
    pub ios_component_view_mm: GeneratedFile,
    pub android_view_manager_kt: GeneratedFile,
    pub windows_component_view_hpp: GeneratedFile,
}

impl ExportRnOutput {
    pub fn files(&self) -> [&GeneratedFile; 6] {
        [
            &self.native_component_ts,
            &self.cpp_bridge_hpp,
            &self.cpp_bridge_cpp,
            &self.ios_component_view_mm,
            &self.android_view_manager_kt,
            &self.windows_component_view_hpp,
        ]
    }

    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        write_generated_files(out_dir, &self.files())
    }
}

/// Output bundle for `export_flutter`: typed Dart `StatefulWidget` using `dart:ffi`
/// + `package:nve_flutter/nve_flutter.dart`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportFlutterOutput {
    pub spec: NveExportSpec,
    pub dart_widget: GeneratedFile,
}

impl ExportFlutterOutput {
    pub fn files(&self) -> [&GeneratedFile; 1] {
        [&self.dart_widget]
    }

    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        write_generated_files(out_dir, &self.files())
    }
}

/// Output bundle for `export_swiftui`: typed SwiftUI `View` wrapping `NveSwiftUI.NveSurfaceRepresentable`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSwiftUiOutput {
    pub spec: NveExportSpec,
    pub swift_view: GeneratedFile,
}

impl ExportSwiftUiOutput {
    pub fn files(&self) -> [&GeneratedFile; 1] {
        [&self.swift_view]
    }

    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        write_generated_files(out_dir, &self.files())
    }
}

/// Output bundle for `export_compose`: typed Kotlin `@Composable` wrapping `dev.nve.compose.NveSurface`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportComposeOutput {
    pub spec: NveExportSpec,
    pub compose_kt: GeneratedFile,
}

impl ExportComposeOutput {
    pub fn files(&self) -> [&GeneratedFile; 1] {
        [&self.compose_kt]
    }

    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        write_generated_files(out_dir, &self.files())
    }
}

/// Output bundle for `export_web_component`: typed Custom Element `<nve-...>` + `.d.ts` wrapping `wasm-bindgen`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportWebComponentOutput {
    pub spec: NveExportSpec,
    pub tag_name: String,
    pub element_js: GeneratedFile,
    pub types_dts: GeneratedFile,
}

impl ExportWebComponentOutput {
    pub fn files(&self) -> [&GeneratedFile; 2] {
        [&self.element_js, &self.types_dts]
    }

    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        write_generated_files(out_dir, &self.files())
    }
}

/// Unified multi-target export bundle returned by [`crate::export_target`] / [`crate::export_all_targets`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiTargetExportOutput {
    pub spec: NveExportSpec,
    pub target: ExportTarget,
    pub files: Vec<GeneratedFile>,
}

impl MultiTargetExportOutput {
    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        let refs: Vec<&GeneratedFile> = self.files.iter().collect();
        write_generated_files(out_dir, &refs)
    }
}

pub(crate) fn write_generated_files(
    out_dir: &Path,
    files: &[&GeneratedFile],
) -> Result<Vec<PathBuf>, CodegenError> {
    let mut written = Vec::with_capacity(files.len());
    for file in files {
        let dest = out_dir.join(&file.relative_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CodegenError::io(parent.display().to_string(), e))?;
        }
        std::fs::write(&dest, &file.content)
            .map_err(|e| CodegenError::io(dest.display().to_string(), e))?;
        written.push(dest);
    }
    Ok(written)
}
