package dev.nve.android;

import android.app.Activity;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Bundle;
import android.text.Editable;
import android.text.StaticLayout;
import android.text.TextPaint;
import android.text.TextWatcher;
import android.util.Base64;
import android.util.DisplayMetrics;
import android.util.Log;
import android.util.LruCache;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.ViewTreeObserver;
import android.view.Window;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.ScrollView;
import android.widget.TextView;

import java.nio.ByteBuffer;
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
 * (zero JSON, `O(changed)` incremental reconciliation via `dirty_indices` and `removed_ids`).
 */
public final class NveMainActivity extends Activity {
    private static final String TAG = "NveMainActivity";
    private static final int ROOT_NODE_ID = 1;

    private static final int SLAB_BIT_NODES = 1 << 0;
    private static final int SLAB_BIT_TEXT = 1 << 1;
    private static final int SLAB_BIT_CHILDREN = 1 << 2;
    private static final int SLAB_BIT_DIRTY = 1 << 3;
    private static final int SLAB_BIT_REMOVED = 1 << 4;

    private static final int RESIZE_MODE_COVER = 0;
    private static final int RESIZE_MODE_CONTAIN = 1;
    private static final int RESIZE_MODE_STRETCH = 2;
    private static final int RESIZE_MODE_CENTER = 3;

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

    private static native int nativeDispatchScroll(
            long handle,
            int nodeId,
            float scrollX,
            float scrollY);

    private static native int nativeRenderFrame(long handle, long[] outMeta);

    private static native int nativeGetSlabEpochMask(long handle);

    private static native int nativeGetImageResizeMode(long handle, int nodeId);

    private static native ByteBuffer nativeGetNodesBuffer(long handle);

    private static native ByteBuffer nativeGetTextSlabBuffer(long handle);

    private static native ByteBuffer nativeGetChildrenSlabBuffer(long handle);

    private static native ByteBuffer nativeGetDirtyIndicesBuffer(long handle);

    private static native ByteBuffer nativeGetRemovedIdsBuffer(long handle);

    private final Map<Integer, View> nodeViews = new HashMap<>();
    private final Map<Integer, Integer> nodeTags = new HashMap<>();
    private final Map<Integer, FrameLayout> scrollInnerContainers = new HashMap<>();
    private final Map<Integer, Integer> nodeIndexById = new HashMap<>();
    private final Map<Integer, String> boundImageSrcById = new HashMap<>();
    private final Set<Integer> scrollViewNodeIds = new HashSet<>();
    private final NveSceneBuffer sceneBuffer = new NveSceneBuffer();
    private final long[] sceneMeta = new long[7];
    private final LruCache<String, Bitmap> imageCache = createImageCache();

    private ByteBuffer cachedNodesBuf;
    private ByteBuffer cachedTextBuf;
    private ByteBuffer cachedChildrenBuf;
    private ByteBuffer cachedDirtyBuf;
    private ByteBuffer cachedRemovedBuf;

    private RootSurfaceLayout rootContainer;
    private long surfaceHandle = 0L;
    private float density = 1.0f;
    private int lastRootWidthPx = 0;
    private int lastRootHeightPx = 0;
    private long lastSceneFrameId = -1L;
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

        String appLibPath = getApplicationInfo().nativeLibraryDir + "/libnve_app.so";
        appLoaded = nativeLoadDylib(surfaceHandle, appLibPath);
        if (!appLoaded) {
            String err = nativeGetLastError(surfaceHandle);
            Log.e(TAG, "Failed to load " + appLibPath + ": " + err);
            showErrorBanner("Failed to load NVE app:\n" + appLibPath + "\n" + err);
            return;
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
        if (surfaceHandle != 0L) {
            nativeDestroySurface(surfaceHandle);
            surfaceHandle = 0L;
        }
        cachedNodesBuf = null;
        cachedTextBuf = null;
        cachedChildrenBuf = null;
        cachedDirtyBuf = null;
        cachedRemovedBuf = null;
        nodeViews.clear();
        nodeTags.clear();
        scrollInnerContainers.clear();
        nodeIndexById.clear();
        boundImageSrcById.clear();
        scrollViewNodeIds.clear();
        imageCache.evictAll();
        super.onDestroy();
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
        if (!fullSync && dirtyCount == 0 && removedCount == 0) {
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
                upsertNodeAtIndex(idx);
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
        }

        // Pass 2: O(dirty) — update root and create/reuse/style only changed native Views.
        for (int d = 0; d < dirtyCount; d++) {
            final int idx = sceneBuffer.dirtyIndex(d);
            if (idx < 0 || idx >= nodeCount) {
                continue;
            }
            upsertNodeAtIndex(idx);
        }

        // Pass 3: O(dirty) — reconcile parent-child hierarchy for dirty containers.
        final Set<View> managedViews = new HashSet<>(nodeViews.values());
        for (int d = 0; d < dirtyCount; d++) {
            final int idx = sceneBuffer.dirtyIndex(d);
            if (idx < 0 || idx >= nodeCount) {
                continue;
            }
            reconcileContainerAtIndex(idx, managedViews);
        }

        refreshScrollViewBounds(nodeCount);
        lastSceneFrameId = frameId;
    }

    private void upsertNodeAtIndex(int idx) {
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

        updateView(view, id, kind, hasChildren, sceneBuffer, idx);
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
            EditText editText = new EditText(this);
            editText.setSingleLine(true);
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
            int idx) {
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
            int resizeMode = nativeGetImageResizeMode(surfaceHandle, nodeId);
            imageView.setScaleType(scaleTypeForResizeMode(resizeMode));
        } else if (kind == NveSceneBuffer.KIND_TEXT_INPUT && view instanceof EditText) {
            EditText editText = (EditText) view;
            String text = scene.readText(idx);
            CharSequence current = editText.getText();
            if (current == null || !text.equals(current.toString())) {
                suppressTextWatcher = true;
                try {
                    editText.setText(text);
                    editText.setSelection(text.length());
                } finally {
                    suppressTextWatcher = false;
                }
            }
            editText.setTextColor(fgRgba != 0 ? rgbaToArgb(fgRgba) : Color.WHITE);
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

        final boolean interactive = scene.isInteractive(idx);
        if (interactive && kind != NveSceneBuffer.KIND_TEXT_INPUT && kind != NveSceneBuffer.KIND_SCROLL_VIEW) {
            view.setClickable(true);
            view.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    if (surfaceHandle == 0L) {
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
