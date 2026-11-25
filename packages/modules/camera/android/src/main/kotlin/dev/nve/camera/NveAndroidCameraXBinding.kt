package dev.nve.camera

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CaptureRequest
import android.os.Build
import android.os.Looper
import android.util.Range
import androidx.annotation.OptIn
import androidx.camera.camera2.interop.Camera2CameraInfo
import androidx.camera.camera2.interop.Camera2Interop
import androidx.camera.camera2.interop.ExperimentalCamera2Interop
import androidx.camera.core.Camera
import androidx.camera.core.CameraInfo
import androidx.camera.core.CameraSelector
import androidx.camera.core.DynamicRange
import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.FocusMeteringAction
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.SurfaceOrientedMeteringPointFactory
import androidx.camera.core.UseCase
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.video.FileOutputOptions
import androidx.camera.video.Recorder
import androidx.camera.video.Recording
import androidx.camera.video.VideoCapture
import androidx.camera.video.VideoRecordEvent
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/**
 * Hardware-backed Jetpack CameraX [CameraXHostBinding] wiring `ProcessCameraProvider`,
 * [PreviewView], `ImageCapture`, `ImageAnalysis` (zero-copy `NVCF` descriptors), and
 * `VideoCapture<Recorder>` with zero React Native or VisionCamera dependencies.
 */
@OptIn(ExperimentalCamera2Interop::class)
class NveAndroidCameraXBinding(
    private val context: Context,
    private val lifecycleOwner: LifecycleOwner,
    private val previewView: PreviewView,
    initialProvider: ProcessCameraProvider? = null,
    private val onAsyncError: ((code: String, message: String) -> Unit)? = null,
) : CameraXHostBinding {

    private val appContext: Context = context.applicationContext
    private val mainExecutor = ContextCompat.getMainExecutor(appContext)
    private val analysisExecutor: ExecutorService = Executors.newSingleThreadExecutor { r ->
        Thread(r, "nve-camerax-analysis").apply { isDaemon = true }
    }

    private var cameraProvider: ProcessCameraProvider? = initialProvider
    private var providerRequested: Boolean = false
    private var pendingConfig: CameraXUseCaseConfig? = null
    private var pendingTorch: NveTorchMode = NveTorchMode.OFF
    private var pendingZoom: Float = 1.0f

    private var activeCamera: Camera? = null
    private var imageCapture: ImageCapture? = null
    private var imageAnalysis: ImageAnalysis? = null
    private var videoCapture: VideoCapture<Recorder>? = null
    private var activeRecording: Recording? = null
    private var activeRecordingPath: String? = null
    private var activeRecordingAudio: Boolean = false
    private var recordingStartMs: Long = 0L

    @Volatile
    private var frameAnalyzer: ((NveCameraFrameDescriptor) -> Unit)? = null
    private val captureSeq = AtomicInteger(0)

    init {
        if (cameraProvider == null) {
            requestProviderAsync()
        }
    }

    override fun availableDevices(): List<NveCameraDeviceInfo> {
        val provider = cameraProvider ?: return InMemoryCameraXHostBinding.DEFAULT_DEVICES
        val infos = provider.availableCameraInfos
        if (infos.isEmpty()) return InMemoryCameraXHostBinding.DEFAULT_DEVICES
        return infos.mapIndexed { idx, info -> mapCameraInfoToDeviceInfo(info, idx) }
    }

    override fun bindUseCases(config: CameraXUseCaseConfig) {
        pendingConfig = config
        val provider = cameraProvider
        if (provider == null) {
            requestProviderAsync()
            return
        }
        bindWithProvider(provider, config)
    }

    override fun unbindAll() {
        pendingConfig = null
        activeRecording?.stop()
        activeRecording = null
        imageAnalysis?.clearAnalyzer()
        cameraProvider?.unbindAll()
        activeCamera = null
        imageCapture = null
        imageAnalysis = null
        videoCapture = null
    }

    override fun setTorchMode(mode: NveTorchMode) {
        pendingTorch = mode
        activeCamera?.cameraControl?.enableTorch(mode == NveTorchMode.ON)
        imageCapture?.flashMode = mapFlashMode(mode.wireCode)
    }

    override fun setZoomRatio(zoom: Float) {
        pendingZoom = zoom
        activeCamera?.cameraControl?.setZoomRatio(zoom)
    }

    override fun startFocusAndMetering(x: Float, y: Float) {
        val camera = activeCamera ?: return
        val factory = SurfaceOrientedMeteringPointFactory(1.0f, 1.0f)
        val point = factory.createPoint(x.coerceIn(0f, 1f), y.coerceIn(0f, 1f))
        val action = FocusMeteringAction.Builder(
            point,
            FocusMeteringAction.FLAG_AF or FocusMeteringAction.FLAG_AE,
        ).build()
        camera.cameraControl.startFocusAndMetering(action)
    }

    override fun takePicture(quality: Int, flashMode: Int): NveCapturedPhoto {
        val capture = imageCapture
            ?: throw IllegalStateException("CameraX ImageCapture is not bound")
        val seq = captureSeq.incrementAndGet()
        val outFile = File(appContext.cacheDir, "nve_camera_capture_$seq.jpg")
        capture.flashMode = mapFlashMode(flashMode)
        val outputOptions = ImageCapture.OutputFileOptions.Builder(outFile).build()

        val isMainThread = Looper.myLooper() == Looper.getMainLooper()
        val latch = if (isMainThread) null else CountDownLatch(1)
        var captureError: ImageCaptureException? = null

        capture.takePicture(
            outputOptions,
            analysisExecutor,
            object : ImageCapture.OnImageSavedCallback {
                override fun onImageSaved(outputFileResults: ImageCapture.OutputFileResults) {
                    latch?.countDown()
                }

                override fun onError(exception: ImageCaptureException) {
                    captureError = exception
                    onAsyncError?.invoke("CAPTURE_PHOTO_FAILED", exception.message ?: "Capture failed")
                    latch?.countDown()
                }
            },
        )

        if (latch != null) {
            latch.await(5, TimeUnit.SECONDS)
            captureError?.let { throw IllegalStateException("CameraX capture failed: ${it.message}", it) }
        }
        return NveCapturedPhoto(
            filePath = outFile.absolutePath,
            width = previewView.width.coerceAtLeast(1),
            height = previewView.height.coerceAtLeast(1),
            quality = quality,
            flashMode = flashMode,
            timestampNs = System.nanoTime(),
        )
    }

    @SuppressLint("MissingPermission")
    override fun startVideoRecording(outputPath: String, audioEnabled: Boolean) {
        val vc = videoCapture
            ?: throw IllegalStateException("CameraX VideoCapture is not bound")
        val outFile = File(outputPath)
        outFile.parentFile?.mkdirs()
        val options = FileOutputOptions.Builder(outFile).build()
        val pending = vc.output.prepareRecording(appContext, options).apply {
            if (audioEnabled && hasRecordAudioPermission()) {
                withAudioEnabled()
            }
        }
        recordingStartMs = System.currentTimeMillis()
        activeRecordingPath = outputPath
        activeRecordingAudio = audioEnabled && hasRecordAudioPermission()
        activeRecording = pending.start(mainExecutor) { event ->
            if (event is VideoRecordEvent.Finalize && event.hasError()) {
                onAsyncError?.invoke(
                    "RECORDING_ERROR",
                    event.cause?.message ?: "CameraX recording error ${event.error}",
                )
            }
        }
    }

    override fun stopVideoRecording(): NveRecordedVideo {
        val path = checkNotNull(activeRecordingPath) { "No active CameraX recording" }
        val durationMs = (System.currentTimeMillis() - recordingStartMs).coerceAtLeast(0L)
        val hasAudio = activeRecordingAudio
        activeRecording?.stop()
        activeRecording = null
        activeRecordingPath = null
        activeRecordingAudio = false
        return NveRecordedVideo(
            outputPath = path,
            durationMs = durationMs,
            width = previewView.width.coerceAtLeast(1),
            height = previewView.height.coerceAtLeast(1),
            hasAudio = hasAudio,
        )
    }

    override fun setImageAnalyzer(analyzer: ((NveCameraFrameDescriptor) -> Unit)?) {
        frameAnalyzer = analyzer
        val analysis = imageAnalysis ?: return
        if (analyzer != null) {
            analysis.setAnalyzer(analysisExecutor) { imageProxy ->
                try {
                    val descriptor = extractFrameDescriptor(imageProxy)
                    if (descriptor != null) {
                        frameAnalyzer?.invoke(descriptor)
                    }
                } finally {
                    imageProxy.close()
                }
            }
        } else {
            analysis.clearAnalyzer()
        }
    }

    fun shutdown() {
        unbindAll()
        analysisExecutor.shutdown()
    }

    private fun requestProviderAsync() {
        if (providerRequested) return
        providerRequested = true
        val future = ProcessCameraProvider.getInstance(appContext)
        future.addListener(
            {
                try {
                    val provider = future.get()
                    cameraProvider = provider
                    pendingConfig?.let { bindWithProvider(provider, it) }
                } catch (err: Throwable) {
                    onAsyncError?.invoke(
                        "CAMERAX_PROVIDER_INIT_FAILED",
                        err.message ?: "Failed to initialize ProcessCameraProvider",
                    )
                }
            },
            mainExecutor,
        )
    }

    private fun bindWithProvider(provider: ProcessCameraProvider, config: CameraXUseCaseConfig) {
        val selector = buildCameraSelector(config)
        val previewBuilder = Preview.Builder()
        applyCamera2Interop(previewBuilder, config)
        val preview = previewBuilder.build().apply {
            surfaceProvider = previewView.surfaceProvider
        }

        val capture = ImageCapture.Builder()
            .setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY)
            .setFlashMode(mapFlashMode(pendingTorch.wireCode))
            .build()

        val recorder = Recorder.Builder().build()
        val video = VideoCapture.withOutput(recorder)

        val useCases = mutableListOf<UseCase>(preview, capture, video)
        var analysis: ImageAnalysis? = null
        if (config.enableImageAnalysis) {
            val analysisBuilder = ImageAnalysis.Builder()
                .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
            applyCamera2Interop(analysisBuilder, config)
            analysis = analysisBuilder.build()
            useCases.add(analysis)
        }

        provider.unbindAll()
        val camera = provider.bindToLifecycle(lifecycleOwner, selector, *useCases.toTypedArray())
        activeCamera = camera
        imageCapture = capture
        imageAnalysis = analysis
        videoCapture = video

        if (analysis != null && frameAnalyzer != null) {
            setImageAnalyzer(frameAnalyzer)
        }
        camera.cameraControl.enableTorch(pendingTorch == NveTorchMode.ON)
        camera.cameraControl.setZoomRatio(pendingZoom)
    }

    private fun buildCameraSelector(config: CameraXUseCaseConfig): CameraSelector {
        val builder = CameraSelector.Builder()
        if (config.deviceId.isNotBlank() && !config.deviceId.startsWith("camerax-")) {
            builder.addCameraFilter { infos ->
                val exact = infos.filter { info ->
                    runCatching { Camera2CameraInfo.from(info).cameraId == config.deviceId }
                        .getOrDefault(false)
                }
                exact.ifEmpty { infos }
            }
        }
        val lensFacing = mapFacingToCameraSelector(config.facing)
        if (lensFacing != null) {
            builder.requireLensFacing(lensFacing)
        }
        return builder.build()
    }

    private fun <T> applyCamera2Interop(
        extenderTarget: androidx.camera.core.ExtendableBuilder<T>,
        config: CameraXUseCaseConfig,
    ) {
        val ext = Camera2Interop.Extender(extenderTarget)
        val fps = config.targetFps.coerceIn(1, NveCameraCodec.MAX_FPS)
        ext.setCaptureRequestOption(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(fps, fps))
        ext.setCaptureRequestOption(
            CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE,
            mapStabilizationToCamera2(config.stabilization),
        )
    }

    private fun hasRecordAudioPermission(): Boolean =
        ContextCompat.checkSelfPermission(appContext, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    @OptIn(ExperimentalGetImage::class)
    private fun extractFrameDescriptor(imageProxy: ImageProxy): NveCameraFrameDescriptor? {
        val pixelFormat = mapImageFormat(imageProxy.format) ?: return null
        val strideBytes = imageProxy.planes.firstOrNull()?.rowStride ?: return null
        val nativeHandleId = resolveNativeHandleId(imageProxy)
        return buildFrameDescriptorFromMetrics(
            width = imageProxy.width,
            height = imageProxy.height,
            strideBytes = strideBytes,
            pixelFormat = pixelFormat,
            rotationDegrees = imageProxy.imageInfo.rotationDegrees,
            timestampNs = imageProxy.imageInfo.timestamp,
            nativeHandleId = nativeHandleId,
        )
    }

    @OptIn(ExperimentalGetImage::class)
    private fun resolveNativeHandleId(imageProxy: ImageProxy): Long {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            val hb = runCatching { imageProxy.image?.hardwareBuffer }.getOrNull()
            if (hb != null) {
                return try {
                    (System.identityHashCode(hb).toLong() and 0xFFFF_FFFFL) or (1L shl 48)
                } finally {
                    hb.close()
                }
            }
        }
        return System.identityHashCode(imageProxy).toLong() and 0xFFFF_FFFFL
    }

    companion object {
        internal fun mapCameraInfoToDeviceInfo(info: CameraInfo, index: Int): NveCameraDeviceInfo {
            val c2 = runCatching { Camera2CameraInfo.from(info) }.getOrNull()
            val id = c2?.cameraId ?: "camerax-$index"
            val facing = mapCameraSelectorToFacing(info.lensFacing)
            val zoom = info.zoomState.value
            val fpsRanges = c2?.getCameraCharacteristic(
                CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES,
            )
            val maxFps = fpsRanges?.maxOfOrNull { it.upper }?.coerceIn(1, NveCameraCodec.MAX_FPS) ?: 30
            val stabModes = c2?.getCameraCharacteristic(
                CameraCharacteristics.CONTROL_AVAILABLE_VIDEO_STABILIZATION_MODES,
            )
            val supportedStab = mapSupportedStabilizationModes(stabModes)
            val supportsHdr = runCatching {
                info.querySupportedDynamicRanges(setOf(DynamicRange.HDR_UNSPECIFIED_10_BIT)).isNotEmpty()
            }.getOrDefault(false)

            return NveCameraDeviceInfo(
                id = id,
                name = "CameraX ${facing.name.lowercase().replaceFirstChar(Char::uppercase)} ($id)",
                facing = facing,
                hasTorch = info.hasFlashUnit(),
                minZoom = zoom?.minZoomRatio ?: 1.0f,
                maxZoom = zoom?.maxZoomRatio ?: 4.0f,
                maxFps = maxFps,
                supportsHdr = supportsHdr,
                supportedStabilization = supportedStab,
            )
        }

        internal fun mapFacingToCameraSelector(facing: NveCameraFacing): Int? = when (facing) {
            NveCameraFacing.BACK -> CameraSelector.LENS_FACING_BACK
            NveCameraFacing.FRONT -> CameraSelector.LENS_FACING_FRONT
            NveCameraFacing.EXTERNAL -> CameraSelector.LENS_FACING_EXTERNAL
        }

        internal fun mapCameraSelectorToFacing(lensFacing: Int): NveCameraFacing = when (lensFacing) {
            CameraSelector.LENS_FACING_FRONT -> NveCameraFacing.FRONT
            CameraSelector.LENS_FACING_EXTERNAL -> NveCameraFacing.EXTERNAL
            else -> NveCameraFacing.BACK
        }

        internal fun mapFlashMode(flashWireCode: Int): Int = when (flashWireCode) {
            NveTorchMode.ON.wireCode -> ImageCapture.FLASH_MODE_ON
            NveTorchMode.AUTO.wireCode -> ImageCapture.FLASH_MODE_AUTO
            else -> ImageCapture.FLASH_MODE_OFF
        }

        internal fun mapStabilizationToCamera2(mode: NveStabilizationMode): Int = when (mode) {
            NveStabilizationMode.OFF -> CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_OFF
            NveStabilizationMode.STANDARD -> CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON
            NveStabilizationMode.CINEMATIC ->
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_PREVIEW_STABILIZATION
                } else {
                    CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON
                }
        }

        internal fun mapSupportedStabilizationModes(rawModes: IntArray?): Set<NveStabilizationMode> {
            val out = LinkedHashSet<NveStabilizationMode>()
            out.add(NveStabilizationMode.OFF)
            if (rawModes != null) {
                for (m in rawModes) {
                    if (m == CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON) {
                        out.add(NveStabilizationMode.STANDARD)
                    } else if (m == 2) { // CONTROL_VIDEO_STABILIZATION_MODE_PREVIEW_STABILIZATION (API 33)
                        out.add(NveStabilizationMode.CINEMATIC)
                    }
                }
            }
            return out
        }

        internal fun mapImageFormat(androidFormat: Int): NveCameraPixelFormat? = when (androidFormat) {
            ImageFormat.YUV_420_888 -> NveCameraPixelFormat.YUV420_888
            android.graphics.PixelFormat.RGBA_8888 -> NveCameraPixelFormat.RGBA8888
            else -> null
        }

        internal fun buildFrameDescriptorFromMetrics(
            width: Int,
            height: Int,
            strideBytes: Int,
            pixelFormat: NveCameraPixelFormat,
            rotationDegrees: Int,
            timestampNs: Long,
            nativeHandleId: Long,
        ): NveCameraFrameDescriptor? {
            if (width <= 0 || height <= 0) return null
            val normalizedRot = ((rotationDegrees % 360) + 360) % 360
            val snappedRot = when (normalizedRot) {
                0, 90, 180, 270 -> normalizedRot
                else -> 0
            }
            val minStride = (width.toLong() * pixelFormat.bytesPerPrimarySample).toInt()
            val safeStride = maxOf(strideBytes, minStride)
            return NveCameraFrameDescriptor(
                width = width,
                height = height,
                strideBytes = safeStride,
                pixelFormat = pixelFormat,
                rotationDeg = snappedRot,
                timestampNs = timestampNs,
                nativeHandleId = nativeHandleId,
            )
        }
    }
}

/**
 * Registers the hardware Jetpack CameraX binding into [NveCameraRegistry] for standalone
 * Android / Jetpack Compose hosts.
 */
fun NveCameraRegistry.registerAndroidCameraX(
    context: Context,
    lifecycleOwner: LifecycleOwner,
    previewView: PreviewView,
    onAsyncError: ((code: String, message: String) -> Unit)? = null,
): NveCameraRegistry = registerCameraX { _, _ ->
    NveAndroidCameraXBinding(
        context = context,
        lifecycleOwner = lifecycleOwner,
        previewView = previewView,
        onAsyncError = onAsyncError,
    )
}
