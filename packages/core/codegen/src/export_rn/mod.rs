//! Direction 2 (`export_rn`): `NveExportSpec` -> Complete drop-in zero-JSON React Native Fabric
//! NativeComponent package:
//! 1. `<Name>NativeComponent.generated.ts` — RN Codegen TypeScript spec
//! 2. `<Name>NveBridge.generated.hpp` / `<Name>NveBridge.generated.cpp` — C++20 glue built on `nve::NveRnComponentBridge`
//! 3. `<Name>ComponentView.generated.mm` — iOS ObjC++ Fabric view subclassing `RCTNveComponentView`
//! 4. `<Name>ViewManager.generated.kt` — Android Kotlin Fabric ViewManager wrapping `dev.nve.host.NveNativeBridge`
//! 5. `<Name>RnwComponentView.generated.hpp` — `react-native-windows` Fabric registration targeting `hosts/windows-shell/NveRnwComponentView.hpp`

mod android_emitter;
mod cpp_emitter;
mod ios_emitter;
mod ts_emitter;
mod windows_emitter;

use crate::error::CodegenError;
use crate::ir::{ExportRnOutput, GeneratedFile, NveExportSpec};

use android_emitter::emit_android_view_manager_kt;
use cpp_emitter::{emit_cpp_bridge_cpp, emit_cpp_bridge_hpp};
use ios_emitter::emit_ios_component_view_mm;
use ts_emitter::emit_native_component_ts;
use windows_emitter::emit_windows_rnw_component_view_hpp;

/// Generates a complete drop-in zero-JSON React Native Fabric NativeComponent package for an NVE component.
pub fn generate_export_rn(spec: &NveExportSpec) -> Result<ExportRnOutput, CodegenError> {
    spec.validate()?;

    let name = &spec.name;
    let native_component_ts = GeneratedFile {
        relative_path: format!("{name}NativeComponent.generated.ts"),
        content: emit_native_component_ts(spec),
    };
    let cpp_bridge_hpp = GeneratedFile {
        relative_path: format!("{name}NveBridge.generated.hpp"),
        content: emit_cpp_bridge_hpp(spec),
    };
    let cpp_bridge_cpp = GeneratedFile {
        relative_path: format!("{name}NveBridge.generated.cpp"),
        content: emit_cpp_bridge_cpp(spec),
    };
    let ios_component_view_mm = GeneratedFile {
        relative_path: format!("{name}ComponentView.generated.mm"),
        content: emit_ios_component_view_mm(spec),
    };
    let android_view_manager_kt = GeneratedFile {
        relative_path: format!("{name}ViewManager.generated.kt"),
        content: emit_android_view_manager_kt(spec),
    };
    let windows_component_view_hpp = GeneratedFile {
        relative_path: format!("{name}RnwComponentView.generated.hpp"),
        content: emit_windows_rnw_component_view_hpp(spec),
    };

    Ok(ExportRnOutput {
        spec: spec.clone(),
        native_component_ts,
        cpp_bridge_hpp,
        cpp_bridge_cpp,
        ios_component_view_mm,
        android_view_manager_kt,
        windows_component_view_hpp,
    })
}
