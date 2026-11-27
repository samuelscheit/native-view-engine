# `NveSwiftUI` — Native View Engine Host Kit for SwiftUI / UIKit / AppKit

Swift Package Manager library that embeds any **Native View Engine (NVE)** Rust component directly into SwiftUI on **iOS 16+** (`UIViewRepresentable`) and **macOS 13+** (`NSViewRepresentable`).

## Features

Every surface runs on **`libnve_runtime`** (`NveRuntimeApi`, linked into the app; resolved with
`dlsym("nve_runtime_get_interface")`): the runtime owns the app, reconcile, layout (with native
CoreText measurement), scroll state, hit testing, and pairing; Swift paints its zero-copy
`NveRuntimeSceneView` through `NveHost.RuntimeSceneReader` and the generated `NveTables` specs.

- **`NveSurfaceRepresentable`**: one SwiftUI `View` on iOS, macOS, tvOS, watchOS, and visionOS: paints the scene (`NveSceneCanvasView`), negotiates intrinsic size via the app's `NveLayoutApi::measure`, and pushes deduplicated props (`set_props`).
- **`NveSurfaceController`**: attaches an app (`nve_get_interface`: explicit, `NveStaticInterfaceTable`, or `dlopen`), forwards props / input / hot patches (`apply_hot_patch`), Tier-2 reloads with `snapshot_state` / `restore_state`, drains outbound app events, and pairs with `nve dev` (`pair_connect`).
- **`NveRuntimeSession`**: RAII Swift wrapper over every `NveRuntimeApi` slot (wake + text-measurer trampolines, `withScene` borrows).
- **`NveSceneStore` / `NveRuntimeTables` / `NveSceneRenderer`**: zero-copy scene reads (visual, inherited typography, text, image `object-fit` + tint, text input, scroll, accessibility, `z_index` order).
- **`NveChannelClient`**: Thread-safe pub/sub and retained shared-state bridge to `NveChannelBus`.

Generated inputs (`cargo run --locked -p nve-cli -- codegen abi`): `CNveAbi` imports
`packages/core/host-cxx/include/nve/host/{nve_abi,nve_runtime_abi}.generated.h` in place, and
`Sources/NveSwiftUI/NveTables.generated.swift` holds the table / enum mirrors.

## Usage Example

```swift
import SwiftUI
import NveSwiftUI

struct CounterHostView: View {
    @StateObject private var controller = try! NveSurfaceController(vtableSymbol: "nve_get_interface")
    @State private var count = 0

    var body: some View {
        VStack(spacing: 16) {
            NveSurfaceRepresentable(
                componentName: "NveCounterCard",
                vtableSymbol: "nve_get_interface",
                props: [
                    "title": "SwiftUI + NVE",
                    "count": count,
                ],
                controller: controller,
                onEvent: { event in
                    if event.name == "onIncrement" {
                        count += 1
                    }
                }
            )
            Button("Reset") {
                controller.dispatchCommand(name: "reset")
            }
        }
        .padding()
    }
}
```

## Codegen Integration

`nve-codegen` (`export_swiftui`) generates typed SwiftUI `View` wrappers targeting `import NveSwiftUI` and `NveSurfaceRepresentable` directly.
