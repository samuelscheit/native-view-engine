package dev.nve.host;

import dev.nve.host.generated.NveJniFrame;
import dev.nve.host.generated.NveRuntimeAbi;
import java.nio.Buffer;
import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;
import java.util.concurrent.Executor;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * One runtime surface for a Java / Kotlin host: owns the {@link NativeRuntime} handle, the packed
 * scene arrays + {@link SceneReader}, a growable {@link EventRingWriter}, and wake
 * coalescing onto the owner thread.
 *
 * <p>Use it on its owner thread only (the thread that constructs it; Android: the main thread),
 * except {@link #isPaired}, {@link #completeHostRequest}, {@link #pushHostEvent}, and
 * {@link #publishHostChannel}, which any thread may call without racing {@link #close}. The wake
 * listener may fire on any thread; it posts at most one pending {@link #render} to
 * {@code ownerThread}.
 *
 * <p>{@link ByteBuffer} overloads require direct buffers ({@link ByteBuffer#isDirect()}) and read
 * slice {@code [0..length)} from byte offset {@code 0} (ignoring {@code position()}/{@code limit()},
 * validating {@code 0 <= length <= capacity()}). Heap {@code byte[]} overloads handle staging
 * automatically.
 *
 * <p>Frames &amp; re-entrancy: each new frame reaches the {@link FrameConsumer} while the scene is
 * acquired. Inside {@code onFrame} the reader and {@link #resource} bytes are valid; nothing may
 * touch the native scene afterwards (the reader and every pooled table reader are
 * {@link SceneReader#detach detached} before release). {@link #send} and {@link #scrollBy} made
 * inside {@code onFrame} (e.g. by listeners that fire while the consumer mutates its UI) are
 * deferred until right after the scene is released, and the frames they cause are delivered by the
 * same {@link #render} loop (bounded; the rest is posted), never by recursion. Structural calls
 * ({@link #attachApp}, {@link #setViewport}, {@link #setProps}, {@link #applyHotPatch},
 * {@link #measureSurface}, ...) are non-reentrant and throw {@link IllegalStateException} inside
 * {@code onFrame} or after {@link #close}, except {@code null}-clearing teardown calls
 * ({@link #setTextMeasurer}, {@link #setIntrinsicSizeInvalidatedListener},
 * {@link #setHostCallbackBridge}), which are idempotent no-ops once closed.
 */
public final class RuntimeSurface implements AutoCloseable, FrameHost {
    /** Receives each new frame while the scene is acquired. */
    public interface FrameConsumer {
        void onFrame(SceneReader scene, RuntimeSurface surface);
    }

    /** Receives raw {@code NVEV} v2 outbound batches drained from the attached app (read via {@link EventRingReader}). */
    public interface EmittedEventConsumer {
        void onEmittedEvents(byte[] nvevBatch);
    }

    /** Measured surface dimensions and optional baselines returned by {@link #measureSurface}. */
    public static final class SurfaceMeasure {
        public final float width;
        public final float height;
        public final float firstBaseline;
        public final float lastBaseline;

        public SurfaceMeasure(float width, float height, float firstBaseline, float lastBaseline) {
            this.width = width;
            this.height = height;
            this.firstBaseline = firstBaseline;
            this.lastBaseline = lastBaseline;
        }

        public boolean hasFirstBaseline() {
            return !Float.isNaN(firstBaseline);
        }

        public boolean hasLastBaseline() {
            return !Float.isNaN(lastBaseline);
        }
    }

    static final int INITIAL_RING_BYTES = 64 * 1024;
    static final int MAX_RING_BYTES = 16 * 1024 * 1024;
    /** Largest direct scratch buffer retained on the surface across calls (oversized spikes are one-shot). */
    static final int MAX_RETAINED_SCRATCH_BYTES = 64 * 1024;
    /** Frames one {@link #render} delivers back to back before posting the rest (UI fairness). */
    static final int MAX_FRAMES_PER_RENDER = 4;
    /** Upper bound of the {@code dt} passed to {@code tick} (a long pause is not one huge step). */
    static final float MAX_TICK_SECONDS = 0.25f;

    private final long[] header = new long[NveJniFrame.HEADER_LEN];
    private final ByteBuffer[] slabs = new ByteBuffer[NveJniFrame.SLABS_LEN];
    private final SceneReader reader = new SceneReader();
    private final int surfaceId;
    private final Executor ownerThread;
    /** The thread that created the native surface: the only one allowed to free it. */
    private final Thread owner = Thread.currentThread();
    private final AtomicBoolean renderPosted = new AtomicBoolean();
    /** Guards {@link #handle} between {@link #close} and cross-thread safe runtime queries/ingress. */
    private final Object handleLock = new Object();
    private final int[] out = new int[1];
    private final long[] kind = new long[1];
    private final float[] scrollOut = new float[2];
    private final float[] measureOut = new float[4];
    private EventRingWriter ring;
    private ByteBuffer scratchDirect;
    private volatile long handle;
    private FrameConsumer consumer;
    private EmittedEventConsumer emittedEventConsumer;
    /** Cached per app attach: skips per-frame JNI drain when no attached app exports `NVE_IFACE_COMPONENT`. */
    private boolean emittedUnsupported = true;
    private boolean needsFrame = true;
    private boolean inFrame;
    private boolean closeAfterFrame;
    /** A {@link #render} loop is on the stack (a synchronous executor must not nest another). */
    private boolean rendering;
    /** Why the last acquired frame was rejected ({@link SceneException}), until one is delivered. */
    private String frameError;
    /** Events go to an {@code nve pair} server (answered asynchronously), not a local app. */
    private boolean paired;
    private long lastTickNanos;
    // Scrolls made inside `onFrame`, replayed after release (typed: node ids are raw u32 bits).
    private int[] deferredScrollIds = new int[4];
    private float[] deferredScrollDeltas = new float[8];
    private int deferredScrolls;

    /** @throws IllegalStateException when the runtime cannot create the surface */
    public RuntimeSurface(int surfaceId, float width, float height, float scaleFactor, Executor ownerThread) {
        this.handle = NativeRuntime.create(surfaceId, width, height, scaleFactor);
        if (handle == 0L) throw new IllegalStateException("nve runtime surface creation failed");
        this.surfaceId = surfaceId;
        this.ownerThread = ownerThread;
        this.ring = new EventRingWriter(ByteBuffer.allocateDirect(INITIAL_RING_BYTES), surfaceId);
        NativeRuntime.setWakeListener(handle, this::onWake);
    }

    public long handle() {
        return handle;
    }

    public void setFrameConsumer(FrameConsumer consumer) {
        this.consumer = consumer;
        needsFrame = true;
    }

    /**
     * Installs ({@code null} removes) the listener receiving outbound app events drained after
     * every tick, event dispatch, prop update, and hot patch.
     */
    public void setEmittedEventConsumer(EmittedEventConsumer listener) {
        this.emittedEventConsumer = listener;
    }

    /** The runtime's last error, else why the last frame was rejected, else {@code ""}. */
    public String lastError() {
        final long h = handle;
        if (h == 0L) return "";
        final String runtime = NativeRuntime.lastError(h);
        return runtime.isEmpty() && frameError != null ? frameError : runtime;
    }

    private void structural(String what) {
        if (inFrame) throw new IllegalStateException(what + " inside onFrame would invalidate the frame being read");
        if (handle == 0L) throw new IllegalStateException(what + " on a closed surface");
    }

    private int afterStructuralChange(int status) {
        if (status == NveRuntimeAbi.STATUS_OK) needsFrame = true;
        return status;
    }

    private boolean isClosedClearing(Object nullableCallback) {
        return nullableCallback == null && !inFrame && handle == 0L;
    }

    /**
     * Loads the app {@code .so} through the runtime ({@code load_app_module}: a private staged copy
     * on desktop, in place on Android) and attaches it, replacing the running app (hot reload) or
     * pairing; {@code carryState} restores the running app's state into it. A failure keeps the
     * running app. Returns a {@code STATUS_*} code.
     */
    public int attachApp(String soPath, boolean carryState) {
        structural("attachApp");
        return source(NativeRuntime.attachApp(handle, soPath, carryState), false, true);
    }

    public int detachApp() {
        structural("detachApp");
        return source(NativeRuntime.detachApp(handle), false, false);
    }

    /** Streams the scene from an {@code nve pair} server, replacing any local app. */
    public int pair(String uri, String token) {
        structural("pair");
        return source(NativeRuntime.pairConnect(handle, uri, token == null ? "" : token), true, false);
    }

    public int unpair() {
        structural("unpair");
        return source(NativeRuntime.pairDisconnect(handle), false, false);
    }

    /** A successful source switch: the next frame is due and dispatch follows the new source. */
    private int source(int status, boolean remote, boolean hasLocalApp) {
        if (status == NveRuntimeAbi.STATUS_OK) {
            paired = remote;
            emittedUnsupported = !hasLocalApp;
        }
        return afterStructuralChange(status);
    }

    @Override
    public boolean dispatchesSynchronously() {
        return !paired;
    }

    /** Cross-thread safe: never races {@link #close}. */
    public boolean isPaired() {
        synchronized (handleLock) {
            final long h = handle;
            return h != 0L && NativeRuntime.pairIsConnected(h);
        }
    }

    /** Applies a host-produced {@code NVE2} frame (direct buffer {@code [0..length)}). */
    public int submitFrame(ByteBuffer nve2, int length) {
        structural("submitFrame");
        return afterStructuralChange(NativeRuntime.submitFrame(handle, nve2, length));
    }

    public int setViewport(float width, float height, float scaleFactor) {
        structural("setViewport");
        return afterStructuralChange(NativeRuntime.setViewport(handle, width, height, scaleFactor));
    }

    /** Installs ({@code null} removes; idempotent no-op when closed) the host text measurer; the next render relayouts. */
    public int setTextMeasurer(NativeRuntime.TextMeasurer measurer) {
        if (isClosedClearing(measurer)) return NveRuntimeAbi.STATUS_OK;
        structural("setTextMeasurer");
        return afterStructuralChange(NativeRuntime.setTextMeasurer(handle, measurer));
    }

    /**
     * Copies {@code bytes} into the owner-thread reusable direct scratch buffer ({@code null} for
     * empty). Oversized spikes ({@code > MAX_RETAINED_SCRATCH_BYTES}) use a one-shot direct buffer
     * so the retained off-heap scratch stays bounded.
     */
    private ByteBuffer toScratchDirect(byte[] bytes) {
        if (bytes == null || bytes.length == 0) {
            if (scratchDirect != null && scratchDirect.capacity() > MAX_RETAINED_SCRATCH_BYTES) {
                scratchDirect = null;
            }
            return null;
        }
        if (bytes.length > MAX_RETAINED_SCRATCH_BYTES) {
            if (scratchDirect != null && scratchDirect.capacity() > MAX_RETAINED_SCRATCH_BYTES) {
                scratchDirect = null;
            }
            return HybridRuntime.directBuffer(bytes);
        }
        if (scratchDirect == null || scratchDirect.capacity() < bytes.length
                || scratchDirect.capacity() > MAX_RETAINED_SCRATCH_BYTES) {
            int cap = 256;
            while (cap < bytes.length && cap < MAX_RETAINED_SCRATCH_BYTES) cap *= 2;
            if (cap < bytes.length) cap = bytes.length;
            scratchDirect = ByteBuffer.allocateDirect(cap).order(ByteOrder.LITTLE_ENDIAN);
        }
        ((Buffer) scratchDirect).clear();
        scratchDirect.put(bytes);
        ((Buffer) scratchDirect).flip();
        return scratchDirect;
    }

    /**
     * Forwards embedding-host props encoded as a single {@link BinWriter} {@code NveBinValue} map
     * (direct buffer {@code [0..length)}). When {@code outChanged} is non-null with length {@code >= 1},
     * {@code outChanged[0]} receives whether the app reported a change.
     */
    public int setProps(ByteBuffer binMap, int length, boolean[] outChanged) {
        structural("setProps");
        out[0] = 0;
        final int st = NativeRuntime.setProps(handle, binMap, length, out);
        final boolean changed = st == NveRuntimeAbi.STATUS_OK && out[0] != 0;
        if (outChanged != null && outChanged.length >= 1) outChanged[0] = changed;
        if (st == NveRuntimeAbi.STATUS_OK) {
            if (changed) needsFrame = true;
            drainEmittedIfListening();
        }
        return st;
    }

    /** Forwards embedding-host props encoded as a single {@link BinWriter} {@code NveBinValue} map (direct buffer {@code [0..length)}). */
    public int setProps(ByteBuffer binMap, int length) {
        return setProps(binMap, length, null);
    }

    /** Forwards embedding-host props encoded as a single {@link BinWriter} {@code NveBinValue} map. */
    public int setProps(byte[] binMap, boolean[] outChanged) {
        structural("setProps");
        final int len = binMap == null ? 0 : binMap.length;
        return setProps(toScratchDirect(binMap), len, outChanged);
    }

    /** Forwards embedding-host props encoded as a single {@link BinWriter} {@code NveBinValue} map. */
    public int setProps(byte[] binMap) {
        return setProps(binMap, null);
    }

    /** Manually drains outbound app events as an {@code NVEV} v2 byte array ({@code null} when empty or unsupported). */
    public byte[] drainEmittedEvents() {
        structural("drainEmittedEvents");
        out[0] = NveRuntimeAbi.STATUS_OK;
        return NativeRuntime.drainEmittedEvents(handle, out);
    }

    /** Snapshots the attached app's state ({@code null} on error; see {@link #lastError}). */
    public byte[] snapshotState() {
        structural("snapshotState");
        out[0] = NveRuntimeAbi.STATUS_OK;
        return NativeRuntime.snapshotState(handle, out);
    }

    /** Restores a state snapshot (direct buffer {@code [0..length)}) into the attached app. */
    public int restoreState(ByteBuffer state, int length) {
        structural("restoreState");
        return afterStructuralChange(NativeRuntime.restoreState(handle, state, length));
    }

    /** Restores a state snapshot into the attached app. */
    public int restoreState(byte[] state) {
        structural("restoreState");
        final int len = state == null ? 0 : state.length;
        return restoreState(toScratchDirect(state), len);
    }

    public int saveStateFile(String path, int schemaVersion) {
        structural("saveStateFile");
        return NativeRuntime.saveStateFile(handle, path, schemaVersion);
    }

    public int restoreStateFile(String path, int schemaVersion) {
        structural("restoreStateFile");
        return afterStructuralChange(NativeRuntime.restoreStateFile(handle, path, schemaVersion));
    }

    public int clearStateFile(String path) {
        structural("clearStateFile");
        return NativeRuntime.clearStateFile(handle, path);
    }

    public int saveFrame0File(String path, long appBuildId, long envHash) {
        structural("saveFrame0File");
        return NativeRuntime.saveFrame0File(handle, path, appBuildId, envHash);
    }

    public int bootFrame0File(String path, long appBuildId, long envHash) {
        structural("bootFrame0File");
        return source(NativeRuntime.bootFrame0File(handle, path, appBuildId, envHash), false, false);
    }

    public int bootFrame0Bytes(ByteBuffer envelope, int length, long appBuildId, long envHash) {
        structural("bootFrame0Bytes");
        return source(NativeRuntime.bootFrame0Bytes(handle, envelope, length, appBuildId, envHash), false, false);
    }

    public int bootFrame0Bytes(byte[] envelope, long appBuildId, long envHash) {
        structural("bootFrame0Bytes");
        final int len = envelope == null ? 0 : envelope.length;
        return bootFrame0Bytes(toScratchDirect(envelope), len, appBuildId, envHash);
    }

    /**
     * Applies a binary hot patch (direct buffer {@code [0..length)}); sets {@code needsFrame} when
     * changed and writes {@code outChanged[0]} when non-null.
     */
    public int applyHotPatch(ByteBuffer patch, int length, boolean[] outChanged) {
        structural("applyHotPatch");
        out[0] = 0;
        final int st = NativeRuntime.applyHotPatch(handle, patch, length, out);
        final boolean changed = st == NveRuntimeAbi.STATUS_OK && out[0] != 0;
        if (outChanged != null && outChanged.length >= 1) outChanged[0] = changed;
        if (st == NveRuntimeAbi.STATUS_OK) {
            if (changed) needsFrame = true;
            drainEmittedIfListening();
        }
        return st;
    }

    /** Applies a binary hot patch (direct buffer {@code [0..length)}); sets {@code needsFrame} when changed. */
    public int applyHotPatch(ByteBuffer patch, int length) {
        return applyHotPatch(patch, length, null);
    }

    /** Applies a binary hot patch; sets {@code needsFrame} when changed and writes {@code outChanged[0]} when non-null. */
    public int applyHotPatch(byte[] patch, boolean[] outChanged) {
        structural("applyHotPatch");
        final int len = patch == null ? 0 : patch.length;
        return applyHotPatch(toScratchDirect(patch), len, outChanged);
    }

    /** Applies a binary hot patch; sets {@code needsFrame} when changed. */
    public int applyHotPatch(byte[] patch) {
        return applyHotPatch(patch, null);
    }

    /** Measures the surface into caller-owned {@code outMeasure[0..4)} ({@code [w, h, firstBaseline, lastBaseline]}). */
    public int measureSurface(
            float minWidth, float minHeight, float maxWidth, float maxHeight, int flags, float[] outMeasure) {
        structural("measureSurface");
        return NativeRuntime.measureSurface(handle, minWidth, minHeight, maxWidth, maxHeight, flags, outMeasure);
    }

    /** Measures the surface, returning {@code null} on failure (see {@link #lastError}). */
    public SurfaceMeasure measureSurface(
            float minWidth, float minHeight, float maxWidth, float maxHeight, int flags) {
        if (measureSurface(minWidth, minHeight, maxWidth, maxHeight, flags, measureOut) != NveRuntimeAbi.STATUS_OK) {
            return null;
        }
        return new SurfaceMeasure(measureOut[0], measureOut[1], measureOut[2], measureOut[3]);
    }

    /** Installs ({@code null} removes; idempotent no-op when closed) the intrinsic-size invalidation callback. */
    public int setIntrinsicSizeInvalidatedListener(Runnable listener) {
        if (isClosedClearing(listener)) return NveRuntimeAbi.STATUS_OK;
        structural("setIntrinsicSizeInvalidatedListener");
        return NativeRuntime.setIntrinsicSizeInvalidatedListener(handle, listener);
    }

    /** Installs ({@code null} removes; idempotent no-op when closed) the {@code NVE_IFACE_HOST = 5} callback bridge. */
    public int setHostCallbackBridge(NativeRuntime.HostCallbackBridge bridge) {
        if (isClosedClearing(bridge)) return NveRuntimeAbi.STATUS_OK;
        structural("setHostCallbackBridge");
        return NativeRuntime.setHostCallbackBridge(handle, bridge);
    }

    /** Cross-thread safe: completes a {@code PENDING} host request (direct buffer {@code [0..length)}) without racing {@link #close}. */
    public int completeHostRequest(long requestId, int hostStatus, ByteBuffer payload, int length) {
        synchronized (handleLock) {
            final long h = handle;
            return h == 0L ? NveRuntimeAbi.STATUS_INVALID_ARG
                    : NativeRuntime.completeHostRequest(h, requestId, hostStatus, payload, length);
        }
    }

    /** Cross-thread safe: completes a {@code PENDING} host request without racing {@link #close} (zero off-heap allocation). */
    public int completeHostRequest(long requestId, int hostStatus, byte[] payload) {
        synchronized (handleLock) {
            final long h = handle;
            return h == 0L ? NveRuntimeAbi.STATUS_INVALID_ARG
                    : NativeRuntime.completeHostRequest(h, requestId, hostStatus, payload);
        }
    }

    /** Cross-thread safe: pushes an unsolicited host event (direct buffer {@code [0..length)}) without racing {@link #close}. */
    public int pushHostEvent(int serviceId, int eventId, ByteBuffer payload, int length) {
        synchronized (handleLock) {
            final long h = handle;
            return h == 0L ? NveRuntimeAbi.STATUS_INVALID_ARG
                    : NativeRuntime.pushHostEvent(h, serviceId, eventId, payload, length);
        }
    }

    /** Cross-thread safe: pushes an unsolicited host event without racing {@link #close} (zero off-heap allocation). */
    public int pushHostEvent(int serviceId, int eventId, byte[] payload) {
        synchronized (handleLock) {
            final long h = handle;
            return h == 0L ? NveRuntimeAbi.STATUS_INVALID_ARG
                    : NativeRuntime.pushHostEvent(h, serviceId, eventId, payload);
        }
    }

    /** Cross-thread safe: publishes an inbound host channel message (direct buffer {@code [0..length)}) without racing {@link #close}. */
    public int publishHostChannel(int channelId, int flags, ByteBuffer payload, int length) {
        synchronized (handleLock) {
            final long h = handle;
            return h == 0L ? NveRuntimeAbi.STATUS_INVALID_ARG
                    : NativeRuntime.publishHostChannel(h, channelId, flags, payload, length);
        }
    }

    /** Cross-thread safe: publishes an inbound host channel message without racing {@link #close} (zero off-heap allocation). */
    public int publishHostChannel(int channelId, int flags, byte[] payload) {
        synchronized (handleLock) {
            final long h = handle;
            return h == 0L ? NveRuntimeAbi.STATUS_INVALID_ARG
                    : NativeRuntime.publishHostChannel(h, channelId, flags, payload);
        }
    }

    private void drainEmittedIfListening() {
        final EmittedEventConsumer listener = emittedEventConsumer;
        if (listener == null || emittedUnsupported || handle == 0L || inFrame || closeAfterFrame) return;
        out[0] = NveRuntimeAbi.STATUS_OK;
        final byte[] batch = NativeRuntime.drainEmittedEvents(handle, out);
        if (out[0] == NveRuntimeAbi.STATUS_UNSUPPORTED) {
            emittedUnsupported = true;
            return;
        }
        if (batch != null && batch.length > 0) listener.onEmittedEvents(batch);
    }

    /**
     * Writes events with {@code writer} and dispatches them (rendering when the app changed), or,
     * inside {@code onFrame}, right after the frame is released. A batch that does not fit flushes
     * the pending events and grows the ring (up to 16 MiB). Returns a {@code STATUS_*} code.
     */
    @Override
    public int send(EventWriter writer) {
        if (handle == 0L) return NveRuntimeAbi.STATUS_INVALID_ARG;
        final int count = ring.count();
        final int arenaLength = ring.arenaLength();
        try {
            writer.write(ring);
        } catch (BufferOverflowException e) {
            ring.truncate(count, arenaLength);  // the batch is all-or-nothing
            final int st = inFrame || count == 0 ? NveRuntimeAbi.STATUS_OK : dispatchPending();
            if (st != NveRuntimeAbi.STATUS_OK) return st;
            if (!growAndRetry(writer)) return NveRuntimeAbi.STATUS_INVALID_ARG;
        } catch (RuntimeException e) {
            ring.truncate(count, arenaLength);
            throw e;
        }
        if (inFrame) return NveRuntimeAbi.STATUS_OK;
        final int st = dispatchPending();
        if (needsFrame) render();
        return st;
    }

    private boolean growAndRetry(EventWriter writer) {
        for (int cap = ring.capacity() * 2; cap <= MAX_RING_BYTES; cap *= 2) {
            final EventRingWriter grown = ring.copyTo(ByteBuffer.allocateDirect(cap));
            final int count = grown.count();
            final int arenaLength = grown.arenaLength();
            try {
                writer.write(grown);
                ring = grown;
                return true;
            } catch (BufferOverflowException retry) {
                grown.truncate(count, arenaLength);  // still too small: double again
            }
        }
        return false;
    }

    /** Dispatches and clears the pending ring; a changed app schedules a frame ({@code needsFrame}). */
    private int dispatchPending() {
        if (ring.count() == 0) return NveRuntimeAbi.STATUS_OK;
        out[0] = 0;
        final int st = NativeRuntime.dispatchEvents(handle, ring.buffer(), ring.length(), out);
        ring.reset();
        if (st == NveRuntimeAbi.STATUS_OK) {
            if (out[0] != 0) needsFrame = true;
            drainEmittedIfListening();
        }
        return st;
    }

    /** Topmost interactive raw {@code NodeId} at {@code (x, y)}, or {@code 0}. */
    public int hitTest(float x, float y) {
        if (handle == 0L || inFrame) return 0;
        out[0] = 0;
        return NativeRuntime.hitTest(handle, x, y, out) == NveRuntimeAbi.STATUS_OK ? out[0] : 0;
    }

    /** Syncs the runtime's scroll offset of {@code nodeId} (hit testing, app scroll state); deferred inside {@code onFrame}. */
    @Override
    public int scrollBy(int nodeId, float dx, float dy) {
        if (handle == 0L) return NveRuntimeAbi.STATUS_INVALID_ARG;
        if (!inFrame) return NativeRuntime.scrollBy(handle, nodeId, dx, dy, scrollOut);
        if (deferredScrolls == deferredScrollIds.length) {
            deferredScrollIds = Arrays.copyOf(deferredScrollIds, deferredScrolls * 2);
            deferredScrollDeltas = Arrays.copyOf(deferredScrollDeltas, deferredScrolls * 4);
        }
        deferredScrollIds[deferredScrolls] = nodeId;
        deferredScrollDeltas[deferredScrolls * 2] = dx;
        deferredScrollDeltas[deferredScrolls * 2 + 1] = dy;
        deferredScrolls++;
        return NveRuntimeAbi.STATUS_OK;
    }

    /**
     * Ticks the runtime and hands each changed frame to the consumer, then applies what the
     * consumer deferred (scrolls, events) and delivers the frames those cause, up to
     * {@link #MAX_FRAMES_PER_RENDER} (the rest is posted to the owner thread). Returns {@code true}
     * when a frame was delivered. Inside {@code onFrame} it only schedules a render. A
     * {@link #close} requested inside {@code onFrame} runs when the frame ends, also when the
     * consumer throws (the exception then propagates).
     */
    public boolean render() {
        if (handle == 0L) return false;
        if (inFrame) {
            needsFrame = true;
            return false;
        }
        boolean delivered = false;
        rendering = true;
        try {
            for (int frames = 0; handle != 0L; frames++) {
                if (frames == MAX_FRAMES_PER_RENDER) {
                    onWake();  // more work is pending: continue on a later owner-thread turn
                    break;
                }
                if (!deliverFrame()) break;
                delivered = true;
                if (handle == 0L || closeAfterFrame || (!replayDeferred() && !needsFrame)) break;
            }
        } finally {
            rendering = false;
            if (closeAfterFrame) close();
        }
        return delivered;
    }

    /**
     * One tick + (when changed) one acquired frame handed to the consumer. A frame the reader
     * rejects (malformed, e.g. from a broken pairing server) is skipped, not thrown at the host:
     * {@link #lastError} says why, and the next published frame is read as a gap.
     */
    private boolean deliverFrame() {
        out[0] = 0;
        final int st = NativeRuntime.tick(handle, tickSeconds(), out);
        if (st != NveRuntimeAbi.STATUS_OK) return false;
        if ((out[0] == 0 && !needsFrame) || consumer == null) {
            drainEmittedIfListening();
            return false;
        }
        if (NativeRuntime.acquireScene(handle, header, slabs) != NveRuntimeAbi.STATUS_OK) {
            return false;
        }
        inFrame = true;
        boolean ok = false;
        try {
            needsFrame = false;  // retrying a rejected frame would fail the same way
            try {
                reader.attach(header, slabs);
            } catch (SceneException e) {
                frameError = "rejected a malformed frame: " + e.getMessage();
                return false;
            }
            frameError = null;
            consumer.onFrame(reader, this);
            ok = true;
        } finally {
            reader.detach();
            Arrays.fill(slabs, null);
            NativeRuntime.releaseScene(handle);
            inFrame = false;
        }
        drainEmittedIfListening();
        return ok;
    }

    /** Seconds since the previous tick, clamped to {@code [0, MAX_TICK_SECONDS]} (0 on the first). */
    private float tickSeconds() {
        final long now = System.nanoTime();
        final long prev = lastTickNanos;
        lastTickNanos = now;
        if (prev == 0L) return 0f;
        return Math.min(Math.max((now - prev) / 1e9f, 0f), MAX_TICK_SECONDS);
    }

    /** Replays deferred scrolls and dispatches deferred events; {@code true} when a scroll ran. */
    private boolean replayDeferred() {
        final boolean scrolled = deferredScrolls > 0;
        for (int i = 0; i < deferredScrolls; i++) {
            NativeRuntime.scrollBy(handle, deferredScrollIds[i], deferredScrollDeltas[i * 2],
                    deferredScrollDeltas[i * 2 + 1], scrollOut);
        }
        deferredScrolls = 0;
        dispatchPending();
        return scrolled;
    }

    /**
     * Bytes of resource {@code id} in the frame being consumed (a private duplicate: callers may
     * move its position freely), or {@code null} outside {@code onFrame} / for dead ids; the kind
     * lands in {@link #resourceKind}.
     */
    @Override
    public ByteBuffer resource(int id) {
        kind[0] = 0L;
        if (!inFrame || handle == 0L) return null;
        final ByteBuffer b = NativeRuntime.resource(handle, id, kind);
        return b == null ? null : b.duplicate();
    }

    @Override
    public Key64 resourceKind() {
        return Key64.of(kind[0]);
    }

    private void onWake() {
        if (renderPosted.compareAndSet(false, true)) ownerThread.execute(this::runPostedRender);
    }

    /**
     * The posted {@link #render}. A synchronous executor runs it inline, possibly from inside a
     * render loop that hit {@link #MAX_FRAMES_PER_RENDER}: nesting would recurse without bound
     * while frames keep coming, so the remaining frames wait for the next wake / send / render.
     */
    private void runPostedRender() {
        renderPosted.set(false);
        if (rendering) {
            needsFrame = true;
            return;
        }
        render();
    }

    /**
     * Destroys the surface (after the current frame when called inside {@code onFrame}).
     * Idempotent: closing a closed surface does nothing.
     *
     * @throws IllegalStateException off the thread that created the surface (before touching any
     *     owner-thread state: a frame may be in flight there); the surface stays open
     */
    @Override
    public void close() {
        if (handle == 0L) return;
        if (Thread.currentThread() != owner) {
            throw new IllegalStateException("close must run on the surface's owner thread (" + owner.getName() + ")");
        }
        if (inFrame) {
            closeAfterFrame = true;
            return;
        }
        closeAfterFrame = false;
        final long h = handle;
        if (h == 0L) return;
        // Clear and drain cross-thread callbacks BEFORE acquiring `handleLock`: in-flight
        // callbacks may call `completeHostRequest`/`pushHostEvent`/`publishHostChannel`/`isPaired`
        // (which synchronize on `handleLock`), so draining outside the lock prevents AB-BA
        // deadlock against `NativeRuntime.destroy`. Only the owner thread can install callbacks
        // or close, so no new callback can start once cleared here.
        NativeRuntime.setWakeListener(h, null);
        NativeRuntime.setIntrinsicSizeInvalidatedListener(h, null);
        NativeRuntime.setHostCallbackBridge(h, null);
        emittedUnsupported = true;
        synchronized (handleLock) {
            if (handle == 0L) return;
            final int st = NativeRuntime.destroy(handle);
            if (st != NveRuntimeAbi.STATUS_OK) {
                throw new IllegalStateException("close refused (status " + st + "): it must run on the surface's owner thread");
            }
            handle = 0L;
        }
    }
}
