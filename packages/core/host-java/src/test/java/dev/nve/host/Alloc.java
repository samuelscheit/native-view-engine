package dev.nve.host;

import java.lang.management.ManagementFactory;

/** Per-thread allocated-bytes probe ({@code com.sun.management.ThreadMXBean}, HotSpot and OpenJ9). */
final class Alloc {
    private Alloc() {}

    private static final com.sun.management.ThreadMXBean BEAN =
            (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();

    /** Escape sink so the probe's self-check allocation cannot be optimized away. */
    static volatile Object sink;

    /** {@code true} when the JVM really counts allocations (a 1 MiB array must register). */
    static boolean supported() {
        return bytes(() -> sink = new byte[1 << 20]) >= 1 << 20;
    }

    /** Bytes the current thread allocated while running {@code body} (the probe itself is subtracted). */
    static long bytes(Runnable body) {
        final long id = Thread.currentThread().getId();
        final long probe0 = BEAN.getThreadAllocatedBytes(id);
        final long probe1 = BEAN.getThreadAllocatedBytes(id);
        final long before = BEAN.getThreadAllocatedBytes(id);
        body.run();
        return BEAN.getThreadAllocatedBytes(id) - before - (probe1 - probe0);
    }
}
