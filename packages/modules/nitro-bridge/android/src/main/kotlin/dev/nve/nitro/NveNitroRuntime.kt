package dev.nve.nitro

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * Pure `<jni.h>` zero-`fbjni` Android/JVM runtime binding for `packages/modules/nitro-bridge`.
 *
 * Follows Kolibri's fastest JNI pattern: passes raw `jlong hybridPtr` directly as the first
 * argument to `@JvmStatic external fun` JNI methods (eliminating `fbjni`'s `mHybridData` field
 * traversal) and transfers binary payloads through reusable native `DirectByteBuffer`s.
 */
object NveNitroRuntime {
    const val ERR_INVALID_HANDLE: Int = -1
    const val ERR_NOT_FOUND: Int = -2
    const val ERR_DISPOSED: Int = -3
    const val ERR_BUFFER_TOO_SMALL: Int = -4
    const val ERR_INVOCATION_FAILED: Int = -5
    const val ERR_INVALID_BUFFER: Int = -6

    const val METHOD_ECHO_BYTES: Int = 1
    const val METHOD_PING: Int = 2
    const val METHOD_INCREMENT: Int = 3
    const val METHOD_TAG: Int = 4
    const val METHOD_FAIL: Int = 5

    @Volatile
    var isNativeLoaded: Boolean = false
        private set

    init {
        isNativeLoaded = try {
            System.loadLibrary("nve_nitro_jni")
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
    }

    /**
     * Loads `libnve_nitro_jni` from an explicit filesystem path (e.g. desktop JVM tests).
     */
    fun loadLibraryPath(absolutePath: String): Boolean = try {
        System.load(absolutePath)
        isNativeLoaded = true
        true
    } catch (_: UnsatisfiedLinkError) {
        false
    }

    @JvmStatic
    external fun nativeCreateHybridObject(name: String): Long

    @JvmStatic
    external fun nativeDisposeHybridObject(hybridPtr: Long)

    @JvmStatic
    external fun nativeGetMemorySize(hybridPtr: Long): Long

    @JvmStatic
    external fun nativeAllocDirectBuffer(bytes: Int): ByteBuffer?

    @JvmStatic
    external fun nativeFreeDirectBuffer(directBuffer: ByteBuffer?)

    @JvmStatic
    external fun nativeInvokeBinary(
        hybridPtr: Long,
        methodId: Int,
        directIn: ByteBuffer?,
        inLen: Int,
        directOut: ByteBuffer?,
    ): Int

    /**
     * Creates a managed [NveHybridObject] using [backend] (defaults to [NveNitroJniBackend]).
     * Returns `null` if [name] is not registered or the native backend is unavailable.
     */
    fun createHybridObject(
        name: String,
        backend: NveNitroBackend = NveNitroJniBackend,
    ): NveHybridObject? {
        if (name.isEmpty() || !backend.isAvailable) return null
        val ptr = backend.createHybridObject(name)
        if (ptr == 0L) return null
        return NveHybridObject(name = name, initialPtr = ptr, backend = backend)
    }

    /**
     * Allocates a managed native `DirectByteBuffer` with little-endian byte order.
     */
    fun allocDirectBuffer(
        bytes: Int,
        backend: NveNitroBackend = NveNitroJniBackend,
    ): NveDirectScratchBuffer? {
        if (bytes <= 0 || !backend.isAvailable) return null
        val buf = backend.allocDirectBuffer(bytes) ?: return null
        if (!buf.isDirect) return null
        buf.order(ByteOrder.LITTLE_ENDIAN)
        return NveDirectScratchBuffer(buffer = buf, backend = backend)
    }
}

/**
 * Pluggable backend contract for [NveNitroRuntime].
 * Production code uses [NveNitroJniBackend]; pure JVM tests may inject an in-memory backend.
 */
interface NveNitroBackend {
    val isAvailable: Boolean

    fun createHybridObject(name: String): Long

    fun disposeHybridObject(hybridPtr: Long)

    fun getMemorySize(hybridPtr: Long): Long

    fun allocDirectBuffer(bytes: Int): ByteBuffer?

    fun freeDirectBuffer(directBuffer: ByteBuffer?)

    fun invokeBinary(
        hybridPtr: Long,
        methodId: Int,
        directIn: ByteBuffer?,
        inLen: Int,
        directOut: ByteBuffer?,
    ): Int
}

/**
 * Default zero-`fbjni` backend delegating directly to [NveNitroRuntime]'s `@JvmStatic external fun` methods.
 */
object NveNitroJniBackend : NveNitroBackend {
    override val isAvailable: Boolean
        get() = NveNitroRuntime.isNativeLoaded

    override fun createHybridObject(name: String): Long =
        NveNitroRuntime.nativeCreateHybridObject(name)

    override fun disposeHybridObject(hybridPtr: Long) =
        NveNitroRuntime.nativeDisposeHybridObject(hybridPtr)

    override fun getMemorySize(hybridPtr: Long): Long =
        NveNitroRuntime.nativeGetMemorySize(hybridPtr)

    override fun allocDirectBuffer(bytes: Int): ByteBuffer? =
        NveNitroRuntime.nativeAllocDirectBuffer(bytes)

    override fun freeDirectBuffer(directBuffer: ByteBuffer?) =
        NveNitroRuntime.nativeFreeDirectBuffer(directBuffer)

    override fun invokeBinary(
        hybridPtr: Long,
        methodId: Int,
        directIn: ByteBuffer?,
        inLen: Int,
        directOut: ByteBuffer?,
    ): Int = NveNitroRuntime.nativeInvokeBinary(hybridPtr, methodId, directIn, inLen, directOut)
}

/**
 * Managed handle to a C++ `nve::nitro::HybridObject` storing the raw native pointer directly
 * in an [AtomicLong] for race-free idempotent teardown and zero-indirection JNI dispatch.
 */
class NveHybridObject internal constructor(
    val name: String,
    initialPtr: Long,
    private val backend: NveNitroBackend,
) : AutoCloseable {
    private val ptr = AtomicLong(initialPtr)

    val rawPointer: Long
        get() = ptr.get()

    val isDisposed: Boolean
        get() = ptr.get() == 0L

    fun memorySize(): Long {
        val h = ptr.get()
        if (h == 0L) return 0L
        return backend.getMemorySize(h)
    }

    /**
     * Invokes [methodId] with `[0, inLen)` of [directIn], writing the binary reply into [directOut].
     * Returns bytes written (`>= 0`) and flips [directOut] (`position = 0`, `limit = written`),
     * or returns a negative `NveNitroRuntime.ERR_*` code on failure.
     */
    fun invokeBinary(
        methodId: Int,
        directIn: ByteBuffer?,
        inLen: Int,
        directOut: ByteBuffer?,
    ): Int {
        val h = ptr.get()
        if (h == 0L) return NveNitroRuntime.ERR_DISPOSED
        if (inLen < 0) return NveNitroRuntime.ERR_INVALID_BUFFER
        if (inLen > 0 && (directIn == null || !directIn.isDirect || inLen > directIn.capacity())) {
            return NveNitroRuntime.ERR_INVALID_BUFFER
        }
        if (directOut != null && !directOut.isDirect) {
            return NveNitroRuntime.ERR_INVALID_BUFFER
        }
        val written = backend.invokeBinary(h, methodId, directIn, inLen, directOut)
        if (written >= 0 && directOut != null) {
            directOut.order(ByteOrder.LITTLE_ENDIAN)
            directOut.position(0)
            directOut.limit(written)
        }
        return written
    }

    fun dispose() {
        val h = ptr.getAndSet(0L)
        if (h != 0L) {
            backend.disposeHybridObject(h)
        }
    }

    override fun close() {
        dispose()
    }
}

/**
 * Managed native `DirectByteBuffer` wrapper with idempotent [close] releasing native storage.
 */
class NveDirectScratchBuffer internal constructor(
    val buffer: ByteBuffer,
    private val backend: NveNitroBackend,
) : AutoCloseable {
    private val closed = AtomicBoolean(false)

    val isClosed: Boolean
        get() = closed.get()

    val capacity: Int
        get() = buffer.capacity()

    override fun close() {
        if (closed.compareAndSet(false, true)) {
            backend.freeDirectBuffer(buffer)
        }
    }
}
