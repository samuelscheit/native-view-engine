package dev.nve.host;

import dev.nve.host.generated.NveRuntimeAbi;
import java.nio.Buffer;
import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.util.concurrent.Executor;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * One runtime surface for a Java / Kotlin host: owns the {@link NativeRuntime} handle, the packed
 * scene arrays + {@link RuntimeSceneReader}, a growable {@link EventRingWriter}, and wake
 * coalescing onto the owner thread.
 *
 * <p>Use it on its owner thread only (the thread that constructs it; Android: the main thread).
 * The wake listener may fire on any thread; it posts at most one pending {@link #render} to
 * {@code ownerThread}.
 *
 * <p>Frames: each new frame reaches the {@link FrameConsumer} while the scene is acquired. Inside
 * {@code onFrame} the reader and {@link #resource} bytes are valid; nothing may touch the native
 * scene afterwards (the reader is {@link RuntimeSceneReader#detach detached} before release). Calls
 * that would write the runtime are re-entrancy safe: {@link #send} and {@link #scrollBy} made
 * inside {@code onFrame} (e.g. by listeners that fire while the consumer mutates its UI) run right
 * after the scene is released; structural calls ({@link #attachApp}, {@link #setViewport}, ...)
 * inside {@code onFrame} are programming errors and throw.
 */
public final class RuntimeSurface implements AutoCloseable {
    /** Receives each new frame while the scene is acquired. */
    public interface FrameConsumer {
        void onFrame(RuntimeSceneReader scene, RuntimeSurface surface);
    }

    /** Writes one or more events into the surface's ring. */
    public interface EventWriter {
        void write(EventRingWriter ring);
    }

    static final int INITIAL_RING_BYTES = 64 * 1024;
    static final int MAX_RING_BYTES = 16 * 1024 * 1024;

    private final long[] header = new long[RuntimeSceneReader.HEADER_LEN];
    private final ByteBuffer[] slabs = new ByteBuffer[RuntimeSceneReader.SLABS_LEN];
    private final RuntimeSceneReader reader = new RuntimeSceneReader();
    private final int surfaceId;
    private final Executor ownerThread;
    private final AtomicBoolean renderPosted = new AtomicBoolean();
    private final int[] out = new int[1];
    private final long[] kind = new long[1];
    private final float[] scrollOut = new float[2];
    private EventRingWriter ring;
    private long handle;
    private FrameConsumer consumer;
    private boolean needsFrame = true;
    private boolean inFrame;
    private boolean closeAfterFrame;
    private final java.util.ArrayList<float[]> deferredScrolls = new java.util.ArrayList<>();

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

    public String lastError() {
        return handle == 0L ? "" : NativeRuntime.lastError(handle);
    }

    private void structural(String what) {
        if (inFrame) throw new IllegalStateException(what + " inside onFrame would invalidate the frame being read");
        if (handle == 0L) throw new IllegalStateException(what + " on a closed surface");
    }

    private int afterStructuralChange(int status) {
        if (status == NveRuntimeAbi.STATUS_OK) needsFrame = true;
        return status;
    }

    /** Loads and attaches (hot-reloads) the app {@code .so}; returns a {@code STATUS_*} code. */
    public int attachApp(String soPath) {
        structural("attachApp");
        return afterStructuralChange(NativeRuntime.attachApp(handle, soPath));
    }

    public int detachApp() {
        structural("detachApp");
        return afterStructuralChange(NativeRuntime.detachApp(handle));
    }

    /** Streams the scene from an {@code nve pair} server instead of a local app. */
    public int pair(String uri, String token) {
        structural("pair");
        return afterStructuralChange(NativeRuntime.pairConnect(handle, uri, token == null ? "" : token));
    }

    public int unpair() {
        structural("unpair");
        return afterStructuralChange(NativeRuntime.pairDisconnect(handle));
    }

    public boolean isPaired() {
        return handle != 0L && NativeRuntime.pairIsConnected(handle);
    }

    /** Applies a host-produced {@code NVE2} frame (direct buffer). */
    public int submitFrame(ByteBuffer nve2, int length) {
        structural("submitFrame");
        return afterStructuralChange(NativeRuntime.submitFrame(handle, nve2, length));
    }

    public int setViewport(float width, float height, float scaleFactor) {
        structural("setViewport");
        return afterStructuralChange(NativeRuntime.setViewport(handle, width, height, scaleFactor));
    }

    /** Installs ({@code null} removes) the host text measurer; the next render relayouts. */
    public int setTextMeasurer(NativeRuntime.TextMeasurer measurer) {
        structural("setTextMeasurer");
        return afterStructuralChange(NativeRuntime.setTextMeasurer(handle, measurer));
    }

    /**
     * Writes events with {@code writer} and dispatches them (rendering when the app changed), or,
     * inside {@code onFrame}, right after the frame is released. A batch that does not fit flushes
     * the pending events and grows the ring (up to 16 MiB). Returns a {@code STATUS_*} code.
     */
    public int send(EventWriter writer) {
        if (handle == 0L) return NveRuntimeAbi.STATUS_INVALID_ARG;
        final int count = ring.count();
        final int length = ring.length();
        try {
            writer.write(ring);
        } catch (BufferOverflowException e) {
            ring.truncate(count, length);  // the batch is all-or-nothing
            final int st = inFrame || count == 0 ? NveRuntimeAbi.STATUS_OK : flush();
            if (st != NveRuntimeAbi.STATUS_OK) return st;
            if (!growAndRetry(writer)) return NveRuntimeAbi.STATUS_INVALID_ARG;
        } catch (RuntimeException e) {
            ring.truncate(count, length);
            throw e;
        }
        return inFrame ? NveRuntimeAbi.STATUS_OK : flush();
    }

    private boolean growAndRetry(EventWriter writer) {
        for (int cap = ring.buffer().capacity() * 2; cap <= MAX_RING_BYTES; cap *= 2) {
            final ByteBuffer bigger = ByteBuffer.allocateDirect(cap);
            final ByteBuffer pending = ring.buffer().duplicate();
            ((Buffer) pending).flip();
            bigger.put(pending);
            final EventRingWriter grown = EventRingWriter.resume(bigger, surfaceId, ring.count());
            final int count = grown.count();
            final int length = grown.length();
            try {
                writer.write(grown);
                ring = grown;
                return true;
            } catch (BufferOverflowException retry) {
                grown.truncate(count, length);  // still too small: double again
            }
        }
        return false;
    }

    /** Dispatches and clears the pending ring, then renders when the app changed. */
    private int flush() {
        if (ring.count() == 0) return NveRuntimeAbi.STATUS_OK;
        out[0] = 0;
        final int st = NativeRuntime.dispatchEvents(handle, ring.buffer(), ring.length(), out);
        ring.reset();
        if (st == NveRuntimeAbi.STATUS_OK && out[0] != 0) needsFrame = true;
        render();
        return st;
    }

    /** Topmost interactive raw {@code NodeId} at {@code (x, y)}, or {@code 0}. */
    public int hitTest(float x, float y) {
        if (handle == 0L || inFrame) return 0;
        out[0] = 0;
        return NativeRuntime.hitTest(handle, x, y, out) == NveRuntimeAbi.STATUS_OK ? out[0] : 0;
    }

    /** Syncs the runtime's scroll offset of {@code nodeId} (hit testing, app scroll state); deferred inside {@code onFrame}. */
    public int scrollBy(int nodeId, float dx, float dy) {
        if (handle == 0L) return NveRuntimeAbi.STATUS_INVALID_ARG;
        if (inFrame) {
            deferredScrolls.add(new float[] {Float.intBitsToFloat(nodeId), dx, dy});
            return NveRuntimeAbi.STATUS_OK;
        }
        return NativeRuntime.scrollBy(handle, nodeId, dx, dy, scrollOut);
    }

    /**
     * Ticks the runtime and hands a changed frame to the consumer. Returns {@code true} when a
     * frame was delivered. Inside {@code onFrame} it only schedules another render.
     */
    public boolean render() {
        if (handle == 0L) return false;
        if (inFrame) {
            needsFrame = true;
            onWake();
            return false;
        }
        out[0] = 0;
        final int st = NativeRuntime.tick(handle, 0f, out);
        if (st != NveRuntimeAbi.STATUS_OK || (out[0] == 0 && !needsFrame) || consumer == null) return false;
        if (NativeRuntime.acquireScene(handle, header, slabs) != NveRuntimeAbi.STATUS_OK) return false;
        inFrame = true;
        try {
            reader.attach(header, slabs);
            needsFrame = false;
            consumer.onFrame(reader, this);
            return true;
        } finally {
            reader.detach();
            java.util.Arrays.fill(slabs, null);
            NativeRuntime.releaseScene(handle);
            inFrame = false;
            afterFrame();
        }
    }

    private void afterFrame() {
        if (closeAfterFrame) {
            close();
            return;
        }
        for (int i = 0; i < deferredScrolls.size(); i++) {
            final float[] s = deferredScrolls.get(i);
            NativeRuntime.scrollBy(handle, Float.floatToRawIntBits(s[0]), s[1], s[2], scrollOut);
        }
        deferredScrolls.clear();
        if (ring.count() > 0) flush();
    }

    /**
     * Bytes of resource {@code id} in the frame being consumed (a private duplicate: callers may
     * move its position freely), or {@code null} outside {@code onFrame} / for dead ids; the kind
     * lands in {@link #resourceKind}.
     */
    public ByteBuffer resource(int id) {
        kind[0] = 0L;
        if (!inFrame || handle == 0L) return null;
        final ByteBuffer b = NativeRuntime.resource(handle, id, kind);
        return b == null ? null : b.duplicate();
    }

    public Key64 resourceKind() {
        return Key64.of(kind[0]);
    }

    private void onWake() {
        if (renderPosted.compareAndSet(false, true)) {
            ownerThread.execute(() -> {
                renderPosted.set(false);
                render();
            });
        }
    }

    /** Destroys the surface (after the current frame when called inside {@code onFrame}). */
    @Override
    public void close() {
        if (handle == 0L) return;
        if (inFrame) {
            closeAfterFrame = true;
            return;
        }
        final long h = handle;
        handle = 0L;
        NativeRuntime.destroy(h);
    }
}
