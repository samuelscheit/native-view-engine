# `dev.nve.compose` — Native View Engine Host Kit for Jetpack Compose

Android Library Gradle module (`dev.nve.compose`) for embedding **Native View Engine (NVE)** Rust components inside Jetpack Compose and Android `ComponentActivity` hosts with binary FFI.

## Architecture

- **`NveSceneBuffer.kt`**: Zero-copy, zero-GC direct `ByteBuffer` reader over 64-byte `NveRenderNode` + `NveSceneView` slabs (`SCHEMA_HASH = 0x00F2CF037DB50CC8`), with schema/bounds/UTF-8 validation and incremental dirty-index diffing.
- **`NveBinValue.kt`**: Compact little-endian binary `NveBinValue` encoder and zero-copy `NveBinValueView` (tags `0..7`, O(1) container skipping, max depth `64`) replacing JSON for props, commands, and channel messages.
- **`NveEventRingCodec.kt`**: Binary `EventRing` encoder & decoder matching `packages/core/abi/src/events.rs` for press, long-press, scroll, text change, focus/blur, layout, and custom binary events.
- **`NveSurfaceState.kt` & `NveNativeBridge.kt`**: Pluggable `NveBridgeBackend`, reactive `NveSurfaceState`, `NveSurfaceController`, and binary `NveChannelClient`.
- **`NveSurfaceComposable.kt`**: `@Composable fun NveSurface(...)`, `rememberNveSurfaceController(...)`, and `NveSurfaceView` (`onMeasure` intrinsic sizing, `onSizeChanged` viewport forwarding, and `requestDisallowInterceptTouchEvent` gesture arbitration).
- **`NveHostActivity.kt`**: Reusable `ComponentActivity` host wiring Android lifecycle transitions (attach/detach/re-attach/dispose), viewport resize, `surfaceId` preservation, and Tier-2 `.so` hot-reload.

## Build & Test

Run all JVM (JUnit 5) and Android lifecycle (Robolectric) unit tests:

```bash
cd hosts/compose-kit
./run-tests.sh
```

Or invoke Gradle directly with a HotSpot/JBR JDK 17–21 and `ANDROID_HOME` configured (avoid Eclipse OpenJ9; see `docs/UPSTREAM.md`):

```bash
cd hosts/compose-kit
JAVA_HOME=/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home \
  ANDROID_HOME="$HOME/Library/Android/sdk" \
  ./gradlew testDebugUnitTest assembleDebug
```

## Usage Example

```kotlin
package com.example.app

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.nve.compose.NveRenderBackend
import dev.nve.compose.NveSurface
import dev.nve.compose.rememberNveSurfaceController

@Composable
fun CounterScreen() {
    var count by remember { mutableIntStateOf(0) }
    val controller = rememberNveSurfaceController(
        componentName = "NveCounterCard",
        vtableSymbol = "nve_get_interface",
        backend = NveRenderBackend.FABRIC,
    )

    Column {
        NveSurface(
            componentName = "NveCounterCard",
            modifier = Modifier
                .fillMaxWidth()
                .height(180.dp),
            vtableSymbol = "nve_get_interface",
            backend = NveRenderBackend.FABRIC,
            props = mapOf(
                "title" to "Compose + NVE",
                "count" to count,
            ),
            controller = controller,
            onEvent = { event ->
                if (event.name == "onIncrement") {
                    count += 1
                }
            },
        )
    }
}
```
