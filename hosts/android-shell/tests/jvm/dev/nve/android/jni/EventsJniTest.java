package dev.nve.android.jni;

import dev.nve.host.BinReader;
import dev.nve.host.BinTag;
import dev.nve.host.BinWriter;
import dev.nve.host.EventRingReader;
import dev.nve.host.EventRingWriter;
import dev.nve.host.HybridRuntime;
import dev.nve.host.NativeRuntime;
import dev.nve.host.RuntimeSurface;
import dev.nve.host.SceneReader;
import dev.nve.host.TableReader;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveRuntimeAbi;
import dev.nve.host.generated.NveTables;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

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

        // Repair the custom name byte and inject non-canonical `0x01000000` handles -> canonicalized
        // (surface -> 0 -> default_surface_id=5, node -> 0) and delivered.
        bad.put(arena0, (byte) 'x');
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_SURFACE, 0x01000000);
        bad.putInt(ev0 + NveAbi.EVENT_OFFSET_NODE, 0x01000000);
        c.eq(NativeRuntime.dispatchEvents(h, bad, 79, out), 0, "the same custom event with a UTF-8 name is accepted");
        final long accepted = Files.size(log);
        c.that(accepted > before, "the accepted custom event reached the app");
        final List<String> allLines = Files.readAllLines(log, java.nio.charset.StandardCharsets.UTF_8);
        c.that(allLines.get(allLines.size() - 1).startsWith("surface=5 node=0 "),
                "non-canonical 0x01000000 handles canonicalize (surface->default 5, node->0): " + allLines.get(allLines.size() - 1));
        ring.reset();
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), out), 0, "empty ring is valid");
        c.eq(out[0], 0, "empty ring changes nothing");
        c.eq(NativeRuntime.dispatchEvents(h, ring.buffer(), ring.length(), null), 0, "null out array is allowed");
        NativeRuntime.destroy(h);

        abiV6EndToEnd(c, fx, Path.of(args[1]), Path.of(args[3]));
        closeDeadlockRegression(c, Path.of(args[1]));
        c.finish("android_events_jni_test");
    }

    static void drainQueue(ConcurrentLinkedQueue<Runnable> queue) {
        Runnable r;
        while ((r = queue.poll()) != null) r.run();
    }

    static void abiV6EndToEnd(Expect c, Fixtures fx, Path showcaseApp, Path layoutOnlyApp) throws Exception {
        final Path tmp = Files.createTempDirectory("nve_android_jni_v6");
        try {
            final List<String> seenCustom = new ArrayList<>();
            final List<byte[]> seenPayloads = new ArrayList<>();
            final AtomicInteger intrinsicInvalidations = new AtomicInteger();
            // Queued owner-thread executor (NOT `Runnable::run`!): background wakes never execute
            // `render()` on worker threads and never mask owner-thread guard tests.
            final ConcurrentLinkedQueue<Runnable> ownerQueue = new ConcurrentLinkedQueue<>();

            final RuntimeSurface closedSurface;
            try (RuntimeSurface sf = new RuntimeSurface(7, 360f, 800f, 2f, ownerQueue::add)) {
                closedSurface = sf;
                sf.setFrameConsumer((s, surface) -> { /* consumed */ });
                sf.setEmittedEventConsumer(nvev -> {
                    final EventRingReader r = new EventRingReader().attach(nvev);
                    for (int i = 0; i < r.count(); i++) {
                        seenCustom.add(r.customName(i));
                        seenPayloads.add(r.customData(i));
                    }
                });
                c.eq(sf.setIntrinsicSizeInvalidatedListener(intrinsicInvalidations::incrementAndGet),
                        NveRuntimeAbi.STATUS_OK, "install intrinsic size listener");
                c.eq(sf.attachApp(showcaseApp.toString(), false), NveRuntimeAbi.STATUS_OK, "attach v6 showcase app");
                c.that(sf.render(), "initial v6 render");
                c.that(intrinsicInvalidations.get() >= 1, "initial layout invalidated intrinsic size");

                // Replacing a text measurer triggers only ONE invalidation (no clear + set double relayout).
                final int invBeforeMeasurer = intrinsicInvalidations.get();
                c.eq(sf.setTextMeasurer((text, sz, weight, maxW, outWh) -> {
                    outWh[0] = 40f;
                    outWh[1] = 16f;
                }), NveRuntimeAbi.STATUS_OK, "install measurer 1");
                c.eq(intrinsicInvalidations.get(), invBeforeMeasurer + 1, "first measurer fires 1 invalidation");
                c.eq(sf.setTextMeasurer((text, sz, weight, maxW, outWh) -> {
                    outWh[0] = 50f;
                    outWh[1] = 18f;
                }), NveRuntimeAbi.STATUS_OK, "replace measurer 2");
                c.eq(intrinsicInvalidations.get(), invBeforeMeasurer + 2, "replacing measurer fires 1 invalidation (not 2)");
                c.eq(sf.setTextMeasurer(null), NveRuntimeAbi.STATUS_OK, "clear measurer");
                drainQueue(ownerQueue);

                // 1. setProps + drainEmittedEvents round-trip
                final BinWriter pw = new BinWriter();
                final long pm = pw.beginMap(4);
                pw.writeMapKey("initialCount");
                pw.writeI64(42L);
                pw.writeMapKey("label");
                pw.writeStr("caf\u00e9");
                pw.writeMapKey("enabled");
                pw.writeBool(true);
                pw.writeMapKey("tags");
                final long am = pw.beginArray(2);
                pw.writeStr("a");
                pw.writeStr("b");
                pw.end(am);
                pw.end(pm);
                final boolean[] propsChanged = new boolean[1];
                c.eq(sf.setProps(pw.toByteArray(), propsChanged), NveRuntimeAbi.STATUS_OK, "setProps valid binary map");
                c.that(propsChanged[0], "setProps outChanged reported true");
                c.eq(seenCustom.size(), 1, "setProps drained onPropsEcho");
                if (seenCustom.size() == 1) {
                    c.eq(seenCustom.get(0), "onPropsEcho", "echo custom event name");
                    final BinReader br = new BinReader(seenPayloads.get(0));
                    c.eq(br.peekTag(), BinTag.MAP, "echo payload is map");
                    final BinReader.ReadMark rm = br.beginMap();
                    c.eq(rm.count, 4, "echo map count");
                    c.eq(br.readMapKey(), "initialCount", "k0");
                    c.eq(br.readI64(), 42L, "v0");
                    c.eq(br.readMapKey(), "label", "k1");
                    c.eq(br.readStr(), "caf\u00e9", "v1");
                    c.eq(br.readMapKey(), "enabled", "k2");
                    c.that(br.readBool(), "v2");
                    c.eq(br.readMapKey(), "tags", "k3");
                    final BinReader.ReadMark arm = br.beginArray();
                    c.eq(arm.count, 2, "array count");
                    c.eq(br.readStr(), "a", "a0");
                    c.eq(br.readStr(), "b", "a1");
                    br.end(arm);
                    br.end(rm);
                    br.expectEnd();
                }

                // Oversized spike > 64 KiB works via one-shot direct buffer and subsequent small calls still work.
                final byte[] bigStr = new byte[70 * 1024];
                Arrays.fill(bigStr, (byte) 'z');
                final BinWriter bigProps = new BinWriter();
                final long bpm = bigProps.beginMap(1);
                bigProps.writeMapKey("blob");
                bigProps.writeStr(new String(bigStr, StandardCharsets.US_ASCII));
                bigProps.end(bpm);
                c.eq(sf.setProps(bigProps.toByteArray()), NveRuntimeAbi.STATUS_OK, "oversized >64KiB setProps succeeds");

                // Malformed & hostile BinCodec payloads rejected cleanly (truncated, invalid UTF-8, depth > 64, trailing).
                final BinWriter nonMap = new BinWriter();
                nonMap.writeI64(5L);
                c.eq(sf.setProps(nonMap.toByteArray()), NveRuntimeAbi.STATUS_INVALID_ARG, "non-map rejected");
                c.eq(NativeRuntime.setProps(sf.handle(), ByteBuffer.allocate(8), 8, null),
                        NveRuntimeAbi.STATUS_INVALID_ARG, "heap buffer setProps rejected");
                // Truncated map claiming 10 payload bytes
                c.eq(sf.setProps(new byte[] {7, 10, 0, 0, 0, 1, 0, 0, 0}),
                        NveRuntimeAbi.STATUS_INVALID_ARG, "truncated BinCodec map rejected");
                c.that(!sf.lastError().isEmpty(), "truncated BinCodec sets lastError");
                // Invalid UTF-8 in map key (tag=7, payload=10, count=1, klen=2, 0xC3 0x28, val=Null)
                c.eq(sf.setProps(new byte[] {7, 11, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, (byte) 0xC3, 0x28, 0}),
                        NveRuntimeAbi.STATUS_INVALID_ARG, "invalid UTF-8 BinCodec map key rejected");
                // Depth > 64 (map -> 65 nested arrays -> null = depth 66)
                final BinWriter deep = new BinWriter();
                final long[] marks = new long[66];
                marks[0] = deep.beginMap(1);
                deep.writeMapKey("d");
                for (int d = 1; d < 66; d++) marks[d] = deep.beginArray(1);
                deep.writeNull();
                for (int d = 65; d >= 0; d--) deep.end(marks[d]);
                c.eq(sf.setProps(deep.toByteArray()), NveRuntimeAbi.STATUS_INVALID_ARG, "depth > 64 BinCodec rejected");
                // Trailing bytes after valid empty map
                final BinWriter emptyMap = new BinWriter();
                final long em = emptyMap.beginMap(0);
                emptyMap.end(em);
                final byte[] validEmptyMap = emptyMap.toByteArray();
                final byte[] withTrailing = Arrays.copyOf(validEmptyMap, validEmptyMap.length + 1);
                withTrailing[validEmptyMap.length] = 0;
                c.eq(sf.setProps(withTrailing), NveRuntimeAbi.STATUS_INVALID_ARG, "trailing byte BinCodec rejected");

                // 0-capacity DirectByteBuffer accepted across direct_buffer_span entry points!
                final ByteBuffer zeroCap = ByteBuffer.allocateDirect(0);
                c.eq(NativeRuntime.setProps(sf.handle(), zeroCap, 0, null), NveRuntimeAbi.STATUS_OK, "0-cap direct setProps");
                c.eq(NativeRuntime.restoreState(sf.handle(), zeroCap, 0), NveRuntimeAbi.STATUS_OK, "0-cap direct restoreState");
                c.eq(NativeRuntime.completeHostRequest(sf.handle(), 1L, 0, zeroCap, 0),
                        NveRuntimeAbi.STATUS_OK, "0-cap direct completeHostRequest");
                c.eq(NativeRuntime.pushHostEvent(sf.handle(), 1, 1, zeroCap, 0),
                        NveRuntimeAbi.STATUS_OK, "0-cap direct pushHostEvent");
                c.eq(NativeRuntime.publishHostChannel(sf.handle(), 1, 0, zeroCap, 0),
                        NveRuntimeAbi.STATUS_OK, "0-cap direct publishHostChannel");
                c.eq(NativeRuntime.setProps(sf.handle(), null, 0, null), NveRuntimeAbi.STATUS_OK, "empty props ok");
                // Drain the two onPropsEcho events queued by the direct NativeRuntime.setProps probes above.
                final byte[] manualDrained = sf.drainEmittedEvents();
                c.that(manualDrained != null && manualDrained.length > 0, "manual drainEmittedEvents returns pending batch");
                if (manualDrained != null) {
                    final EventRingReader mr = new EventRingReader().attach(manualDrained);
                    c.eq(mr.count(), 2, "manual drainEmittedEvents returned both direct setProps echoes");
                    c.eq(mr.customName(0), "onPropsEcho", "manual drain[0]");
                    c.eq(mr.customName(1), "onPropsEcho", "manual drain[1]");
                }
                c.that(sf.drainEmittedEvents() == null, "second manual drainEmittedEvents is empty");
                // Flush the 0-cap host ingress batch via tick/render so app->emitted is clean before applyHotPatch.
                drainQueue(ownerQueue);
                sf.render();
                c.that(seenCustom.contains("onHostEvents"), "0-cap host ingress batch reached app");

                // 2. snapshotState / restoreState + StateFile persistence + null path lastError checks
                c.eq(sf.restoreState("init".getBytes(StandardCharsets.UTF_8)), NveRuntimeAbi.STATUS_OK, "reset state to init");
                c.eq(new String(sf.snapshotState(), StandardCharsets.UTF_8), "init", "initial snapshot");
                c.eq(sf.restoreState("restored-42".getBytes(StandardCharsets.UTF_8)), NveRuntimeAbi.STATUS_OK, "restoreState");
                c.eq(new String(sf.snapshotState(), StandardCharsets.UTF_8), "restored-42", "snapshot after restore");
                final Path stateFile = tmp.resolve("app.nve");
                c.eq(sf.saveStateFile(stateFile.toString(), 3), NveRuntimeAbi.STATUS_OK, "saveStateFile");
                c.eq(sf.restoreState("overwritten".getBytes(StandardCharsets.UTF_8)), NveRuntimeAbi.STATUS_OK, "overwrite state");
                c.eq(sf.restoreStateFile(stateFile.toString(), 3), NveRuntimeAbi.STATUS_OK, "restoreStateFile");
                c.eq(new String(sf.snapshotState(), StandardCharsets.UTF_8), "restored-42", "state file round-trip");
                c.eq(sf.clearStateFile(stateFile.toString()), NveRuntimeAbi.STATUS_OK, "clearStateFile");
                c.eq(sf.restoreStateFile(stateFile.toString(), 3), NveRuntimeAbi.STATUS_NOT_FOUND, "cleared state file absent");

                // Null paths must return INVALID_ARG AND update lastError()
                c.eq(NativeRuntime.tick(sf.handle(), 0f, null), NveRuntimeAbi.STATUS_OK, "clear lastError before null saveStateFile");
                c.eq(sf.saveStateFile(null, 1), NveRuntimeAbi.STATUS_INVALID_ARG, "null saveStateFile path");
                c.that(sf.lastError().contains("saveStateFile"), "null saveStateFile updates lastError");
                c.eq(NativeRuntime.tick(sf.handle(), 0f, null), NveRuntimeAbi.STATUS_OK, "clear lastError before null restoreStateFile");
                c.eq(sf.restoreStateFile(null, 1), NveRuntimeAbi.STATUS_INVALID_ARG, "null restoreStateFile path");
                c.that(sf.lastError().contains("restoreStateFile"), "null restoreStateFile updates lastError");
                c.eq(NativeRuntime.tick(sf.handle(), 0f, null), NveRuntimeAbi.STATUS_OK, "clear lastError before null clearStateFile");
                c.eq(sf.clearStateFile(null), NveRuntimeAbi.STATUS_INVALID_ARG, "null clearStateFile path");
                c.that(sf.lastError().contains("clearStateFile"), "null clearStateFile updates lastError");
                c.eq(NativeRuntime.tick(sf.handle(), 0f, null), NveRuntimeAbi.STATUS_OK, "clear lastError before null saveFrame0File");
                c.eq(sf.saveFrame0File(null, 0L, 0L), NveRuntimeAbi.STATUS_INVALID_ARG, "null saveFrame0File path");
                c.that(sf.lastError().contains("saveFrame0File"), "null saveFrame0File updates lastError");
                c.eq(NativeRuntime.tick(sf.handle(), 0f, null), NveRuntimeAbi.STATUS_OK, "clear lastError before null bootFrame0File");
                c.eq(sf.bootFrame0File(null, 0L, 0L), NveRuntimeAbi.STATUS_INVALID_ARG, "null bootFrame0File path");
                c.that(sf.lastError().contains("bootFrame0File"), "null bootFrame0File updates lastError");

                // 3. applyHotPatch (valid hot-patch DevStatus frame + malformed rejection)
                final BinWriter hotPatch = new BinWriter();
                hotPatch.raw(new byte[] {'N', 'V', 'P', '1'}, 0, 4);
                hotPatch.u8(3); // DevStatus variant
                hotPatch.u32(2);
                hotPatch.raw("ok".getBytes(StandardCharsets.US_ASCII), 0, 2);
                hotPatch.u64(12L);
                final int beforePatch = seenCustom.size();
                final boolean[] patchChanged = new boolean[1];
                c.eq(sf.applyHotPatch(hotPatch.toByteArray(), patchChanged), NveRuntimeAbi.STATUS_OK, "applyHotPatch valid frame");
                c.that(patchChanged[0], "applyHotPatch outChanged reported true");
                c.that(seenCustom.size() == beforePatch + 1 && "onHotPatch".equals(seenCustom.get(beforePatch)),
                        "hot patch reached app");
                c.eq(sf.applyHotPatch(new byte[] {1, 2, 3}), NveRuntimeAbi.STATUS_INVALID_ARG, "bad hot patch rejected");

                // 4. measureSurface: error paths + finite baselines (showcase via Taffy) vs NaN baselines (NVE_IFACE_LAYOUT)
                c.eq(NativeRuntime.tick(sf.handle(), 0f, null), NveRuntimeAbi.STATUS_OK, "clear lastError before null measure");
                c.eq(sf.measureSurface(0f, 0f, 200f, 400f, 0, null), NveRuntimeAbi.STATUS_INVALID_ARG, "null outMeasure rejected");
                c.that(sf.lastError().contains("measureSurface"), "null outMeasure sets lastError");
                c.eq(NativeRuntime.tick(sf.handle(), 0f, null), NveRuntimeAbi.STATUS_OK, "clear lastError before short measure");
                c.eq(sf.measureSurface(0f, 0f, 200f, 400f, 0, new float[3]), NveRuntimeAbi.STATUS_INVALID_ARG, "short outMeasure rejected");
                c.that(sf.lastError().contains("measureSurface"), "short outMeasure sets lastError");

                final float[] rawMeasure = new float[] {-1f, -1f, 999f, 999f};
                c.eq(sf.measureSurface(0f, 0f, 200f, 400f, NveRuntimeAbi.MEASURE_FLAG_DRY_RUN, rawMeasure),
                        NveRuntimeAbi.STATUS_OK, "measureSurface 4-float overwrite");
                c.that(Float.isFinite(rawMeasure[0]) && rawMeasure[0] > 0f && rawMeasure[0] <= 200f
                        && Float.isFinite(rawMeasure[1]) && rawMeasure[1] > 0f && rawMeasure[1] <= 400f,
                        "measureSurface respects [min,max] bounds");
                c.that(Float.isNaN(rawMeasure[2]) && Float.isNaN(rawMeasure[3]),
                        "measureSurface overwrites baseline slots (ComputeSize -> NaN)");
                final RuntimeSurface.SurfaceMeasure dry = sf.measureSurface(
                        0f, 0f, 200f, 400f, NveRuntimeAbi.MEASURE_FLAG_DRY_RUN);
                c.that(dry != null && dry.width == rawMeasure[0] && dry.height == rawMeasure[1]
                        && !dry.hasFirstBaseline() && !dry.hasLastBaseline(),
                        "SurfaceMeasure wrapper mirrors JNI measureSurface output");
                final RuntimeSurface.SurfaceMeasure finiteBase = new RuntimeSurface.SurfaceMeasure(100f, 40f, 12f, 34f);
                c.that(finiteBase.hasFirstBaseline() && finiteBase.hasLastBaseline(),
                        "SurfaceMeasure finite baselines report true");
                final RuntimeSurface.SurfaceMeasure ideal = sf.measureSurface(
                        0f, 0f, Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY,
                        NveRuntimeAbi.MEASURE_FLAG_DRY_RUN | NveRuntimeAbi.MEASURE_FLAG_IDEAL);
                c.that(ideal != null && Float.isFinite(ideal.width) && Float.isFinite(ideal.height),
                        "measureSurface ideal finite");

                // Separate surface with NVE_IFACE_LAYOUT + NO NVE_IFACE_COMPONENT:
                // proves NaN baselines from NVE_IFACE_LAYOUT AND lastError() staying empty after render()!
                try (RuntimeSurface layoutSf = new RuntimeSurface(19, 360f, 800f, 1f, ownerQueue::add)) {
                    layoutSf.setFrameConsumer((s, surface) -> { /* consumed */ });
                    layoutSf.setEmittedEventConsumer(nvev -> { /* listener active */ });
                    c.eq(layoutSf.attachApp(layoutOnlyApp.toString(), false), NveRuntimeAbi.STATUS_OK, "attach layout-only app");
                    c.that(layoutSf.render(), "render layout-only app (1st)");
                    c.eq(layoutSf.lastError(), "", "lastError stays empty after render without NVE_IFACE_COMPONENT (1st)");
                    c.eq(layoutSf.setViewport(300f, 600f, 1f), NveRuntimeAbi.STATUS_OK, "resize layout-only app");
                    c.that(layoutSf.render(), "render layout-only app (2nd, cached unsupported)");
                    c.eq(layoutSf.lastError(), "", "lastError stays empty after render without NVE_IFACE_COMPONENT (2nd)");
                    final RuntimeSurface.SurfaceMeasure custom = layoutSf.measureSurface(
                            0f, 0f, 500f, 500f, NveRuntimeAbi.MEASURE_FLAG_DRY_RUN);
                    c.that(custom != null && custom.width == 123.5f && custom.height == 45.25f,
                            "NVE_IFACE_LAYOUT measureSurface returns app dimensions");
                    c.that(custom != null && !custom.hasFirstBaseline() && !custom.hasLastBaseline()
                            && Float.isNaN(custom.firstBaseline) && Float.isNaN(custom.lastBaseline),
                            "NVE_IFACE_LAYOUT measureSurface reports NaN baselines");
                }

                // 5. Frame-0 save / boot file & boot bytes
                final Path f0Path = tmp.resolve("frame0.nve2");
                c.eq(sf.saveFrame0File(f0Path.toString(), 0x1234L, 0x5678L), NveRuntimeAbi.STATUS_OK, "saveFrame0File");
                final byte[] f0Bytes = Files.readAllBytes(f0Path);
                c.that(f0Bytes.length > 0, "frame0.nve2 non-empty");
                try (RuntimeSurface booter = new RuntimeSurface(8, 360f, 800f, 2f, ownerQueue::add)) {
                    final String[] bootedTitle = new String[1];
                    booter.setFrameConsumer((s, surface) -> {
                        final TableReader text = s.findTable(NveTables.Text.KEY);
                        final int r = text == null ? -1 : s.rowOf(text, s.resolve(fx.id("title")));
                        bootedTitle[0] = r < 0 ? null : s.text(text, r, NveTables.Text.TEXT);
                    });
                    booter.setEmittedEventConsumer(nvev -> { /* listener active on Frame-0 surface */ });
                    c.eq(booter.bootFrame0File(f0Path.toString(), 0x1234L, 0x5678L),
                            NveRuntimeAbi.STATUS_OK, "bootFrame0File matching metadata");
                    c.that(booter.render(), "render booted Frame-0 file");
                    c.eq(booter.lastError(), "", "Frame-0 render leaves lastError empty");
                    c.that(bootedTitle[0] != null && !bootedTitle[0].isEmpty(), "Frame-0 file scene live");
                    c.eq(booter.bootFrame0Bytes(f0Bytes, 0x9999L, 0x5678L),
                            NveRuntimeAbi.STATUS_NOT_FOUND, "bootFrame0Bytes rejects mismatched buildId");
                    c.eq(booter.bootFrame0Bytes(f0Bytes, 0x1234L, 0x5678L),
                            NveRuntimeAbi.STATUS_OK, "bootFrame0Bytes matching metadata");
                    c.that(booter.render(), "render booted Frame-0 bytes");
                }

                // 6. NVE_IFACE_HOST = 5 thin bridge (sync OK, async PENDING + cross-thread complete, push, channel, + error paths)
                final AtomicLong pendingReqId = new AtomicLong();
                final AtomicInteger publishedChannel = new AtomicInteger();
                final AtomicInteger javaInvocations = new AtomicInteger();
                final byte[][] publishedPayload = new byte[1][];
                c.eq(sf.setHostCallbackBridge(new NativeRuntime.HostCallbackBridge() {
                    @Override
                    public int invoke(int serviceId, int methodId, long requestId, byte[] request, byte[][] outPayload) {
                        javaInvocations.incrementAndGet();
                        if (serviceId == 0x0010 && methodId == 1) {
                            outPayload[0] = "pong".getBytes(StandardCharsets.UTF_8);
                            return HybridRuntime.HostStatus.OK.code;
                        }
                        if (serviceId == 0x0010 && methodId == 2) {
                            pendingReqId.set(requestId);
                            return HybridRuntime.HostStatus.PENDING.code;
                        }
                        if (serviceId == 0x0010 && methodId == 3) {
                            outPayload[0] = "0123456789ABCDEF".getBytes(StandardCharsets.US_ASCII); // 16 bytes > 4-byte out_buf
                            return HybridRuntime.HostStatus.OK.code;
                        }
                        if (serviceId == 0x0010 && methodId == 4) {
                            outPayload[0] = null; // empty OK reply
                            return HybridRuntime.HostStatus.OK.code;
                        }
                        if (serviceId == 0x0010 && methodId == 5) {
                            throw new IllegalStateException("simulated host invoke failure");
                        }
                        return HybridRuntime.HostStatus.UNSUPPORTED.code;
                    }

                    @Override
                    public int publishChannel(int channelId, int flags, byte[] payload) {
                        javaInvocations.incrementAndGet();
                        if (channelId == 99) {
                            throw new IllegalStateException("simulated host publish failure");
                        }
                        publishedChannel.set(channelId);
                        publishedPayload[0] = payload;
                        return HybridRuntime.HostStatus.OK.code;
                    }
                }), NveRuntimeAbi.STATUS_OK, "install HostCallbackBridge");

                seenCustom.clear();
                seenPayloads.clear();
                c.eq(sf.send(r -> r.custom(0, 200L, "host_call_sync", "ping".getBytes(StandardCharsets.UTF_8))),
                        NveRuntimeAbi.STATUS_OK, "trigger sync host invoke");
                c.that(seenCustom.contains("onHostSyncReply"), "sync host reply emitted");
                final int syncIdx = seenCustom.indexOf("onHostSyncReply");
                if (syncIdx >= 0) {
                    final BinReader sr = new BinReader(seenPayloads.get(syncIdx));
                    final BinReader.ReadMark sm = sr.beginMap();
                    c.eq(sr.readMapKey(), "status", "sync status key");
                    c.eq(sr.readI64(), 0L, "sync status OK");
                    c.eq(sr.readMapKey(), "reply", "sync reply key");
                    c.eq(new String(sr.readBytes(), StandardCharsets.UTF_8), "pong", "sync reply payload");
                    sr.end(sm);
                }

                // HostCallbackBridge error & edge cases: BUFFER_TOO_SMALL, null outPayload[0], Java exceptions, null ptr+len>0
                seenCustom.clear();
                seenPayloads.clear();
                c.eq(sf.send(r -> r.custom(0, 205L, "host_call_small_buf", new byte[] {1})),
                        NveRuntimeAbi.STATUS_OK, "trigger BUFFER_TOO_SMALL host invoke");
                final int smallIdx = seenCustom.indexOf("onHostSmallBuf");
                c.that(smallIdx >= 0, "onHostSmallBuf emitted");
                if (smallIdx >= 0) {
                    final BinReader sr = new BinReader(seenPayloads.get(smallIdx));
                    final BinReader.ReadMark sm = sr.beginMap();
                    c.eq(sr.readMapKey(), "status", "small status key");
                    c.eq(sr.readI64(), HybridRuntime.HostStatus.BUFFER_TOO_SMALL.code, "BUFFER_TOO_SMALL status");
                    c.eq(sr.readMapKey(), "outLen", "small outLen key");
                    c.eq(sr.readI64(), 16L, "BUFFER_TOO_SMALL reports required 16 bytes");
                    sr.end(sm);
                }

                seenCustom.clear();
                seenPayloads.clear();
                c.eq(sf.send(r -> r.custom(0, 206L, "host_call_null_reply", new byte[] {1})),
                        NveRuntimeAbi.STATUS_OK, "trigger null outPayload[0] host invoke");
                final int nullRepIdx = seenCustom.indexOf("onHostNullReply");
                c.that(nullRepIdx >= 0, "onHostNullReply emitted");
                if (nullRepIdx >= 0) {
                    final BinReader sr = new BinReader(seenPayloads.get(nullRepIdx));
                    final BinReader.ReadMark sm = sr.beginMap();
                    c.eq(sr.readMapKey(), "status", "null reply status key");
                    c.eq(sr.readI64(), HybridRuntime.HostStatus.OK.code, "null outPayload[0] returns OK");
                    c.eq(sr.readMapKey(), "outLen", "null reply outLen key");
                    c.eq(sr.readI64(), 0L, "null outPayload[0] reports outLen 0");
                    sr.end(sm);
                }

                seenCustom.clear();
                seenPayloads.clear();
                c.eq(sf.send(r -> r.custom(0, 207L, "host_call_throw", new byte[] {1})),
                        NveRuntimeAbi.STATUS_OK, "trigger Java exception in invoke");
                final int throwIdx = seenCustom.indexOf("onHostThrow");
                c.that(throwIdx >= 0, "onHostThrow emitted");
                if (throwIdx >= 0) {
                    final BinReader sr = new BinReader(seenPayloads.get(throwIdx));
                    final BinReader.ReadMark sm = sr.beginMap();
                    c.eq(sr.readMapKey(), "status", "throw status key");
                    c.eq(sr.readI64(), HybridRuntime.HostStatus.INTERNAL_ERROR.code, "Java exception maps to INTERNAL_ERROR");
                    sr.end(sm);
                }

                seenCustom.clear();
                seenPayloads.clear();
                c.eq(sf.send(r -> r.custom(0, 208L, "host_publish_throw", new byte[] {1})),
                        NveRuntimeAbi.STATUS_OK, "trigger Java exception in publishChannel");
                final int pubThrowIdx = seenCustom.indexOf("onHostPublishThrow");
                c.that(pubThrowIdx >= 0, "onHostPublishThrow emitted");
                if (pubThrowIdx >= 0) {
                    final BinReader sr = new BinReader(seenPayloads.get(pubThrowIdx));
                    final BinReader.ReadMark sm = sr.beginMap();
                    c.eq(sr.readMapKey(), "status", "publish throw status key");
                    c.eq(sr.readI64(), HybridRuntime.HostStatus.INTERNAL_ERROR.code, "publishChannel exception maps to INTERNAL_ERROR");
                    sr.end(sm);
                }

                // Invalid slice (ptr == null, len > 0) rejected with NVE_HOST_INVALID_ARG without calling Java!
                seenCustom.clear();
                seenPayloads.clear();
                final int invokesBeforeNullPtr = javaInvocations.get();
                c.eq(sf.send(r -> {
                    r.custom(0, 209L, "host_call_null_ptr", null);
                    r.custom(0, 210L, "host_publish_null_ptr", null);
                }), NveRuntimeAbi.STATUS_OK, "trigger null-ptr slice host calls");
                c.eq(javaInvocations.get(), invokesBeforeNullPtr, "null-ptr+len>0 never invokes Java");
                final int nullPtrIdx = seenCustom.indexOf("onHostNullPtr");
                if (nullPtrIdx >= 0) {
                    final BinReader sr = new BinReader(seenPayloads.get(nullPtrIdx));
                    final BinReader.ReadMark sm = sr.beginMap();
                    c.eq(sr.readMapKey(), "status", "null-ptr invoke status key");
                    c.eq(sr.readI64(), HybridRuntime.HostStatus.INVALID_ARG.code, "null-ptr invoke returns INVALID_ARG");
                    sr.end(sm);
                } else {
                    c.that(false, "onHostNullPtr emitted");
                }
                final int pubNullPtrIdx = seenCustom.indexOf("onHostPublishNullPtr");
                if (pubNullPtrIdx >= 0) {
                    final BinReader sr = new BinReader(seenPayloads.get(pubNullPtrIdx));
                    final BinReader.ReadMark sm = sr.beginMap();
                    c.eq(sr.readMapKey(), "status", "null-ptr publish status key");
                    c.eq(sr.readI64(), HybridRuntime.HostStatus.INVALID_ARG.code, "null-ptr publish returns INVALID_ARG");
                    sr.end(sm);
                } else {
                    c.that(false, "onHostPublishNullPtr emitted");
                }

                c.eq(sf.send(r -> r.custom(0, 201L, "host_publish", "msg".getBytes(StandardCharsets.UTF_8))),
                        NveRuntimeAbi.STATUS_OK, "trigger outbound channel publish");
                c.eq(publishedChannel.get(), 42, "outbound channel id");
                c.eq(new String(publishedPayload[0], StandardCharsets.UTF_8), "msg", "outbound channel payload");

                seenCustom.clear();
                seenPayloads.clear();
                c.eq(sf.send(r -> r.custom(0, 202L, "host_call_async", new byte[] {1})),
                        NveRuntimeAbi.STATUS_OK, "trigger async PENDING host invoke");
                c.that(pendingReqId.get() != 0L, "PENDING captured host requestId");

                // Complete from a background thread (small stack-buffer + large >512B heap-fallback + null byte[])
                final byte[] largePayload = new byte[1024];
                Arrays.fill(largePayload, (byte) 0x5A);
                final int[] workerStatuses = new int[5];
                final Thread worker = new Thread(() -> {
                    workerStatuses[0] = sf.completeHostRequest(
                            pendingReqId.get(), HybridRuntime.HostStatus.OK.code, "async-ok".getBytes(StandardCharsets.UTF_8));
                    workerStatuses[1] = sf.pushHostEvent(0x0014, 7, new byte[] {9, 9});
                    workerStatuses[2] = sf.publishHostChannel(42, 1, largePayload);
                    workerStatuses[3] = sf.pushHostEvent(0x0014, 8, (byte[]) null);
                    workerStatuses[4] = sf.publishHostChannel(43, 0, new byte[0]);
                });
                worker.start();
                worker.join();
                for (int i = 0; i < workerStatuses.length; i++) {
                    c.eq(workerStatuses[i], NveRuntimeAbi.STATUS_OK, "cross-thread byte[] host ingress #" + i);
                }

                seenCustom.clear();
                seenPayloads.clear();
                drainQueue(ownerQueue);
                sf.render();
                c.that(seenCustom.contains("onHostEvents"), "tick drained NVHB host event batch into app");

                // Honest off-thread owner-thread guards: verify lastError is EMPTY before EACH individual
                // off-thread probe so no earlier refusal can mask a missing guard!
                final long h = sf.handle();
                final Runnable[] probes = new Runnable[] {
                    () -> c.eq(NativeRuntime.setProps(h, null, 0, null), NveRuntimeAbi.STATUS_INVALID_ARG, "off-thread setProps"),
                    () -> c.eq(NativeRuntime.measureSurface(h, 0f, 0f, 100f, 100f, 0, new float[4]),
                            NveRuntimeAbi.STATUS_INVALID_ARG, "off-thread measureSurface"),
                    () -> c.eq(NativeRuntime.applyHotPatch(h, null, 0, null),
                            NveRuntimeAbi.STATUS_INVALID_ARG, "off-thread applyHotPatch"),
                    () -> c.eq(NativeRuntime.setHostCallbackBridge(h, null),
                            NveRuntimeAbi.STATUS_INVALID_ARG, "off-thread setHostCallbackBridge"),
                    () -> c.eq(NativeRuntime.setIntrinsicSizeInvalidatedListener(h, null),
                            NveRuntimeAbi.STATUS_INVALID_ARG, "off-thread setIntrinsicSizeInvalidatedListener"),
                };
                for (int p = 0; p < probes.length; p++) {
                    c.eq(NativeRuntime.tick(h, 0f, null), NveRuntimeAbi.STATUS_OK, "clear error before off-thread probe " + p);
                    c.eq(sf.lastError(), "", "lastError clean before off-thread probe " + p);
                    final int idx = p;
                    final Thread badThread = new Thread(() -> probes[idx].run());
                    badThread.start();
                    badThread.join();
                    c.that(sf.lastError().contains("owner thread"), "owner sees off-thread refusal for probe " + p);
                }

                c.eq(sf.setHostCallbackBridge(null), NveRuntimeAbi.STATUS_OK, "clear HostCallbackBridge");
                c.eq(sf.setIntrinsicSizeInvalidatedListener(null), NveRuntimeAbi.STATUS_OK, "clear intrinsic listener");
            }

            // Null-clearing teardown calls on a closed RuntimeSurface must be idempotent no-ops, while non-null throws.
            c.eq(closedSurface.setTextMeasurer(null), NveRuntimeAbi.STATUS_OK, "closed setTextMeasurer(null) is no-op");
            c.eq(closedSurface.setHostCallbackBridge(null), NveRuntimeAbi.STATUS_OK, "closed setHostCallbackBridge(null) is no-op");
            c.eq(closedSurface.setIntrinsicSizeInvalidatedListener(null), NveRuntimeAbi.STATUS_OK,
                    "closed setIntrinsicSizeInvalidatedListener(null) is no-op");
            c.throwsAny(() -> closedSurface.setTextMeasurer((t, s, w, m, o) -> {}), "closed non-null setTextMeasurer throws");
            c.throwsAny(() -> closedSurface.setIntrinsicSizeInvalidatedListener(() -> {}),
                    "closed non-null setIntrinsicSizeInvalidatedListener throws");
        } finally {
            Files.walk(tmp).sorted((a, b) -> b.compareTo(a)).forEach(p -> p.toFile().delete());
        }
    }

    /**
     * Proves `RuntimeSurface.close()` has no AB-BA deadlock with in-flight cross-thread callbacks:
     * a worker thread triggers `WakeBridge` -> `onWake` (holding `WakeBridge`'s read lock) and
     * calls `isPaired()` / `completeHostRequest()` / `pushHostEvent()` / `publishHostChannel()`
     * (all acquiring `handleLock`) while the owner thread concurrently runs `close()`.
     */
    static void closeDeadlockRegression(Expect c, Path showcaseApp) throws Exception {
        final CountDownLatch inWakeCallback = new CountDownLatch(1);
        final AtomicBoolean callbackSawHandleLockCalls = new AtomicBoolean(false);
        final RuntimeSurface[] holder = new RuntimeSurface[1];

        final RuntimeSurface sf = new RuntimeSurface(21, 360f, 800f, 1f, task -> {
            if (holder[0] == null) return;
            if (inWakeCallback.getCount() == 0) return;
            inWakeCallback.countDown();
            try {
                // Give owner thread time to enter `close()` and block on draining `WakeBridge`.
                Thread.sleep(40);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            // If `close()` held `handleLock` while clearing callbacks, these would AB-BA deadlock!
            holder[0].isPaired();
            holder[0].completeHostRequest(1L, HybridRuntime.HostStatus.OK.code, new byte[] {1});
            holder[0].pushHostEvent(0x0010, 1, new byte[] {2});
            holder[0].publishHostChannel(1, 0, new byte[] {3});
            callbackSawHandleLockCalls.set(true);
        });
        holder[0] = sf;
        c.eq(sf.attachApp(showcaseApp.toString(), false), NveRuntimeAbi.STATUS_OK, "deadlock test attach");

        final Thread worker = new Thread(() -> {
            sf.pushHostEvent(0x0010, 99, new byte[] {7});
        });
        worker.start();

        c.that(inWakeCallback.await(5, TimeUnit.SECONDS), "worker entered wake callback");
        sf.close();
        worker.join(5000);
        c.that(!worker.isAlive(), "worker exited without AB-BA deadlock during close()");
        c.that(callbackSawHandleLockCalls.get(), "in-flight wake callback completed handleLock calls cleanly");
        c.eq(sf.handle(), 0L, "surface closed cleanly");
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
