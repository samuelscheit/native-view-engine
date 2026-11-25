package dev.nve.compose.camera

/**
 * Concrete Android camera backend implementation powering an `"nve.camera.preview"` node.
 */
enum class NveCameraBackendKind {
    /** Margelo `react-native-vision-camera` v5 Nitro backend (preferred inside RN / Expo hosts). */
    VISION_CAMERA_V5,

    /** Standalone Jetpack CameraX backend (zero React Native / VisionCamera dependencies). */
    CAMERAX,

    /** Host-supplied custom camera backend. */
    CUSTOM,
}

/**
 * Lifecycle state machine for an active [NveCameraBackend] session.
 */
enum class NveCameraSessionState {
    IDLE,
    CONFIGURED,
    RUNNING,
    RECORDING,
    STOPPED,
    RELEASED,
}

/**
 * Hardware camera capability descriptor exposed by both VisionCamera v5 and CameraX adapters.
 */
data class NveCameraDeviceInfo(
    val id: String,
    val name: String,
    val facing: NveCameraFacing,
    val hasTorch: Boolean = true,
    val minZoom: Float = 1.0f,
    val maxZoom: Float = 8.0f,
    val maxFps: Int = 60,
    val supportsHdr: Boolean = false,
    val supportedStabilization: Set<NveStabilizationMode> = setOf(
        NveStabilizationMode.OFF,
        NveStabilizationMode.STANDARD,
    ),
)

/**
 * Metadata returned when a still photo capture completes.
 */
data class NveCapturedPhoto(
    val filePath: String,
    val width: Int,
    val height: Int,
    val quality: Int,
    val flashMode: Int,
    val timestampNs: Long,
)

/**
 * Metadata returned when an active video recording stops cleanly.
 */
data class NveRecordedVideo(
    val outputPath: String,
    val durationMs: Long,
    val width: Int,
    val height: Int,
    val hasAudio: Boolean,
)

/**
 * Callback interface bridging camera lifecycle, capture results, errors, and zero-copy
 * 40-byte `NVCF` frame descriptors back to the NVE host/runtime.
 */
interface NveCameraEventListener {
    fun onStateChanged(state: NveCameraSessionState) {}

    /**
     * Delivers a validated zero-copy [NveCameraFrameDescriptor] alongside its encoded 40-byte
     * `NVCF` binary slice ready to forward directly into Rust frame processors.
     */
    fun onFrame(descriptor: NveCameraFrameDescriptor, nvcfBytes: ByteArray) {}

    fun onPhotoCaptured(photo: NveCapturedPhoto) {}

    fun onRecordingStarted(outputPath: String) {}

    fun onRecordingFinished(video: NveRecordedVideo) {}

    fun onError(code: String, message: String) {}
}

/**
 * Pluggable Android camera backend contract backing `<NativeView<CameraPreview>>`
 * (`"nve.camera.preview"`).
 *
 * Accepts typed Kotlin props/commands as well as raw zero-JSON `NVNP` / `NVNC` byte arrays
 * straight from the NVE `CommandBuffer`.
 */
interface NveCameraBackend {
    val backendKind: NveCameraBackendKind
    val backendName: String
    val state: NveCameraSessionState
    val activeProps: NveCameraPreviewProps
    val selectedDevice: NveCameraDeviceInfo?

    fun listDevices(): List<NveCameraDeviceInfo>

    fun attachListener(listener: NveCameraEventListener?)

    fun applyProps(props: NveCameraPreviewProps)

    fun applyEncodedProps(nvnpBytes: ByteArray) {
        applyProps(NveCameraCodec.decodeProps(nvnpBytes))
    }

    fun dispatchCommand(command: NveCameraCommand)

    fun dispatchEncodedCommand(nvncBytes: ByteArray) {
        dispatchCommand(NveCameraCodec.decodeCommand(nvncBytes))
    }

    fun start()

    fun stop()

    fun release()
}

/**
 * Factory creating a backend instance bound to a specific `(surfaceId, nodeId)` native view node.
 */
fun interface NveCameraBackendFactory {
    fun create(surfaceId: Int, nodeId: Int): NveCameraBackend
}

/**
 * Negotiated camera settings clamped to a concrete [NveCameraDeviceInfo]'s hardware capabilities.
 */
internal data class NegotiatedCameraSettings(
    val device: NveCameraDeviceInfo,
    val effectiveFps: Int,
    val effectiveHdr: Boolean,
    val effectiveStabilization: NveStabilizationMode,
    val effectiveTorch: NveTorchMode,
    val effectiveZoom: Float,
    val enableFrameProcessor: Boolean,
    val audioEnabled: Boolean,
)

/**
 * Shared device selection, capability negotiation, and zero-copy `NVCF` frame dispatch helpers
 * reused across [NveVisionCameraAdapter] and [NveCameraXAdapter].
 */
internal object NveCameraNegotiation {
    fun selectDevice(
        devices: List<NveCameraDeviceInfo>,
        props: NveCameraPreviewProps,
    ): NveCameraDeviceInfo {
        if (devices.isEmpty()) {
            throw IllegalStateException("No camera devices available on host")
        }
        val explicitId = props.deviceId
        if (!explicitId.isNullOrEmpty()) {
            devices.firstOrNull { it.id == explicitId }?.let { return it }
            throw IllegalArgumentException("Requested camera deviceId '$explicitId' not found")
        }
        return devices.firstOrNull { it.facing == props.facing }
            ?: devices.first()
    }

    fun negotiate(
        device: NveCameraDeviceInfo,
        props: NveCameraPreviewProps,
    ): NegotiatedCameraSettings {
        val clampedFps = props.fps.coerceIn(1, device.maxFps.coerceAtLeast(1))
        val effectiveHdr = props.hdr && device.supportsHdr
        val effectiveStabilization = if (props.stabilization in device.supportedStabilization) {
            props.stabilization
        } else {
            NveStabilizationMode.OFF
        }
        val effectiveTorch = if (device.hasTorch) props.torch else NveTorchMode.OFF
        val clampedZoom = props.zoom.coerceIn(device.minZoom, device.maxZoom)
        return NegotiatedCameraSettings(
            device = device,
            effectiveFps = clampedFps,
            effectiveHdr = effectiveHdr,
            effectiveStabilization = effectiveStabilization,
            effectiveTorch = effectiveTorch,
            effectiveZoom = clampedZoom,
            enableFrameProcessor = props.enableFrameProcessor,
            audioEnabled = props.audioEnabled,
        )
    }

    fun emitValidatedFrame(
        listener: NveCameraEventListener?,
        descriptor: NveCameraFrameDescriptor,
        scratch: ByteArray,
    ) {
        if (listener == null) return
        try {
            NveCameraCodec.encodeFrameDescriptorInto(descriptor, scratch, 0)
            listener.onFrame(descriptor, scratch.copyOf())
        } catch (err: NveCameraCodecException) {
            listener.onError("INVALID_FRAME_DESCRIPTOR", err.message ?: "Malformed NVCF frame")
        }
    }
}

/**
 * Thread-safe registry selecting the active Android camera backend for `"nve.camera.preview"`.
 *
 * Precedence (when `preferredKind == null`):
 * 1. [NveCameraBackendKind.VISION_CAMERA_V5] when registered by an RN / Expo host.
 * 2. [NveCameraBackendKind.CUSTOM] when registered by a custom host.
 * 3. [NveCameraBackendKind.CAMERAX] standalone fallback (always available with zero RN deps).
 */
class NveCameraRegistry(
    defaultCameraXFactory: NveCameraBackendFactory = NveCameraBackendFactory { _, _ ->
        NveCameraXAdapter()
    },
) {
    private val lock = Any()
    private val factories = LinkedHashMap<NveCameraBackendKind, NveCameraBackendFactory>()
    private var fallbackCameraXFactory: NveCameraBackendFactory = defaultCameraXFactory

    fun register(kind: NveCameraBackendKind, factory: NveCameraBackendFactory): NveCameraRegistry {
        synchronized(lock) {
            if (kind == NveCameraBackendKind.CAMERAX) {
                fallbackCameraXFactory = factory
            }
            factories[kind] = factory
        }
        return this
    }

    fun registerVisionCamera(
        bridgeFactory: (surfaceId: Int, nodeId: Int) -> VisionCameraSessionBridge,
    ): NveCameraRegistry = register(
        NveCameraBackendKind.VISION_CAMERA_V5,
        NveCameraBackendFactory { surfaceId, nodeId ->
            NveVisionCameraAdapter(bridgeFactory(surfaceId, nodeId))
        },
    )

    fun registerCameraX(
        bindingFactory: (surfaceId: Int, nodeId: Int) -> CameraXHostBinding,
    ): NveCameraRegistry = register(
        NveCameraBackendKind.CAMERAX,
        NveCameraBackendFactory { surfaceId, nodeId ->
            NveCameraXAdapter(bindingFactory(surfaceId, nodeId))
        },
    )

    fun unregister(kind: NveCameraBackendKind): Boolean = synchronized(lock) {
        factories.remove(kind) != null
    }

    fun clearRegistrations() {
        synchronized(lock) {
            factories.clear()
        }
    }

    fun isVisionCameraRegistered(): Boolean = synchronized(lock) {
        factories.containsKey(NveCameraBackendKind.VISION_CAMERA_V5)
    }

    fun resolveBackendKind(): NveCameraBackendKind = synchronized(lock) {
        when {
            factories.containsKey(NveCameraBackendKind.VISION_CAMERA_V5) ->
                NveCameraBackendKind.VISION_CAMERA_V5
            factories.containsKey(NveCameraBackendKind.CUSTOM) ->
                NveCameraBackendKind.CUSTOM
            else -> NveCameraBackendKind.CAMERAX
        }
    }

    fun createBackend(
        surfaceId: Int,
        nodeId: Int,
        preferredKind: NveCameraBackendKind? = null,
    ): NveCameraBackend {
        val factory = synchronized(lock) {
            if (preferredKind != null) {
                factories[preferredKind]
                    ?: if (preferredKind == NveCameraBackendKind.CAMERAX) {
                        fallbackCameraXFactory
                    } else {
                        throw IllegalStateException("Camera backend $preferredKind is not registered")
                    }
            } else {
                factories[NveCameraBackendKind.VISION_CAMERA_V5]
                    ?: factories[NveCameraBackendKind.CUSTOM]
                    ?: factories[NveCameraBackendKind.CAMERAX]
                    ?: fallbackCameraXFactory
            }
        }
        return factory.create(surfaceId, nodeId)
    }

    companion object {
        /** Global default registry used by `compose-kit` camera hosts. */
        val default: NveCameraRegistry = NveCameraRegistry()
    }
}
