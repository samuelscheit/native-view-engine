package dev.nve.camera

import android.graphics.ImageFormat
import android.graphics.PixelFormat
import android.hardware.camera2.CaptureRequest
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageCapture
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Robolectric tests verifying [NveAndroidCameraXBinding] Camera2/CameraX mapping helpers and
 * [NveCameraPreview] / [NveCameraPreviewEncoded] Compose mounting + Android lifecycle wiring.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NveCameraPreviewComposableTest {

    @Test
    fun androidCameraXMappingHelpersAlignWithWireFormatsAndCamera2Constants() {
        assertEquals(CameraSelector.LENS_FACING_BACK, NveAndroidCameraXBinding.mapFacingToCameraSelector(NveCameraFacing.BACK))
        assertEquals(CameraSelector.LENS_FACING_FRONT, NveAndroidCameraXBinding.mapFacingToCameraSelector(NveCameraFacing.FRONT))
        assertEquals(CameraSelector.LENS_FACING_EXTERNAL, NveAndroidCameraXBinding.mapFacingToCameraSelector(NveCameraFacing.EXTERNAL))

        assertEquals(NveCameraFacing.BACK, NveAndroidCameraXBinding.mapCameraSelectorToFacing(CameraSelector.LENS_FACING_BACK))
        assertEquals(NveCameraFacing.FRONT, NveAndroidCameraXBinding.mapCameraSelectorToFacing(CameraSelector.LENS_FACING_FRONT))
        assertEquals(NveCameraFacing.EXTERNAL, NveAndroidCameraXBinding.mapCameraSelectorToFacing(CameraSelector.LENS_FACING_EXTERNAL))

        assertEquals(ImageCapture.FLASH_MODE_OFF, NveAndroidCameraXBinding.mapFlashMode(0))
        assertEquals(ImageCapture.FLASH_MODE_ON, NveAndroidCameraXBinding.mapFlashMode(1))
        assertEquals(ImageCapture.FLASH_MODE_AUTO, NveAndroidCameraXBinding.mapFlashMode(2))

        assertEquals(
            CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_OFF,
            NveAndroidCameraXBinding.mapStabilizationToCamera2(NveStabilizationMode.OFF),
        )
        assertEquals(
            CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON,
            NveAndroidCameraXBinding.mapStabilizationToCamera2(NveStabilizationMode.STANDARD),
        )

        val stabs = NveAndroidCameraXBinding.mapSupportedStabilizationModes(
            intArrayOf(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON, 2),
        )
        assertTrue(NveStabilizationMode.OFF in stabs)
        assertTrue(NveStabilizationMode.STANDARD in stabs)
        assertTrue(NveStabilizationMode.CINEMATIC in stabs)

        assertEquals(NveCameraPixelFormat.YUV420_888, NveAndroidCameraXBinding.mapImageFormat(ImageFormat.YUV_420_888))
        assertEquals(NveCameraPixelFormat.RGBA8888, NveAndroidCameraXBinding.mapImageFormat(PixelFormat.RGBA_8888))
        assertNull(NveAndroidCameraXBinding.mapImageFormat(ImageFormat.JPEG))

        // Frame descriptor construction + NVCF encode/decode parity
        val desc = NveAndroidCameraXBinding.buildFrameDescriptorFromMetrics(
            width = 1920,
            height = 1080,
            strideBytes = 2048,
            pixelFormat = NveCameraPixelFormat.YUV420_888,
            rotationDegrees = 270,
            timestampNs = 55_000_000L,
            nativeHandleId = 0x1234_5678L,
        )
        assertNotNull(desc)
        assertEquals(desc, NveCameraCodec.decodeFrameDescriptor(NveCameraCodec.encodeFrameDescriptor(desc!!)))
        assertNull(
            NveAndroidCameraXBinding.buildFrameDescriptorFromMetrics(
                0, 1080, 1080, NveCameraPixelFormat.YUV420_888, 0, 1L, 1L,
            ),
        )
    }

    @Test
    fun composeCameraPreviewMountsPreviewViewAppliesEncodedPropsAndForwardsLifecycle() {
        val binding = InMemoryCameraXHostBinding()
        val adapter = NveCameraXAdapter(binding)
        var readyBackend: NveCameraBackend? = null
        val capturedPhotos = mutableListOf<NveCapturedPhoto>()

        val rawProps = NveCameraCodec.encodeProps(
            NveCameraPreviewProps(
                facing = NveCameraFacing.BACK,
                torch = NveTorchMode.ON,
                zoom = 2.0f,
                fps = 60,
                hdr = true,
                stabilization = NveStabilizationMode.STANDARD,
                enableFrameProcessor = true,
                active = true,
            ),
        )

        val controller = Robolectric.buildActivity(ComponentActivity::class.java).setup()
        val activity = controller.get()
        activity.setContent {
            NveCameraPreviewEncoded(
                nvnpBytes = rawProps,
                modifier = Modifier.fillMaxSize(),
                surfaceId = 3,
                nodeId = 42,
                backendOverride = adapter,
                listener = object : NveCameraEventListener {
                    override fun onPhotoCaptured(photo: NveCapturedPhoto) {
                        capturedPhotos.add(photo)
                    }
                },
                onBackendReady = { readyBackend = it },
            )
        }

        // Verify PreviewView was mounted into the Activity hierarchy
        val previewView = findPreviewView(activity.window.decorView as ViewGroup)
        assertNotNull("Expected CameraX PreviewView in Compose hierarchy", previewView)
        assertEquals(adapter, readyBackend)
        assertEquals(NveCameraSessionState.RUNNING, adapter.state)
        assertEquals(NveTorchMode.ON, binding.torchMode)
        assertEquals(2.0f, binding.zoomRatio, 0.001f)
        assertEquals(1, binding.bindCount)

        // Dispatch imperative command through the mounted backend
        adapter.dispatchEncodedCommand(
            NveCameraCodec.encodeCommand(NveCameraCommand.CapturePhoto(quality = 90, flash = 2)),
        )
        assertEquals(1, capturedPhotos.size)

        // Android Activity stop/start lifecycle flows into CameraX unbind/rebind
        controller.pause().stop()
        assertEquals(NveCameraSessionState.STOPPED, adapter.state)
        assertEquals(1, binding.unbindCount)

        controller.start().resume()
        assertEquals(NveCameraSessionState.RUNNING, adapter.state)
        assertEquals(2, binding.bindCount)

        controller.pause().stop().destroy()
    }

    @Test
    fun composeCameraPreviewEncodedReportsCorruptNvnpBufferWithoutCrashing() {
        val errors = mutableListOf<Pair<String, String>>()
        val controller = Robolectric.buildActivity(ComponentActivity::class.java).setup()
        controller.get().setContent {
            NveCameraPreviewEncoded(
                nvnpBytes = byteArrayOf(0x01, 0x02, 0x03),
                listener = object : NveCameraEventListener {
                    override fun onError(code: String, message: String) {
                        errors.add(code to message)
                    }
                },
            )
        }
        assertEquals(1, errors.size)
        assertEquals("INVALID_NVNP_PROPS", errors[0].first)
        controller.pause().stop().destroy()
    }

    private fun findPreviewView(root: ViewGroup): PreviewView? {
        for (i in 0 until root.childCount) {
            val child = root.getChildAt(i)
            if (child is PreviewView) return child
            if (child is ViewGroup) {
                findPreviewView(child)?.let { return it }
            }
        }
        return null
    }
}
