package dev.nve.compose.camera

/**
 * CameraX use-case binding configuration mirroring `ProcessCameraProvider.bindToLifecycle(...)`
 * (`Preview` + `ImageCapture` + optional `ImageAnalysis` + `VideoCapture`).
 */
data class CameraXUseCaseConfig(
    val deviceId: String,
    val facing: NveCameraFacing,
    val targetFps: Int,
    val hdrEnabled: Boolean,
    val stabilization: NveStabilizationMode,
    val enableImageAnalysis: Boolean,
    val enableAudio: Boolean,
)

/**
 * Pluggable Android CameraX lifecycle/binding hook interface.
 *
 * Android apps linking `androidx.camera:camera-{core,camera2,lifecycle,video,view}` implement
 * this interface directly against `ProcessCameraProvider` and `CameraControl`, while standalone
 * unit/headless builds of `compose-kit` use [InMemoryCameraXHostBinding] with zero extra Gradle
 * dependencies and zero React Native / VisionCamera coupling.
 */
interface CameraXHostBinding {
    fun availableDevices(): List<NveCameraDeviceInfo>

    fun bindUseCases(config: CameraXUseCaseConfig)

    fun unbindAll()

    fun setTorchMode(mode: NveTorchMode)

    fun setZoomRatio(zoom: Float)

    fun startFocusAndMetering(x: Float, y: Float)

    fun takePicture(quality: Int, flashMode: Int): NveCapturedPhoto

    fun startVideoRecording(outputPath: String, audioEnabled: Boolean)

    fun stopVideoRecording(): NveRecordedVideo

    fun setImageAnalyzer(analyzer: ((NveCameraFrameDescriptor) -> Unit)?)
}

/**
 * Lightweight standalone [CameraXHostBinding] implementation used when no hardware CameraX
 * provider is attached (e.g., previews, headless JVM tests, and default registry fallback).
 */
class InMemoryCameraXHostBinding(
    private val devices: List<NveCameraDeviceInfo> = DEFAULT_DEVICES,
) : CameraXHostBinding {
    var boundConfig: CameraXUseCaseConfig? = null
        private set
    var torchMode: NveTorchMode = NveTorchMode.OFF
        private set
    var zoomRatio: Float = 1.0f
        private set
    var lastFocusPoint: Pair<Float, Float>? = null
        private set
    var activeRecordingPath: String? = null
        private set
    var activeRecordingAudio: Boolean = false
        private set
    var bindCount: Int = 0
        private set
    var unbindCount: Int = 0
        private set

    private var analyzer: ((NveCameraFrameDescriptor) -> Unit)? = null
    private var captureSeq: Int = 0

    override fun availableDevices(): List<NveCameraDeviceInfo> = devices

    override fun bindUseCases(config: CameraXUseCaseConfig) {
        boundConfig = config
        bindCount++
    }

    override fun unbindAll() {
        boundConfig = null
        unbindCount++
    }

    override fun setTorchMode(mode: NveTorchMode) {
        torchMode = mode
    }

    override fun setZoomRatio(zoom: Float) {
        zoomRatio = zoom
    }

    override fun startFocusAndMetering(x: Float, y: Float) {
        lastFocusPoint = x to y
    }

    override fun takePicture(quality: Int, flashMode: Int): NveCapturedPhoto {
        captureSeq++
        return NveCapturedPhoto(
            filePath = "/cache/camerax_photo_$captureSeq.jpg",
            width = 1920,
            height = 1080,
            quality = quality,
            flashMode = flashMode,
            timestampNs = captureSeq * 16_666_666L,
        )
    }

    override fun startVideoRecording(outputPath: String, audioEnabled: Boolean) {
        activeRecordingPath = outputPath
        activeRecordingAudio = audioEnabled
    }

    override fun stopVideoRecording(): NveRecordedVideo {
        val path = checkNotNull(activeRecordingPath) { "No active CameraX recording" }
        val audio = activeRecordingAudio
        activeRecordingPath = null
        activeRecordingAudio = false
        return NveRecordedVideo(
            outputPath = path,
            durationMs = 1_000L,
            width = 1920,
            height = 1080,
            hasAudio = audio,
        )
    }

    override fun setImageAnalyzer(analyzer: ((NveCameraFrameDescriptor) -> Unit)?) {
        this.analyzer = analyzer
    }

    /** Injects a synthetic zero-copy hardware frame descriptor for testing / frame pipelines. */
    fun emitSimulatedFrame(descriptor: NveCameraFrameDescriptor) {
        analyzer?.invoke(descriptor)
    }

    companion object {
        val DEFAULT_DEVICES: List<NveCameraDeviceInfo> = listOf(
            NveCameraDeviceInfo(
                id = "camerax-back-0",
                name = "Back Wide Camera",
                facing = NveCameraFacing.BACK,
                hasTorch = true,
                minZoom = 0.5f,
                maxZoom = 10.0f,
                maxFps = 60,
                supportsHdr = true,
                supportedStabilization = setOf(
                    NveStabilizationMode.OFF,
                    NveStabilizationMode.STANDARD,
                    NveStabilizationMode.CINEMATIC,
                ),
            ),
            NveCameraDeviceInfo(
                id = "camerax-front-1",
                name = "Front Selfie Camera",
                facing = NveCameraFacing.FRONT,
                hasTorch = false,
                minZoom = 1.0f,
                maxZoom = 3.0f,
                maxFps = 30,
                supportsHdr = false,
                supportedStabilization = setOf(
                    NveStabilizationMode.OFF,
                    NveStabilizationMode.STANDARD,
                ),
            ),
        )
    }
}

/**
 * Standalone zero-RN Android CameraX [NveCameraBackend] implementation with explicit Android
 * lifecycle hooks (`onLifecycleStart` / `onLifecycleStop` / `onLifecycleDestroy`).
 */
class NveCameraXAdapter(
    private val binding: CameraXHostBinding = InMemoryCameraXHostBinding(),
) : NveCameraBackend {
    override val backendKind: NveCameraBackendKind = NveCameraBackendKind.CAMERAX
    override val backendName: String = "camerax"

    override var state: NveCameraSessionState = NveCameraSessionState.IDLE
        private set

    override var activeProps: NveCameraPreviewProps = NveCameraPreviewProps()
        private set

    override var selectedDevice: NveCameraDeviceInfo? = null
        private set

    var boundUseCaseConfig: CameraXUseCaseConfig? = null
        private set

    private var listener: NveCameraEventListener? = null
    private var isBound: Boolean = false
    private var analyzerAttached: Boolean = false
    private var lifecycleActive: Boolean = true
    private val frameScratch = ByteArray(NveCameraCodec.NVCF_SIZE)

    override fun listDevices(): List<NveCameraDeviceInfo> {
        ensureNotReleased()
        return binding.availableDevices()
    }

    override fun attachListener(listener: NveCameraEventListener?) {
        this.listener = listener
    }

    override fun applyProps(props: NveCameraPreviewProps) {
        ensureNotReleased()
        props.validate()

        val device = NveCameraNegotiation.selectDevice(binding.availableDevices(), props)
        val negotiated = NveCameraNegotiation.negotiate(device, props)

        val desiredConfig = CameraXUseCaseConfig(
            deviceId = device.id,
            facing = device.facing,
            targetFps = negotiated.effectiveFps,
            hdrEnabled = negotiated.effectiveHdr,
            stabilization = negotiated.effectiveStabilization,
            enableImageAnalysis = negotiated.enableFrameProcessor,
            enableAudio = negotiated.audioEnabled,
        )

        selectedDevice = device
        activeProps = props.copy(
            deviceId = device.id,
            facing = device.facing,
            torch = negotiated.effectiveTorch,
            zoom = negotiated.effectiveZoom,
            fps = negotiated.effectiveFps,
            hdr = negotiated.effectiveHdr,
            stabilization = negotiated.effectiveStabilization,
        )

        val shouldRun = props.active && lifecycleActive
        if (shouldRun) {
            val needsBind = !isBound || boundUseCaseConfig != desiredConfig
            if (needsBind) {
                if (state == NveCameraSessionState.RECORDING) {
                    val video = binding.stopVideoRecording()
                    listener?.onRecordingFinished(video)
                }
                binding.bindUseCases(desiredConfig)
                boundUseCaseConfig = desiredConfig
                isBound = true
            }
            syncAnalyzer(negotiated.enableFrameProcessor)
            binding.setTorchMode(negotiated.effectiveTorch)
            binding.setZoomRatio(negotiated.effectiveZoom)
            if (state != NveCameraSessionState.RECORDING) {
                transitionTo(NveCameraSessionState.RUNNING)
            }
        } else {
            boundUseCaseConfig = desiredConfig
            unbindIfNeeded()
            transitionTo(
                if (state == NveCameraSessionState.IDLE) NveCameraSessionState.CONFIGURED
                else NveCameraSessionState.STOPPED,
            )
        }
    }

    override fun dispatchCommand(command: NveCameraCommand) {
        ensureNotReleased()
        when (command) {
            is NveCameraCommand.SwitchFacing -> {
                applyProps(activeProps.copy(deviceId = null, facing = command.facing))
            }
            is NveCameraCommand.SetTorch -> {
                val device = ensureBoundDevice()
                val effective = if (device.hasTorch) command.mode else NveTorchMode.OFF
                binding.setTorchMode(effective)
                activeProps = activeProps.copy(torch = effective)
            }
            is NveCameraCommand.SetZoom -> {
                NveCameraCodec.validateZoom(command.zoom)
                val device = ensureBoundDevice()
                val clamped = command.zoom.coerceIn(device.minZoom, device.maxZoom)
                binding.setZoomRatio(clamped)
                activeProps = activeProps.copy(zoom = clamped)
            }
            is NveCameraCommand.FocusPoint -> {
                NveCameraCodec.validateUnitCoord(command.x, "focus_point.x")
                NveCameraCodec.validateUnitCoord(command.y, "focus_point.y")
                ensureBoundDevice()
                binding.startFocusAndMetering(command.x, command.y)
            }
            is NveCameraCommand.CapturePhoto -> {
                NveCameraCodec.validateQuality(command.quality)
                NveCameraCodec.validateFlash(command.flash)
                ensureBoundDevice()
                val photo = binding.takePicture(command.quality, command.flash)
                listener?.onPhotoCaptured(photo)
            }
            is NveCameraCommand.StartRecording -> {
                require(command.outputPath.isNotBlank()) { "Recording outputPath must not be blank" }
                ensureBoundDevice()
                check(state != NveCameraSessionState.RECORDING) { "CameraX is already recording" }
                binding.startVideoRecording(command.outputPath, activeProps.audioEnabled)
                transitionTo(NveCameraSessionState.RECORDING)
                listener?.onRecordingStarted(command.outputPath)
            }
            NveCameraCommand.StopRecording -> {
                check(state == NveCameraSessionState.RECORDING) { "CameraX is not currently recording" }
                val video = binding.stopVideoRecording()
                transitionTo(NveCameraSessionState.RUNNING)
                listener?.onRecordingFinished(video)
            }
        }
    }

    override fun start() {
        ensureNotReleased()
        lifecycleActive = true
        applyProps(activeProps.copy(active = true))
    }

    override fun stop() {
        ensureNotReleased()
        activeProps = activeProps.copy(active = false)
        unbindIfNeeded()
        if (state != NveCameraSessionState.IDLE) {
            transitionTo(NveCameraSessionState.STOPPED)
        }
    }

    override fun release() {
        if (state == NveCameraSessionState.RELEASED) return
        unbindIfNeeded()
        selectedDevice = null
        boundUseCaseConfig = null
        transitionTo(NveCameraSessionState.RELEASED)
    }

    /** Lifecycle hook intended for Compose `LifecycleEventObserver` (`ON_START` / `ON_RESUME`). */
    fun onLifecycleStart() {
        if (state == NveCameraSessionState.RELEASED) return
        lifecycleActive = true
        if (activeProps.active && selectedDevice != null && !isBound) {
            applyProps(activeProps)
        }
    }

    /** Lifecycle hook intended for Compose `LifecycleEventObserver` (`ON_STOP` / `ON_PAUSE`). */
    fun onLifecycleStop() {
        if (state == NveCameraSessionState.RELEASED) return
        lifecycleActive = false
        unbindIfNeeded()
        if (state == NveCameraSessionState.RUNNING || state == NveCameraSessionState.RECORDING) {
            transitionTo(NveCameraSessionState.STOPPED)
        }
    }

    /** Lifecycle hook intended for Compose `DisposableEffect.onDispose` (`ON_DESTROY`). */
    fun onLifecycleDestroy() {
        release()
    }

    private fun unbindIfNeeded() {
        if (state == NveCameraSessionState.RECORDING) {
            val video = binding.stopVideoRecording()
            listener?.onRecordingFinished(video)
        }
        if (analyzerAttached) {
            binding.setImageAnalyzer(null)
            analyzerAttached = false
        }
        if (isBound) {
            binding.unbindAll()
            isBound = false
        }
    }

    private fun syncAnalyzer(enable: Boolean) {
        if (enable && !analyzerAttached) {
            binding.setImageAnalyzer { descriptor ->
                NveCameraNegotiation.emitValidatedFrame(listener, descriptor, frameScratch)
            }
            analyzerAttached = true
        } else if (!enable && analyzerAttached) {
            binding.setImageAnalyzer(null)
            analyzerAttached = false
        }
    }

    private fun ensureBoundDevice(): NveCameraDeviceInfo {
        if (selectedDevice == null || !isBound) {
            applyProps(activeProps.copy(active = true))
        }
        return checkNotNull(selectedDevice) { "CameraX device failed to bind" }
    }

    private fun ensureNotReleased() {
        check(state != NveCameraSessionState.RELEASED) {
            "NveCameraXAdapter has already been released"
        }
    }

    private fun transitionTo(next: NveCameraSessionState) {
        if (state != next) {
            state = next
            listener?.onStateChanged(next)
        }
    }
}
