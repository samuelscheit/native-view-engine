package dev.nve.host;

import java.nio.ByteBuffer;
import java.util.concurrent.Executor;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * One runtime surface for a Java / Kotlin host: owns the {@link NativeRuntime} handle, the packed
 * scene arrays + {@link RuntimeSceneReader}, a reusable {@link EventRingWriter}, and wake
 * coalescing onto the owner thread.
 *
 * <p>Use it on its owner thread only (the thread that constructs it; Android: the main thread).
 * The runtime's wake listener may fire on any thread; it posts at most one pending
 * {@link #render} to {@code ownerThread}. Each rendered frame is handed to the
 * {@link FrameConsumer} while the scene is acquired, so its buffers and {@link #resource} bytes are
 * valid only inside {@link FrameConsumer#onFrame}.
 */
public final class RuntimeSurface implements AutoCloseable {
    /** Receives each new frame while the scene is acquired. */
    public interface FrameConsumer {
        void onFrame(RuntimeSceneReader scene, RuntimeSurface surface);
    }

    private static final int EVENT_RING_BYTES = 64 * 1024;

    private final long[] header = new long[RuntimeSceneReader.HEADER_LEN];
    private final ByteBuffer[] slabs = new ByteBuffer[RuntimeSceneReader.SLABS_LEN];
    private final RuntimeSceneReader reader = new RuntimeSceneReader();
    private final EventRingWriter events;
    private final Executor ownerThread;
    private final AtomicBoolean renderPosted = new AtomicBoolean();
    private final long[] kindFlags = new long[2];
    private long handle;
    private FrameConsumer consumer;
    private boolean needsFrame = true;
    private long lastFrameId = -1L;

    /** @throws IllegalStateException when the runtime cannot create the surface */
    public RuntimeSurface(int surfaceId, float width, float height, float scaleFactor, Executor ownerThread) {
        this.handle = NativeRuntime.create(surfaceId, width, height, scaleFactor);
        if (handle == 0L) throw new IllegalStateException("nve runtime surface creation failed");
        this.ownerThread = ownerThread;
        this.events = new EventRingWriter(ByteBuffer.allocateDirect(EVENT_RING_BYTES), surfaceId);
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

    /** Loads and attaches (hot-reloads) the app {@code .so}; returns a {@code STATUS_*} code. */
    public int attachApp(String soPath) {
        return afterStructuralChange(NativeRuntime.attachApp(handle, soPath));
    }

    public int detachApp() {
        return afterStructuralChange(NativeRuntime.detachApp(handle));
    }

    /** Streams the scene from an {@code nve pair} server instead of a local app. */
    public int pair(String uri, String token) {
        return afterStructuralChange(NativeRuntime.pairConnect(handle, uri, token == null ? "" : token));
    }

    public int unpair() {
        return afterStructuralChange(NativeRuntime.pairDisconnect(handle));
    }

    public boolean isPaired() {
        return handle != 0L && NativeRuntime.pairIsConnected(handle);
    }

    /** Applies a host-produced {@code NVE2} frame (direct buffer). */
    public int submitFrame(ByteBuffer nve2, int length) {
        return afterStructuralChange(NativeRuntime.submitFrame(handle, nve2, length));
    }

    public int setViewport(float width, float height, float scaleFactor) {
        return afterStructuralChange(NativeRuntime.setViewport(handle, width, height, scaleFactor));
    }

    /** Installs ({@code null} removes) the host text measurer; relayouts every node. */
    public int setTextMeasurer(NativeRuntime.TextMeasurer measurer) {
        return afterStructuralChange(NativeRuntime.setTextMeasurer(handle, measurer));
    }

    private int afterStructuralChange(int status) {
        if (status == NativeRuntime.STATUS_OK) needsFrame = true;
        return status;
    }

    /** The reusable ring for this surface's next {@link #dispatch}. */
    public EventRingWriter events() {
        return events;
    }

    /**
     * Dispatches and clears the pending ring; renders when the app changed. Returns {@code 1}
     * changed, {@code 0} unchanged, {@code -status} on failure.
     */
    public int dispatch() {
        if (events.count() == 0) return 0;
        final int r = NativeRuntime.dispatchEvents(handle, events.buffer(), events.length());
        events.reset();
        if (r > 0) needsFrame = true;
        render();
        return r;
    }

    public int hitTest(float x, float y) {
        return NativeRuntime.hitTest(handle, x, y);
    }

    /** Syncs the runtime's scroll offset of {@code nodeId} (hit testing, app scroll state). */
    public int scrollBy(int nodeId, float dx, float dy, float[] outXY) {
        return NativeRuntime.scrollBy(handle, nodeId, dx, dy, outXY);
    }

    /**
     * Ticks the runtime and hands a changed frame to the consumer. Returns {@code true} when a
     * frame was delivered.
     */
    public boolean render() {
        if (handle == 0L) return false;
        final int ticked = NativeRuntime.tick(handle, 0f);
        if (ticked < 0 || (ticked == 0 && !needsFrame) || consumer == null) return false;
        if (NativeRuntime.acquireScene(handle, header, slabs) != NativeRuntime.STATUS_OK) return false;
        try {
            reader.attach(header, slabs);
            if (!needsFrame && reader.frameId() == lastFrameId) return false;
            lastFrameId = reader.frameId();
            needsFrame = false;
            consumer.onFrame(reader, this);
            return true;
        } finally {
            NativeRuntime.releaseScene(handle);
        }
    }

    /**
     * Wire bytes of resource {@code id} in the frame being consumed (or {@code null}); kind and
     * flags land in {@link #resourceKind} / {@link #resourceFlags}.
     */
    public ByteBuffer resource(int id) {
        kindFlags[0] = 0L;
        kindFlags[1] = 0L;
        return NativeRuntime.resource(handle, id, kindFlags);
    }

    public Key64 resourceKind() {
        return Key64.of(kindFlags[0]);
    }

    public long resourceFlags() {
        return kindFlags[1];
    }

    private void onWake() {
        if (renderPosted.compareAndSet(false, true)) {
            ownerThread.execute(() -> {
                renderPosted.set(false);
                render();
            });
        }
    }

    @Override
    public void close() {
        if (handle == 0L) return;
        NativeRuntime.destroy(handle);
        handle = 0L;
    }
}
