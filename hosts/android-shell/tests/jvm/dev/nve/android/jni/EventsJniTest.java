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
import java.util.List;
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

        abiV6EndToEnd(c, fx, Path.of(args[1]));
        c.finish("android_events_jni_test");
    }

    static void abiV6EndToEnd(Expect c, Fixtures fx, Path showcaseApp) throws Exception {
        final Path tmp = Files.createTempDirectory("nve_android_jni_v6");
        try {
            final List<String> seenCustom = new ArrayList<>();
            final List<byte[]> seenPayloads = new ArrayList<>();
            final AtomicInteger intrinsicInvalidations = new AtomicInteger();

            try (RuntimeSurface sf = new RuntimeSurface(7, 360f, 800f, 2f, Runnable::run)) {
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
                c.eq(sf.setProps(pw.toByteArray()), NveRuntimeAbi.STATUS_OK, "setProps valid binary map");
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

                // Malformed props rejected cleanly
                final BinWriter nonMap = new BinWriter();
                nonMap.writeI64(5L);
                c.eq(sf.setProps(nonMap.toByteArray()), NveRuntimeAbi.STATUS_INVALID_ARG, "non-map rejected");
                c.eq(NativeRuntime.setProps(sf.handle(), ByteBuffer.allocate(8), 8, null),
                        NveRuntimeAbi.STATUS_INVALID_ARG, "heap buffer setProps rejected");
                c.eq(NativeRuntime.setProps(sf.handle(), null, 0, null), NveRuntimeAbi.STATUS_OK, "empty props ok");

                // 2. snapshotState / restoreState + StateFile persistence
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

                // 3. applyHotPatch (valid NVP1 DevStatus frame + malformed rejection)
                final BinWriter nvp = new BinWriter();
                nvp.raw("NVP1".getBytes(StandardCharsets.US_ASCII), 0, 4);
                nvp.u8(3); // DevStatus variant
                nvp.u32(2);
                nvp.raw("ok".getBytes(StandardCharsets.US_ASCII), 0, 2);
                nvp.u64(12L);
                final int beforePatch = seenCustom.size();
                c.eq(sf.applyHotPatch(nvp.toByteArray()), NveRuntimeAbi.STATUS_OK, "applyHotPatch valid NVP1");
                c.that(seenCustom.size() == beforePatch + 1 && "onHotPatch".equals(seenCustom.get(beforePatch)),
                        "hot patch reached app");
                c.eq(sf.applyHotPatch(new byte[] {1, 2, 3}), NveRuntimeAbi.STATUS_INVALID_ARG, "bad NVP1 rejected");

                // 4. measureSurface (DRY_RUN + IDEAL)
                final RuntimeSurface.SurfaceMeasure dry = sf.measureSurface(
                        0f, 0f, 200f, 400f, NveRuntimeAbi.MEASURE_FLAG_DRY_RUN);
                c.that(dry != null && Float.isFinite(dry.width) && Float.isFinite(dry.height)
                        && dry.width > 0f && dry.height > 0f, "measureSurface dry-run finite positive");
                final RuntimeSurface.SurfaceMeasure ideal = sf.measureSurface(
                        0f, 0f, Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY,
                        NveRuntimeAbi.MEASURE_FLAG_DRY_RUN | NveRuntimeAbi.MEASURE_FLAG_IDEAL);
                c.that(ideal != null && Float.isFinite(ideal.width) && Float.isFinite(ideal.height),
                        "measureSurface ideal finite");

                // 5. Frame-0 save / boot file & boot bytes
                final Path f0Path = tmp.resolve("frame0.nve2");
                c.eq(sf.saveFrame0File(f0Path.toString(), 0x1234L, 0x5678L), NveRuntimeAbi.STATUS_OK, "saveFrame0File");
                final byte[] f0Bytes = Files.readAllBytes(f0Path);
                c.that(f0Bytes.length > 0, "frame0.nve2 non-empty");
                try (RuntimeSurface booter = new RuntimeSurface(8, 360f, 800f, 2f, Runnable::run)) {
                    final String[] bootedTitle = new String[1];
                    booter.setFrameConsumer((s, surface) -> {
                        final TableReader text = s.findTable(NveTables.Text.KEY);
                        final int r = text == null ? -1 : s.rowOf(text, s.resolve(fx.id("title")));
                        bootedTitle[0] = r < 0 ? null : s.text(text, r, NveTables.Text.TEXT);
                    });
                    c.eq(booter.bootFrame0File(f0Path.toString(), 0x1234L, 0x5678L),
                            NveRuntimeAbi.STATUS_OK, "bootFrame0File matching metadata");
                    c.that(booter.render(), "render booted Frame-0 file");
                    c.that(bootedTitle[0] != null && !bootedTitle[0].isEmpty(), "Frame-0 file scene live");
                    c.eq(booter.bootFrame0Bytes(f0Bytes, 0x9999L, 0x5678L),
                            NveRuntimeAbi.STATUS_NOT_FOUND, "bootFrame0Bytes rejects mismatched buildId");
                    c.eq(booter.bootFrame0Bytes(f0Bytes, 0x1234L, 0x5678L),
                            NveRuntimeAbi.STATUS_OK, "bootFrame0Bytes matching metadata");
                    c.that(booter.render(), "render booted Frame-0 bytes");
                }

                // 6. NVE_IFACE_HOST = 5 thin bridge (sync OK, async PENDING + cross-thread complete, push, channel)
                final AtomicLong pendingReqId = new AtomicLong();
                final AtomicInteger publishedChannel = new AtomicInteger();
                final byte[][] publishedPayload = new byte[1][];
                c.eq(sf.setHostCallbackBridge(new NativeRuntime.HostCallbackBridge() {
                    @Override
                    public int invoke(int serviceId, int methodId, long requestId, byte[] request, byte[][] outPayload) {
                        if (serviceId == 0x0010 && methodId == 1) {
                            outPayload[0] = "pong".getBytes(StandardCharsets.UTF_8);
                            return HybridRuntime.HostStatus.OK.code;
                        }
                        if (serviceId == 0x0010 && methodId == 2) {
                            pendingReqId.set(requestId);
                            return HybridRuntime.HostStatus.PENDING.code;
                        }
                        return HybridRuntime.HostStatus.UNSUPPORTED.code;
                    }

                    @Override
                    public int publishChannel(int channelId, int flags, byte[] payload) {
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

                c.eq(sf.send(r -> r.custom(0, 201L, "host_publish", "msg".getBytes(StandardCharsets.UTF_8))),
                        NveRuntimeAbi.STATUS_OK, "trigger outbound channel publish");
                c.eq(publishedChannel.get(), 42, "outbound channel id");
                c.eq(new String(publishedPayload[0], StandardCharsets.UTF_8), "msg", "outbound channel payload");

                seenCustom.clear();
                seenPayloads.clear();
                c.eq(sf.send(r -> r.custom(0, 202L, "host_call_async", new byte[] {1})),
                        NveRuntimeAbi.STATUS_OK, "trigger async PENDING host invoke");
                c.that(pendingReqId.get() != 0L, "PENDING captured host requestId");

                // Complete from a background thread (cross-thread safe!) alongside push & channel ingress.
                final int[] workerStatuses = new int[3];
                final Thread worker = new Thread(() -> {
                    workerStatuses[0] = sf.completeHostRequest(
                            pendingReqId.get(), HybridRuntime.HostStatus.OK.code, "async-ok".getBytes(StandardCharsets.UTF_8));
                    workerStatuses[1] = sf.pushHostEvent(0x0014, 7, new byte[] {9, 9});
                    workerStatuses[2] = sf.publishHostChannel(42, 1, new byte[] {5});
                });
                worker.start();
                worker.join();
                c.eq(workerStatuses[0], NveRuntimeAbi.STATUS_OK, "cross-thread completeHostRequest");
                c.eq(workerStatuses[1], NveRuntimeAbi.STATUS_OK, "cross-thread pushHostEvent");
                c.eq(workerStatuses[2], NveRuntimeAbi.STATUS_OK, "cross-thread publishHostChannel");

                seenCustom.clear();
                seenPayloads.clear();
                sf.render();
                c.that(seenCustom.contains("onHostEvents"), "tick drained NVHB host event batch into app");

                // Off-thread owner-thread guards for ABI v6 slots
                final int[] off = new int[5];
                final Thread badThread = new Thread(() -> {
                    final long h = sf.handle();
                    off[0] = NativeRuntime.setProps(h, null, 0, null);
                    off[1] = NativeRuntime.measureSurface(h, 0f, 0f, 100f, 100f, 0, new float[4]);
                    off[2] = NativeRuntime.applyHotPatch(h, null, 0, null);
                    off[3] = NativeRuntime.setHostCallbackBridge(h, null);
                    off[4] = NativeRuntime.setIntrinsicSizeInvalidatedListener(h, null);
                });
                badThread.start();
                badThread.join();
                for (int st : off) c.eq(st, NveRuntimeAbi.STATUS_INVALID_ARG, "off-thread v6 owner call refused");
                c.that(sf.lastError().contains("owner thread"), "owner sees off-thread refusal");

                c.eq(sf.setHostCallbackBridge(null), NveRuntimeAbi.STATUS_OK, "clear HostCallbackBridge");
                c.eq(sf.setIntrinsicSizeInvalidatedListener(null), NveRuntimeAbi.STATUS_OK, "clear intrinsic listener");
            }
        } finally {
            Files.walk(tmp).sorted((a, b) -> b.compareTo(a)).forEach(p -> p.toFile().delete());
        }
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
