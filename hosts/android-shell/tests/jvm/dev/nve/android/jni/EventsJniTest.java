package dev.nve.android.jni;

import dev.nve.host.EventRingWriter;
import dev.nve.host.NativeRuntime;
import dev.nve.host.SceneReader;
import dev.nve.host.TableReader;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveRuntimeAbi;
import dev.nve.host.generated.NveTables;
import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/**
 * `EventRingWriter` `NVEV` v2 frames dispatched through `libnve_android_shell` into a real
 * attached app (`mock_app`, which logs every `NveEvent` to `$NVE_MOCK_EVENT_LOG` and re-renders
 * the next fixture frame after the first batch).
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
        c.eq(NativeRuntime.attachApp(h, args[1], false), 0, "attach showcase app");
        final int[] out = new int[1];
        c.eq(NativeRuntime.tick(h, 0f, out), 0, "first render");
        c.eq(title(scene, fx), "Hello NVE", "first frame");

        final int keyFlags = NveAbi.EVENT_FLAG_MOD_CTRL | NveAbi.EVENT_FLAG_COMPOSING;
        final int ptrFlags = NveAbi.EVENT_FLAG_MOD_SHIFT | EventRingWriter.pointerButtonFlag(2) | NveAbi.EVENT_FLAG_CAPTURE;
        final EventRingWriter ring = new EventRingWriter(ByteBuffer.allocateDirect(4096), 0);
        ring.press(fx.id("button"), 11L, 60f, 22f)
                .textChange(fx.id("input"), 12L, "h\u00e9llo \ud83d\ude80")
                .custom(fx.id("button"), 13L, "submit", new byte[] {1, 2, 3})
                .keyDown(fx.id("input"), 14L, "\u00e9", "KeyE", keyFlags, 69)
                .keyUp(fx.id("input"), 15L, "VeryLongCustomKeyName", "CodeX", NveAbi.EVENT_FLAG_MOD_SHIFT, 70)
                .pointer(NveAbi.INTERACTION_EVENT_POINTER_DOWN, fx.id("button"), 16L, 3f, 4f, 7, ptrFlags)
                .wheel(fx.id("scroll"), 17L, 0f, -12f, NveAbi.EVENT_FLAG_MOD_META)
                .scroll(fx.id("scroll"), 18L, 0f, 30f, 300f, 150f)
                .focus(fx.id("input"), 19L)
                .tableEvent(0x1122_3344_5566_7788L, 42, fx.id("canvas"), 20L, 0, 100, 200, 0, 0, 0, 0, new byte[] {1, 2})
                .layout(fx.id("button"), 21L, 1f, 2f, 3f, 4f);
        out[0] = -1;
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), out), 0, "dispatch status");
        c.eq(out[0], 1, "dispatch reports the app's change");
        final List<String> lines = Files.readAllLines(log, java.nio.charset.StandardCharsets.UTF_8);
        c.eq(lines.size(), 11, "every event reached the app");
        if (lines.size() == 11) {
            final String interaction = String.format("%016x", NveAbi.TABLE_INTERACTION.value());
            final String textInput = String.format("%016x", NveAbi.TABLE_TEXT_INPUT.value());
            final String scroll = String.format("%016x", NveAbi.TABLE_SCROLL.value());
            final String layout = String.format("%016x", NveAbi.TABLE_LAYOUT.value());
            final String custom = String.format("%016x", NveAbi.TABLE_CUSTOM.value());
            c.that(lines.get(0).startsWith("surface=5 node=" + fx.id("button") + " ts=11 table=" + interaction
                    + " event=" + NveAbi.INTERACTION_EVENT_PRESS) && lines.get(0).contains("f0=60 f1=22"), "press: " + lines.get(0));
            c.that(lines.get(1).contains("table=" + textInput + " event=" + NveAbi.TEXT_INPUT_EVENT_CHANGE)
                    && lines.get(1).contains("text=h\u00e9llo \ud83d\ude80 "), "text change keeps UTF-8: " + lines.get(1));
            c.that(lines.get(2).contains("table=" + custom + " event=0")
                    && lines.get(2).contains("cname=submit cdata=3"), "custom name + bytes: " + lines.get(2));
            final int expectedInlineFlags = keyFlags | NveAbi.EVENT_FLAG_INLINE_KEY_UTF8 | NveAbi.EVENT_FLAG_UTF8_BYTES;
            c.that(lines.get(3).contains("table=" + interaction + " event=" + NveAbi.INTERACTION_EVENT_KEY_DOWN
                    + " flags=" + expectedInlineFlags + " p0=69")
                    && lines.get(3).contains("key=\u00e9 code=KeyE "), "inline key: " + lines.get(3));
            final int expectedSplitFlags = NveAbi.EVENT_FLAG_MOD_SHIFT | NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES | NveAbi.EVENT_FLAG_UTF8_BYTES;
            c.that(lines.get(4).contains("table=" + interaction + " event=" + NveAbi.INTERACTION_EVENT_KEY_UP
                    + " flags=" + expectedSplitFlags + " p0=70")
                    && lines.get(4).contains("key=VeryLongCustomKeyName code=CodeX "), "split key: " + lines.get(4));
            c.that(lines.get(5).contains("table=" + interaction + " event=" + NveAbi.INTERACTION_EVENT_POINTER_DOWN
                    + " flags=" + ptrFlags) && lines.get(5).contains("p2=7 f0=3 f1=4"), "pointer: " + lines.get(5));
            c.that(lines.get(6).contains("table=" + interaction + " event=" + NveAbi.INTERACTION_EVENT_WHEEL
                    + " flags=" + NveAbi.EVENT_FLAG_MOD_META) && lines.get(6).contains("f0=0 f1=-12"), "wheel: " + lines.get(6));
            c.that(lines.get(7).contains("table=" + scroll + " event=" + NveAbi.SCROLL_EVENT_SCROLL)
                    && lines.get(7).contains("f1=30 f2=300 f3=150"), "scroll: " + lines.get(7));
            c.that(lines.get(8).contains("table=" + interaction + " event=" + NveAbi.INTERACTION_EVENT_FOCUS), "focus: " + lines.get(8));
            c.that(lines.get(9).contains("table=1122334455667788 event=42")
                    && lines.get(9).contains("p0=100 p1=200")
                    && lines.get(9).endsWith("bytes=\\x01\\x02"), "table event: " + lines.get(9));
            c.that(lines.get(10).contains("table=" + layout + " event=" + NveAbi.LAYOUT_EVENT_LAYOUT)
                    && lines.get(10).contains("f0=1 f1=2 f2=3 f3=4"), "layout: " + lines.get(10));
        }
        c.eq(NativeRuntime.tick(h, 0f, out), 0, "render after events");
        c.eq(title(scene, fx), "Hello again", "app re-rendered after the batch");

        // Malformed NVEV v2 rings are rejected before anything reaches the app.
        final long before = Files.size(log);
        final ByteBuffer raw = ring.buffer();
        c.eq(NativeRuntime.dispatchEvents(h, raw, ring.length() - 1, out), NveRuntimeAbi.STATUS_INVALID_ARG, "truncated ring");
        c.eq(NativeRuntime.dispatchEvents(h, raw, raw.capacity() + 4, out), NveRuntimeAbi.STATUS_INVALID_ARG, "length past capacity");
        c.eq(NativeRuntime.dispatchEvents(h, raw, EventRingWriter.HEADER_SIZE - 1, out), NveRuntimeAbi.STATUS_INVALID_ARG, "shorter than header");
        c.eq(NativeRuntime.dispatchEvents(h, ByteBuffer.allocate(64), EventRingWriter.HEADER_SIZE, out), NveRuntimeAbi.STATUS_INVALID_ARG, "heap buffer");

        final ByteBuffer bad = ByteBuffer.allocateDirect(128).order(java.nio.ByteOrder.LITTLE_ENDIAN);
        // Bad magic.
        bad.putInt(0, 0x5845564E).putShort(4, (short) 2).putInt(6, 0).putInt(10, 0);
        c.eq(NativeRuntime.dispatchEvents(h, bad, EventRingWriter.HEADER_SIZE, out), NveRuntimeAbi.STATUS_INVALID_ARG, "bad magic");
        // Wrong version (v1).
        bad.putInt(0, EventRingWriter.WIRE_MAGIC).putShort(4, (short) 1).putInt(6, 0).putInt(10, 0);
        c.eq(NativeRuntime.dispatchEvents(h, bad, EventRingWriter.HEADER_SIZE, out), NveRuntimeAbi.STATUS_INVALID_ARG, "wrong version");
        // Count past the ring.
        bad.putShort(4, (short) 2).putInt(6, 0x7FFFFFFF).putInt(10, 0);
        c.eq(NativeRuntime.dispatchEvents(h, bad, EventRingWriter.HEADER_SIZE + NveAbi.SIZEOF_EVENT, out), NveRuntimeAbi.STATUS_INVALID_ARG, "count past ring");

        // Single-event hostile cases (header 14 + 64-byte event + 2-byte arena = 80 bytes).
        final int ev0 = EventRingWriter.HEADER_SIZE;
        final int arena0 = ev0 + NveAbi.SIZEOF_EVENT;
        for (int i = 0; i < 80; i++) bad.put(i, (byte) 0);
        bad.putInt(0, EventRingWriter.WIRE_MAGIC).putShort(4, (short) 2).putInt(6, 1).putInt(10, 2);
        // Out-of-bounds arena span.
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_OFFSET, 1);
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_LEN, 2);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 80, out), NveRuntimeAbi.STATUS_INVALID_ARG, "out-of-bounds arena span");
        // Conflicting framing flags.
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_OFFSET, 0);
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_LEN, 2);
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_FLAGS, NveAbi.EVENT_FLAG_INLINE_KEY_UTF8 | NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 80, out), NveRuntimeAbi.STATUS_INVALID_ARG, "conflicting framing flags");
        // Invalid UTF-8 in text arena slice (0xC3 0x28).
        bad.putLong(ev0 + NveAbi.EVENT_OFFSET_TABLE, NveAbi.TABLE_TEXT_INPUT.value());
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_FLAGS, NveAbi.EVENT_FLAG_UTF8_BYTES);
        bad.put(arena0, (byte) 0xC3).put(arena0 + 1, (byte) 0x28);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 80, out), NveRuntimeAbi.STATUS_INVALID_ARG, "malformed UTF-8 text");
        // Invalid UTF-8 in inline key.
        bad.putLong(ev0 + NveAbi.EVENT_OFFSET_TABLE, NveAbi.TABLE_INTERACTION.value());
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_FLAGS, NveAbi.EVENT_FLAG_INLINE_KEY_UTF8);
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_PAYLOAD + 4, 1);
        bad.put(ev0 + NveAbi.EVENT_OFFSET_PAYLOAD + 8, (byte) 0xFF);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 80, out), NveRuntimeAbi.STATUS_INVALID_ARG, "malformed UTF-8 inline key");
        // Invalid UTF-8 in custom event name.
        bad.putLong(ev0 + NveAbi.EVENT_OFFSET_TABLE, NveAbi.TABLE_CUSTOM.value());
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_FLAGS, NveAbi.EVENT_FLAG_CUSTOM_NAMED);
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_PAYLOAD, 1);
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_LEN, 1);
        bad.putInt(10, 1);
        bad.put(arena0, (byte) 0xFF);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 79, out), NveRuntimeAbi.STATUS_INVALID_ARG, "malformed UTF-8 custom name");
        // Trailing byte.
        ring.reset().blur(fx.id("input"), 1L);
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length() + 1, out), NveRuntimeAbi.STATUS_INVALID_ARG, "trailing byte");
        c.eq(Files.size(log), before, "rejected rings never reach the app");

        // Repair the custom name byte -> accepted and delivered.
        bad.put(arena0, (byte) 'x');
        c.eq(NativeRuntime.dispatchEvents(h, bad, 79, out), 0, "the same custom event with a UTF-8 name is accepted");
        final long accepted = Files.size(log);
        c.that(accepted > before, "the accepted custom event reached the app");
        ring.reset();
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), out), 0, "empty ring is valid");
        c.eq(out[0], 0, "empty ring changes nothing");
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), null), 0, "null out array is allowed");
        NativeRuntime.destroy(h);
        c.finish("android_events_jni_test");
    }

    static String title(Fixtures.Scene scene, Fixtures fx) {
        final SceneReader s = scene.acquire();
        final TableReader text = s.findTable(NveTables.Text.KEY);
        final int r = text == null ? -1 : s.rowOf(text, s.resolve(fx.id("title")));
        final String out = r < 0 ? null : s.text(text, r, NveTables.Text.TEXT);
        scene.release();
        return out;
    }
}
