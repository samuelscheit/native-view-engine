package dev.nve.android.jni;

import dev.nve.host.EventRingWriter;
import dev.nve.host.NativeRuntime;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.TableReader;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveRuntimeAbi;
import dev.nve.host.generated.NveTables;
import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/**
 * `EventRingWriter` rings dispatched through `libnve_android_shell` into a real attached app
 * (`mock_app`, which logs every `NveDirectEvent` to `$NVE_MOCK_EVENT_LOG` and re-renders the next
 * fixture frame after the first batch).
 *
 * <p>Args: fixture dir, showcase app, event log path.
 */
public final class EventsJniTest {
    private EventsJniTest() {}

    public static void main(String[] args) throws Exception {
        System.loadLibrary("nve_android_shell");
        final Expect c = new Expect();
        final Fixtures fx = new Fixtures(Path.of(args[0]));
        final Path log = Path.of(args[2]);
        Files.deleteIfExists(log);
        final long h = NativeRuntime.create(5, 360f, 800f, 1f);
        final Fixtures.Scene scene = new Fixtures.Scene(h);
        c.eq(NativeRuntime.attachApp(h, args[1]), 0, "attach showcase app");
        final int[] out = new int[1];
        c.eq(NativeRuntime.tick(h, 0f, out), 0, "first render");
        c.eq(title(scene, fx), "Hello NVE", "first frame");

        final EventRingWriter ring = new EventRingWriter(ByteBuffer.allocateDirect(4096), 0);
        ring.point(NveAbi.EVENT_PRESS, fx.id("button"), 11L, 60f, 22f)
                .text(NveAbi.EVENT_TEXT_CHANGE, fx.id("input"), 12L, "h\u00e9llo \ud83d\ude80")
                .custom(NveAbi.EVENT_CUSTOM, fx.id("button"), 13L, "submit", new byte[] {1, 2, 3})
                .key(NveAbi.EVENT_KEY_DOWN, fx.id("input"), 14L, "\u00e9", "KeyE", 2, 69, true)
                .pointer(NveAbi.EVENT_POINTER_DOWN, fx.id("button"), 15L, 3f, 4f, 0x04, 0x09)
                .wheel(NveAbi.EVENT_WHEEL, fx.id("scroll"), 16L, 0f, -12f, 0)
                .scroll(NveAbi.EVENT_SCROLL, fx.id("scroll"), 17L, 0f, 30f, 300f, 150f)
                .empty(NveAbi.EVENT_FOCUS, fx.id("input"), 18L)
                .tableEvent(NveAbi.EVENT_TABLE_EVENT, fx.id("canvas"), 19L, 0x1122_3344_5566_7788L, 42, new byte[] {1, 2})
                .layout(NveAbi.EVENT_LAYOUT, fx.id("button"), 20L, 1f, 2f, 3f, 4f);
        out[0] = -1;
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), out), 0, "dispatch status");
        c.eq(out[0], 1, "dispatch reports the app's change");
        final List<String> lines = Files.readAllLines(log, java.nio.charset.StandardCharsets.UTF_8);
        c.eq(lines.size(), 10, "every event reached the app");
        if (lines.size() == 10) {
            c.that(lines.get(0).startsWith("surface=5 node=" + fx.id("button") + " ts=11 kind=1 tag=1") && lines.get(0).contains("x=60 y=22"), "press: " + lines.get(0));
            c.that(lines.get(1).contains("kind=4 tag=3") && lines.get(1).contains("text=h\u00e9llo \ud83d\ude80 "), "text change keeps UTF-8: " + lines.get(1));
            c.that(lines.get(2).contains("tag=5") && lines.get(2).contains("name=submit value=3"), "custom name + bytes: " + lines.get(2));
            c.that(lines.get(3).contains("tag=6 mods=2 flags=16 key=69") && lines.get(3).contains("text=\u00e9 name=KeyE "), "key: " + lines.get(3));
            c.that(lines.get(4).contains("tag=7 mods=4 flags=9") && lines.get(4).contains("x=3 y=4"), "pointer modifiers / flags not swapped: " + lines.get(4));
            c.that(lines.get(5).contains("tag=8") && lines.get(5).contains("y=-12"), "wheel: " + lines.get(5));
            c.that(lines.get(6).contains("tag=2") && lines.get(6).contains("y=30 w=300 h=150"), "scroll: " + lines.get(6));
            c.that(lines.get(7).contains("kind=5 tag=0"), "focus: " + lines.get(7));
            c.that(lines.get(8).contains("kind=21 tag=9") && lines.get(8).contains("key=42")
                    && lines.get(8).contains("text=\\x01\\x02 ") && lines.get(8).endsWith("i64=1122334455667788"), "table event: " + lines.get(8));
            c.that(lines.get(9).contains("kind=7 tag=4") && lines.get(9).contains("x=1 y=2 w=3 h=4"), "layout: " + lines.get(9));
        }
        c.eq(NativeRuntime.tick(h, 0f, out), 0, "render after events");
        c.eq(title(scene, fx), "Hello again", "app re-rendered after the batch");

        // Malformed rings are rejected before anything reaches the app.
        final long before = Files.size(log);
        final ByteBuffer raw = ring.buffer();
        c.eq(NativeRuntime.dispatchEvents(h, raw, ring.length() - 1, out), NveRuntimeAbi.STATUS_INVALID_ARG, "truncated ring");
        c.eq(NativeRuntime.dispatchEvents(h, raw, raw.capacity() + 4, out), NveRuntimeAbi.STATUS_INVALID_ARG, "length past capacity");
        c.eq(NativeRuntime.dispatchEvents(h, raw, 3, out), NveRuntimeAbi.STATUS_INVALID_ARG, "shorter than the count");
        c.eq(NativeRuntime.dispatchEvents(h, ByteBuffer.allocate(64), 4, out), NveRuntimeAbi.STATUS_INVALID_ARG, "heap buffer");
        final ByteBuffer bad = ByteBuffer.allocateDirect(64).order(java.nio.ByteOrder.LITTLE_ENDIAN);
        bad.putInt(0, 1).putInt(4, 0).putInt(8, 1).putLong(12, 0L).put(20, (byte) 1).put(21, (byte) 42);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 22, out), NveRuntimeAbi.STATUS_INVALID_ARG, "unknown payload tag");
        bad.putInt(0, 0x7FFFFFFF);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 22, out), NveRuntimeAbi.STATUS_INVALID_ARG, "count past the ring");
        ring.reset().empty(NveAbi.EVENT_BLUR, fx.id("input"), 1L);
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length() + 1, out), NveRuntimeAbi.STATUS_INVALID_ARG, "trailing byte");
        c.eq(Files.size(log), before, "rejected rings never reach the app");
        ring.reset();
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), out), 0, "empty ring is valid");
        c.eq(out[0], 0, "empty ring changes nothing");
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), null), 0, "null out array is allowed");
        NativeRuntime.destroy(h);
        c.finish("android_events_jni_test");
    }

    static String title(Fixtures.Scene scene, Fixtures fx) {
        final RuntimeSceneReader s = scene.acquire();
        final TableReader text = s.findTable(NveTables.Text.KEY);
        final int r = text == null ? -1 : s.rowOf(text, s.resolve(fx.id("title")));
        final String out = r < 0 ? null : s.text(text, r, NveTables.Text.TEXT);
        scene.release();
        return out;
    }
}
