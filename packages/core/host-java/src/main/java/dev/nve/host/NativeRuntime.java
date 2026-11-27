package dev.nve.host;

import java.nio.ByteBuffer;

/**
 * JNI binding of {@code libnve_runtime}'s {@code NveRuntimeApi} for Java / Kotlin hosts (Android
 * View hosts, Jetpack Compose, desktop JVM suites). Implemented by the host's JNI library
 * ({@code hosts/android-shell}: {@code libnve_android_shell}); load it before the first call.
 *
 * <p>A {@code long} handle is one runtime surface. All calls except {@link #pairIsConnected} must
 * run on the thread that called {@link #create} (the runtime's owner thread). Every {@code int}
 * result is an {@code NveRuntimeAbi.STATUS_*} code (never negative); values come back through
 * caller-owned out arrays so they never collide with statuses (node ids are raw unsigned
 * {@code u32} bits). {@link #lastError} explains failures.
 *
 * <p>Scene lifetime: {@link #acquireScene} fills a packed frame for
 * {@link RuntimeSceneReader#attach(long[], ByteBuffer[])}; its direct buffers (and those of
 * {@link #resource}) stay valid only until {@link #releaseScene} or any other call on the handle,
 * which releases the scene first so the runtime can write again.
 */
public final class NativeRuntime {
    private NativeRuntime() {}

    /** Host text measurer installed with {@link #setTextMeasurer}; called during layout. */
    public interface TextMeasurer {
        /**
         * Writes the size of {@code text} at {@code fontSize} logical points and CSS
         * {@code fontWeight} to {@code out[0..2)}: unconstrained when {@code maxWidth} is
         * {@code +Infinity}, the min-content size when it is not a positive number, otherwise
         * wrapped at {@code maxWidth}. Non-finite or negative sizes count as 0.
         */
        void measure(String text, float fontSize, int fontWeight, float maxWidth, float[] out);
    }

    /** New surface, or {@code 0} on failure. */
    public static native long create(int surfaceId, float width, float height, float scaleFactor);

    /** Releases the scene, clears callbacks, detaches the app, and frees the surface. */
    public static native void destroy(long handle);

    /** Last failure message of this handle ({@code ""} when none). */
    public static native String lastError(long handle);

    /** Loads {@code soPath} and attaches its {@code nve_get_interface} app (hot reload when one is attached). */
    public static native int attachApp(long handle, String soPath);

    public static native int detachApp(long handle);

    /** Applies the first {@code length} bytes of {@code nve2} (an {@code NVE2} frame, direct buffer). */
    public static native int submitFrame(long handle, ByteBuffer nve2, int length);

    public static native int setViewport(long handle, float width, float height, float scaleFactor);

    /** Renders / drains pending work; {@code outChanged[0]} is 1 when the scene changed. */
    public static native int tick(long handle, float dtSeconds, int[] outChanged);

    /**
     * Dispatches the first {@code length} bytes of an {@link EventRingWriter} ring (direct
     * buffer); {@code outChanged[0]} is 1 when the app changed.
     */
    public static native int dispatchEvents(long handle, ByteBuffer ring, int length, int[] outChanged);

    /** Topmost interactive raw {@code NodeId} at {@code (x, y)} logical points into {@code outNodeId[0]} ({@code 0} = none). */
    public static native int hitTest(long handle, float x, float y, int[] outNodeId);

    /** Scrolls {@code nodeId} by {@code (dx, dy)}; writes the clamped offset to {@code outXY} when non-null. */
    public static native int scrollBy(long handle, int nodeId, float dx, float dy, float[] outXY);

    /**
     * Acquires the published scene into {@code header} (length {@link RuntimeSceneReader#HEADER_LEN})
     * and {@code slabs} (length {@link RuntimeSceneReader#SLABS_LEN}).
     */
    public static native int acquireScene(long handle, long[] header, ByteBuffer[] slabs);

    public static native void releaseScene(long handle);

    /**
     * Canonical bytes of resource {@code resourceId} in the acquired scene (an empty buffer for an
     * empty payload), or {@code null} when it is not live or no scene is held; {@code outKind[0]}
     * receives the resource kind key.
     */
    public static native ByteBuffer resource(long handle, int resourceId, long[] outKind);

    /** Installs ({@code null} removes) the host text measurer; the next tick relayouts. */
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
