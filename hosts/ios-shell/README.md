# iOS Prebuilt Host Shell (`hosts/ios-shell`)

This host shell links prebuilt React Native Fabric XCFrameworks (`ReactNativeDependencies`, `React-Fabric`) plus `cxx/nve-fabric-bridge` **once**.

During daily UI development:
- **Tier 1 (<50 ms, Simulator & physical iPhone):** `nve dev` pushes JSON `HotPatchMsg::TemplatePatch` messages over WebSocket to `-[NveFabricHostController applyHotPatchJson:]`.
- **Tier 2 (150–400 ms, iOS Simulator):** `nve dev` recompiles only the Rust app crate for `aarch64-apple-ios-sim` as a `cdylib` and signals `-[NveFabricHostController reloadRustAppDylibAtPath:error:]`, preserving state via `NveAppVTable::snapshot_state` / `restore_state` without invoking `xcodebuild`.
