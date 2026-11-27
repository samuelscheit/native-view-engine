package dev.nve.host;

import java.nio.ByteBuffer;

/**
 * JNI binding of {@code libnve_runtime}'s {@code NveRuntimeApi} for Java / Kotlin hosts (Android
 * View hosts, Jetpack Compose, desktop JVM test suites). Implemented by the host's JNI library
 * ({@code hosts/android-shell}: {@code libnve_android_shell}); load it before the first call.
 *
 * <p>A {@code long} handle is one runtime surface. All calls except {@link #pairIsConnected} must
 * run on the thread that called {@link #create} (the runtime's owner thread). Status-returning
 * calls return a {@code STATUS_*} code; {@link #lastError} explains failures.
 *
 * <p>Scene lifetime: {@link #acquireScene} fills a packed frame for
 * {@link RuntimeSceneReader#attach(long[], ByteBuffer[])}; its direct buffers (and those of
 * {@link #resource}) stay valid only until {@link #releaseScene} or any other call on the handle,
 * which releases the scene first so the runtime can write again.
 */
public final class NativeRuntime {
    private NativeRuntime() {}

    // `NveRuntimeStatus` (locked against the generated C header by the JNI library).
    public static final int STATUS_OK = 0;
    public static final int STATUS_INVALID_ARG = 1;
    public static final int STATUS_UNSUPPORTED = 2;
    public static final int STATUS_BUSY = 3;
    public static final int STATUS_IO = 4;
    public static final int STATUS_PANIC = 5;
    public static final int STATUS_NOT_FOUND = 6;
    public static final int STATUS_ERROR = 7;

    /** {@code outKindFlags[1]} bit of an ephemeral resource ({@code NVE_RESOURCE_FLAG_EPHEMERAL}). */
    public static final long RESOURCE_FLAG_EPHEMERAL = 1L;

    /** Host text measurer installed with {@link #setTextMeasurer}; called during layout. */
    public interface TextMeasurer {
        /**
         * Measures {@code text} at {@code fontSize} logical points and CSS {@code fontWeight} under
         * {@code maxWidth} ({@code +Infinity} unconstrained, {@code 0} min-content, otherwise the
         * wrap width). Writes {@code [width, height]} to {@code out}; returns {@code false} to fall
         * back to the runtime's heuristic.
         */
        boolean measure(String text, float fontSize, int fontWeight, float maxWidth, float[] out);
    }

    /** New surface, or {@code 0} on failure. */
    public static native long create(int surfaceId, float width, float height, float scaleFactor);

    /** Releases the scene, clears callbacks, detaches the app, and frees the surface. */
    public static native void destroy(long handle);

    /** Last failure message of the calling thread / handle ({@code ""} when none). */
    public static native String lastError(long handle);

    /** Loads {@code soPath} and attaches its {@code nve_get_interface} app (hot reload when one is attached). */
    public static native int attachApp(long handle, String soPath);

    public static native int detachApp(long handle);

    /** Applies the first {@code length} bytes of {@code nve2} (an {@code NVE2} frame, direct buffer). */
    public static native int submitFrame(long handle, ByteBuffer nve2, int length);

    public static native int setViewport(long handle, float width, float height, float scaleFactor);

    /** Renders / drains pending work: {@code 1} changed, {@code 0} unchanged, {@code -status} on failure. */
    public static native int tick(long handle, float dtSeconds);

    /**
     * Dispatches the first {@code length} bytes of an {@link EventRingWriter} ring (direct buffer):
     * {@code 1} changed, {@code 0} unchanged, {@code -status} on failure.
     */
    public static native int dispatchEvents(long handle, ByteBuffer ring, int length);

    /** Topmost interactive node id at {@code (x, y)} logical points, {@code 0} when none, {@code -status} on failure. */
    public static native int hitTest(long handle, float x, float y);

    /** Scrolls {@code nodeId} by {@code (dx, dy)}; writes the clamped offset to {@code outXY} when non-null. */
    public static native int scrollBy(long handle, int nodeId, float dx, float dy, float[] outXY);

    /**
     * Acquires the published scene into {@code header} (length {@link RuntimeSceneReader#HEADER_LEN})
     * and {@code slabs} (length {@link RuntimeSceneReader#SLABS_LEN}).
     */
    public static native int acquireScene(long handle, long[] header, ByteBuffer[] slabs);

    public static native void releaseScene(long handle);

    /**
     * Wire bytes of live resource {@code resourceId} in the acquired scene (empty buffer for a
     * typed value without bytes), or {@code null}; {@code outKindFlags = [kind, flags]}.
     */
    public static native ByteBuffer resource(long handle, int resourceId, long[] outKindFlags);

    /** Installs ({@code null} removes) the host text measurer and relayouts every node. */
    public static native int setTextMeasurer(long handle, TextMeasurer measurer);

    /**
     * Installs ({@code null} removes) the wake listener: {@code run()} is called on ANY thread when
     * frames or pairing work are ready; it must only post a {@link #tick} to the owner thread.
     */
    public static native int setWakeListener(long handle, Runnable listener);

    /** Connects to an {@code nve pair} server ({@code ws://host:port/...}, or TLS with {@code cert_sha256}). */
    public static native int pairConnect(long handle, String uri, String token);

    public static native int pairDisconnect(long handle);

    /** Cross-thread safe. */
    public static native boolean pairIsConnected(long handle);
}
