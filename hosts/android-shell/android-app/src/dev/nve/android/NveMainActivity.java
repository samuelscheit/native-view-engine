package dev.nve.android;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.DisplayMetrics;
import android.util.Log;
import android.util.TypedValue;
import android.view.KeyEvent;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.TextView;

import dev.nve.host.AndroidKeys;
import dev.nve.host.RuntimeSurface;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveRuntimeAbi;

/**
 * Standalone zero-Gradle Activity hosting an NVE app on native Android Views through
 * {@code libnve_runtime}: the bundled {@code libnve_app.so} (attached via {@code nve_get_interface},
 * hot-reloadable), or a paired {@code nve pair} server when launched with
 * {@code pair_connect} / {@code nve://pair?endpoint=...}. Layout runs in the runtime with Android's
 * own text measurement; {@link NveSceneRenderer} realizes each frame.
 */
public final class NveMainActivity extends Activity {
    private static final String TAG = "NveMainActivity";
    static final String EXTRA_PAIR_CONNECT = "pair_connect";
    static final String EXTRA_PAIR_TOKEN = "pair_token";

    static {
        System.loadLibrary("nve_android_shell");
    }

    private final Handler main = new Handler(Looper.getMainLooper());
    private FrameLayout root;
    private RuntimeSurface surface;
    private NveSceneRenderer renderer;
    private float density = 1f;
    private int lastWidthPx;
    private int lastHeightPx;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        final DisplayMetrics metrics = getResources().getDisplayMetrics();
        density = metrics.density > 0f ? metrics.density : 1f;
        root = new FrameLayout(this) {
            @Override
            protected void onSizeChanged(int w, int h, int oldw, int oldh) {
                super.onSizeChanged(w, h, oldw, oldh);
                resize(w, h);
            }
        };
        root.setFitsSystemWindows(true);
        root.setBackgroundColor(Color.rgb(15, 23, 42));
        root.setLayoutParams(new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        setContentView(root);

        surface = new RuntimeSurface(1, metrics.widthPixels / density, metrics.heightPixels / density, density, main::post);
        renderer = new NveSceneRenderer(this, root, density);
        surface.setFrameConsumer(renderer);
        surface.setTextMeasurer(new NveTextMeasurer());

        final String[] pair = pairTarget(getIntent());
        final int status;
        final String what;
        if (pair != null) {
            what = pair[0];
            status = surface.pair(pair[0], pair[1]);
        } else {
            what = getApplicationInfo().nativeLibraryDir + "/libnve_app.so";
            status = surface.attachApp(what, false);
        }
        if (status != NveRuntimeAbi.STATUS_OK) {
            final String err = surface.lastError();
            Log.e(TAG, "Failed to start " + what + ": " + err);
            showError("Failed to start NVE:\n" + what + "\n" + err);
            return;
        }
        surface.render();
    }

    /** {@code [uri, token]} from {@code pair_connect} / {@code pair_token} extras or an {@code nve://pair} link. */
    static String[] pairTarget(Intent intent) {
        if (intent == null) return null;
        String uri = trimmed(intent.getStringExtra(EXTRA_PAIR_CONNECT));
        String token = trimmed(intent.getStringExtra(EXTRA_PAIR_TOKEN));
        final Uri data = intent.getData();
        if (uri == null && data != null && "nve".equalsIgnoreCase(data.getScheme()) && "pair".equalsIgnoreCase(data.getHost())) {
            uri = trimmed(data.getQueryParameter("endpoint"));
            if (token == null) token = trimmed(data.getQueryParameter("token"));
        }
        return uri == null ? null : new String[] {uri, token == null ? "" : token};
    }

    private static String trimmed(String s) {
        return s == null || s.trim().isEmpty() ? null : s.trim();
    }

    private void resize(int widthPx, int heightPx) {
        if (surface == null || widthPx <= 0 || heightPx <= 0 || (widthPx == lastWidthPx && heightPx == lastHeightPx)) return;
        lastWidthPx = widthPx;
        lastHeightPx = heightPx;
        surface.setViewport(widthPx / density, heightPx / density, density);
        surface.render();
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (surface != null && event != null
                && (event.getAction() == KeyEvent.ACTION_DOWN || event.getAction() == KeyEvent.ACTION_UP)) {
            final int kind = event.getAction() == KeyEvent.ACTION_DOWN
                    ? NveAbi.INTERACTION_EVENT_KEY_DOWN : NveAbi.INTERACTION_EVENT_KEY_UP;
            final int code = event.getKeyCode();
            final String key = AndroidKeys.key(code, event.getUnicodeChar());
            final String physical = AndroidKeys.code(code);
            final int node = renderer.nodeIdOf(getCurrentFocus());  // the focused node, never a hit test
            final int flags = modifiers(event);
            surface.send(r -> r.key(kind, node, NveSceneRenderer.now(), key, physical, flags, code));
        }
        return super.dispatchKeyEvent(event);
    }

    /** `NVE_EVENT_FLAG_MOD_*`: meta, ctrl, alt, shift, repeat. */
    private static int modifiers(KeyEvent e) {
        return (e.isMetaPressed() ? NveAbi.EVENT_FLAG_MOD_META : 0)
                | (e.isCtrlPressed() ? NveAbi.EVENT_FLAG_MOD_CTRL : 0)
                | (e.isAltPressed() ? NveAbi.EVENT_FLAG_MOD_ALT : 0)
                | (e.isShiftPressed() ? NveAbi.EVENT_FLAG_MOD_SHIFT : 0)
                | (e.getRepeatCount() > 0 ? NveAbi.EVENT_FLAG_MOD_REPEAT : 0);
    }

    private void showError(String message) {
        final TextView tv = new TextView(this);
        tv.setText(message);
        tv.setTextColor(Color.WHITE);
        tv.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 15f);
        final int pad = Math.round(20f * density);
        tv.setPadding(pad, pad, pad, pad);
        root.addView(tv, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
    }

    @Override
    protected void onDestroy() {
        if (renderer != null) renderer.clear();
        if (surface != null) {
            surface.close();
            surface = null;
        }
        main.removeCallbacksAndMessages(null);
        super.onDestroy();
    }
}
