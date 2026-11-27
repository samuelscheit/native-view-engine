package dev.nve.android.jni;

import dev.nve.host.NativeRuntime;
import dev.nve.host.generated.NveRuntimeAbi;
import dev.nve.host.generated.NveTables;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.TableReader;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.Base64;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Embedded runtime pairing (`nve_runtime_pair_*`) through `libnve_android_shell`: a loopback
 * RFC 6455 server sends `HelloAck` + a `SceneKeyframe` carrying the showcase `NVE2` frame; the wake
 * listener fires on a runtime thread and the owner thread ticks the scene in.
 *
 * <p>Args: fixture dir.
 */
public final class PairJniTest {
    private PairJniTest() {}

    public static void main(String[] args) throws Exception {
        System.loadLibrary("nve_android_shell");
        final Expect c = new Expect();
        final Fixtures fx = new Fixtures(Path.of(args[0]));
        final long h = NativeRuntime.create(1, 360f, 800f, 1f);

        c.eq(NativeRuntime.pairConnect(h, "", "t"), NveRuntimeAbi.STATUS_INVALID_ARG, "empty uri");
        c.eq(NativeRuntime.pairConnect(h, "wss://127.0.0.1:443/nve-wire", "t"), NveRuntimeAbi.STATUS_INVALID_ARG, "TLS without pin");
        c.eq(NativeRuntime.pairConnect(h, "ws://127.0.0.1:0/nve-wire", "t"), NveRuntimeAbi.STATUS_INVALID_ARG, "port 0");
        c.that(!NativeRuntime.lastError(h).isEmpty(), "pair errors explain themselves");

        final byte[] keyframe = new byte[fx.frame("showcase_1.nve2").remaining()];
        fx.frame("showcase_1.nve2").get(keyframe);
        final CountDownLatch woke = new CountDownLatch(1);
        final AtomicInteger wakeThreads = new AtomicInteger();
        final Thread owner = Thread.currentThread();
        c.eq(NativeRuntime.setWakeListener(h, () -> {
            if (Thread.currentThread() != owner) wakeThreads.incrementAndGet();
            woke.countDown();
        }), 0, "wake listener");
        try (ServerSocket server = new ServerSocket(0, 1, InetAddress.getLoopbackAddress())) {
            final Thread peer = new Thread(() -> serve(server, keyframe), "nve-pair-peer");
            peer.setDaemon(true);
            peer.start();
            c.eq(NativeRuntime.pairConnect(h, "ws://127.0.0.1:" + server.getLocalPort() + "/nve-wire", "jvm-token"), 0, "connect");
            c.that(woke.await(5, TimeUnit.SECONDS), "wake listener fired");
            c.that(wakeThreads.get() > 0, "wake arrives on a runtime thread");
            String title = null;
            final long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
            final Fixtures.Scene scene = new Fixtures.Scene(h);
            final int[] changed = new int[1];
            while (title == null && System.nanoTime() < deadline) {
                if (NativeRuntime.tick(h, 0.016f, changed) == 0 && changed[0] != 0) {
                    final RuntimeSceneReader s = scene.acquire();
                    final TableReader text = s.findTable(NveTables.Text.KEY);
                    final int r = text == null ? -1 : s.rowOf(text, s.resolve(fx.id("title")));
                    title = r < 0 ? null : s.text(text, r, NveTables.Text.TEXT);
                    scene.release();
                }
                Thread.sleep(10);
            }
            c.eq(title, "Hello NVE", "paired keyframe rendered");
            c.that(NativeRuntime.pairIsConnected(h), "connected");
            final boolean[] fromOther = new boolean[1];
            final Thread reader = new Thread(() -> fromOther[0] = NativeRuntime.pairIsConnected(h));
            reader.start();
            reader.join();
            c.that(fromOther[0], "pairIsConnected is cross-thread safe");
            c.eq(NativeRuntime.pairDisconnect(h), 0, "disconnect");
            c.that(!NativeRuntime.pairIsConnected(h), "disconnected");
            c.eq(NativeRuntime.pairDisconnect(h), 0, "disconnect is idempotent");
        }
        c.eq(NativeRuntime.setWakeListener(h, null), 0, "clear wake");
        NativeRuntime.destroy(h);
        c.finish("android_pair_jni_test");
    }

    /** `NW` v2 wire frame: magic, version, channel, kind, flags, reserved, seq, len, payload. */
    static byte[] wire(int channel, int kind, int seq, byte[] payload) {
        final ByteBuffer b = ByteBuffer.allocate(16 + payload.length).order(ByteOrder.LITTLE_ENDIAN);
        b.put((byte) 'N').put((byte) 'W').put((byte) 2).put((byte) channel).put((byte) kind).put((byte) 0).putShort((short) 0);
        b.putInt(seq).putInt(payload.length).put(payload);
        return b.array();
    }

    static void sendBinary(OutputStream out, byte[] payload) throws Exception {
        final ByteArrayOutputStream f = new ByteArrayOutputStream();
        f.write(0x82);
        if (payload.length < 126) {
            f.write(payload.length);
        } else if (payload.length < 65536) {
            f.write(126);
            f.write(payload.length >>> 8);
            f.write(payload.length & 0xFF);
        } else {
            f.write(127);
            for (int i = 7; i >= 0; i--) f.write((int) (((long) payload.length >>> (8 * i)) & 0xFF));
        }
        f.write(payload);
        out.write(f.toByteArray());
        out.flush();
    }

    static void serve(ServerSocket server, byte[] keyframe) {
        try (Socket s = server.accept()) {
            s.setSoTimeout(5000);
            final InputStream in = s.getInputStream();
            final OutputStream out = s.getOutputStream();
            final StringBuilder req = new StringBuilder();
            while (!req.toString().endsWith("\r\n\r\n")) {
                final int ch = in.read();
                if (ch < 0) return;
                req.append((char) ch);
            }
            String key = "";
            for (String line : req.toString().split("\r\n")) {
                if (line.toLowerCase(java.util.Locale.ROOT).startsWith("sec-websocket-key:")) key = line.substring(18).trim();
            }
            final byte[] digest = MessageDigest.getInstance("SHA-1")
                    .digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").getBytes(StandardCharsets.US_ASCII));
            out.write(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: "
                    + Base64.getEncoder().encodeToString(digest) + "\r\n\r\n").getBytes(StandardCharsets.US_ASCII));
            out.flush();
            in.read(new byte[1024]);  // client Hello
            final ByteBuffer ack = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN).putInt(1).put((byte) 2).put(new byte[3]).putInt(0);
            sendBinary(out, wire(0, 2, 1, ack.array()));  // Control / HelloAck
            sendBinary(out, wire(1, 4, 2, keyframe));     // UiScene / SceneKeyframe
            while (in.read(new byte[1024]) >= 0) {
                // Hold the session open until the client disconnects.
            }
        } catch (Exception ignored) {
            // The client closing the socket ends the peer.
        }
    }
}
