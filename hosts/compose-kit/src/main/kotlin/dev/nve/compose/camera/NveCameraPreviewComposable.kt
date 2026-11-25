package dev.nve.compose.camera

import android.content.Context
import android.view.ViewGroup
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner

/**
 * First-class Jetpack Compose `<NativeView<CameraPreview>>` (`"nve.camera.preview"`) host view.
 *
 * Mounts a native Jetpack CameraX [PreviewView] inside [AndroidView] and wires Android
 * `LifecycleEventObserver` (`ON_START`, `ON_STOP`, `ON_DESTROY`) directly into the active
 * [NveCameraBackend]. When a React Native / Expo host registers VisionCamera v5 in [registry],
 * it automatically delegates to [NveVisionCameraAdapter]; on standalone Android / Compose apps
 * it binds [NveAndroidCameraXBinding] directly with zero React Native dependencies.
 */
@Composable
fun NveCameraPreview(
    props: NveCameraPreviewProps = NveCameraPreviewProps(),
    modifier: Modifier = Modifier,
    surfaceId: Int = 1,
    nodeId: Int = 0,
    registry: NveCameraRegistry = NveCameraRegistry.default,
    preferredBackend: NveCameraBackendKind? = null,
    backendOverride: NveCameraBackend? = null,
    listener: NveCameraEventListener? = null,
    scaleType: PreviewView.ScaleType = PreviewView.ScaleType.FILL_CENTER,
    onBackendReady: ((NveCameraBackend) -> Unit)? = null,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var activeBackend by remember(surfaceId, nodeId, backendOverride, preferredBackend, registry) {
        mutableStateOf(backendOverride)
    }
    var ownedCameraXBinding by remember { mutableStateOf<NveAndroidCameraXBinding?>(null) }

    val ownsBackend = backendOverride == null

    DisposableEffect(activeBackend, listener) {
        activeBackend?.attachListener(listener)
        onDispose {
            activeBackend?.attachListener(null)
        }
    }

    DisposableEffect(lifecycleOwner, activeBackend) {
        val backend = activeBackend
        if (backend == null) {
            onDispose { }
        } else {
            val observer = LifecycleEventObserver { _, event ->
                when (event) {
                    Lifecycle.Event.ON_START -> {
                        if (backend is NveCameraXAdapter) backend.onLifecycleStart()
                        else if (backend.activeProps.active) backend.start()
                    }
                    Lifecycle.Event.ON_STOP -> {
                        if (backend is NveCameraXAdapter) backend.onLifecycleStop()
                        else backend.stop()
                    }
                    Lifecycle.Event.ON_DESTROY -> {
                        if (ownsBackend) backend.release()
                    }
                    else -> Unit
                }
            }
            lifecycleOwner.lifecycle.addObserver(observer)
            onDispose {
                lifecycleOwner.lifecycle.removeObserver(observer)
                if (ownsBackend) {
                    backend.release()
                    ownedCameraXBinding?.shutdown()
                    ownedCameraXBinding = null
                }
            }
        }
    }

    LaunchedEffect(activeBackend, props) {
        activeBackend?.applyProps(props)
    }

    AndroidView(
        modifier = modifier,
        factory = { viewContext: Context ->
            val previewView = PreviewView(viewContext).apply {
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                )
                this.scaleType = scaleType
                implementationMode = PreviewView.ImplementationMode.COMPATIBLE
            }
            val resolved = backendOverride ?: createPreviewBackend(
                context = context,
                lifecycleOwner = lifecycleOwner,
                previewView = previewView,
                surfaceId = surfaceId,
                nodeId = nodeId,
                registry = registry,
                preferredBackend = preferredBackend,
                onCameraXBindingCreated = { ownedCameraXBinding = it },
                onAsyncError = { code, message -> listener?.onError(code, message) },
            )
            resolved.attachListener(listener)
            resolved.applyProps(props)
            activeBackend = resolved
            onBackendReady?.invoke(resolved)
            previewView
        },
        update = { previewView ->
            previewView.scaleType = scaleType
        },
    )
}

/**
 * Zero-JSON variant of [NveCameraPreview] accepting raw binary `NVNP` prop bytes straight from
 * the NVE `CommandBuffer` / `nve.native_props` attribute.
 */
@Composable
fun NveCameraPreviewEncoded(
    nvnpBytes: ByteArray,
    modifier: Modifier = Modifier,
    surfaceId: Int = 1,
    nodeId: Int = 0,
    registry: NveCameraRegistry = NveCameraRegistry.default,
    preferredBackend: NveCameraBackendKind? = null,
    backendOverride: NveCameraBackend? = null,
    listener: NveCameraEventListener? = null,
    onBackendReady: ((NveCameraBackend) -> Unit)? = null,
) {
    val decoded = remember(nvnpBytes) {
        runCatching { NveCameraCodec.decodeProps(nvnpBytes) }
    }
    val props = decoded.getOrNull()
    if (props != null) {
        NveCameraPreview(
            props = props,
            modifier = modifier,
            surfaceId = surfaceId,
            nodeId = nodeId,
            registry = registry,
            preferredBackend = preferredBackend,
            backendOverride = backendOverride,
            listener = listener,
            onBackendReady = onBackendReady,
        )
    } else {
        DisposableEffect(nvnpBytes, listener) {
            val err = decoded.exceptionOrNull()
            listener?.onError("INVALID_NVNP_PROPS", err?.message ?: "Failed to decode NVNP camera props")
            onDispose { }
        }
    }
}

private fun createPreviewBackend(
    context: Context,
    lifecycleOwner: LifecycleOwner,
    previewView: PreviewView,
    surfaceId: Int,
    nodeId: Int,
    registry: NveCameraRegistry,
    preferredBackend: NveCameraBackendKind?,
    onCameraXBindingCreated: (NveAndroidCameraXBinding) -> Unit,
    onAsyncError: (code: String, message: String) -> Unit,
): NveCameraBackend {
    val targetKind = preferredBackend ?: registry.resolveBackendKind()
    if (targetKind == NveCameraBackendKind.CAMERAX && !registry.isVisionCameraRegistered()) {
        val binding = NveAndroidCameraXBinding(
            context = context,
            lifecycleOwner = lifecycleOwner,
            previewView = previewView,
            onAsyncError = onAsyncError,
        )
        onCameraXBindingCreated(binding)
        return NveCameraXAdapter(binding)
    }
    return registry.createBackend(surfaceId, nodeId, preferredBackend)
}
