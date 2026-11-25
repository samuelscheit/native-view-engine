package dev.nve.camera

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class NveCameraBackendTest {

    // =========================================================================
    // 1. Cross-language NVNP / NVNC / NVCF byte parity with packages/core/interop
    // =========================================================================

    @Test
    fun matchesExactRustGoldenWireVectorsForNvnpNvncAndNvcf() {
        // Golden NVNP bytes for default NveCameraPreviewProps (must match Rust test byte-for-byte)
        val expectedDefaultProps = byteArrayOf(
            'N'.code.toByte(), 'V'.code.toByte(), 'N'.code.toByte(), 'P'.code.toByte(), 1, 10, 0,
            1, 0, 0, // field 1: device_id = Null
            2, 0, 2, 1, 0, 0, 0, // field 2: facing = U32(1 = BACK)
            3, 0, 2, 0, 0, 0, 0, // field 3: torch = U32(0 = OFF)
            4, 0, 5, 0x00, 0x00, 0x80.toByte(), 0x3F, // field 4: zoom = F32(1.0)
            5, 0, 2, 30, 0, 0, 0, // field 5: fps = U32(30)
            6, 0, 1, 0, // field 6: hdr = Bool(false)
            7, 0, 2, 0, 0, 0, 0, // field 7: stabilization = U32(0 = OFF)
            8, 0, 1, 0, // field 8: enable_frame_processor = Bool(false)
            9, 0, 1, 0, // field 9: audio_enabled = Bool(false)
            10, 0, 1, 1, // field 10: active = Bool(true)
        )
        val encodedDefault = NveCameraCodec.encodeProps(NveCameraPreviewProps())
        assertArrayEquals(expectedDefaultProps, encodedDefault)
        assertEquals(NveCameraPreviewProps(), NveCameraCodec.decodeProps(expectedDefaultProps))

        // Golden NVNC bytes for FocusPoint(0.25f, 0.75f)
        val expectedFocus = byteArrayOf(
            'N'.code.toByte(), 'V'.code.toByte(), 'N'.code.toByte(), 'C'.code.toByte(), 1, 4, 0, 2,
            5, 0x00, 0x00, 0x80.toByte(), 0x3E, // F32(0.25)
            5, 0x00, 0x00, 0x40, 0x3F, // F32(0.75)
        )
        val encodedFocus = NveCameraCodec.encodeCommand(NveCameraCommand.FocusPoint(0.25f, 0.75f))
        assertArrayEquals(expectedFocus, encodedFocus)
        assertEquals(NveCameraCommand.FocusPoint(0.25f, 0.75f), NveCameraCodec.decodeCommand(expectedFocus))

        // Golden NVCF 40-byte vector
        val expectedNvcf = byteArrayOf(
            'N'.code.toByte(), 'V'.code.toByte(), 'C'.code.toByte(), 'F'.code.toByte(), 1, 3, 0x0E, 0x01,
            0x80.toByte(), 0x07, 0x00, 0x00, // width = 1920
            0x38, 0x04, 0x00, 0x00, // height = 1080
            0x80.toByte(), 0x07, 0x00, 0x00, // stride_bytes = 1920
            0x00, 0x00, 0x00, 0x00, // reserved = 0
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // timestamp_ns
            0x88.toByte(), 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // native_handle_id
        )
        val desc = NveCameraFrameDescriptor(
            width = 1920,
            height = 1080,
            strideBytes = 1920,
            pixelFormat = NveCameraPixelFormat.YUV420_888,
            rotationDeg = 270,
            timestampNs = 0x0102_0304_0506_0708L,
            nativeHandleId = 0x1122_3344_5566_7788L,
        )
        val encodedNvcf = NveCameraCodec.encodeFrameDescriptor(desc)
        assertEquals(40, encodedNvcf.size)
        assertArrayEquals(expectedNvcf, encodedNvcf)
        assertEquals(desc, NveCameraCodec.decodeFrameDescriptor(expectedNvcf))
    }

    @Test
    fun roundTripsAllCameraPropsCommandsAndPixelFormats() {
        val customProps = NveCameraPreviewProps(
            deviceId = "camera-ultra-wide-0",
            facing = NveCameraFacing.EXTERNAL,
            torch = NveTorchMode.AUTO,
            zoom = 0.5f,
            fps = 120,
            hdr = true,
            stabilization = NveStabilizationMode.CINEMATIC,
            enableFrameProcessor = true,
            audioEnabled = true,
            active = false,
        )
        assertEquals(customProps, NveCameraCodec.decodeProps(NveCameraCodec.encodeProps(customProps)))

        val allCommands = listOf(
            NveCameraCommand.SwitchFacing(NveCameraFacing.BACK),
            NveCameraCommand.SwitchFacing(NveCameraFacing.FRONT),
            NveCameraCommand.SwitchFacing(NveCameraFacing.EXTERNAL),
            NveCameraCommand.SetTorch(NveTorchMode.OFF),
            NveCameraCommand.SetTorch(NveTorchMode.ON),
            NveCameraCommand.SetTorch(NveTorchMode.AUTO),
            NveCameraCommand.SetZoom(4.25f),
            NveCameraCommand.FocusPoint(0.0f, 1.0f),
            NveCameraCommand.CapturePhoto(quality = 95, flash = 2),
            NveCameraCommand.StartRecording("/data/user/0/dev.nve/files/demo_🎥.mp4"),
            NveCameraCommand.StopRecording,
        )
        for (cmd in allCommands) {
            val encoded = NveCameraCodec.encodeCommand(cmd)
            assertEquals(cmd, NveCameraCodec.decodeCommand(encoded))
        }

        for ((idx, fmt) in NveCameraPixelFormat.entries.withIndex()) {
            val minStride = (1280L * fmt.bytesPerPrimarySample).toInt()
            val frame = NveCameraFrameDescriptor(
                width = 1280,
                height = 720,
                strideBytes = minStride + 32,
                pixelFormat = fmt,
                rotationDeg = idx * 90,
                timestampNs = 123_456_789L + idx,
                nativeHandleId = -1L xor idx.toLong(),
            )
            assertEquals(frame, NveCameraCodec.decodeFrameDescriptor(NveCameraCodec.encodeFrameDescriptor(frame)))
        }
    }

    @Test
    fun rejectsCorruptTruncatedNanAndOutOfRangeBinaryPayloads() {
        // Invalid props values on encode
        assertThrows(NveCameraCodecException.InvalidValue::class.java) {
            NveCameraCodec.encodeProps(NveCameraPreviewProps(zoom = Float.NaN))
        }
        assertThrows(NveCameraCodecException.InvalidValue::class.java) {
            NveCameraCodec.encodeProps(NveCameraPreviewProps(zoom = Float.POSITIVE_INFINITY))
        }
        assertThrows(NveCameraCodecException.InvalidValue::class.java) {
            NveCameraCodec.encodeProps(NveCameraPreviewProps(zoom = 0.0f))
        }
        assertThrows(NveCameraCodecException.InvalidEnum::class.java) {
            NveCameraCodec.encodeProps(NveCameraPreviewProps(fps = 0))
        }
        assertThrows(NveCameraCodecException.InvalidEnum::class.java) {
            NveCameraCodec.encodeProps(NveCameraPreviewProps(fps = 241))
        }

        // Truncated NVNP at every prefix + trailing byte
        val validProps = NveCameraCodec.encodeProps(NveCameraPreviewProps(deviceId = "cam0"))
        for (cut in 0 until validProps.size) {
            assertThrows(NveCameraCodecException::class.java) {
                NveCameraCodec.decodeProps(validProps.copyOfRange(0, cut))
            }
        }
        assertThrows(NveCameraCodecException.TrailingBytes::class.java) {
            NveCameraCodec.decodeProps(validProps + byteArrayOf(0))
        }

        // Bad magic, unsupported version, invalid UTF-8, invalid bool, unknown opcode
        val badMagic = validProps.copyOf().also { it[0] = 'X'.code.toByte() }
        assertThrows(NveCameraCodecException.InvalidMagic::class.java) {
            NveCameraCodec.decodeProps(badMagic)
        }
        val badVer = validProps.copyOf().also { it[4] = 9 }
        assertThrows(NveCameraCodecException.UnsupportedVersion::class.java) {
            NveCameraCodec.decodeProps(badVer)
        }
        val badUtf8Cmd = byteArrayOf(
            'N'.code.toByte(), 'V'.code.toByte(), 'N'.code.toByte(), 'C'.code.toByte(),
            1, 6, 0, 1, 7, 2, 0, 0, 0, 0xFF.toByte(), 0xFE.toByte(),
        )
        assertThrows(NveCameraCodecException.InvalidUtf8::class.java) {
            NveCameraCodec.decodeCommand(badUtf8Cmd)
        }
        val unknownOp = byteArrayOf(
            'N'.code.toByte(), 'V'.code.toByte(), 'N'.code.toByte(), 'C'.code.toByte(),
            1, 42, 0, 0,
        )
        assertThrows(NveCameraCodecException.UnknownCommandOpcode::class.java) {
            NveCameraCodec.decodeCommand(unknownOp)
        }

        // Invalid FocusPoint / CapturePhoto arguments
        assertThrows(NveCameraCodecException.InvalidValue::class.java) {
            NveCameraCodec.encodeCommand(NveCameraCommand.FocusPoint(Float.NaN, 0.5f))
        }
        assertThrows(NveCameraCodecException.InvalidValue::class.java) {
            NveCameraCodec.encodeCommand(NveCameraCommand.FocusPoint(-0.1f, 0.5f))
        }
        assertThrows(NveCameraCodecException.InvalidEnum::class.java) {
            NveCameraCodec.encodeCommand(NveCameraCommand.CapturePhoto(101, 0))
        }
        assertThrows(NveCameraCodecException.InvalidEnum::class.java) {
            NveCameraCodec.encodeCommand(NveCameraCommand.CapturePhoto(90, 3))
        }

        // Corrupt & truncated NVCF
        val validFrame = NveCameraCodec.encodeFrameDescriptor(
            NveCameraFrameDescriptor(640, 480, 640 * 4, NveCameraPixelFormat.RGBA8888, 0, 1L, 2L),
        )
        for (cut in 0 until NveCameraCodec.NVCF_SIZE) {
            assertThrows(NveCameraCodecException.Truncated::class.java) {
                NveCameraCodec.decodeFrameDescriptor(validFrame.copyOfRange(0, cut))
            }
        }
        assertThrows(NveCameraCodecException.TrailingBytes::class.java) {
            NveCameraCodec.decodeFrameDescriptor(validFrame + byteArrayOf(1))
        }
        val badRot = validFrame.copyOf().also { it[6] = 45; it[7] = 0 }
        assertThrows(NveCameraCodecException.InvalidEnum::class.java) {
            NveCameraCodec.decodeFrameDescriptor(badRot)
        }
        val badReserved = validFrame.copyOf().also { it[20] = 1 }
        assertThrows(NveCameraCodecException.InvalidEnum::class.java) {
            NveCameraCodec.decodeFrameDescriptor(badReserved)
        }
        val smallStride = validFrame.copyOf().also {
            it[16] = 100; it[17] = 0; it[18] = 0; it[19] = 0
        }
        assertThrows(NveCameraCodecException.InvalidValue::class.java) {
            NveCameraCodec.decodeFrameDescriptor(smallStride)
        }
    }

    // =========================================================================
    // 2. VisionCamera v5 Adapter Delegation, Constraint Negotiation & NVCF Bridge
    // =========================================================================

    @Test
    fun visionCameraAdapterNegotiatesConstraintsDelegatesCommandsAndBridgesZeroCopyFrames() {
        val fakeBridge = FakeVisionCameraBridge()
        val adapter = NveVisionCameraAdapter(fakeBridge)
        val events = RecordingListener()
        adapter.attachListener(events)

        assertEquals(NveCameraBackendKind.VISION_CAMERA_V5, adapter.backendKind)
        assertEquals(NveCameraSessionState.IDLE, adapter.state)

        // Apply encoded NVNP props requesting 240fps, HDR, Cinematic, zoom 99x -> clamped to device caps
        val rawProps = NveCameraCodec.encodeProps(
            NveCameraPreviewProps(
                facing = NveCameraFacing.FRONT,
                torch = NveTorchMode.ON, // Front camera has no torch -> coerced to OFF
                zoom = 99.0f, // Front maxZoom is 3.0 -> clamped to 3.0
                fps = 240, // Front maxFps is 60 -> clamped to 60
                hdr = true, // Front supportsHdr=false -> coerced to false
                stabilization = NveStabilizationMode.CINEMATIC, // Unsupported on front -> OFF
                enableFrameProcessor = true,
                audioEnabled = true,
                active = true,
            ),
        )
        adapter.applyEncodedProps(rawProps)

        assertEquals(NveCameraSessionState.RUNNING, adapter.state)
        assertEquals("vc5-front", adapter.selectedDevice?.id)
        assertEquals(1, fakeBridge.configureCount)
        assertEquals(
            VisionCameraConstraints(
                targetFps = 60,
                enableHdr = false,
                stabilization = NveStabilizationMode.OFF,
                enableFrameProcessor = true,
                enableAudio = true,
                pixelFormat = NveCameraPixelFormat.YUV420_888,
            ),
            fakeBridge.lastConstraints,
        )
        assertEquals(NveTorchMode.OFF, fakeBridge.torch)
        assertEquals(3.0f, fakeBridge.zoom)
        assertTrue(fakeBridge.active)

        // Diff-aware update: changing only zoom must NOT reconfigure the camera session!
        adapter.applyProps(adapter.activeProps.copy(zoom = 2.0f))
        assertEquals(1, fakeBridge.configureCount)
        assertEquals(2.0f, fakeBridge.zoom)

        // Zero-copy NVCF frame emitted by VisionCamera v5 reaches listener with exact 40-byte payload
        val frame = NveCameraFrameDescriptor(
            width = 1920,
            height = 1080,
            strideBytes = 1920,
            pixelFormat = NveCameraPixelFormat.NV12,
            rotationDeg = 90,
            timestampNs = 42_000_000L,
            nativeHandleId = 0xCAFE_BABEL,
        )
        fakeBridge.pushFrame(frame)
        assertEquals(1, events.frames.size)
        assertEquals(frame, events.frames[0].first)
        assertEquals(frame, NveCameraCodec.decodeFrameDescriptor(events.frames[0].second))

        // Dispatch encoded NVNC commands: switch to BACK, enable torch, zoom, focus, photo, record
        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.SwitchFacing(NveCameraFacing.BACK)),
        )
        assertEquals("vc5-back", adapter.selectedDevice?.id)
        assertEquals(2, fakeBridge.configureCount)

        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.SetTorch(NveTorchMode.AUTO)),
        )
        assertEquals(NveTorchMode.AUTO, fakeBridge.torch)

        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.SetZoom(6.5f)),
        )
        assertEquals(6.5f, fakeBridge.zoom)

        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.FocusPoint(0.4f, 0.6f)),
        )
        assertEquals(0.4f to 0.6f, fakeBridge.lastFocus)

        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.CapturePhoto(quality = 88, flash = 1)),
        )
        assertEquals(1, events.photos.size)
        assertEquals(88, events.photos[0].quality)

        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.StartRecording("/tmp/vc5_clip.mp4")),
        )
        assertEquals(NveCameraSessionState.RECORDING, adapter.state)
        assertEquals(listOf("/tmp/vc5_clip.mp4"), events.recordingStarts)

        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.StopRecording),
        )
        assertEquals(NveCameraSessionState.RUNNING, adapter.state)
        assertEquals(1, events.videos.size)
        assertEquals("/tmp/vc5_clip.mp4", events.videos[0].outputPath)

        // Release cleans up callback and prevents further use
        adapter.release()
        assertEquals(NveCameraSessionState.RELEASED, adapter.state)
        assertTrue(fakeBridge.released)
        assertNull(fakeBridge.frameCallback)
        assertThrows(IllegalStateException::class.java) {
            adapter.dispatchCommand(NveCameraCommand.SetZoom(2.0f))
        }
    }

    // =========================================================================
    // 3. Registry Precedence (VisionCamera v5 > Custom, explicit error when empty)
    // =========================================================================

    @Test
    fun registryRequiresRegisteredVisionCameraOrCustomBackend() {
        val registry = NveCameraRegistry()

        assertFalse(registry.isVisionCameraRegistered())
        assertNull(registry.resolveBackendKind())
        assertThrows(IllegalStateException::class.java) {
            registry.createBackend(surfaceId = 1, nodeId = 10)
        }

        // Register VisionCamera v5 bridge -> auto-selected with highest precedence
        registry.registerVisionCamera { _, _ -> FakeVisionCameraBridge() }
        assertTrue(registry.isVisionCameraRegistered())
        assertEquals(NveCameraBackendKind.VISION_CAMERA_V5, registry.resolveBackendKind())
        val vcBackend = registry.createBackend(surfaceId = 1, nodeId = 11)
        assertEquals(NveCameraBackendKind.VISION_CAMERA_V5, vcBackend.backendKind)

        // Unregister VisionCamera -> returns null / throws explicitly
        assertTrue(registry.unregister(NveCameraBackendKind.VISION_CAMERA_V5))
        assertFalse(registry.isVisionCameraRegistered())
        assertNull(registry.resolveBackendKind())
        assertThrows(IllegalStateException::class.java) {
            registry.createBackend(1, 14, preferredKind = NveCameraBackendKind.VISION_CAMERA_V5)
        }
    }

    // =========================================================================
    // Test Doubles
    // =========================================================================

    private class RecordingListener : NveCameraEventListener {
        val states = mutableListOf<NveCameraSessionState>()
        val frames = mutableListOf<Pair<NveCameraFrameDescriptor, ByteArray>>()
        val photos = mutableListOf<NveCapturedPhoto>()
        val recordingStarts = mutableListOf<String>()
        val videos = mutableListOf<NveRecordedVideo>()
        val errors = mutableListOf<Pair<String, String>>()

        override fun onStateChanged(state: NveCameraSessionState) {
            states.add(state)
        }

        override fun onFrame(descriptor: NveCameraFrameDescriptor, nvcfBytes: ByteArray) {
            frames.add(descriptor to nvcfBytes)
        }

        override fun onPhotoCaptured(photo: NveCapturedPhoto) {
            photos.add(photo)
        }

        override fun onRecordingStarted(outputPath: String) {
            recordingStarts.add(outputPath)
        }

        override fun onRecordingFinished(video: NveRecordedVideo) {
            videos.add(video)
        }

        override fun onError(code: String, message: String) {
            errors.add(code to message)
        }
    }

    private class FakeVisionCameraBridge : VisionCameraSessionBridge {
        var configureCount: Int = 0
        var lastDeviceId: String? = null
        var lastConstraints: VisionCameraConstraints? = null
        var active: Boolean = false
            private set
        var torch: NveTorchMode = NveTorchMode.OFF
            private set
        var zoom: Float = 1.0f
            private set
        var lastFocus: Pair<Float, Float>? = null
            private set
        var recordingPath: String? = null
            private set
        var recordingAudio: Boolean = false
            private set
        var frameCallback: ((NveCameraFrameDescriptor) -> Unit)? = null
            private set
        var released: Boolean = false
            private set

        private val devices = listOf(
            NveCameraDeviceInfo(
                id = "vc5-back",
                name = "VisionCamera Back Triple",
                facing = NveCameraFacing.BACK,
                hasTorch = true,
                minZoom = 0.5f,
                maxZoom = 12.0f,
                maxFps = 120,
                supportsHdr = true,
                supportedStabilization = setOf(
                    NveStabilizationMode.OFF,
                    NveStabilizationMode.STANDARD,
                    NveStabilizationMode.CINEMATIC,
                ),
            ),
            NveCameraDeviceInfo(
                id = "vc5-front",
                name = "VisionCamera Front TrueDepth",
                facing = NveCameraFacing.FRONT,
                hasTorch = false,
                minZoom = 1.0f,
                maxZoom = 3.0f,
                maxFps = 60,
                supportsHdr = false,
                supportedStabilization = setOf(NveStabilizationMode.OFF),
            ),
        )

        override fun enumerateDevices(): List<NveCameraDeviceInfo> = devices

        override fun configureSession(deviceId: String, constraints: VisionCameraConstraints) {
            configureCount++
            lastDeviceId = deviceId
            lastConstraints = constraints
        }

        override fun setActive(active: Boolean) {
            this.active = active
        }

        override fun setTorch(mode: NveTorchMode) {
            this.torch = mode
        }

        override fun setZoom(zoom: Float) {
            this.zoom = zoom
        }

        override fun focus(x: Float, y: Float) {
            this.lastFocus = x to y
        }

        override fun capturePhoto(quality: Int, flashMode: Int): NveCapturedPhoto =
            NveCapturedPhoto(
                filePath = "/cache/vc5_photo.jpg",
                width = 4032,
                height = 3024,
                quality = quality,
                flashMode = flashMode,
                timestampNs = 1_000_000L,
            )

        override fun startRecording(outputPath: String, enableAudio: Boolean) {
            recordingPath = outputPath
            recordingAudio = enableAudio
        }

        override fun stopRecording(): NveRecordedVideo {
            val path = checkNotNull(recordingPath)
            val audio = recordingAudio
            recordingPath = null
            recordingAudio = false
            return NveRecordedVideo(
                outputPath = path,
                durationMs = 2_500L,
                width = 1920,
                height = 1080,
                hasAudio = audio,
            )
        }

        override fun setFrameCallback(callback: ((NveCameraFrameDescriptor) -> Unit)?) {
            frameCallback = callback
        }

        override fun release() {
            released = true
        }

        fun pushFrame(descriptor: NveCameraFrameDescriptor) {
            frameCallback?.invoke(descriptor)
        }
    }
}
