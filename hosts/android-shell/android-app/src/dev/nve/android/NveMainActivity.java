package dev.nve.android;

import android.app.Activity;
import android.content.Context;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.GradientDrawable;
import android.os.Bundle;
import android.text.Editable;
import android.text.StaticLayout;
import android.text.TextPaint;
import android.text.TextWatcher;
import android.util.DisplayMetrics;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.ViewTreeObserver;
import android.view.Window;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Standalone zero-Gradle Android Activity hosting any compiled NVE Rust app (`libnve_app.so`)
 * directly on native Android Views via `NveAndroidSurfaceJni.cpp`.
 */
public final class NveMainActivity extends Activity {
    private static final String TAG = "NveMainActivity";

    private static final int ROOT_NODE_ID = 1;
    private static final int COMP_VIEW = 1;
    private static final int COMP_TEXT = 2;
    private static final int COMP_SCROLL_VIEW = 3;
    private static final int COMP_TEXT_INPUT = 5;
    private static final int COMP_PRESSABLE = 6;

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

    private static native String nativeRenderAndSnapshotJson(long handle);

    private final Map<Integer, View> nodeViews = new HashMap<>();
    private final Map<Integer, Integer> nodeTags = new HashMap<>();
    private final Map<Integer, FrameLayout> scrollInnerContainers = new HashMap<>();

    private RootSurfaceLayout rootContainer;
    private long surfaceHandle = 0L;
    private float density = 1.0f;
    private int lastRootWidthPx = 0;
    private int lastRootHeightPx = 0;
    private boolean appLoaded = false;
    private boolean suppressTextWatcher = false;

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
        nodeViews.clear();
        nodeTags.clear();
        scrollInnerContainers.clear();
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
        String snapshotJson = nativeRenderAndSnapshotJson(surfaceHandle);
        if (snapshotJson == null || snapshotJson.isEmpty()) {
            return;
        }

        final JSONArray nodes;
        try {
            nodes = new JSONArray(snapshotJson);
        } catch (JSONException e) {
            Log.e(TAG, "Failed to parse NVE snapshot JSON", e);
            return;
        }

        final int count = nodes.length();
        final Set<Integer> liveIds = new HashSet<>(count * 2);
        final Map<Integer, JSONObject> snapById = new HashMap<>(count * 2);

        for (int i = 0; i < count; i++) {
            JSONObject snap = nodes.optJSONObject(i);
            if (snap == null) {
                continue;
            }
            int id = snap.optInt("id", 0);
            if (id <= 0) {
                continue;
            }
            liveIds.add(id);
            snapById.put(id, snap);
        }

        // Pass 1: Prune removed node IDs.
        Iterator<Map.Entry<Integer, View>> pruneIt = nodeViews.entrySet().iterator();
        while (pruneIt.hasNext()) {
            Map.Entry<Integer, View> entry = pruneIt.next();
            int id = entry.getKey();
            if (!liveIds.contains(id)) {
                detachFromParent(entry.getValue());
                nodeTags.remove(id);
                scrollInnerContainers.remove(id);
                pruneIt.remove();
            }
        }

        // Pass 2: Update root and create/reuse/style retained native Android Views.
        for (int i = 0; i < count; i++) {
            JSONObject snap = nodes.optJSONObject(i);
            if (snap == null) {
                continue;
            }
            final int id = snap.optInt("id", 0);
            if (id <= 0) {
                continue;
            }
            if (id == ROOT_NODE_ID) {
                applyRootStyle(snap);
                continue;
            }

            final int kind = snap.optInt("kind", COMP_VIEW);
            final JSONArray children = snap.optJSONArray("ch");
            final boolean hasChildren = children != null && children.length() > 0;
            final int tag = (kind == COMP_TEXT && hasChildren) ? (kind | 0x10000) : kind;

            View view = nodeViews.get(id);
            Integer existingTag = nodeTags.get(id);
            if (view == null || existingTag == null || existingTag.intValue() != tag) {
                if (view != null) {
                    detachFromParent(view);
                    scrollInnerContainers.remove(id);
                }
                view = createViewForNode(id, kind, hasChildren);
                nodeViews.put(id, view);
                nodeTags.put(id, tag);
            }

            updateView(view, id, kind, hasChildren, snap);
        }

        // Pass 3: Reconcile parent-child hierarchy in exact child order + scroll content bounds.
        final Set<View> managedViews = new HashSet<>(nodeViews.values());
        final Set<Integer> mountedChildIds = new HashSet<>(nodeViews.size() * 2);

        for (int i = 0; i < count; i++) {
            JSONObject snap = nodes.optJSONObject(i);
            if (snap == null) {
                continue;
            }
            final int id = snap.optInt("id", 0);
            final int kind = snap.optInt("kind", COMP_VIEW);
            final JSONArray childIds = snap.optJSONArray("ch");

            final ViewGroup parentGroup;
            if (id == ROOT_NODE_ID) {
                parentGroup = rootContainer;
            } else if (kind == COMP_SCROLL_VIEW) {
                parentGroup = scrollInnerContainers.get(id);
            } else {
                View candidate = nodeViews.get(id);
                parentGroup = (candidate instanceof ViewGroup) ? (ViewGroup) candidate : null;
            }
            if (parentGroup == null) {
                continue;
            }

            final int childCount = childIds != null ? childIds.length() : 0;
            final List<View> desiredChildren = new ArrayList<>(childCount);
            final Set<View> desiredSet = new HashSet<>(childCount * 2);
            float contentMaxX = 0.0f;
            float contentMaxY = 0.0f;

            for (int c = 0; c < childCount; c++) {
                int childId = childIds.optInt(c, 0);
                View childView = nodeViews.get(childId);
                if (childView == null) {
                    continue;
                }
                desiredChildren.add(childView);
                desiredSet.add(childView);
                mountedChildIds.add(childId);

                JSONObject childSnap = snapById.get(childId);
                if (childSnap != null) {
                    float cx = (float) childSnap.optDouble("x", 0.0);
                    float cy = (float) childSnap.optDouble("y", 0.0);
                    float cw = (float) childSnap.optDouble("w", 0.0);
                    float ch = (float) childSnap.optDouble("h", 0.0);
                    contentMaxX = Math.max(contentMaxX, cx + cw);
                    contentMaxY = Math.max(contentMaxY, cy + ch);
                }
            }

            reconcileChildren(parentGroup, desiredChildren, desiredSet, managedViews);

            if (kind == COMP_SCROLL_VIEW) {
                float viewW = (float) snap.optDouble("w", 0.0);
                float viewH = (float) snap.optDouble("h", 0.0);
                int innerW = dpToPx(Math.max(viewW, contentMaxX));
                int innerH = dpToPx(Math.max(viewH, contentMaxY));
                ViewGroup.LayoutParams existingLp = parentGroup.getLayoutParams();
                if (existingLp == null || existingLp.width != innerW || existingLp.height != innerH) {
                    parentGroup.setLayoutParams(new FrameLayout.LayoutParams(innerW, innerH));
                }
            }
        }

        // Detach any live views that are no longer referenced by any parent's children list.
        for (Map.Entry<Integer, View> entry : nodeViews.entrySet()) {
            if (!mountedChildIds.contains(entry.getKey())) {
                detachFromParent(entry.getValue());
            }
        }
    }

    private void applyRootStyle(JSONObject snap) {
        int bgRgba = (int) snap.optLong("bg", 0L);
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
        if (kind == COMP_SCROLL_VIEW) {
            ScrollView scrollView = new ScrollView(this);
            scrollView.setFillViewport(false);
            FrameLayout inner = new FrameLayout(this);
            inner.setClipChildren(false);
            scrollView.addView(inner, new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT));
            scrollInnerContainers.put(nodeId, inner);
            return scrollView;
        }

        if (kind == COMP_TEXT_INPUT) {
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

        if (kind == COMP_TEXT && !hasChildren) {
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
            JSONObject snap) {
        final float xDp = (float) snap.optDouble("x", 0.0);
        final float yDp = (float) snap.optDouble("y", 0.0);
        final float wDp = Math.max(0.0f, (float) snap.optDouble("w", 0.0));
        final float hDp = Math.max(0.0f, (float) snap.optDouble("h", 0.0));

        final int leftPx = dpToPx(xDp);
        final int topPx = dpToPx(yDp);
        final int widthPx = dpToPx(wDp);
        final int heightPx = dpToPx(hDp);

        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(widthPx, heightPx);
        lp.leftMargin = leftPx;
        lp.topMargin = topPx;
        lp.gravity = Gravity.TOP | Gravity.START;
        view.setLayoutParams(lp);

        final int bgRgba = (int) snap.optLong("bg", 0L);
        final int fgRgba = (int) snap.optLong("fg", 0L);
        final int bcRgba = (int) snap.optLong("bc", 0L);
        final float bwDp = Math.max(0.0f, (float) snap.optDouble("bw", 0.0));
        final float brDp = Math.max(0.0f, (float) snap.optDouble("br", 0.0));
        final float opacity = (float) snap.optDouble("op", 1.0);
        final float fontSizeDp = (float) snap.optDouble("fs", 16.0);
        final int zIndex = snap.optInt("z", 0);
        final long eventMask = snap.optLong("em", 0L);

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

        if (kind == COMP_TEXT_INPUT && view instanceof EditText) {
            EditText editText = (EditText) view;
            String text = snap.optString("text", "");
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
        } else if (kind == COMP_TEXT && !hasChildren && view instanceof TextView) {
            TextView tv = (TextView) view;
            String text = snap.optString("text", "");
            if (!text.equals(tv.getText().toString())) {
                tv.setText(text);
            }
            tv.setTextColor(fgRgba != 0 ? rgbaToArgb(fgRgba) : Color.BLACK);
            tv.setTextSize(TypedValue.COMPLEX_UNIT_DIP, fontSizeDp > 0.0f ? fontSizeDp : 16.0f);
        }

        final boolean interactive = (kind == COMP_PRESSABLE || eventMask != 0L);
        if (interactive) {
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
        } else if (kind != COMP_TEXT_INPUT && kind != COMP_SCROLL_VIEW) {
            view.setOnClickListener(null);
            view.setClickable(false);
        }
    }

    private static void reconcileChildren(
            ViewGroup parent,
            List<View> desiredChildren,
            Set<View> desiredSet,
            Set<View> managedViews) {
        // Remove managed children that no longer belong to this parent.
        for (int i = parent.getChildCount() - 1; i >= 0; i--) {
            View child = parent.getChildAt(i);
            if (managedViews.contains(child) && !desiredSet.contains(child)) {
                parent.removeViewAt(i);
            }
        }

        // Check whether existing managed child order already matches `desiredChildren`.
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
