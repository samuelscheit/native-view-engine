package dev.nve.host;

import java.nio.ByteBuffer;

/**
 * What a frame consumer (a retained renderer) calls on its runtime surface: the acquired frame's
 * resources and the input it sends back. {@link RuntimeSurface} implements it over
 * {@code libnve_runtime}; renderer suites implement it with fakes, so renderers never depend on
 * the native surface itself.
 */
public interface FrameHost {
    /** Writes one or more events into the surface's ring. */
    interface EventWriter {
        void write(EventRingWriter ring);
    }

    /** Dispatches the events {@code writer} writes (deferred while a frame is consumed); a {@code STATUS_*} code. */
    int send(EventWriter writer);

    /** Syncs the runtime scroll offset of {@code nodeId} after a native scroll; a {@code STATUS_*} code. */
    int scrollBy(int nodeId, float dx, float dy);

    /**
     * Bytes of resource {@code id} in the frame being consumed (the caller may move its position),
     * or {@code null} outside a frame / for dead ids; the kind lands in {@link #resourceKind}.
     */
    ByteBuffer resource(int id);

    /** Kind of the resource last returned by {@link #resource}. */
    Key64 resourceKind();
}
