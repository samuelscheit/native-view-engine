package dev.nve.android;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.text.Editable;
import android.text.TextWatcher;
import android.util.Base64;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.HorizontalScrollView;
import android.widget.ImageView;
import android.widget.ScrollView;
import android.widget.TextView;

import dev.nve.host.ImageSource;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.RuntimeSurface;
import dev.nve.host.SceneSemantics;
import dev.nve.host.SceneSemantics.Kind;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveModuleTables;
import dev.nve.host.generated.NveTables;

import java.io.InputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Reconciles each runtime frame ({@link RuntimeSceneReader}) into native Android Views: one View
 * per live node, kinds realized by {@link SceneSemantics}, styles from `nve.visual` /
 * `nve.typography` / `nve.accessibility`, absolute rects converted to parent-relative positions,
 * and user input sent back as `EventRing` events.
 *
 * <p>Frames apply incrementally (dirty nodes + resources) only when they directly follow the last
 * one in the same scene epoch; any gap or reset re-reconciles the whole scene. Listener callbacks
 * that fire while a frame is being applied (focus, scroll, text changes caused by the renderer
 * itself) are not user input and never reach the app.
 */
final class NveSceneRenderer implements RuntimeSurface.FrameConsumer {
    private static final class Node {
        final int id;
        final Kind kind;
        final View view;
        /** Children host (the scroll content for scroll containers, else the view itself). */
        final ViewGroup content;
        int resourceId;
        float x;
        float y;
        float w = -1f;
        float h = -1f;

        Node(int id, Kind kind, View view, ViewGroup content) {
            this.id = id;
            this.kind = kind;
            this.view = view;
            this.content = content;
        }
    }

    private final Context context;
    private final FrameLayout root;
    private final float density;
    private final Map<Integer, Node> nodes = new HashMap<>();
    private final Map<View, Node> byView = new IdentityHashMap<>();
    private final SceneSemantics.Tables tables = new SceneSemantics.Tables();
    private final SceneSemantics.TextStyle textStyle = new SceneSemantics.TextStyle();
    private final Set<Integer> dirtyResources = new HashSet<>();
    private final List<View> desired = new ArrayList<>();
    /** True while a frame is applied: listener callbacks are renderer-caused, not user input. */
    private boolean reconciling;
    private long lastFrameId = -1L;
    private int lastEpoch;

    NveSceneRenderer(Context context, FrameLayout root, float density) {
        this.context = context;
        this.root = root;
        this.density = density;
    }

    @Override
    public void onFrame(RuntimeSceneReader s, RuntimeSurface surface) {
        reconciling = true;
        try {
            apply(s, surface);
        } finally {
            reconciling = false;
        }
    }

    private void apply(RuntimeSceneReader s, RuntimeSurface surface) {
        tables.bind(s);
        // Incremental only for the direct successor in the same epoch: a skipped frame's dirty
        // lists are lost, and a new epoch reuses ids for unrelated nodes and resources.
        final boolean full = lastFrameId < 0 || s.sceneEpoch() != lastEpoch || s.frameId() != lastFrameId + 1;
        lastFrameId = s.frameId();
        lastEpoch = s.sceneEpoch();
        dirtyResources.clear();
        for (int i = 0; i < s.dirtyResourceCount(); i++) dirtyResources.add(s.dirtyResource(i));
        for (int i = 0; i < s.removedCount(); i++) drop(s.removedId(i));
        if (full) {
            for (Iterator<Map.Entry<Integer, Node>> it = nodes.entrySet().iterator(); it.hasNext(); ) {
                final Node n = it.next().getValue();
                if (s.resolve(n.id) < 0) {
                    byView.remove(n.view);
                    detach(n.view);
                    it.remove();
                } else {
                    n.resourceId = 0;  // resource ids are only meaningful within one frame chain
                }
            }
            for (int i = 1; i < s.nodeCapacity(); i++) upsert(s, surface, i);
        } else {
            for (int d = 0; d < s.dirtyCount(); d++) upsert(s, surface, s.dirtyIndex(d));
            if (!dirtyResources.isEmpty()) rebindDirtyResources(s, surface);
            // Inherited typography: any style or structure change can restyle untouched descendants.
            if (s.dirtyCount() > 0 || s.removedCount() > 0) restyleText(s);
        }
        // Layout may move any node without dirtying it: place every live view (cheap when unchanged).
        for (int i = 1; i < s.nodeCapacity(); i++) {
            if (s.isLive(i)) {
                reconcileChildren(s, i);
                place(s, i);
            }
        }
    }

    void clear() {
        for (Node n : nodes.values()) detach(n.view);
        nodes.clear();
        byView.clear();
        lastFrameId = -1L;
    }

    /** Raw {@code NodeId} realized by {@code view} or its nearest realized ancestor ({@code 0} = none). */
    int nodeIdOf(View view) {
        for (View v = view; v != null; ) {
            final Node n = byView.get(v);
            if (n != null) return n.id;
            final ViewParent p = v.getParent();
            v = p instanceof View ? (View) p : null;
        }
        return 0;
    }

    private void drop(int nodeId) {
        final Node n = nodes.remove(nodeId);
        if (n != null) {
            byView.remove(n.view);
            detach(n.view);
        }
    }

    private void upsert(RuntimeSceneReader s, RuntimeSurface surface, int i) {
        if (!s.isLive(i)) return;
        final int id = s.nodeId(i);
        if (i == SceneSemantics.ROOT_INDEX) {
            NveViewStyle.applyBox(root, tables, s, i, density);
            return;
        }
        final Kind kind = SceneSemantics.kindOf(s, tables, i);
        Node n = nodes.get(id);
        if (n == null || n.kind != kind) {
            if (n != null) {
                byView.remove(n.view);
                detach(n.view);
            }
            n = create(kind, id, s, i, surface);
            nodes.put(id, n);
            byView.put(n.view, n);
        }
        NveViewStyle.applyBox(n.view, tables, s, i, density);
        final boolean pressable = SceneSemantics.pressable(s, tables, i);
        NveViewStyle.applyAccessibility(n.view, tables, s, i, pressable);
        bindPress(n, pressable, surface);
        switch (kind) {
            case TEXT: {
                final TextView tv = (TextView) n.view;
                final String text = s.text(tables.text, s.rowOf(tables.text, i), NveTables.Text.TEXT);
                if (!text.contentEquals(tv.getText())) tv.setText(text);
                NveViewStyle.applyText(tv, textStyle.resolve(s, tables, i));
                break;
            }
            case INPUT: {
                final EditText edit = (EditText) n.view;
                NveViewStyle.applyInput(edit, tables, s, i);
                NveViewStyle.applyText(edit, textStyle.resolve(s, tables, i));
                final String value = s.text(tables.input, s.rowOf(tables.input, i), NveTables.TextInput.VALUE);
                if (!value.contentEquals(edit.getText())) {
                    edit.setText(value);
                    edit.setSelection(value.length());
                }
                break;
            }
            case IMAGE:
                bindImage((ImageView) n.view, n, s, i, surface);
                break;
            case CANVAS:
                bindCanvas((NveCanvasView) n.view, n, s, i, surface);
                break;
            default:
                break;
        }
    }

    private void restyleText(RuntimeSceneReader s) {
        for (Node n : nodes.values()) {
            if (n.kind != Kind.TEXT && n.kind != Kind.INPUT) continue;
            final int i = s.resolve(n.id);
            if (i > 0) NveViewStyle.applyText((TextView) n.view, textStyle.resolve(s, tables, i));
        }
    }

    /** Images / canvases whose resource bytes changed in place (recycled or ephemeral ids). */
    private void rebindDirtyResources(RuntimeSceneReader s, RuntimeSurface surface) {
        for (Node n : nodes.values()) {
            if (n.resourceId == 0 || !dirtyResources.contains(n.resourceId)) continue;
            final int i = s.resolve(n.id);
            if (i <= 0) continue;
            if (n.kind == Kind.IMAGE) bindImage((ImageView) n.view, n, s, i, surface);
            if (n.kind == Kind.CANVAS) bindCanvas((NveCanvasView) n.view, n, s, i, surface);
        }
    }

    private Node create(Kind kind, int id, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        switch (kind) {
            case TEXT: {
                final TextView tv = new TextView(context);
                tv.setIncludeFontPadding(false);
                tv.setPadding(0, 0, 0, 0);
                return new Node(id, kind, tv, null);
            }
            case INPUT:
                return new Node(id, kind, input(id, surface), null);
            case IMAGE:
                return new Node(id, kind, new ImageView(context), null);
            case CANVAS: {
                final NveCanvasView canvas = new NveCanvasView(context);
                canvas.setDensity(density);
                return new Node(id, kind, canvas, null);
            }
            case SCROLL:
                return scroll(id, s, i, surface);
            default: {
                final FrameLayout frame = new FrameLayout(context);
                frame.setClipChildren(false);
                return new Node(id, kind, frame, frame);
            }
        }
    }

    private EditText input(int id, RuntimeSurface surface) {
        final EditText edit = new EditText(context);
        edit.setIncludeFontPadding(false);
        edit.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        edit.setGravity(Gravity.CENTER_VERTICAL | Gravity.START);
        edit.addTextChangedListener(new TextWatcher() {
            @Override
            public void beforeTextChanged(CharSequence s, int start, int count, int after) {}

            @Override
            public void onTextChanged(CharSequence s, int start, int before, int count) {}

            @Override
            public void afterTextChanged(Editable e) {
                if (reconciling) return;
                final String text = e.toString();
                surface.send(r -> r.text(NveAbi.EVENT_TEXT_CHANGE, id, now(), text));
            }
        });
        edit.setOnEditorActionListener((v, actionId, event) -> {
            final String text = v.getText().toString();
            surface.send(r -> r.text(NveAbi.EVENT_SUBMIT, id, now(), text));
            return true;
        });
        edit.setOnFocusChangeListener((v, focused) -> {
            if (reconciling) return;  // detaching / re-adding views moves focus without the user
            surface.send(r -> r.empty(focused ? NveAbi.EVENT_FOCUS : NveAbi.EVENT_BLUR, id, now()));
        });
        return edit;
    }

    private Node scroll(int id, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        final int axes = tables.scroll.u32(s.rowOf(tables.scroll, i), NveTables.Scroll.AXES);
        final FrameLayout content = new FrameLayout(context);
        content.setClipChildren(false);
        final ViewGroup scroller = axes == NveTables.Scroll.AXES_HORIZONTAL ? new HorizontalScrollView(context) : new ScrollView(context);
        scroller.addView(content, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        scroller.setOnScrollChangeListener((v, x, y, oldX, oldY) -> {
            if (reconciling || (x == oldX && y == oldY)) return;  // content resizes clamp scroll too
            surface.scrollBy(id, (x - oldX) / density, (y - oldY) / density);
            final float cw = content.getWidth() / density;
            final float ch = content.getHeight() / density;
            surface.send(r -> r.scroll(NveAbi.EVENT_SCROLL, id, now(), x / density, y / density, cw, ch));
        });
        return new Node(id, Kind.SCROLL, scroller, content);
    }

    private void bindPress(Node n, boolean pressable, RuntimeSurface surface) {
        if (n.kind == Kind.INPUT || n.kind == Kind.SCROLL) return;
        if (!pressable) {
            n.view.setOnClickListener(null);
            n.view.setClickable(false);
            return;
        }
        n.view.setOnClickListener(v -> surface.send(r -> r.point(NveAbi.EVENT_PRESS, n.id, now(), n.w * 0.5f, n.h * 0.5f)));
    }

    private void bindImage(ImageView view, Node n, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        final int row = s.rowOf(tables.image, i);
        view.setScaleType(NveViewStyle.scaleType(tables.image.u32(row, NveTables.Image.FIT)));
        final String alt = s.text(tables.image, row, NveTables.Image.ALT);
        if (view.getContentDescription() == null && !alt.isEmpty()) view.setContentDescription(alt);
        final int res = tables.image.u32(row, NveTables.Image.SRC);
        if (res == n.resourceId && !dirtyResources.contains(res)) return;
        n.resourceId = res;
        final ByteBuffer bytes = res == 0 ? null : surface.resource(res);
        Bitmap bitmap = null;
        if (bytes != null && surface.resourceKind().equals(ImageSource.KIND)) {
            try {
                bitmap = decode(ImageSource.decode(bytes));
            } catch (RuntimeException corrupt) {
                bitmap = null;  // a malformed source renders empty, like a broken <img>
            }
        }
        view.setImageBitmap(bitmap);
    }

    private void bindCanvas(NveCanvasView view, Node n, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        final int res = tables.canvas.u32(s.rowOf(tables.canvas, i), NveModuleTables.Canvas.DRAW_LIST);
        if (res == n.resourceId && !dirtyResources.contains(res)) return;
        n.resourceId = res;
        final ByteBuffer bytes = res == 0 ? null : surface.resource(res);
        if (bytes == null || bytes.remaining() == 0) {
            view.clearDrawList();
        } else {
            view.setDrawListCopy(bytes, bytes.position(), bytes.remaining());  // the frame's buffer dies at release
        }
    }

    private Bitmap decode(ImageSource src) {
        try {
            if (src.encoded != null) {
                final byte[] b = new byte[src.encoded.remaining()];
                src.encoded.duplicate().get(b);
                return BitmapFactory.decodeByteArray(b, 0, b.length);
            }
            final String url = src.url;
            if (url.startsWith("asset://")) {
                try (InputStream in = context.getAssets().open(url.substring("asset://".length()))) {
                    return BitmapFactory.decodeStream(in);
                }
            }
            if (url.startsWith("file://")) return BitmapFactory.decodeFile(url.substring("file://".length()));
            final int comma = url.indexOf(',');
            if (url.startsWith("data:image/") && comma > 0 && url.substring(0, comma).endsWith(";base64")) {
                final byte[] b = Base64.decode(url.substring(comma + 1), Base64.DEFAULT);
                return BitmapFactory.decodeByteArray(b, 0, b.length);
            }
        } catch (Exception e) {
            return null;  // an unreadable image renders empty, like a broken <img>
        }
        return null;
    }

    private void reconcileChildren(RuntimeSceneReader s, int i) {
        final ViewGroup host = i == SceneSemantics.ROOT_INDEX ? root : contentOf(s.nodeId(i));
        if (host == null) return;
        desired.clear();
        for (int k = 0; k < s.childCount(i); k++) {
            final Node child = nodes.get(s.childId(i, k));
            if (child != null) desired.add(child.view);
        }
        boolean same = host.getChildCount() == desired.size();
        for (int k = 0; same && k < desired.size(); k++) same = host.getChildAt(k) == desired.get(k);
        if (same) return;
        for (int k = 0; k < desired.size(); k++) {
            final View v = desired.get(k);
            if (k < host.getChildCount() && host.getChildAt(k) == v) continue;
            detach(v);
            host.addView(v, Math.min(k, host.getChildCount()));
        }
        while (host.getChildCount() > desired.size()) host.removeViewAt(host.getChildCount() - 1);
    }

    private ViewGroup contentOf(int nodeId) {
        final Node n = nodes.get(nodeId);
        return n == null ? null : n.content;
    }

    /** Absolute rects → position relative to the parent's content (scroll content is unscrolled). */
    private void place(RuntimeSceneReader s, int i) {
        if (i == SceneSemantics.ROOT_INDEX) return;
        final Node n = nodes.get(s.nodeId(i));
        if (n == null) return;
        final int parentIndex = s.parent(i) == 0 ? 0 : s.resolve(s.parent(i));
        final boolean nested = parentIndex > 0 && parentIndex != SceneSemantics.ROOT_INDEX;
        final float x = SceneSemantics.finite(s.x(i) - (nested ? s.x(parentIndex) : 0f), 0f);
        final float y = SceneSemantics.finite(s.y(i) - (nested ? s.y(parentIndex) : 0f), 0f);
        final float w = Math.max(0f, SceneSemantics.finite(s.width(i), 0f));
        final float h = Math.max(0f, SceneSemantics.finite(s.height(i), 0f));
        // Scroll content follows its children, which can move without moving the scroller.
        if (n.kind == Kind.SCROLL) sizeScrollContent(s, i, n, w, h);
        if (x == n.x && y == n.y && w == n.w && h == n.h) return;
        n.x = x;
        n.y = y;
        n.w = w;
        n.h = h;
        final FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(px(w), px(h));
        lp.leftMargin = px(x);
        lp.topMargin = px(y);
        lp.gravity = Gravity.TOP | Gravity.START;
        n.view.setLayoutParams(lp);
    }

    private void sizeScrollContent(RuntimeSceneReader s, int i, Node n, float w, float h) {
        float right = w;
        float bottom = h;
        for (int k = 0; k < s.childCount(i); k++) {
            final int c = s.resolve(s.childId(i, k));
            if (c < 0) continue;
            right = Math.max(right, SceneSemantics.finite(s.x(c) - s.x(i) + s.width(c), 0f));
            bottom = Math.max(bottom, SceneSemantics.finite(s.y(c) - s.y(i) + s.height(c), 0f));
        }
        final ViewGroup.LayoutParams lp = n.content.getLayoutParams();
        if (lp != null && lp.width == px(right) && lp.height == px(bottom)) return;
        n.content.setLayoutParams(new FrameLayout.LayoutParams(px(right), px(bottom)));
    }

    private int px(float dp) {
        return Float.isFinite(dp) ? Math.round(dp * density) : 0;
    }

    private static void detach(View v) {
        final ViewParent p = v.getParent();
        if (p instanceof ViewGroup) ((ViewGroup) p).removeView(v);
    }

    static long now() {
        return System.nanoTime() / 1000L;
    }
}
