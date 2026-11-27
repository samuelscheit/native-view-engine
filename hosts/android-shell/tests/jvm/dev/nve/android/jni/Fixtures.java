package dev.nve.android.jni;

import dev.nve.host.NativeRuntime;
import dev.nve.host.generated.NveRuntimeAbi;
import dev.nve.host.RuntimeSceneReader;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/** `nve-host-fixtures` frames and ids, plus a packed-scene helper over {@link NativeRuntime}. */
final class Fixtures {
    final Path dir;
    final Map<String, Integer> ids = new HashMap<>();

    Fixtures(Path dir) throws IOException {
        this.dir = dir;
        final List<String> lines = Files.readAllLines(dir.resolve("showcase.ids"));
        for (String line : lines) {
            final int eq = line.indexOf('=');
            if (eq > 0) ids.put(line.substring(0, eq), Integer.parseInt(line.substring(eq + 1)));
        }
    }

    int id(String name) {
        final Integer v = ids.get(name);
        if (v == null) throw new IllegalStateException("fixture id " + name + " missing");
        return v;
    }

    /** `NVE2` frame `name` in a direct buffer. */
    ByteBuffer frame(String name) throws IOException {
        final byte[] bytes = Files.readAllBytes(dir.resolve(name));
        final ByteBuffer b = ByteBuffer.allocateDirect(bytes.length);
        b.put(bytes).flip();
        return b;
    }

    /** One reusable packed frame + reader; {@link #acquire} attaches it, {@link #release} drops it. */
    static final class Scene {
        final long[] header = new long[RuntimeSceneReader.HEADER_LEN];
        final java.nio.ByteBuffer[] slabs = new java.nio.ByteBuffer[RuntimeSceneReader.SLABS_LEN];
        final RuntimeSceneReader reader = new RuntimeSceneReader();
        final long handle;

        Scene(long handle) {
            this.handle = handle;
        }

        RuntimeSceneReader acquire() {
            final int st = NativeRuntime.acquireScene(handle, header, slabs);
            if (st != NveRuntimeAbi.STATUS_OK) {
                throw new IllegalStateException("acquireScene " + st + ": " + NativeRuntime.lastError(handle));
            }
            try {
                return reader.attach(header, slabs);
            } catch (RuntimeException e) {
                NativeRuntime.releaseScene(handle);
                throw e;
            }
        }

        void release() {
            reader.detach();
            NativeRuntime.releaseScene(handle);
        }
    }
}
