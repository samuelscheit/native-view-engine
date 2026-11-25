# `NveSwiftUI` — Native View Engine Host Kit for SwiftUI / UIKit / AppKit

Swift Package Manager library that embeds any **Native View Engine (NVE)** Rust component directly into SwiftUI on **iOS 16+** (`UIViewRepresentable`) and **macOS 13+** (`NSViewRepresentable`).

## Features

- **`NveSurfaceRepresentable`**: Unified SwiftUI `View` across iOS and macOS with `sizeThatFits` intrinsic layout negotiation, reactive prop updates (`set_props`), and clean `dismantleUIView` / `dismantleNSView` teardown.
- **`NveSurfaceController`**: Direct C ABI binding to modular `nve_get_interface` (`CNveAbi`: `NveSurfaceApi`, `NveLayoutApi`, `NveComponentApi`, `NveDevApi`).
- **`NveEventRingCodec`**: Zero-dependency Swift binary decoder/encoder matching `packages/core/abi/src/events.rs`.
- **`NveChannelClient`**: Thread-safe pub/sub and retained shared-state bridge to `NveChannelBus`.

## Usage Example

```swift
import SwiftUI
import NveSwiftUI

struct CounterHostView: View {
    @StateObject private var controller = NveSurfaceController(
        vtableSymbol: "nve_get_interface",
        backend: .fabric
    )
    @State private var count = 0

    var body: some View {
        VStack(spacing: 16) {
            NveSurfaceRepresentable(
                componentName: "NveCounterCard",
                vtableSymbol: "nve_get_interface",
                backend: .fabric,
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
