package dev.nve.compose.camera

/**
 * Declarative session constraint bundle mapped onto Margelo `react-native-vision-camera` v5's
 * Nitro `HybridCameraSession` configuration (`CameraSessionConfig` / `CameraOutputConfiguration`).
 */
data class VisionCameraConstraints(
    val targetFps: Int,
    val enableHdr: Boolean,
    val stabilization: NveStabilizationMode,
    val enableFrameProcessor: Boolean,
    val enableAudio: Boolean,
    val pixelFormat: NveCameraPixelFormat = NveCameraPixelFormat.YUV420_888,
)

/**
 * Zero-reflection pluggable bridge interface connecting [NveVisionCameraAdapter] to a host
 * app's `react-native-vision-camera` v5 Nitro `HybridCameraSession` / `HybridCameraView`.
 *
 * Keeping this interface free of React Native and VisionCamera types allows standalone
 * Android / Jetpack Compose builds of `compose-kit` to compile with zero RN dependencies
 * while RN / Expo hosts plug in their compiled Nitro bridge directly.
 */
interface VisionCameraSessionBridge {
    fun enumerateDevices(): List<NveCameraDeviceInfo>

    fun configureSession(deviceId: String, constraints: VisionCameraConstraints)

    fun setActive(active: Boolean)

    fun setTorch(mode: NveTorchMode)

    fun setZoom(zoom: Float)

    fun focus(x: Float, y: Float)

    fun capturePhoto(quality: Int, flashMode: Int): NveCapturedPhoto

    fun startRecording(outputPath: String, enableAudio: Boolean)

    fun stopRecording(): NveRecordedVideo

    fun setFrameCallback(callback: ((NveCameraFrameDescriptor) -> Unit)?)

    fun release()
}

/**
 * Pluggable [NveCameraBackend] adapter delegating to Margelo `react-native-vision-camera` v5
 * through [VisionCameraSessionBridge].
 *
 * Performs hardware capability negotiation (FPS clamping, HDR/stabilization/torch/zoom
 * fallback), diff-aware reconfiguration to avoid unnecessary camera session restarts, and
 * zero-copy 40-byte `NVCF` frame descriptor forwarding.
 */
class NveVisionCameraAdapter(
    private val bridge: VisionCameraSessionBridge,
    private val preferredPixelFormat: NveCameraPixelFormat = NveCameraPixelFormat.YUV420_888,
) : NveCameraBackend {
    override val backendKind: NveCameraBackendKind = NveCameraBackendKind.VISION_CAMERA_V5
    override val backendName: String = "vision-camera-v5"

    override var state: NveCameraSessionState = NveCameraSessionState.IDLE
        private set

    override var activeProps: NveCameraPreviewProps = NveCameraPreviewProps()
        private set

    override var selectedDevice: NveCameraDeviceInfo? = null
        private set

    var lastNegotiatedConstraints: VisionCameraConstraints? = null
        private set

    private var listener: NveCameraEventListener? = null
    private var lastAppliedTorch: NveTorchMode? = null
    private var lastAppliedZoom: Float? = null
    private var frameCallbackInstalled: Boolean = false
    private val frameScratch = ByteArray(NveCameraCodec.NVCF_SIZE)

    override fun listDevices(): List<NveCameraDeviceInfo> {
        ensureNotReleased()
        return bridge.enumerateDevices()
    }

    override fun attachListener(listener: NveCameraEventListener?) {
        this.listener = listener
    }

    override fun applyProps(props: NveCameraPreviewProps) {
        ensureNotReleased()
        props.validate()

        val devices = bridge.enumerateDevices()
        val device = NveCameraNegotiation.selectDevice(devices, props)
        val negotiated = NveCameraNegotiation.negotiate(device, props)

        val constraints = VisionCameraConstraints(
            targetFps = negotiated.effectiveFps,
            enableHdr = negotiated.effectiveHdr,
            stabilization = negotiated.effectiveStabilization,
            enableFrameProcessor = negotiated.enableFrameProcessor,
            enableAudio = negotiated.audioEnabled,
            pixelFormat = preferredPixelFormat,
        )

        val needsReconfigure =
            selectedDevice?.id != device.id || lastNegotiatedConstraints != constraints
        if (needsReconfigure) {
            bridge.configureSession(device.id, constraints)
            selectedDevice = device
            lastNegotiatedConstraints = constraints
            if (state == NveCameraSessionState.IDLE) {
                transitionTo(NveCameraSessionState.CONFIGURED)
            }
        }

        syncFrameCallback(negotiated.enableFrameProcessor)

        if (needsReconfigure || lastAppliedTorch != negotiated.effectiveTorch) {
            bridge.setTorch(negotiated.effectiveTorch)
            lastAppliedTorch = negotiated.effectiveTorch
        }
        if (needsReconfigure || lastAppliedZoom != negotiated.effectiveZoom) {
            bridge.setZoom(negotiated.effectiveZoom)
            lastAppliedZoom = negotiated.effectiveZoom
        }

        activeProps = props.copy(
            deviceId = device.id,
            facing = device.facing,
            torch = negotiated.effectiveTorch,
            zoom = negotiated.effectiveZoom,
            fps = negotiated.effectiveFps,
            hdr = negotiated.effectiveHdr,
            stabilization = negotiated.effectiveStabilization,
        )

        if (props.active && state != NveCameraSessionState.RUNNING && state != NveCameraSessionState.RECORDING) {
            bridge.setActive(true)
            transitionTo(NveCameraSessionState.RUNNING)
        } else if (!props.active && (state == NveCameraSessionState.RUNNING || state == NveCameraSessionState.RECORDING)) {
            if (state == NveCameraSessionState.RECORDING) {
                val video = bridge.stopRecording()
                listener?.onRecordingFinished(video)
            }
            bridge.setActive(false)
            transitionTo(NveCameraSessionState.STOPPED)
        }
    }

    override fun dispatchCommand(command: NveCameraCommand) {
        ensureNotReleased()
        when (command) {
            is NveCameraCommand.SwitchFacing -> {
                applyProps(activeProps.copy(deviceId = null, facing = command.facing))
            }
            is NveCameraCommand.SetTorch -> {
                val device = requireConfiguredDevice()
                val effective = if (device.hasTorch) command.mode else NveTorchMode.OFF
                bridge.setTorch(effective)
                lastAppliedTorch = effective
                activeProps = activeProps.copy(torch = effective)
            }
            is NveCameraCommand.SetZoom -> {
                NveCameraCodec.validateZoom(command.zoom)
                val device = requireConfiguredDevice()
                val clamped = command.zoom.coerceIn(device.minZoom, device.maxZoom)
                bridge.setZoom(clamped)
                lastAppliedZoom = clamped
                activeProps = activeProps.copy(zoom = clamped)
            }
            is NveCameraCommand.FocusPoint -> {
                NveCameraCodec.validateUnitCoord(command.x, "focus_point.x")
                NveCameraCodec.validateUnitCoord(command.y, "focus_point.y")
                requireConfiguredDevice()
                bridge.focus(command.x, command.y)
            }
            is NveCameraCommand.CapturePhoto -> {
                NveCameraCodec.validateQuality(command.quality)
                NveCameraCodec.validateFlash(command.flash)
                requireConfiguredDevice()
                val photo = bridge.capturePhoto(command.quality, command.flash)
                listener?.onPhotoCaptured(photo)
            }
            is NveCameraCommand.StartRecording -> {
                require(command.outputPath.isNotBlank()) { "Recording outputPath must not be blank" }
                requireConfiguredDevice()
                check(state != NveCameraSessionState.RECORDING) { "Camera is already recording" }
                if (state != NveCameraSessionState.RUNNING) {
                    bridge.setActive(true)
                }
                bridge.startRecording(command.outputPath, activeProps.audioEnabled)
                transitionTo(NveCameraSessionState.RECORDING)
                listener?.onRecordingStarted(command.outputPath)
            }
            NveCameraCommand.StopRecording -> {
                check(state == NveCameraSessionState.RECORDING) { "Camera is not currently recording" }
                val video = bridge.stopRecording()
                transitionTo(NveCameraSessionState.RUNNING)
                listener?.onRecordingFinished(video)
            }
        }
    }

    override fun start() {
        ensureNotReleased()
        if (selectedDevice == null) {
            applyProps(activeProps.copy(active = true))
            return
        }
        if (state != NveCameraSessionState.RUNNING && state != NveCameraSessionState.RECORDING) {
            bridge.setActive(true)
            activeProps = activeProps.copy(active = true)
            transitionTo(NveCameraSessionState.RUNNING)
        }
    }

    override fun stop() {
        ensureNotReleased()
        if (state == NveCameraSessionState.RECORDING) {
            val video = bridge.stopRecording()
            listener?.onRecordingFinished(video)
        }
        if (state == NveCameraSessionState.RUNNING || state == NveCameraSessionState.RECORDING) {
            bridge.setActive(false)
            activeProps = activeProps.copy(active = false)
            transitionTo(NveCameraSessionState.STOPPED)
        }
    }

    override fun release() {
        if (state == NveCameraSessionState.RELEASED) return
        if (state == NveCameraSessionState.RECORDING) {
            val video = bridge.stopRecording()
            listener?.onRecordingFinished(video)
        }
        if (frameCallbackInstalled) {
            bridge.setFrameCallback(null)
            frameCallbackInstalled = false
        }
        if (state == NveCameraSessionState.RUNNING || state == NveCameraSessionState.RECORDING) {
            bridge.setActive(false)
        }
        bridge.release()
        selectedDevice = null
        lastNegotiatedConstraints = null
        lastAppliedTorch = null
        lastAppliedZoom = null
        transitionTo(NveCameraSessionState.RELEASED)
    }

    private fun syncFrameCallback(enable: Boolean) {
        if (enable && !frameCallbackInstalled) {
            bridge.setFrameCallback { descriptor ->
                NveCameraNegotiation.emitValidatedFrame(listener, descriptor, frameScratch)
            }
            frameCallbackInstalled = true
        } else if (!enable && frameCallbackInstalled) {
            bridge.setFrameCallback(null)
            frameCallbackInstalled = false
        }
    }

    private fun requireConfiguredDevice(): NveCameraDeviceInfo {
        if (selectedDevice == null) {
            applyProps(activeProps)
        }
        return checkNotNull(selectedDevice) { "Camera device failed to initialize" }
    }

    private fun ensureNotReleased() {
        check(state != NveCameraSessionState.RELEASED) {
            "NveVisionCameraAdapter has already been released"
        }
    }

    private fun transitionTo(next: NveCameraSessionState) {
        if (state != next) {
            state = next
            listener?.onStateChanged(next)
        }
    }
}
