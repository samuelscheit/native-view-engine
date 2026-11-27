# `dev.nve.compose` — Native View Engine Host Kit for Jetpack Compose

Android Library Gradle module (`dev.nve.compose`) for embedding **Native View Engine (NVE)** apps inside Jetpack Compose and Android `ComponentActivity` hosts through `libnve_runtime`.

## Architecture

Compose binds to `libnve_runtime` through the shared `dev.nve:nve-host-java` runtime (included
build of `packages/core/host-java`) and `libnve_android_shell` (JNI):

- **`NveSurfaceController.kt`**: `NveRuntimeHost` (`NativeRuntimeHost` over `RuntimeSurface`; JVM
  fakes in tests), `NveSource` (app `.so`, hot-reloadable, or `nve pair` pairing), the observable
  `NveSceneSnapshot`, and input → `EventRing` events (press, text change, submit, focus / blur,
  scroll — which also syncs the runtime's scroll offsets for hit testing).
- **`NveScene.kt`**: `NveSceneCapture` copies each acquired frame (`RuntimeSceneReader` over the
  generated `NveTables` specs) into an immutable snapshot: kinds from table presence, parent-relative
  rects, `nve.visual` boxes, inherited `nve.typography`, `nve.text_input`, images
  (`ImageSource`) and canvas draw lists copied out of the frame (reused while their resource id is
  unchanged and not ephemeral), and accessibility / test-id semantics.
- **`NveSurface.kt`** / **`NveText.kt`**: `@Composable fun NveSurface(controller, modifier)` draws the
  snapshot (`BasicText`, `BasicTextField`, `Image`, `Canvas` + `NveDrawListRenderer`, scroll
  containers, boxes) and installs `NveComposeTextMeasurer`, so runtime layout measures text with
  Compose's own `TextMeasurer`.
- **`NveHostActivity.kt`**: reusable `ComponentActivity` for the bundled `libnve_app.so`,
  `nve_app_path`, or `pair_connect` / `pair_token`.

Props, commands, and app → host emitted events are not part of the runtime ABI (Model A apps own
their state); they are tracked in `docs/BACKLOG.md`.

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
class MainActivity : ComponentActivity() {
    private lateinit var controller: NveSurfaceController

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val d = resources.displayMetrics.density
        val main = Handler(Looper.getMainLooper())
        controller = NveSurfaceController(
            NativeRuntimeHost(1, 360f, 800f, d, Executor { main.post(it) }),
        )
        controller.start(NveSource.App(applicationInfo.nativeLibraryDir + "/libnve_app.so"))
        setContent { NveSurface(controller, Modifier.fillMaxSize()) }
    }

    override fun onDestroy() {
        controller.close()
        super.onDestroy()
    }
}
```
