# `dev.nve.compose` — Native View Engine Host Kit for Jetpack Compose

Idiomatic Jetpack Compose adapter (`NveSurfaceComposable.kt`) for embedding any **Native View Engine (NVE)** Rust component natively inside an Android Jetpack Compose tree.

## Features

- **`@Composable fun NveSurface(...)`**: Wraps `AndroidView` + `NveSurfaceView`, syncing Compose state maps into `NveComponentApi.set_props` via `LaunchedEffect`, participating in `Modifier` sizing, and disposing native surfaces cleanly via `DisposableEffect`.
- **Intrinsic Layout & Nested Scroll Arbitration**: `NveSurfaceView` hooks into `onMeasure` (`NveNativeBridge.nativeMeasureSurface`) and `onTouchEvent` (`requestDisallowInterceptTouchEvent`) to cooperate with parent Compose `LazyColumn` / `HorizontalPager` scrollables.
- **`NveSurfaceController` & `rememberNveSurfaceController`**: Imperative command dispatch (`dispatchCommand`), explicit render triggering, and event draining.
- **`NveChannelClient`**: Pub/sub and retained shared-state bridge to `NveChannelBus`.

## Usage Example

```kotlin
package com.example.app

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Button
import androidx.compose.material3.Text
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
        Button(onClick = { controller.dispatchCommand("reset") }) {
            Text("Reset")
        }
    }
}
```

## Codegen Integration

`nve-codegen` (`export_compose`) generates strongly typed `@Composable` functions targeting `package dev.nve.compose` (`NveSurface`, `NveRenderBackend`, `NveEvent`, `NveSurfaceController`, `rememberNveSurfaceController`).
