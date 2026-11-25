package dev.nve.android;

import dev.nve.android.generated.NveSceneBuffer;

import android.app.Activity;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.text.Editable;
import android.text.InputType;
import android.text.StaticLayout;
import android.text.TextPaint;
import android.text.TextWatcher;
import android.text.method.PasswordTransformationMethod;
import android.util.Base64;
import android.util.DisplayMetrics;
import android.util.Log;
import android.util.LruCache;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.ViewTreeObserver;
import android.view.Window;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.RadioButton;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * Standalone zero-Gradle Android Activity hosting any compiled NVE Rust app (`libnve_app.so`)
 * directly on native Android Views via cached zero-copy `DirectByteBuffer` `NveSceneBuffer` slabs
 * + batched `NVE_MET1` dirty-node metadata slab (binary FFI, zero per-prop JNI chatter,
 * `O(changed)` incremental reconciliation via `dirty_indices`, `removed_ids`, and `NVE_DRAW1`
 * `NveCanvasView`).
 */
public final class NveMainActivity extends Activity {
    private static final String TAG = "NveMainActivity";
    private static final int ROOT_NODE_ID = 1;

    private static final int SLAB_BIT_NODES = 1 << 0;
    private static final int SLAB_BIT_TEXT = 1 << 1;
    private static final int SLAB_BIT_CHILDREN = 1 << 2;
    private static final int SLAB_BIT_DIRTY = 1 << 3;
    private static final int SLAB_BIT_REMOVED = 1 << 4;
    private static final int SLAB_BIT_METADATA = 1 << 5;

    private static final int RESIZE_MODE_COVER = 0;
    private static final int RESIZE_MODE_CONTAIN = 1;
    private static final int RESIZE_MODE_STRETCH = 2;
    private static final int RESIZE_MODE_CENTER = 3;

    private static final class NodeMetadata {
        String label = "";
        String role = "";
        String hint = "";
        String stateDesc = "";
        String placeholder = "";
        String testId = "";
        boolean disabled = false;
        boolean accessibilityHidden = false;
        boolean secureTextEntry = false;
        boolean multiline = false;
        boolean drawListCorrupt = false;
        int resizeMode = RESIZE_MODE_COVER;
        // Transient byte range into `cachedMetadataBuf` for the current dirty frame only.
        int frameDrawOffset = -1;
        int frameDrawLen = 0;
        long frameDrawFrameId = -1L;
    }

    /**
     * Structured `ByteOrder.LITTLE_ENDIAN` decoder for the batched `NVE_MET1` binary metadata slab
     * matching `NveNodeMetadataCodec.kt` and `NveAndroidMetadataPacker.cpp` (zero manual bit-shifts,
     * reusable bulk UTF-8 scratch buffer).
     */
    private static final class NveMetadataSlabDecoder {
        static final int HEADER_SIZE = 32;
        static final int RECORD_HEADER_SIZE = 28;
        static final int FORMAT_VERSION = 1;

        static final int OFFSET_VERSION = 8;
        static final int OFFSET_RECORD_COUNT = 12;
        static final int OFFSET_FRAME_ID = 16;
        static final int OFFSET_TOTAL_BYTES = 24;

        static final int REC_OFFSET_NODE_ID = 0;
        static final int REC_OFFSET_FLAGS = 4;
        static final int REC_OFFSET_LABEL_LEN = 8;
        static final int REC_OFFSET_ROLE_LEN = 10;
        static final int REC_OFFSET_HINT_LEN = 12;
        static final int REC_OFFSET_STATE_LEN = 14;
        static final int REC_OFFSET_PLACEHOLDER_LEN = 16;
        static final int REC_OFFSET_TEST_ID_LEN = 18;
        static final int REC_OFFSET_DRAW_LIST_LEN = 20;

        static final int FLAG_DISABLED = 1 << 0;
        static final int FLAG_SECURE_TEXT_ENTRY = 1 << 1;
        static final int FLAG_MULTILINE = 1 << 2;
        static final int FLAG_HAS_DRAW_LIST = 1 << 3;
        static final int FLAG_DRAW_LIST_CORRUPT = 1 << 4;
        static final int FLAG_ACCESSIBILITY_HIDDEN = 1 << 5;
        static final int RESIZE_MODE_SHIFT = 8;
        static final int RESIZE_MODE_MASK = 0x3;

        private static final byte[] MAGIC = {'N', 'V', 'E', '_', 'M', 'E', 'T', '1'};
        private byte[] utf8Scratch = new byte[128];

        void decodeInto(
                ByteBuffer buf,
                int declaredBytes,
                long expectedFrameId,
                Map<Integer, NodeMetadata> target,
                List<Integer> dirtyOut) {
            if (buf == null || buf.limit() < HEADER_SIZE) {
                return;
            }
            for (int i = 0; i < MAGIC.length; i++) {
                if (buf.get(i) != MAGIC[i]) {
                    return;
                }
            }
            final ByteBuffer leBuf = buf.duplicate().order(ByteOrder.LITTLE_ENDIAN);
            if ((leBuf.getShort(OFFSET_VERSION) & 0xFFFF) != FORMAT_VERSION) {
                return;
            }
            final int recordCount = leBuf.getInt(OFFSET_RECORD_COUNT);
            final long slabFrameId = leBuf.getLong(OFFSET_FRAME_ID);
            final int totalBytes = leBuf.getInt(OFFSET_TOTAL_BYTES);
            if (slabFrameId != expectedFrameId || recordCount <= 0) {
                return;
            }
            final int bound = declaredBytes > 0 ? Math.min(declaredBytes, leBuf.limit()) : leBuf.limit();
            if (totalBytes < HEADER_SIZE || totalBytes > bound) {
                return;
            }
            if ((long) recordCount * RECORD_HEADER_SIZE > (long) (totalBytes - HEADER_SIZE)) {
                return;
            }

            int pos = HEADER_SIZE;
            for (int i = 0; i < recordCount; i++) {
                if (totalBytes - pos < RECORD_HEADER_SIZE) {
                    break;
                }
                final int nodeId = leBuf.getInt(pos + REC_OFFSET_NODE_ID);
                final int flags = leBuf.getInt(pos + REC_OFFSET_FLAGS);
                final int labelLen = leBuf.getShort(pos + REC_OFFSET_LABEL_LEN) & 0xFFFF;
                final int roleLen = leBuf.getShort(pos + REC_OFFSET_ROLE_LEN) & 0xFFFF;
                final int hintLen = leBuf.getShort(pos + REC_OFFSET_HINT_LEN) & 0xFFFF;
                final int stateLen = leBuf.getShort(pos + REC_OFFSET_STATE_LEN) & 0xFFFF;
                final int placeholderLen = leBuf.getShort(pos + REC_OFFSET_PLACEHOLDER_LEN) & 0xFFFF;
                final int testIdLen = leBuf.getShort(pos + REC_OFFSET_TEST_ID_LEN) & 0xFFFF;
                final int drawListLen = leBuf.getInt(pos + REC_OFFSET_DRAW_LIST_LEN);
                pos += RECORD_HEADER_SIZE;

                if (drawListLen < 0) {
                    break;
                }
                final long payloadLen = (long) labelLen + roleLen + hintLen + stateLen
                        + placeholderLen + testIdLen + drawListLen;
                if (payloadLen > (long) (totalBytes - pos)) {
                    break;
                }

                NodeMetadata meta = target.get(nodeId);
                if (meta == null) {
                    meta = new NodeMetadata();
                    target.put(nodeId, meta);
                }
                meta.label = readUtf8(leBuf, pos, labelLen); pos += labelLen;
                meta.role = readUtf8(leBuf, pos, roleLen); pos += roleLen;
                meta.hint = readUtf8(leBuf, pos, hintLen); pos += hintLen;
                meta.stateDesc = readUtf8(leBuf, pos, stateLen); pos += stateLen;
                meta.placeholder = readUtf8(leBuf, pos, placeholderLen); pos += placeholderLen;
                meta.testId = readUtf8(leBuf, pos, testIdLen); pos += testIdLen;

                meta.disabled = (flags & FLAG_DISABLED) != 0;
                meta.secureTextEntry = (flags & FLAG_SECURE_TEXT_ENTRY) != 0;
                meta.multiline = (flags & FLAG_MULTILINE) != 0;
                meta.drawListCorrupt = (flags & FLAG_DRAW_LIST_CORRUPT) != 0;
                meta.accessibilityHidden = (flags & FLAG_ACCESSIBILITY_HIDDEN) != 0;
                meta.resizeMode = (flags >>> RESIZE_MODE_SHIFT) & RESIZE_MODE_MASK;

                if (drawListLen > 0 && (flags & FLAG_HAS_DRAW_LIST) != 0) {
                    meta.frameDrawOffset = pos;
                    meta.frameDrawLen = drawListLen;
                    meta.frameDrawFrameId = expectedFrameId;
                } else {
                    meta.frameDrawOffset = -1;
                    meta.frameDrawLen = 0;
                    meta.frameDrawFrameId = expectedFrameId;
                }
                pos += drawListLen;
                dirtyOut.add(nodeId);
            }
        }

        private String readUtf8(ByteBuffer leBuf, int offset, int len) {
            if (len <= 0) {
                return "";
            }
            if (utf8Scratch.length < len) {
                utf8Scratch = new byte[Math.max(len, utf8Scratch.length << 1)];
            }
            leBuf.position(offset);
            leBuf.get(utf8Scratch, 0, len);
            return new String(utf8Scratch, 0, len, StandardCharsets.UTF_8);
        }
    }

    private static final NodeMetadata EMPTY_METADATA = new NodeMetadata();

    static {
        System.loadLibrary("nve_android_shell");
    }

    private static native long nativeCreateSurface(
            int surfaceId,
            float width,
            float height,
            float scaleFactor);

    private static native void nativeDestroySurface(long handle);

    private static native boolean nativeLoadDylib(long handle, String soPath);

    private static native String nativeGetLastError(long handle);

    private static native void nativeResize(
            long handle,
            float width,
            float height,
            float scaleFactor);

    private static native int nativeDispatchPress(
            long handle,
            int nodeId,
            float x,
            float y);

    private static native int nativeDispatchTextChange(
            long handle,
            int nodeId,
            String text);

    private static native int nativeDispatchCustomEvent(
            long handle,
            int nodeId,
            String name,
            String payload);

    private static native int nativeDispatchScroll(
            long handle,
            int nodeId,
            float scrollX,
            float scrollY);

    private static native int nativeRenderFrame(long handle, long[] outMeta);

    private static native int nativeGetSlabEpochMask(long handle);

    private static native ByteBuffer nativeGetNodesBuffer(long handle);

    private static native ByteBuffer nativeGetTextSlabBuffer(long handle);

    private static native ByteBuffer nativeGetChildrenSlabBuffer(long handle);

    private static native ByteBuffer nativeGetDirtyIndicesBuffer(long handle);

    private static native ByteBuffer nativeGetRemovedIdsBuffer(long handle);

    private static native ByteBuffer nativeGetMetadataBuffer(long handle);

    private final Map<Integer, View> nodeViews = new HashMap<>();
    private final Map<Integer, Integer> nodeTags = new HashMap<>();
    private final Map<Integer, FrameLayout> scrollInnerContainers = new HashMap<>();
    private final Map<Integer, Integer> nodeIndexById = new HashMap<>();
    private final Map<Integer, String> boundImageSrcById = new HashMap<>();
    private final Map<Integer, NodeMetadata> nodeMetadataById = new HashMap<>();
    private final Set<Integer> scrollViewNodeIds = new HashSet<>();
    private final List<Integer> metadataDirtyNodeIds = new ArrayList<>();
    private final NveSceneBuffer sceneBuffer = new NveSceneBuffer();
    private final long[] sceneMeta = new long[8];
    private final LruCache<String, Bitmap> imageCache = createImageCache();
    private final NveMetadataSlabDecoder metadataDecoder = new NveMetadataSlabDecoder();

    private ByteBuffer cachedNodesBuf;
    private ByteBuffer cachedTextBuf;
    private ByteBuffer cachedChildrenBuf;
    private ByteBuffer cachedDirtyBuf;
    private ByteBuffer cachedRemovedBuf;
    private ByteBuffer cachedMetadataBuf;

    private RootSurfaceLayout rootContainer;
    private NvePairController pairController;
    private long surfaceHandle = 0L;
    private float density = 1.0f;
    private int lastRootWidthPx = 0;
    private int lastRootHeightPx = 0;
    private long lastSceneFrameId = -1L;
    private long lastParsedMetadataFrameId = -1L;
    private boolean appLoaded = false;
    private boolean suppressTextWatcher = false;

    private static LruCache<String, Bitmap> createImageCache() {
        int maxKb = (int) Math.min(8L * 1024L, Math.max(1024L, Runtime.getRuntime().maxMemory() / 1024L / 8L));
        return new LruCache<String, Bitmap>(maxKb) {
            @Override
            protected int sizeOf(String key, Bitmap value) {
                return Math.max(1, value.getByteCount() / 1024);
            }
        };
    }

    private final class RootSurfaceLayout extends FrameLayout {
        RootSurfaceLayout(Context context) {
            super(context);
        }

        @Override
        protected void onSizeChanged(int w, int h, int oldw, int oldh) {
            super.onSizeChanged(w, h, oldw, oldh);
            handleRootSize(w, h);
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        DisplayMetrics metrics = getResources().getDisplayMetrics();
        density = metrics.density > 0.0f ? metrics.density : 1.0f;
        float initialWidthDp = metrics.widthPixels / density;
        float initialHeightDp = metrics.heightPixels / density;

        rootContainer = new RootSurfaceLayout(this);
        rootContainer.setFitsSystemWindows(true);
        rootContainer.setBackgroundColor(Color.rgb(15, 23, 42));
        rootContainer.setLayoutParams(new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT));
        setContentView(rootContainer);

        surfaceHandle = nativeCreateSurface(1, initialWidthDp, initialHeightDp, density);

        NvePairController.PairLaunchConfig pairConfig = NvePairController.fromIntent(getIntent());
        if (pairConfig != null) {
            pairController = new NvePairController(new Runnable() {
                @Override
                public void run() {
                    syncViews();
                }
            });
            appLoaded = pairController.connect(
                    surfaceHandle,
                    pairConfig.endpoint,
                    pairConfig.token,
                    pairConfig.transport);
            if (!appLoaded) {
                String err = nativeGetLastError(surfaceHandle);
                Log.e(TAG, "Failed to connect paired endpoint " + pairConfig.endpoint + ": " + err);
                showErrorBanner("Failed to connect NVE paired host:\n" + pairConfig.endpoint + "\n" + err);
                return;
            }
        } else {
            String appLibPath = getApplicationInfo().nativeLibraryDir + "/libnve_app.so";
            appLoaded = nativeLoadDylib(surfaceHandle, appLibPath);
            if (!appLoaded) {
                String err = nativeGetLastError(surfaceHandle);
                Log.e(TAG, "Failed to load " + appLibPath + ": " + err);
                showErrorBanner("Failed to load NVE app:\n" + appLibPath + "\n" + err);
                return;
            }
        }

        rootContainer.getViewTreeObserver().addOnGlobalLayoutListener(
                new ViewTreeObserver.OnGlobalLayoutListener() {
                    @Override
                    public void onGlobalLayout() {
                        int w = rootContainer.getWidth();
                        int h = rootContainer.getHeight();
                        if (w > 0 && h > 0) {
                            handleRootSize(w, h);
                        }
                    }
                });

        syncViews();
    }

    @Override
    protected void onDestroy() {
        if (pairController != null) {
            pairController.disconnect();
            pairController = null;
        }
        if (surfaceHandle != 0L) {
            nativeDestroySurface(surfaceHandle);
            surfaceHandle = 0L;
        }
        cachedNodesBuf = null;
        cachedTextBuf = null;
        cachedChildrenBuf = null;
        cachedDirtyBuf = null;
        cachedRemovedBuf = null;
        cachedMetadataBuf = null;
        nodeViews.clear();
        nodeTags.clear();
        scrollInnerContainers.clear();
        nodeIndexById.clear();
        boundImageSrcById.clear();
        nodeMetadataById.clear();
        scrollViewNodeIds.clear();
        metadataDirtyNodeIds.clear();
        imageCache.evictAll();
        super.onDestroy();
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (appLoaded && surfaceHandle != 0L && event != null && event.getAction() == KeyEvent.ACTION_DOWN) {
            final View focused = getCurrentFocus();
            final boolean editFocused = (focused instanceof EditText);
            final String shorthand = formatKeyDownShorthand(event, editFocused);
            if (shorthand != null) {
                int dirty = nativeDispatchCustomEvent(surfaceHandle, ROOT_NODE_ID, "keydown", shorthand);
                if (dirty != 0) {
                    syncViews();
                    if (isGlobalShortcut(event)) {
                        return true;
                    }
                }
            }
        }
        return super.dispatchKeyEvent(event);
    }

    /**
     * Formats hardware keyboard events into compact binary shorthands matching
     * `examples/studio-workspace/src/ui_palette.rs::parse_keydown_payload`
     * (e.g. `"Cmd+k"`, `"Ctrl+k"`, `"ArrowDown"`, `"ArrowUp"`, `"Enter"`, `"Escape"`).
     */
    static String formatKeyDownShorthand(KeyEvent event, boolean editFocused) {
        final int keyCode = event.getKeyCode();
        final boolean meta = event.isMetaPressed();
        final boolean ctrl = event.isCtrlPressed();
        final boolean shift = event.isShiftPressed();
        final boolean alt = event.isAltPressed();
        final boolean hasPrimaryMod = meta || ctrl;

        String keyName = null;
        switch (keyCode) {
            case KeyEvent.KEYCODE_DPAD_UP:
                keyName = "ArrowUp";
                break;
            case KeyEvent.KEYCODE_DPAD_DOWN:
                keyName = "ArrowDown";
                break;
            case KeyEvent.KEYCODE_DPAD_LEFT:
                if (!editFocused || hasPrimaryMod || alt) {
                    keyName = "ArrowLeft";
                }
                break;
            case KeyEvent.KEYCODE_DPAD_RIGHT:
                if (!editFocused || hasPrimaryMod || alt) {
                    keyName = "ArrowRight";
                }
                break;
            case KeyEvent.KEYCODE_ENTER:
            case KeyEvent.KEYCODE_NUMPAD_ENTER:
                if (!editFocused || hasPrimaryMod) {
                    keyName = "Enter";
                }
                break;
            case KeyEvent.KEYCODE_ESCAPE:
                keyName = "Escape";
                break;
            case KeyEvent.KEYCODE_TAB:
                keyName = "Tab";
                break;
            default:
                if (hasPrimaryMod || alt) {
                    if (keyCode >= KeyEvent.KEYCODE_A && keyCode <= KeyEvent.KEYCODE_Z) {
                        keyName = String.valueOf((char) ('a' + (keyCode - KeyEvent.KEYCODE_A)));
                    } else if (keyCode >= KeyEvent.KEYCODE_0 && keyCode <= KeyEvent.KEYCODE_9) {
                        keyName = String.valueOf((char) ('0' + (keyCode - KeyEvent.KEYCODE_0)));
                    } else if (keyCode == KeyEvent.KEYCODE_SPACE) {
                        keyName = "Space";
                    }
                }
                break;
        }
        if (keyName == null) {
            return null;
        }
        if (!meta && !ctrl && !shift && !alt) {
            return keyName;
        }
        StringBuilder sb = new StringBuilder(24);
        if (meta) sb.append("Cmd+");
        if (ctrl) sb.append("Ctrl+");
        if (shift) sb.append("Shift+");
        if (alt) sb.append("Alt+");
        sb.append(keyName);
        return sb.toString();
    }

    private static boolean isGlobalShortcut(KeyEvent event) {
        final int code = event.getKeyCode();
        if (code == KeyEvent.KEYCODE_ESCAPE) {
            return true;
        }
        return (event.isMetaPressed() || event.isCtrlPressed())
                && code >= KeyEvent.KEYCODE_A
                && code <= KeyEvent.KEYCODE_Z;
    }

    private void handleRootSize(int widthPx, int heightPx) {
        if (!appLoaded || surfaceHandle == 0L || widthPx <= 0 || heightPx <= 0) {
            return;
        }
        if (widthPx == lastRootWidthPx && heightPx == lastRootHeightPx) {
            return;
        }
        lastRootWidthPx = widthPx;
        lastRootHeightPx = heightPx;

        float widthDp = widthPx / density;
        float heightDp = heightPx / density;
        nativeResize(surfaceHandle, widthDp, heightDp, density);
        syncViews();
    }

    private void showErrorBanner(String message) {
        TextView tv = new TextView(this);
        tv.setText(message);
        tv.setTextColor(Color.WHITE);
        tv.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 15.0f);
        int pad = dpToPx(20.0f);
        tv.setPadding(pad, pad, pad, pad);
        rootContainer.addView(tv, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT));
    }

    private void syncViews() {
        if (!appLoaded || surfaceHandle == 0L || rootContainer == null) {
            return;
        }
        int renderedCount = nativeRenderFrame(surfaceHandle, sceneMeta);
        if (renderedCount < 0) {
            return;
        }

        // Only re-fetch DirectByteBuffer references for slabs whose native `(ptr, byte_len)` changed.
        int slabMask = nativeGetSlabEpochMask(surfaceHandle);
        if (cachedNodesBuf == null || (slabMask & SLAB_BIT_NODES) != 0) {
            cachedNodesBuf = nativeGetNodesBuffer(surfaceHandle);
        }
        if (cachedTextBuf == null || (slabMask & SLAB_BIT_TEXT) != 0) {
            cachedTextBuf = nativeGetTextSlabBuffer(surfaceHandle);
        }
        if (cachedChildrenBuf == null || (slabMask & SLAB_BIT_CHILDREN) != 0) {
            cachedChildrenBuf = nativeGetChildrenSlabBuffer(surfaceHandle);
        }
        if (cachedDirtyBuf == null || (slabMask & SLAB_BIT_DIRTY) != 0) {
            cachedDirtyBuf = nativeGetDirtyIndicesBuffer(surfaceHandle);
        }
        if (cachedRemovedBuf == null || (slabMask & SLAB_BIT_REMOVED) != 0) {
            cachedRemovedBuf = nativeGetRemovedIdsBuffer(surfaceHandle);
        }
        if (cachedMetadataBuf == null || (slabMask & SLAB_BIT_METADATA) != 0) {
            cachedMetadataBuf = nativeGetMetadataBuffer(surfaceHandle);
        }

        sceneBuffer.attach(
                sceneMeta[0],
                sceneMeta[1],
                (int) sceneMeta[2],
                (int) sceneMeta[3],
                (int) sceneMeta[4],
                (int) sceneMeta[5],
                (int) sceneMeta[6],
                cachedNodesBuf,
                cachedTextBuf,
                cachedChildrenBuf,
                cachedDirtyBuf,
                cachedRemovedBuf);

        final long frameId = sceneBuffer.frameId();
        final boolean fullSync = (lastSceneFrameId < 0L || frameId < lastSceneFrameId);
        final int nodeCount = sceneBuffer.nodeCount();
        final int dirtyCount = sceneBuffer.dirtyCount();
        final int removedCount = sceneBuffer.removedCount();

        metadataDirtyNodeIds.clear();
        if (fullSync) {
            nodeMetadataById.clear();
            lastParsedMetadataFrameId = -1L;
        }
        if (frameId != lastParsedMetadataFrameId) {
            metadataDecoder.decodeInto(
                    cachedMetadataBuf, (int) sceneMeta[7], frameId, nodeMetadataById, metadataDirtyNodeIds);
            lastParsedMetadataFrameId = frameId;
        }

        if (!fullSync && dirtyCount == 0 && removedCount == 0 && metadataDirtyNodeIds.isEmpty()) {
            return;
        }

        if (fullSync) {
            for (View old : nodeViews.values()) {
                detachFromParent(old);
            }
            nodeViews.clear();
            nodeTags.clear();
            scrollInnerContainers.clear();
            scrollViewNodeIds.clear();
            nodeIndexById.clear();
            boundImageSrcById.clear();

            for (int idx = 0; idx < nodeCount; idx++) {
                upsertNodeAtIndex(idx, frameId);
            }
            final Set<View> managedViews = new HashSet<>(nodeViews.values());
            for (int idx = 0; idx < nodeCount; idx++) {
                reconcileContainerAtIndex(idx, managedViews);
            }
            refreshScrollViewBounds(nodeCount);
            lastSceneFrameId = frameId;
            return;
        }

        // Pass 1: O(removed) — prune removed node IDs directly from `removed_ids`.
        for (int r = 0; r < removedCount; r++) {
            int removedId = sceneBuffer.removedId(r);
            View oldView = nodeViews.remove(removedId);
            if (oldView != null) {
                detachFromParent(oldView);
            }
            nodeTags.remove(removedId);
            scrollInnerContainers.remove(removedId);
            scrollViewNodeIds.remove(removedId);
            nodeIndexById.remove(removedId);
            boundImageSrcById.remove(removedId);
            nodeMetadataById.remove(removedId);
        }

        // Pass 2: O(dirty) — update root and create/reuse/style only changed native Views.
        final Set<Integer> updatedInDirtyPass = metadataDirtyNodeIds.isEmpty() ? null : new HashSet<Integer>();
        for (int d = 0; d < dirtyCount; d++) {
            final int idx = sceneBuffer.dirtyIndex(d);
            if (idx < 0 || idx >= nodeCount) {
                continue;
            }
            upsertNodeAtIndex(idx, frameId);
            if (updatedInDirtyPass != null) {
                updatedInDirtyPass.add(sceneBuffer.nodeId(idx));
            }
        }

        // Pass 2b: O(dirty_metadata) — apply raw-prop-only metadata/drawList updates for nodes not
        // already touched in `dirty_indices`.
        for (int i = 0; i < metadataDirtyNodeIds.size(); i++) {
            final int nid = metadataDirtyNodeIds.get(i);
            if (updatedInDirtyPass != null && updatedInDirtyPass.contains(nid)) {
                continue;
            }
            final View view = nodeViews.get(nid);
            if (view == null) {
                continue;
            }
            Integer idxObj = nodeIndexById.get(nid);
            int idx = idxObj != null ? idxObj.intValue() : -1;
            if (idx < 0 || idx >= nodeCount || sceneBuffer.nodeId(idx) != nid) {
                idx = findSceneIndexByNodeId(nid, nodeCount);
                if (idx >= 0) {
                    nodeIndexById.put(nid, idx);
                }
            }
            if (idx >= 0 && idx < nodeCount) {
                applyNodeMetadataToView(view, nid, sceneBuffer.kind(idx), sceneBuffer, idx, frameId);
            }
        }

        // Pass 3: O(dirty) — reconcile parent-child hierarchy for dirty containers.
        if (dirtyCount > 0) {
            final Set<View> managedViews = new HashSet<>(nodeViews.values());
            for (int d = 0; d < dirtyCount; d++) {
                final int idx = sceneBuffer.dirtyIndex(d);
                if (idx < 0 || idx >= nodeCount) {
                    continue;
                }
                reconcileContainerAtIndex(idx, managedViews);
            }
            refreshScrollViewBounds(nodeCount);
        }

        lastSceneFrameId = frameId;
    }

    private void upsertNodeAtIndex(int idx, long frameId) {
        final int id = sceneBuffer.nodeId(idx);
        if (id <= 0) {
            return;
        }
        nodeIndexById.put(id, idx);

        if (id == ROOT_NODE_ID) {
            applyRootStyle(sceneBuffer, idx);
            return;
        }

        final int kind = sceneBuffer.kind(idx);
        final boolean hasChildren = sceneBuffer.childCount(idx) > 0;
        final int tag = (kind == NveSceneBuffer.KIND_TEXT && hasChildren) ? (kind | 0x10000) : kind;

        View view = nodeViews.get(id);
        Integer existingTag = nodeTags.get(id);
        if (view == null || existingTag == null || existingTag.intValue() != tag) {
            if (view != null) {
                detachFromParent(view);
                scrollInnerContainers.remove(id);
                scrollViewNodeIds.remove(id);
                boundImageSrcById.remove(id);
            }
            view = createViewForNode(id, kind, hasChildren);
            nodeViews.put(id, view);
            nodeTags.put(id, tag);
            if (kind == NveSceneBuffer.KIND_SCROLL_VIEW) {
                scrollViewNodeIds.add(id);
            }
        }

        updateView(view, id, kind, hasChildren, sceneBuffer, idx, frameId);
    }

    private void reconcileContainerAtIndex(int idx, Set<View> managedViews) {
        final int id = sceneBuffer.nodeId(idx);
        final int kind = sceneBuffer.kind(idx);

        final ViewGroup parentGroup;
        if (id == ROOT_NODE_ID) {
            parentGroup = rootContainer;
        } else if (kind == NveSceneBuffer.KIND_SCROLL_VIEW) {
            parentGroup = scrollInnerContainers.get(id);
        } else {
            View candidate = nodeViews.get(id);
            parentGroup = (candidate instanceof ViewGroup) ? (ViewGroup) candidate : null;
        }
        if (parentGroup == null) {
            return;
        }

        final int childCount = sceneBuffer.childCount(idx);
        final List<View> desiredChildren = new ArrayList<>(childCount);
        final Set<View> desiredSet = new HashSet<>(childCount * 2);
        for (int c = 0; c < childCount; c++) {
            int childId = sceneBuffer.childId(idx, c);
            View childView = nodeViews.get(childId);
            if (childView == null) {
                continue;
            }
            desiredChildren.add(childView);
            desiredSet.add(childView);
        }

        reconcileChildren(parentGroup, desiredChildren, desiredSet, managedViews);
    }

    private void refreshScrollViewBounds(int nodeCount) {
        for (int scrollId : scrollViewNodeIds) {
            Integer scrollIdxObj = nodeIndexById.get(scrollId);
            FrameLayout inner = scrollInnerContainers.get(scrollId);
            if (scrollIdxObj == null || inner == null) {
                continue;
            }
            int scrollIdx = scrollIdxObj.intValue();
            if (scrollIdx < 0 || scrollIdx >= nodeCount || sceneBuffer.nodeId(scrollIdx) != scrollId) {
                scrollIdx = findSceneIndexByNodeId(scrollId, nodeCount);
                if (scrollIdx < 0) {
                    continue;
                }
                nodeIndexById.put(scrollId, scrollIdx);
            }
            float contentMaxX = sceneBuffer.width(scrollIdx);
            float contentMaxY = sceneBuffer.height(scrollIdx);
            int childCount = sceneBuffer.childCount(scrollIdx);
            for (int c = 0; c < childCount; c++) {
                int childId = sceneBuffer.childId(scrollIdx, c);
                Integer childIdxObj = nodeIndexById.get(childId);
                int childIdx = childIdxObj != null ? childIdxObj.intValue() : -1;
                if (childIdx < 0 || childIdx >= nodeCount || sceneBuffer.nodeId(childIdx) != childId) {
                    childIdx = findSceneIndexByNodeId(childId, nodeCount);
                    if (childIdx >= 0) {
                        nodeIndexById.put(childId, childIdx);
                    }
                }
                if (childIdx >= 0 && childIdx < nodeCount) {
                    contentMaxX = Math.max(contentMaxX, sceneBuffer.x(childIdx) + sceneBuffer.width(childIdx));
                    contentMaxY = Math.max(contentMaxY, sceneBuffer.y(childIdx) + sceneBuffer.height(childIdx));
                }
            }
            int innerW = dpToPx(contentMaxX);
            int innerH = dpToPx(contentMaxY);
            ViewGroup.LayoutParams existingLp = inner.getLayoutParams();
            if (existingLp == null || existingLp.width != innerW || existingLp.height != innerH) {
                inner.setLayoutParams(new FrameLayout.LayoutParams(innerW, innerH));
            }
        }
    }

    private int findSceneIndexByNodeId(int nodeId, int nodeCount) {
        for (int i = 0; i < nodeCount; i++) {
            if (sceneBuffer.nodeId(i) == nodeId) {
                return i;
            }
        }
        return -1;
    }

    private void applyRootStyle(NveSceneBuffer scene, int idx) {
        int bgRgba = scene.bgRgba(idx);
        if ((bgRgba >>> 24) != 0) {
            int argb = rgbaToArgb(bgRgba);
            rootContainer.setBackgroundColor(argb);
            Window window = getWindow();
            if (window != null) {
                window.setBackgroundDrawable(new ColorDrawable(argb));
            }
        }
    }

    private View createViewForNode(final int nodeId, int kind, boolean hasChildren) {
        if (kind == NveSceneBuffer.KIND_SKIA_CANVAS) {
            NveCanvasView canvasView = new NveCanvasView(this);
            canvasView.setDensity(density);
            return canvasView;
        }

        if (kind == NveSceneBuffer.KIND_SCROLL_VIEW) {
            ScrollView scrollView = new ScrollView(this);
            scrollView.setFillViewport(false);
            FrameLayout inner = new FrameLayout(this);
            inner.setClipChildren(false);
            scrollView.addView(inner, new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT));
            scrollView.setOnScrollChangeListener(new View.OnScrollChangeListener() {
                @Override
                public void onScrollChange(View v, int scrollX, int scrollY, int oldScrollX, int oldScrollY) {
                    if (surfaceHandle == 0L || (scrollX == oldScrollX && scrollY == oldScrollY)) {
                        return;
                    }
                    int dirty = nativeDispatchScroll(
                            surfaceHandle, nodeId, scrollX / density, scrollY / density);
                    if (dirty != 0) {
                        syncViews();
                    }
                }
            });
            scrollInnerContainers.put(nodeId, inner);
            return scrollView;
        }

        if (kind == NveSceneBuffer.KIND_IMAGE) {
            ImageView imageView = new ImageView(this);
            imageView.setScaleType(ImageView.ScaleType.CENTER_CROP);
            return imageView;
        }

        if (kind == NveSceneBuffer.KIND_TEXT_INPUT) {
            final EditText editText = new EditText(this);
            editText.setSingleLine(true);
            editText.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
            editText.setIncludeFontPadding(false);
            editText.setGravity(Gravity.CENTER_VERTICAL | Gravity.START);
            int padH = dpToPx(8.0f);
            editText.setPadding(padH, 0, padH, 0);
            editText.addTextChangedListener(new TextWatcher() {
                @Override
                public void beforeTextChanged(CharSequence s, int start, int count, int after) {}

                @Override
                public void onTextChanged(CharSequence s, int start, int before, int count) {}

                @Override
                public void afterTextChanged(Editable s) {
                    if (suppressTextWatcher || surfaceHandle == 0L) {
                        return;
                    }
                    int dirty = nativeDispatchTextChange(
                            surfaceHandle, nodeId, s != null ? s.toString() : "");
                    if (dirty != 0) {
                        syncViews();
                    }
                }
            });
            editText.setOnEditorActionListener(new TextView.OnEditorActionListener() {
                @Override
                public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                    if (surfaceHandle == 0L) {
                        return false;
                    }
                    final boolean isSubmitAction = actionId == EditorInfo.IME_ACTION_DONE
                            || actionId == EditorInfo.IME_ACTION_SEND
                            || actionId == EditorInfo.IME_ACTION_GO
                            || (actionId == EditorInfo.IME_NULL
                                && event != null
                                && event.getAction() == KeyEvent.ACTION_DOWN
                                && (event.getKeyCode() == KeyEvent.KEYCODE_ENTER
                                    || event.getKeyCode() == KeyEvent.KEYCODE_NUMPAD_ENTER));
                    if (!isSubmitAction) {
                        return false;
                    }
                    final CharSequence cs = v.getText();
                    final String value = cs != null ? cs.toString() : "";
                    int dirty = nativeDispatchCustomEvent(surfaceHandle, nodeId, "submit", value);
                    if (dirty != 0) {
                        syncViews();
                    }
                    return true;
                }
            });
            return editText;
        }

        if (kind == NveSceneBuffer.KIND_TEXT && !hasChildren) {
            TextView tv = new TextView(this);
            tv.setIncludeFontPadding(false);
            tv.setPadding(0, 0, 0, 0);
            tv.setGravity(Gravity.TOP | Gravity.START);
            return tv;
        }

        FrameLayout frame = new FrameLayout(this);
        frame.setClipChildren(false);
        return frame;
    }

    private void updateView(
            View view,
            final int nodeId,
            int kind,
            boolean hasChildren,
            NveSceneBuffer scene,
            int idx,
            long frameId) {
        final float xDp = scene.x(idx);
        final float yDp = scene.y(idx);
        final float wDp = Math.max(0.0f, scene.width(idx));
        final float hDp = Math.max(0.0f, scene.height(idx));

        final int leftPx = dpToPx(xDp);
        final int topPx = dpToPx(yDp);
        final int widthPx = dpToPx(wDp);
        final int heightPx = dpToPx(hDp);

        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(widthPx, heightPx);
        lp.leftMargin = leftPx;
        lp.topMargin = topPx;
        lp.gravity = Gravity.TOP | Gravity.START;
        view.setLayoutParams(lp);

        final int bgRgba = scene.bgRgba(idx);
        final int fgRgba = scene.textRgba(idx);
        final int bcRgba = scene.borderRgba(idx);
        final float bwDp = Math.max(0.0f, scene.borderWidth(idx));
        final float brDp = Math.max(0.0f, scene.borderRadius(idx));
        final float opacity = scene.opacity(idx);
        final float fontSizeDp = scene.fontSize(idx);
        final int zIndex = scene.zIndex(idx);

        view.setAlpha(Math.max(0.0f, Math.min(1.0f, opacity)));
        view.setTranslationZ(zIndex != 0 ? zIndex * density : 0.0f);

        final boolean hasBg = (bgRgba >>> 24) != 0;
        final boolean hasBorder = bwDp > 0.0f && (bcRgba >>> 24) != 0;
        if (hasBg || hasBorder || brDp > 0.0f) {
            GradientDrawable drawable = new GradientDrawable();
            drawable.setShape(GradientDrawable.RECTANGLE);
            drawable.setColor(hasBg ? rgbaToArgb(bgRgba) : Color.TRANSPARENT);
            if (brDp > 0.0f) {
                drawable.setCornerRadius(brDp * density);
            }
            if (hasBorder) {
                drawable.setStroke(Math.max(1, dpToPx(bwDp)), rgbaToArgb(bcRgba));
            }
            view.setBackground(drawable);
            view.setClipToOutline(brDp > 0.0f);
        } else {
            view.setBackground(null);
            view.setClipToOutline(false);
        }

        if (kind == NveSceneBuffer.KIND_IMAGE && view instanceof ImageView) {
            ImageView imageView = (ImageView) view;
            String src = scene.readText(idx);
            String prevSrc = boundImageSrcById.get(nodeId);
            if (!src.equals(prevSrc)) {
                boundImageSrcById.put(nodeId, src);
                imageView.setImageBitmap(resolveBitmap(src));
            }
        } else if (kind == NveSceneBuffer.KIND_TEXT_INPUT && view instanceof EditText) {
            EditText editText = (EditText) view;
            String text = scene.readText(idx);
            Editable current = editText.getText();
            boolean composing = current != null && BaseInputConnection.getComposingSpanStart(current) >= 0;
            if (!composing && (current == null || !text.equals(current.toString()))) {
                suppressTextWatcher = true;
                try {
                    editText.setText(text);
                    editText.setSelection(text.length());
                } finally {
                    suppressTextWatcher = false;
                }
            }
            editText.setTextColor(fgRgba != 0 ? rgbaToArgb(fgRgba) : Color.WHITE);
            editText.setHintTextColor(Color.argb(138, 226, 232, 240));
            editText.setTextSize(TypedValue.COMPLEX_UNIT_DIP, fontSizeDp > 0.0f ? fontSizeDp : 16.0f);
        } else if (kind == NveSceneBuffer.KIND_TEXT && !hasChildren && view instanceof TextView) {
            TextView tv = (TextView) view;
            String text = scene.readText(idx);
            if (!text.equals(tv.getText().toString())) {
                tv.setText(text);
            }
            tv.setTextColor(fgRgba != 0 ? rgbaToArgb(fgRgba) : Color.BLACK);
            tv.setTextSize(TypedValue.COMPLEX_UNIT_DIP, fontSizeDp > 0.0f ? fontSizeDp : 16.0f);
        }

        applyNodeMetadataToView(view, nodeId, kind, scene, idx, frameId);

        final NodeMetadata meta = nodeMetadataById.getOrDefault(nodeId, EMPTY_METADATA);
        final boolean interactive = scene.isInteractive(idx) && !meta.disabled;
        if (interactive && kind != NveSceneBuffer.KIND_TEXT_INPUT && kind != NveSceneBuffer.KIND_SCROLL_VIEW) {
            view.setClickable(true);
            view.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    if (surfaceHandle == 0L || !v.isEnabled()) {
                        return;
                    }
                    nativeDispatchPress(surfaceHandle, nodeId, wDp * 0.5f, hDp * 0.5f);
                    syncViews();
                }
            });
        } else if (kind != NveSceneBuffer.KIND_TEXT_INPUT && kind != NveSceneBuffer.KIND_SCROLL_VIEW) {
            view.setOnClickListener(null);
            view.setClickable(false);
        }
    }

    private void applyNodeMetadataToView(
            View view,
            int nodeId,
            int kind,
            NveSceneBuffer scene,
            int idx,
            long frameId) {
        final NodeMetadata meta = nodeMetadataById.getOrDefault(nodeId, EMPTY_METADATA);

        // TalkBack accessibility properties from batched `NVE_MET1` slab (zero per-prop JNI).
        view.setEnabled(!meta.disabled);
        view.setContentDescription(meta.label.isEmpty() ? null : meta.label);
        if (meta.accessibilityHidden) {
            view.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
        } else if (!meta.label.isEmpty() || !meta.role.isEmpty() || !meta.hint.isEmpty()
                || !meta.stateDesc.isEmpty() || scene.isInteractive(idx)) {
            view.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_YES);
        } else {
            view.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_AUTO);
        }

        if (!meta.role.isEmpty() || !meta.hint.isEmpty() || !meta.stateDesc.isEmpty() || meta.disabled) {
            final String role = meta.role;
            final String hint = meta.hint;
            final String stateDesc = meta.stateDesc;
            final boolean disabled = meta.disabled;
            view.setAccessibilityDelegate(new View.AccessibilityDelegate() {
                @Override
                public void onInitializeAccessibilityNodeInfo(View host, AccessibilityNodeInfo info) {
                    super.onInitializeAccessibilityNodeInfo(host, info);
                    info.setEnabled(!disabled);
                    String className = androidClassNameForRole(role);
                    if (className != null) {
                        info.setClassName(className);
                    }
                    if ("header".equalsIgnoreCase(role) && Build.VERSION.SDK_INT >= 28) {
                        info.setHeading(true);
                    }
                    if (!hint.isEmpty() && Build.VERSION.SDK_INT >= 26) {
                        info.setHintText(hint);
                    }
                    if (!stateDesc.isEmpty() && Build.VERSION.SDK_INT >= 30) {
                        info.setStateDescription(stateDesc);
                    }
                }
            });
        } else {
            view.setAccessibilityDelegate(null);
        }

        if (kind == NveSceneBuffer.KIND_IMAGE && view instanceof ImageView) {
            ((ImageView) view).setScaleType(scaleTypeForResizeMode(meta.resizeMode));
        } else if (kind == NveSceneBuffer.KIND_TEXT_INPUT && view instanceof EditText) {
            EditText editText = (EditText) view;
            CharSequence prevHint = editText.getHint();
            String nextHint = meta.placeholder;
            if (nextHint.isEmpty()) {
                if (prevHint != null) {
                    editText.setHint(null);
                }
            } else if (prevHint == null || !nextHint.equals(prevHint.toString())) {
                editText.setHint(nextHint);
            }

            final int desiredInputType;
            if (meta.secureTextEntry) {
                desiredInputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD;
            } else if (meta.multiline) {
                desiredInputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE;
            } else {
                desiredInputType = InputType.TYPE_CLASS_TEXT;
            }
            if (editText.getInputType() != desiredInputType) {
                int selStart = editText.getSelectionStart();
                int selEnd = editText.getSelectionEnd();
                suppressTextWatcher = true;
                try {
                    editText.setSingleLine(!meta.multiline);
                    editText.setInputType(desiredInputType);
                    editText.setTransformationMethod(
                            meta.secureTextEntry ? PasswordTransformationMethod.getInstance() : null);
                    if (selStart >= 0 && selEnd >= 0) {
                        int len = editText.getText() != null ? editText.getText().length() : 0;
                        editText.setSelection(Math.min(selStart, len), Math.min(selEnd, len));
                    }
                } finally {
                    suppressTextWatcher = false;
                }
            }
        } else if (kind == NveSceneBuffer.KIND_SKIA_CANVAS && view instanceof NveCanvasView) {
            NveCanvasView canvasView = (NveCanvasView) view;
            canvasView.setDensity(density);
            final int tLen = scene.textLen(idx);
            final int tOff = scene.textOffset(idx);
            if (cachedTextBuf != null && tLen >= NveCanvasView.HEADER_SIZE
                    && NveCanvasView.hasDrawMagic(cachedTextBuf, tOff, tLen)) {
                canvasView.setDrawListSlice(cachedTextBuf, tOff, tLen);
            } else if (meta.frameDrawFrameId == frameId) {
                if (meta.frameDrawOffset >= 0 && meta.frameDrawLen >= NveCanvasView.HEADER_SIZE
                        && cachedMetadataBuf != null) {
                    canvasView.setDrawListCopy(cachedMetadataBuf, meta.frameDrawOffset, meta.frameDrawLen);
                } else {
                    canvasView.clearDrawList();
                }
            } else if (meta.drawListCorrupt) {
                canvasView.clearDrawList();
            }
        }
    }

    private static String androidClassNameForRole(String role) {
        if (role == null || role.isEmpty()) {
            return null;
        }
        String r = role.trim().toLowerCase(Locale.US);
        switch (r) {
            case "button":
            case "pressable":
            case "link":
                return Button.class.getName();
            case "checkbox":
                return CheckBox.class.getName();
            case "switch":
            case "toggle":
                return Switch.class.getName();
            case "radio":
            case "radiobutton":
                return RadioButton.class.getName();
            case "image":
            case "img":
                return ImageView.class.getName();
            default:
                return null;
        }
    }

    private Bitmap resolveBitmap(String src) {
        if (src == null || src.isEmpty()) {
            return null;
        }
        Bitmap cached = imageCache.get(src);
        if (cached != null && !cached.isRecycled()) {
            return cached;
        }
        Bitmap decoded = decodeBitmapSource(src);
        if (decoded != null) {
            imageCache.put(src, decoded);
        }
        return decoded;
    }

    private static Bitmap decodeBitmapSource(String src) {
        String lower = src.toLowerCase(Locale.US);
        if (lower.startsWith("data:")) {
            int comma = src.indexOf(',');
            if (comma <= 0 || comma + 1 >= src.length()) {
                return null;
            }
            String header = lower.substring(0, comma);
            if (!header.startsWith("data:image/") || !header.contains(";base64")) {
                return null;
            }
            try {
                byte[] bytes = Base64.decode(src.substring(comma + 1), Base64.DEFAULT);
                if (bytes == null || bytes.length == 0) {
                    return null;
                }
                return BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
            } catch (IllegalArgumentException e) {
                return null;
            }
        }
        if (lower.startsWith("file://")) {
            try {
                Uri uri = Uri.parse(src);
                String path = uri != null ? uri.getPath() : null;
                if (path == null || path.isEmpty()) {
                    path = src.substring(7);
                }
                return BitmapFactory.decodeFile(path);
            } catch (RuntimeException e) {
                return null;
            }
        }
        return null;
    }

    private static ImageView.ScaleType scaleTypeForResizeMode(int resizeMode) {
        switch (resizeMode) {
            case RESIZE_MODE_CONTAIN:
                return ImageView.ScaleType.FIT_CENTER;
            case RESIZE_MODE_STRETCH:
                return ImageView.ScaleType.FIT_XY;
            case RESIZE_MODE_CENTER:
                return ImageView.ScaleType.CENTER;
            case RESIZE_MODE_COVER:
            default:
                return ImageView.ScaleType.CENTER_CROP;
        }
    }

    private static void reconcileChildren(
            ViewGroup parent,
            List<View> desiredChildren,
            Set<View> desiredSet,
            Set<View> managedViews) {
        for (int i = parent.getChildCount() - 1; i >= 0; i--) {
            View child = parent.getChildAt(i);
            if (managedViews.contains(child) && !desiredSet.contains(child)) {
                parent.removeViewAt(i);
            }
        }

        List<View> currentManaged = new ArrayList<>(parent.getChildCount());
        for (int i = 0; i < parent.getChildCount(); i++) {
            View child = parent.getChildAt(i);
            if (managedViews.contains(child)) {
                currentManaged.add(child);
            }
        }
        boolean orderMatches = currentManaged.size() == desiredChildren.size();
        if (orderMatches) {
            for (int i = 0; i < desiredChildren.size(); i++) {
                if (currentManaged.get(i) != desiredChildren.get(i)) {
                    orderMatches = false;
                    break;
                }
            }
        }
        if (orderMatches) {
            return;
        }

        for (View child : desiredChildren) {
            detachFromParent(child);
            parent.addView(child);
        }
    }

    private static void detachFromParent(View view) {
        ViewParent parent = view.getParent();
        if (parent instanceof ViewGroup) {
            ((ViewGroup) parent).removeView(view);
        }
    }

    private int dpToPx(float dp) {
        if (!Float.isFinite(dp)) {
            return 0;
        }
        return Math.round(dp * density);
    }

    private static TextPaint measurePaint;

    /**
     * Called from `NveAndroidSurfaceJni.cpp` during layout passes to measure exact Android
     * Roboto glyph bounds in DP units.
     */
    private static float[] measureTextDp(String text, float fontSizeDp, float maxWidthDp) {
        if (text == null || text.isEmpty()) {
            return new float[]{0.0f, 0.0f};
        }
        if (measurePaint == null) {
            measurePaint = new TextPaint(Paint.ANTI_ALIAS_FLAG);
        }
        float size = (Float.isFinite(fontSizeDp) && fontSizeDp > 0.0f) ? fontSizeDp : 16.0f;
        measurePaint.setTextSize(size);
        float singleW = (float) Math.ceil(measurePaint.measureText(text));
        int constraintW;
        if (Float.isFinite(maxWidthDp) && maxWidthDp > 0.0f) {
            constraintW = Math.max(1, (int) Math.ceil(maxWidthDp));
        } else {
            constraintW = Math.max(1, (int) Math.ceil(singleW));
        }
        StaticLayout layout = StaticLayout.Builder.obtain(
                        text, 0, text.length(), measurePaint, constraintW)
                .setIncludePad(false)
                .build();
        float maxLineW = 0.0f;
        for (int i = 0; i < layout.getLineCount(); i++) {
            maxLineW = Math.max(maxLineW, layout.getLineWidth(i));
        }
        return new float[]{
                (float) Math.ceil(maxLineW) + 2.0f,
                (float) Math.ceil(layout.getHeight())
        };
    }

    private static int rgbaToArgb(int rgba) {
        int r = rgba & 0xFF;
        int g = (rgba >>> 8) & 0xFF;
        int b = (rgba >>> 16) & 0xFF;
        int a = (rgba >>> 24) & 0xFF;
        return (a << 24) | (r << 16) | (g << 8) | b;
    }
}
