package dev.nve.host;

import java.nio.ByteBuffer;

/**
 * JNI binding of {@code libnve_runtime}'s {@code NveRuntimeApi} for Java / Kotlin hosts (Android
 * View hosts, Jetpack Compose, desktop JVM suites). Implemented by the host's JNI library
 * ({@code hosts/android-shell}: {@code libnve_android_shell}); load it before the first call.
 *
 * <p>A {@code long} handle is one runtime surface. All calls except {@link #pairIsConnected},
 * {@link #completeHostRequest}, {@link #pushHostEvent}, and {@link #publishHostChannel} must run on
 * the thread that called {@link #create} (the runtime's owner thread; {@link #destroy} from another
 * thread is refused and leaks the surface rather than freeing it under its owner). Cross-thread
 * calls must not race {@link #destroy} of the same handle ({@link RuntimeSurface} serializes them).
 * Every {@code int} result is an {@code NveRuntimeAbi.STATUS_*} code (never negative); values come
 * back through caller-owned out arrays so they never collide with statuses (node ids are raw
 * unsigned {@code u32} bits). {@link #lastError} explains failures.
 *
 * <p>Scene lifetime: {@link #acquireScene} fills a packed frame for
 * {@link SceneReader#attach(long[], ByteBuffer[])}; its direct buffers (and those of
 * {@link #resource}) stay valid only until {@link #releaseScene} or any other call on the handle,
 * which releases the scene first so the runtime can write again.
 *
 * <p>{@link ByteBuffer} arguments MUST be direct ({@link ByteBuffer#isDirect()}); heap buffers
 * fail with {@code STATUS_INVALID_ARG}. Native JNI always reads byte slice {@code [0..length)}
 * from the buffer's base address (ignoring {@code position()} and {@code limit()}, validating
 * {@code 0 <= length <= capacity()}). Except for {@link #submitFrame}, {@code (null, 0)} or a
 * 0-capacity direct buffer with {@code length == 0} is accepted as an empty slice.
 *
 * <p>Re-entrancy: owner-thread callbacks ({@link TextMeasurer}, {@link HostCallbackBridge}) run
 * synchronously while the runtime holds internal state locks and MUST NOT re-enter owner-thread
 * mutating JNI entry points on the same handle (cross-thread safe methods such as
 * {@link #completeHostRequest}, {@link #pushHostEvent}, {@link #publishHostChannel}, and
 * {@link #pairIsConnected} remain safe).
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

    /**
     * Thin raw callback bridge for {@code NVE_IFACE_HOST = 5} installed with
     * {@link #setHostCallbackBridge}. Methods return an {@link HybridRuntime.HostStatus} code
     * ({@code 0} = OK, {@code 1} = PENDING, negative = error).
     */
    public interface HostCallbackBridge {
        /**
         * Handles a raw capability invocation. For synchronous {@code OK} replies, store the
         * reply bytes in {@code outPayload[0]} (leave {@code null} for an empty reply). For
         * {@code PENDING}, complete later from any thread via {@link #completeHostRequest}.
         */
        int invoke(int serviceId, int methodId, long requestId, byte[] request, byte[][] outPayload);

        /** Handles an outbound channel publish from the app. */
        int publishChannel(int channelId, int flags, byte[] payload);
    }

    /** New surface, or {@code 0} on failure. */
    public static native long create(int surfaceId, float width, float height, float scaleFactor);

    /**
     * Releases the scene, clears callbacks, detaches the app, and frees the surface; refused
     * ({@code STATUS_INVALID_ARG}, the surface stays alive) off the owner thread.
     */
    public static native int destroy(long handle);

    /** Last failure message of this handle ({@code ""} when none). */
    public static native String lastError(long handle);

    /**
     * Loads the app module at {@code soPath} through the runtime's {@code load_app_module} (a
     * private staged copy on desktop, in place on Android / iOS) and attaches it, replacing the
     * running app or pairing; the previous module unloads after its app is destroyed.
     * {@code carryState} restores the running app's {@code NVE_IFACE_DEV} state into the new app.
     * A failure ({@code STATUS_IO}, {@code STATUS_NOT_FOUND}, {@code STATUS_UNSUPPORTED}) keeps the
     * running app.
     */
    public static native int attachApp(long handle, String soPath, boolean carryState);

    public static native int detachApp(long handle);

    /** Applies the first {@code length} bytes of {@code nve2} (an {@code NVE2} frame, direct buffer). */
    public static native int submitFrame(long handle, ByteBuffer nve2, int length);

    public static native int setViewport(long handle, float width, float height, float scaleFactor);

    /** Renders / drains pending work; {@code outChanged[0]} is 1 when the scene changed. */
    public static native int tick(long handle, float dtSeconds, int[] outChanged);

    /**
     * Dispatches the first {@code length} bytes of an {@link EventRingWriter} {@code NVEV} v2
     * frame (direct buffer); {@code outChanged[0]} is 1 when the app changed.
     */
    public static native int dispatchEvents(long handle, ByteBuffer ring, int length, int[] outChanged);

    /** Topmost interactive raw {@code NodeId} at {@code (x, y)} logical points into {@code outNodeId[0]} ({@code 0} = none). */
    public static native int hitTest(long handle, float x, float y, int[] outNodeId);

    /** Scrolls {@code nodeId} by {@code (dx, dy)}; writes the clamped offset to {@code outXY} when non-null. */
    public static native int scrollBy(long handle, int nodeId, float dx, float dy, float[] outXY);

    /**
     * Acquires the published scene into {@code header} (length {@code NveJniFrame.HEADER_LEN}) and
     * {@code slabs} (length {@code NveJniFrame.SLABS_LEN}).
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

    /**
     * Connects to an {@code nve pair} server ({@code ws://host:port/...}, or TLS with
     * {@code cert_sha256}), replacing a running local app; an invalid URI keeps it running.
     */
    public static native int pairConnect(long handle, String uri, String token);

    public static native int pairDisconnect(long handle);

    /** Cross-thread safe. */
    public static native boolean pairIsConnected(long handle);

    /**
     * Forwards embedding-host props encoded as a single {@link BinWriter} {@code NveBinValue}
     * {@code Map} in the first {@code length} bytes of {@code binMap} (direct buffer; {@code length == 0}
     * sends an empty prop map); {@code outChanged[0]} is 1 when the app changed.
     */
    public static native int setProps(long handle, ByteBuffer binMap, int length, int[] outChanged);

    /**
     * Drains app-emitted events encoded as an {@code NVEV} v2 frame readable by
     * {@link EventRingReader}, or {@code null} when empty or on error; {@code outStatus[0]}
     * receives the {@code STATUS_*} code when non-null.
     */
    public static native byte[] drainEmittedEvents(long handle, int[] outStatus);

    /**
     * Snapshots the attached app's {@code NVE_IFACE_DEV} state, or {@code null} on error;
     * {@code outStatus[0]} receives the {@code STATUS_*} code when non-null.
     */
    public static native byte[] snapshotState(long handle, int[] outStatus);

    /** Restores the first {@code length} bytes of {@code state} (direct buffer) into the app. */
    public static native int restoreState(long handle, ByteBuffer state, int length);

    /** Persists the app state snapshot as a crash-atomic checksummed {@code StateFile}. */
    public static native int saveStateFile(long handle, String path, int schemaVersion);

    /** Restores a {@code StateFile} written by {@link #saveStateFile}. */
    public static native int restoreStateFile(long handle, String path, int schemaVersion);

    /** Deletes a {@code StateFile} (cold start; {@code STATUS_OK} when already absent). */
    public static native int clearStateFile(long handle, String path);

    /** Atomically writes the active scene as a checksummed Frame-0 {@code frame0.nve2} file. */
    public static native int saveFrame0File(long handle, String path, long appBuildId, long envHash);

    /** Validates and boots a persisted Frame-0 {@code frame0.nve2} file into the replica scene. */
    public static native int bootFrame0File(long handle, String path, long appBuildId, long envHash);

    /** Validates and boots the first {@code length} bytes of an in-memory Frame-0 envelope (direct buffer). */
    public static native int bootFrame0Bytes(long handle, ByteBuffer envelope, int length, long appBuildId, long envHash);

    /** Applies the first {@code length} bytes of a binary hot patch (direct buffer); {@code outChanged[0]} is 1 when applied. */
    public static native int applyHotPatch(long handle, ByteBuffer patch, int length, int[] outChanged);

    /**
     * Measures the surface under constraints and {@code NveRuntimeAbi.MEASURE_FLAG_*} bits; writes
     * {@code [width, height, firstBaseline, lastBaseline]} (NaN baselines = absent) to {@code outMeasure[0..4)}.
     */
    public static native int measureSurface(
            long handle, float minWidth, float minHeight, float maxWidth, float maxHeight, int flags, float[] outMeasure);

    /** Installs ({@code null} removes and drains) the intrinsic-size invalidation callback. */
    public static native int setIntrinsicSizeInvalidatedListener(long handle, Runnable listener);

    /** Installs ({@code null} removes and drains) the {@code NVE_IFACE_HOST = 5} callback bridge. */
    public static native int setHostCallbackBridge(long handle, HostCallbackBridge bridge);

    /**
     * Completes a {@code PENDING} host request from ANY thread and triggers wake (direct buffer
     * {@code [0..length)}).
     */
    public static native int completeHostRequest(
            long handle, long requestId, int hostStatus, ByteBuffer payload, int length);

    /** Completes a {@code PENDING} host request from ANY thread using a heap {@code byte[]} ({@code null} = empty). */
    public static native int completeHostRequest(long handle, long requestId, int hostStatus, byte[] payload);

    /**
     * Enqueues an unsolicited host push event from ANY thread and triggers wake (direct buffer
     * {@code [0..length)}).
     */
    public static native int pushHostEvent(long handle, int serviceId, int eventId, ByteBuffer payload, int length);

    /** Enqueues an unsolicited host push event from ANY thread using a heap {@code byte[]} ({@code null} = empty). */
    public static native int pushHostEvent(long handle, int serviceId, int eventId, byte[] payload);

    /**
     * Enqueues an inbound host channel message from ANY thread and triggers wake (direct buffer
     * {@code [0..length)}).
     */
    public static native int publishHostChannel(long handle, int channelId, int flags, ByteBuffer payload, int length);

    /** Enqueues an inbound host channel message from ANY thread using a heap {@code byte[]} ({@code null} = empty). */
    public static native int publishHostChannel(long handle, int channelId, int flags, byte[] payload);
}
