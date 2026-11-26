package dev.nve.compose

import java.nio.ByteBuffer

/**
 * Binary bridge contract connecting Jetpack Compose hosts to NVE's native runtime.
 *
 * Production code uses [NveNativeBridge] (backed by JNI [NveJniBridge]); JVM & Robolectric unit
 * tests inject a deterministic in-memory implementation without requiring `libnve_android_shell.so`.
 */
interface NveBridgeBackend {
    val isAvailable: Boolean

    fun createSurface(
        surfaceId: Int,
        componentName: String,
        vtableSymbol: String,
        backendCode: Int,
    ): Long

    fun destroySurface(handlePtr: Long)

    fun resizeSurface(
        handlePtr: Long,
        width: Float,
        height: Float,
        scaleFactor: Float,
    ): Boolean

    fun setPropsBinary(handlePtr: Long, propsBin: ByteArray): Boolean

    fun dispatchCommandBinary(handlePtr: Long, commandName: String, argsBin: ByteArray): Boolean

    fun submitEvents(handlePtr: Long, eventRingBytes: ByteArray): Boolean

    /**
     * Returns a reusable native-owned direct [ByteBuffer] for zero-copy `EventRing` submission,
     * or `null` when this backend only supports `ByteArray` submission (e.g. JVM mocks).
     */
    fun getEventScratchBuffer(handlePtr: Long): ByteBuffer? = null

    /**
     * Submits the first [byteLength] bytes written into [getEventScratchBuffer] with zero JNI
     * array pinning or copying.
     */
    fun submitEventsDirect(handlePtr: Long, byteLength: Int): Boolean = false

    /**
     * Returns a reusable native-owned direct [ByteBuffer] for zero-copy binary prop updates,
     * or `null` when this backend only supports `ByteArray` props.
     */
    fun getPropScratchBuffer(handlePtr: Long): ByteBuffer? = null

    /**
     * Submits the first [byteLength] bytes written into [getPropScratchBuffer] with zero JNI
     * array pinning or copying.
     */
    fun setPropsDirect(handlePtr: Long, byteLength: Int): Boolean = false

    /**
     * Renders one frame and writes the 7-element metadata tuple into [outMeta]:
     * `[schema_hash, frame_id, node_count, dirty_count, removed_count, text_slab_len, children_slab_len]`.
     * Returns the rendered node count, or `-1` on failure.
     */
    fun renderFrame(handlePtr: Long, frameId: Long, outMeta: LongArray): Int

    /**
     * Returns direct `ByteBuffer` slabs for the most recently rendered frame (zero-copy over native memory).
     */
    fun getSceneSlabs(handlePtr: Long): NveSceneSlabs?

    fun drainEmittedEvents(handlePtr: Long): ByteArray?

    /**
     * Tier-2 hot-swap reload of a compiled Rust component `.so` generation.
     */
    fun hotReloadApp(handlePtr: Long, soPath: String): Boolean

    /**
     * Intrinsic measurement hook returning `[widthDp, heightDp]` for `View.onMeasure`.
     */
    fun measureSurface(
        handlePtr: Long,
        minWidth: Float,
        minHeight: Float,
        maxWidth: Float,
        maxHeight: Float,
    ): FloatArray?

    /**
     * Cross-framework pointer arbitration hook returning an [NveGestureDisposition] wire code.
     */
    fun hitTestPointer(
        handlePtr: Long,
        pointerId: Int,
        phase: Int,
        x: Float,
        y: Float,
    ): Int
}

/**
 * Raw JNI declarations bound to `libnve_android_shell.so`.
 */
object NveJniBridge {
    @Volatile
    var isNativeLoaded: Boolean = false
        private set

    init {
        isNativeLoaded = try {
            System.loadLibrary("nve_android_shell")
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
    }

    external fun nativeCreateSurface(
        surfaceId: Int,
        componentName: String,
        vtableSymbol: String,
        backendCode: Int,
    ): Long

    external fun nativeDestroySurface(handlePtr: Long)

    external fun nativeResizeSurface(
        handlePtr: Long,
        width: Float,
        height: Float,
        scaleFactor: Float,
    ): Boolean

    external fun nativeSetPropsBinary(handlePtr: Long, propsBin: ByteArray): Boolean

    external fun nativeDispatchCommandBinary(
        handlePtr: Long,
        commandName: String,
        argsBin: ByteArray,
    ): Boolean

    external fun nativeSubmitEvents(handlePtr: Long, eventRingBytes: ByteArray): Boolean

    external fun nativeGetEventScratchBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeSubmitEventsDirect(handlePtr: Long, byteLength: Int): Boolean

    external fun nativeGetPropScratchBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeSetPropsDirect(handlePtr: Long, byteLength: Int): Boolean

    external fun nativeRenderFrame(handlePtr: Long, frameId: Long, outMeta: LongArray): Int

    external fun nativeGetNodesBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeGetTextSlabBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeGetChildrenSlabBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeGetDirtyIndicesBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeGetRemovedIdsBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeGetMetaRecordsBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeGetMetaBlobBuffer(handlePtr: Long): ByteBuffer?

    external fun nativeDrainEmittedEvents(handlePtr: Long): ByteArray?

    external fun nativeHotReloadApp(handlePtr: Long, soPath: String): Boolean

    external fun nativeMeasureSurface(
        handlePtr: Long,
        minWidth: Float,
        minHeight: Float,
        maxWidth: Float,
        maxHeight: Float,
    ): FloatArray?

    external fun nativeHitTestPointer(
        handlePtr: Long,
        pointerId: Int,
        phase: Int,
        x: Float,
        y: Float,
    ): Int
}

/**
 * Default [NveBridgeBackend] delegating to [NveJniBridge].
 * Gracefully reports [isAvailable] = `false` in Android Studio previews and pure JVM unit tests.
 */
object NveNativeBridge : NveBridgeBackend {
    val isNativeLoaded: Boolean
        get() = NveJniBridge.isNativeLoaded

    override val isAvailable: Boolean
        get() = NveJniBridge.isNativeLoaded

    override fun createSurface(
        surfaceId: Int,
        componentName: String,
        vtableSymbol: String,
        backendCode: Int,
    ): Long = NveJniBridge.nativeCreateSurface(surfaceId, componentName, vtableSymbol, backendCode)

    override fun destroySurface(handlePtr: Long) = NveJniBridge.nativeDestroySurface(handlePtr)

    override fun resizeSurface(
        handlePtr: Long,
        width: Float,
        height: Float,
        scaleFactor: Float,
    ): Boolean = NveJniBridge.nativeResizeSurface(handlePtr, width, height, scaleFactor)

    override fun setPropsBinary(handlePtr: Long, propsBin: ByteArray): Boolean =
        NveJniBridge.nativeSetPropsBinary(handlePtr, propsBin)

    override fun dispatchCommandBinary(
        handlePtr: Long,
        commandName: String,
        argsBin: ByteArray,
    ): Boolean = NveJniBridge.nativeDispatchCommandBinary(handlePtr, commandName, argsBin)

    override fun submitEvents(handlePtr: Long, eventRingBytes: ByteArray): Boolean =
        NveJniBridge.nativeSubmitEvents(handlePtr, eventRingBytes)

    override fun getEventScratchBuffer(handlePtr: Long): ByteBuffer? =
        NveJniBridge.nativeGetEventScratchBuffer(handlePtr)

    override fun submitEventsDirect(handlePtr: Long, byteLength: Int): Boolean =
        NveJniBridge.nativeSubmitEventsDirect(handlePtr, byteLength)

    override fun getPropScratchBuffer(handlePtr: Long): ByteBuffer? =
        NveJniBridge.nativeGetPropScratchBuffer(handlePtr)

    override fun setPropsDirect(handlePtr: Long, byteLength: Int): Boolean =
        NveJniBridge.nativeSetPropsDirect(handlePtr, byteLength)

    override fun renderFrame(handlePtr: Long, frameId: Long, outMeta: LongArray): Int =
        NveJniBridge.nativeRenderFrame(handlePtr, frameId, outMeta)

    override fun getSceneSlabs(handlePtr: Long): NveSceneSlabs = NveSceneSlabs(
        nodes = NveJniBridge.nativeGetNodesBuffer(handlePtr),
        textSlab = NveJniBridge.nativeGetTextSlabBuffer(handlePtr),
        childrenSlab = NveJniBridge.nativeGetChildrenSlabBuffer(handlePtr),
        dirtyIndices = NveJniBridge.nativeGetDirtyIndicesBuffer(handlePtr),
        removedIds = NveJniBridge.nativeGetRemovedIdsBuffer(handlePtr),
        metaRecords = NveJniBridge.nativeGetMetaRecordsBuffer(handlePtr),
        metaBlob = NveJniBridge.nativeGetMetaBlobBuffer(handlePtr),
    )

    override fun drainEmittedEvents(handlePtr: Long): ByteArray? =
        NveJniBridge.nativeDrainEmittedEvents(handlePtr)

    override fun hotReloadApp(handlePtr: Long, soPath: String): Boolean =
        NveJniBridge.nativeHotReloadApp(handlePtr, soPath)

    override fun measureSurface(
        handlePtr: Long,
        minWidth: Float,
        minHeight: Float,
        maxWidth: Float,
        maxHeight: Float,
    ): FloatArray? = NveJniBridge.nativeMeasureSurface(handlePtr, minWidth, minHeight, maxWidth, maxHeight)

    override fun hitTestPointer(
        handlePtr: Long,
        pointerId: Int,
        phase: Int,
        x: Float,
        y: Float,
    ): Int = NveJniBridge.nativeHitTestPointer(handlePtr, pointerId, phase, x, y)
}
