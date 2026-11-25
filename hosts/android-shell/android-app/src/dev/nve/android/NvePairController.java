package dev.nve.android;

import android.content.Intent;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;

import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Manages zero-cdylib paired thin-client streaming (`NVT1`/`NVD1` over UDP/TCP/WebSocket)
 * on Android, bridging background `PairClient` frame wakeups onto the main UI {@link Looper}.
 */
public final class NvePairController {
    public static final String EXTRA_PAIR_CONNECT = "pair_connect";
    public static final String EXTRA_PAIR_TOKEN = "pair_token";
    public static final String EXTRA_PAIR_TRANSPORT = "pair_transport";

    public static final class PairLaunchConfig {
        public final String endpoint;
        public final String token;
        public final String transport;

        public PairLaunchConfig(String endpoint, String token, String transport) {
            this.endpoint = endpoint;
            this.token = token != null ? token : "";
            this.transport = transport != null ? transport : "";
        }
    }

    private static native boolean nativeConnectPaired(
            long handle,
            String endpoint,
            String token,
            String transport,
            Object wakeTarget);

    private static native int nativePollPairedFrames(long handle);

    private static native void nativeDisconnectPaired(long handle);

    private static native boolean nativeIsPaired(long handle);

    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private final AtomicBoolean wakePending = new AtomicBoolean(false);
    private final Runnable onFrameApplied;
    private volatile long surfaceHandle = 0L;

    public NvePairController(Runnable onFrameApplied) {
        this.onFrameApplied = onFrameApplied;
    }

    /**
     * Extracts paired launch configuration from Activity {@link Intent} extras
     * (`pair_connect`, `pair_token`, `pair_transport`) or an `nve://pair?...` deep-link URI.
     */
    public static PairLaunchConfig fromIntent(Intent intent) {
        if (intent == null) {
            return null;
        }
        String endpoint = trimOrNull(intent.getStringExtra(EXTRA_PAIR_CONNECT));
        String token = trimOrNull(intent.getStringExtra(EXTRA_PAIR_TOKEN));
        String transport = trimOrNull(intent.getStringExtra(EXTRA_PAIR_TRANSPORT));

        if (endpoint == null) {
            Uri data = intent.getData();
            if (data != null && "nve".equalsIgnoreCase(data.getScheme())
                    && "pair".equalsIgnoreCase(data.getHost())) {
                endpoint = trimOrNull(data.getQueryParameter("endpoint"));
                if (endpoint == null) {
                    endpoint = trimOrNull(data.getQueryParameter("connect"));
                }
                if (token == null) {
                    token = trimOrNull(data.getQueryParameter("token"));
                }
                if (transport == null) {
                    transport = trimOrNull(data.getQueryParameter("transport"));
                }
            }
        }
        if (endpoint == null) {
            return null;
        }
        return new PairLaunchConfig(endpoint, token, transport);
    }

    public boolean connect(long handle, String endpoint, String token, String transport) {
        if (handle == 0L || endpoint == null || endpoint.trim().isEmpty()) {
            return false;
        }
        surfaceHandle = handle;
        boolean ok = nativeConnectPaired(
                handle,
                endpoint.trim(),
                token != null ? token : "",
                transport != null ? transport.trim() : "",
                this);
        if (!ok) {
            surfaceHandle = 0L;
        }
        return ok;
    }

    public int pollFrames() {
        long h = surfaceHandle;
        if (h == 0L) {
            return 0;
        }
        return nativePollPairedFrames(h);
    }

    public boolean isPaired() {
        long h = surfaceHandle;
        return h != 0L && nativeIsPaired(h);
    }

    public void disconnect() {
        long h = surfaceHandle;
        surfaceHandle = 0L;
        wakePending.set(false);
        if (h != 0L) {
            nativeDisconnectPaired(h);
        }
    }

    /**
     * Invoked from `NveAndroidPairJni.cpp` on the `PairClient` reader thread when a new
     * reconstructed `NVE1` frame arrives. Coalesces bursts onto the main UI thread.
     */
    @SuppressWarnings("unused")
    public void onNativePairFrameReady() {
        if (surfaceHandle == 0L) {
            return;
        }
        if (!wakePending.compareAndSet(false, true)) {
            return;
        }
        mainHandler.post(new Runnable() {
            @Override
            public void run() {
                wakePending.set(false);
                long h = surfaceHandle;
                if (h == 0L || !nativeIsPaired(h)) {
                    return;
                }
                if (nativePollPairedFrames(h) > 0 && onFrameApplied != null) {
                    onFrameApplied.run();
                }
            }
        });
    }

    private static String trimOrNull(String value) {
        if (value == null) {
            return null;
        }
        String trimmed = value.trim();
        return trimmed.isEmpty() ? null : trimmed;
    }
}
