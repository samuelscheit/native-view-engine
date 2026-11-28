# `dev.nve.compose` — Native View Engine Host Kit for Jetpack Compose

Android Library Gradle module (`dev.nve.compose`) for embedding **Native View Engine (NVE)** apps inside Jetpack Compose and Android `ComponentActivity` hosts through `libnve_runtime`.

## Architecture

Compose binds to `libnve_runtime` through the shared `dev.nve:nve-host-java` runtime (included
build of `packages/core/host-java`) and `libnve_android_shell` (JNI):

- **`NveSurfaceController.kt`**: `NveRuntimeHost` (`NativeRuntimeHost` over `RuntimeSurface`; JVM
  fakes in tests), `NveSource` (app `.so` loaded by the runtime's `load_app_module`, hot-reloadable
  with `carryState`, or `nve pair` pairing), the observable `NveSceneSnapshot`, and input →
  `EventRing` events (press, text change, submit, focus / blur, scroll — which also syncs the
  runtime's scroll offsets for hit testing).
- **`NveScene.kt`**: `NveSceneCapture` turns each acquired frame (`SceneReader` over the
  generated `NveTables` specs) into an immutable snapshot in O(changed): on the frame right after
  the previous one it rebuilds only removed, content-dirty (incl. inherited typography), and moved
  (geometry only) nodes and reuses every other `NveNode`; a redelivered frame is a no-op; a gap or
  new `scene_epoch` rebuilds all. Kinds come from table presence; rects are parent-relative;
  images (`ImageSource`, `data:` URLs decoded by `ImageSource.bytes`) and canvas draw lists are
  copied out of the frame and reused while their id is clean (not in `dirty_resources`); runtime
  `scroll_offsets` are captured per container.
- **`NveSurface.kt`** / **`NveText.kt`**: `@Composable fun NveSurface(controller, modifier)` draws the
  snapshot (`BasicText`, `BasicTextField`, `Image`, `Canvas` + `NveDrawListRenderer`, scroll
  containers for every `nve.scroll.axes` value incl. both / none, boxes) and installs
  `NveComposeTextMeasurer`, so runtime layout measures text with Compose's own `TextMeasurer`.
  Nodes are keyed by `(scene epoch, node id)`, so no scroll / field / focus state leaks into a new
  epoch; runtime scroll offsets drive scroll-to / restore. `visibility: hidden` nodes keep their
  box but take no clicks, focus, or semantics; disabled inputs are disabled. Text fields are
  controlled: a keystroke the app rejects or rewrites snaps back to the runtime value, and lone
  surrogates are repaired (`Utf8.wellFormed`) before they are sent.
- **`NveHostActivity.kt`**: reusable `ComponentActivity` for the bundled `libnve_app.so`,
  `nve_app_path`, or `pair_connect` / `pair_token`.

Props, commands, and app → host emitted events are not part of the runtime ABI (Model A apps own
their state); they are tracked in `docs/BACKLOG.md`.

## Build & Test

Run all JVM (JUnit 5) and Android lifecycle (Robolectric) unit tests — including the
`hosts/android-shell` View renderer suites (`dev.nve.android.NveSceneRendererTest`: O(changed)
frames, epoch resets, scroll axes, runtime scroll offsets, disabled / hidden input, single submit),
whose sources this module adds to its test source set:

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
