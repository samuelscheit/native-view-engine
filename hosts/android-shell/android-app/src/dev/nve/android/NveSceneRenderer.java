package dev.nve.android;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.Gravity;
import android.view.KeyEvent;
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

import dev.nve.host.FrameHost;
import dev.nve.host.ImageSource;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.RuntimeSurface;
import dev.nve.host.SceneSemantics;
import dev.nve.host.SceneSemantics.Kind;
import dev.nve.host.Utf8;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveModuleTables;
import dev.nve.host.generated.NveTables;

import java.io.InputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

/**
 * Reconciles each runtime frame ({@link RuntimeSceneReader}) into native Android Views: one View
 * per live node, kinds realized by {@link SceneSemantics}, styles from `nve.visual` /
 * `nve.typography` / `nve.accessibility`, parent-relative positions, and user input sent back as
 * `EventRing` events.
 *
 * <p>O(changed): a frame that directly follows the last applied one ({@link
 * RuntimeSceneReader#FRAME_NEXT}) touches only its removed ids, content-dirty nodes (the runtime
 * includes text inheriting a changed typography), parents whose children changed or whose child
 * view was re-created, moved nodes (geometry only), and the views bound to its dirty resource ids
 * (indexed by resource id). A redelivered frame is a no-op. A new scene epoch drops every view
 * first (ids and resources of another epoch are unrelated), and any other gap reconciles all live
 * nodes. Listener callbacks that fire while a frame is applied (focus, scroll, text changes caused
 * by the renderer itself) are not user input and never reach the app.
 */
final class NveSceneRenderer implements RuntimeSurface.FrameConsumer {
    private static final class Node {
        final int id;
        final Kind kind;
        final View view;
        /** Children host (the scroll content for scroll containers, else the view itself). */
        final ViewGroup content;
        /** `nve.scroll.axes` the scroll view was built for. */
        final int axes;
        /** Runtime scroll offset (dp) last applied to a scroll view. */
        float scrollX;
        float scrollY;
        final NveViewStyle.ViewState style = new NveViewStyle.ViewState();
        int resourceId;
        float x;
        float y;
        float w = -1f;
        float h = -1f;

        Node(int id, Kind kind, View view, ViewGroup content, int axes) {
            this.id = id;
            this.kind = kind;
            this.view = view;
            this.content = content;
            this.axes = axes;
        }
    }

    private final Context context;
    private final FrameLayout root;
    private final float density;
    private final NveViewStyle.ViewState rootStyle = new NveViewStyle.ViewState();
    private final Map<Integer, Node> nodes = new HashMap<>();
    private final Map<View, Node> byView = new IdentityHashMap<>();
    /** Image / canvas nodes by the resource id they show: dirty resources rebind in O(dirty). */
    private final Map<Integer, List<Node>> byResource = new HashMap<>();
    private final SceneSemantics.Tables tables = new SceneSemantics.Tables();
    private final SceneSemantics.TextStyle textStyle = new SceneSemantics.TextStyle();
    private final List<View> desired = new ArrayList<>();
    /** Scroll nodes whose applied runtime offset is non-zero (reset when they leave `scroll_offsets`). */
    private final List<Node> scrolled = new ArrayList<>();
    /** Node indexes whose child list must be reconciled this frame. */
    private int[] relink = new int[16];
    private int relinkCount;
    /** True while a frame is applied: listener callbacks are renderer-caused, not user input. */
    private boolean reconciling;
    private RuntimeSceneReader frame;
    private long lastFrameId;
    private int lastEpoch;

    NveSceneRenderer(Context context, FrameLayout root, float density) {
        this.context = context;
        this.root = root;
        this.density = density;
    }

    @Override
    public void onFrame(RuntimeSceneReader s, RuntimeSurface surface) {
        apply(s, surface);
    }

    /** Applies frame {@code s}; {@code surface} serves its resources and receives the input. */
    void apply(RuntimeSceneReader s, FrameHost surface) {
        final int relation = s.relationTo(lastFrameId, lastEpoch);
        if (relation == RuntimeSceneReader.FRAME_SAME) return;
        reconciling = true;
        frame = s;
        try {
            tables.bind(s);
            if (relation == RuntimeSceneReader.FRAME_NEXT) {
                applyDelta(s, surface);
            } else {
                if (s.sceneEpoch() != lastEpoch) clear();
                applyAll(s, surface);
            }
            applyScrollOffsets(s);
            lastFrameId = s.frameId();
            lastEpoch = s.sceneEpoch();
        } finally {
            frame = null;
            reconciling = false;
        }
    }

    /** Every live node: removes views of dead ids, upserts, links, and places all. */
    private void applyAll(RuntimeSceneReader s, FrameHost surface) {
        final List<Integer> dead = new ArrayList<>();
        for (Node n : nodes.values()) {
            if (s.resolve(n.id) < 0) dead.add(n.id);
        }
        for (int id : dead) drop(id);
        for (int i = 1; i < s.nodeCapacity(); i++) upsert(s, surface, i, true);
        for (int i = 1; i < s.nodeCapacity(); i++) {
            if (s.isLive(i)) reconcileChildren(s, i);
        }
        for (int i = 1; i < s.nodeCapacity(); i++) {
            if (s.isLive(i)) place(s, i);
        }
    }

    /** Only what changed since the previous frame. */
    private void applyDelta(RuntimeSceneReader s, FrameHost surface) {
        for (int k = 0; k < s.removedCount(); k++) drop(s.removedId(k));
        relinkCount = 0;
        for (int d = 0; d < s.dirtyCount(); d++) {
            final int i = s.dirtyIndex(d);
            if (upsert(s, surface, i, false)) relink(parentIndex(s, i));  // a re-created view moves into its parent
            relink(i);
        }
        for (int k = 0; k < relinkCount; k++) reconcileChildren(s, relink[k]);
        for (int d = 0; d < s.dirtyCount(); d++) place(s, s.dirtyIndex(d));
        for (int m = 0; m < s.movedCount(); m++) place(s, s.movedIndex(m));
        for (int r = 0; r < s.dirtyResourceCount(); r++) rebindResource(s, surface, s.dirtyResource(r));
    }

    /**
     * Runtime / app scroll positions (`scroll_offsets`, O(scrolled containers)): scroll-to and
     * restore reach the views; a user scroll already synced through `scrollBy` is a no-op here.
     */
    private void applyScrollOffsets(RuntimeSceneReader s) {
        for (int k = scrolled.size() - 1; k >= 0; k--) {
            final Node n = scrolled.get(k);
            if (s.scrollIndexOf(n.id) < 0) scrollTo(n, 0f, 0f);
        }
        scrolled.clear();
        for (int k = 0; k < s.scrollCount(); k++) {
            final Node n = nodes.get(s.scrollNodeId(k));
            if (n == null || n.kind != Kind.SCROLL) continue;
            scrollTo(n, s.scrollX(n.id), s.scrollY(n.id));
            if (n.scrollX != 0f || n.scrollY != 0f) scrolled.add(n);
        }
    }

    private void scrollTo(Node n, float x, float y) {
        n.scrollX = x;
        n.scrollY = y;
        final View outer = n.view;
        if (n.axes == NveTables.Values.ScrollAxes.BOTH) {
            outer.scrollTo(0, px(y));
            ((ViewGroup) outer).getChildAt(0).scrollTo(px(x), 0);
        } else if (n.axes != NveTables.Values.ScrollAxes.NONE) {
            outer.scrollTo(px(x), px(y));
        }
    }

    private void relink(int index) {
        if (index <= 0) return;
        for (int k = 0; k < relinkCount; k++) {
            if (relink[k] == index) return;
        }
        if (relinkCount == relink.length) relink = java.util.Arrays.copyOf(relink, relinkCount * 2);
        relink[relinkCount++] = index;
    }

    /** Drops every view and binding (a new scene epoch, or the activity going away). */
    void clear() {
        for (Node n : nodes.values()) detach(n.view);
        nodes.clear();
        byView.clear();
        byResource.clear();
        scrolled.clear();
        lastFrameId = 0L;
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

    /** The view realizing {@code nodeId}, or {@code null} (tests, accessibility tooling). */
    View viewOf(int nodeId) {
        final Node n = nodes.get(nodeId);
        return n == null ? null : n.view;
    }

    private void drop(int nodeId) {
        final Node n = nodes.remove(nodeId);
        if (n == null) return;
        byView.remove(n.view);
        scrolled.remove(n);
        bindResource(n, 0);
        detach(n.view);
    }

    private static int parentIndex(RuntimeSceneReader s, int i) {
        final int parent = s.parent(i);
        return parent == 0 ? 0 : s.resolve(parent);
    }

    /** Creates or restyles node {@code i}; {@code true} when its view was (re-)created. */
    private boolean upsert(RuntimeSceneReader s, FrameHost surface, int i, boolean full) {
        if (!s.isLive(i)) return false;
        if (i == SceneSemantics.ROOT_INDEX) {
            NveViewStyle.applyBox(root, rootStyle, tables, s, i, density);
            return false;
        }
        final int id = s.nodeId(i);
        final Kind kind = SceneSemantics.kindOf(s, tables, i);
        final int axes = kind == Kind.SCROLL ? tables.scroll.u32(s.rowOf(tables.scroll, i), NveTables.Scroll.AXES) : 0;
        Node n = nodes.get(id);
        final boolean created = n == null || n.kind != kind || n.axes != axes;
        if (created) {
            if (n != null) drop(id);
            n = create(kind, axes, id, surface);
            nodes.put(id, n);
            byView.put(n.view, n);
        }
        NveViewStyle.applyBox(n.view, n.style, tables, s, i, density);
        final boolean pressable = SceneSemantics.pressable(s, tables, i);
        NveViewStyle.applyAccessibility(n.view, n.style, tables, s, i, pressable);
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
                NveViewStyle.applyInput(edit, n.style, tables, s, i);
                NveViewStyle.applyText(edit, textStyle.resolve(s, tables, i));
                final String value = s.text(tables.input, s.rowOf(tables.input, i), NveTables.TextInput.VALUE);
                if (!value.contentEquals(edit.getText())) {
                    // The app rewrote the value: keep the caret / selection where it was (clamped).
                    final int start = Math.min(Math.max(edit.getSelectionStart(), 0), value.length());
                    final int end = Math.min(Math.max(edit.getSelectionEnd(), 0), value.length());
                    edit.setText(value);
                    edit.setSelection(start, end);
                }
                break;
            }
            case IMAGE:
                bindImage((ImageView) n.view, n, s, i, surface, full || created);
                break;
            case CANVAS:
                bindCanvas((NveCanvasView) n.view, n, s, i, surface, full || created);
                break;
            default:
                break;
        }
        return created;
    }

    /** Re-reads the bytes of every image / canvas showing resource {@code id}. */
    private void rebindResource(RuntimeSceneReader s, FrameHost surface, int id) {
        final List<Node> users = byResource.get(id);
        if (users == null) return;
        for (Node n : new ArrayList<>(users)) {
            final int i = s.resolve(n.id);
            if (i <= 0) continue;
            if (n.kind == Kind.IMAGE) bindImage((ImageView) n.view, n, s, i, surface, true);
            if (n.kind == Kind.CANVAS) bindCanvas((NveCanvasView) n.view, n, s, i, surface, true);
        }
    }

    private void bindResource(Node n, int id) {
        if (n.resourceId == id) return;
        if (n.resourceId != 0) {
            final List<Node> users = byResource.get(n.resourceId);
            if (users != null && users.remove(n) && users.isEmpty()) byResource.remove(n.resourceId);
        }
        n.resourceId = id;
        if (id != 0) byResource.computeIfAbsent(id, k -> new ArrayList<>(1)).add(n);
    }

    private Node create(Kind kind, int axes, int id, FrameHost surface) {
        switch (kind) {
            case TEXT: {
                final TextView tv = new TextView(context);
                tv.setIncludeFontPadding(false);
                tv.setPadding(0, 0, 0, 0);
                return new Node(id, kind, tv, null, 0);
            }
            case INPUT:
                return new Node(id, kind, input(id, surface), null, 0);
            case IMAGE:
                return new Node(id, kind, new ImageView(context), null, 0);
            case CANVAS: {
                final NveCanvasView canvas = new NveCanvasView(context);
                canvas.setDensity(density);
                return new Node(id, kind, canvas, null, 0);
            }
            case SCROLL:
                return scroll(id, axes, surface);
            default: {
                final FrameLayout frame = new FrameLayout(context);
                frame.setClipChildren(false);
                return new Node(id, kind, frame, frame, 0);
            }
        }
    }

    private EditText input(int id, FrameHost surface) {
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
                final String text = Utf8.wellFormed(e);  // IMEs can leave lone surrogates
                surface.send(r -> r.text(NveAbi.EVENT_TEXT_CHANGE, id, now(), text));
            }
        });
        edit.setOnEditorActionListener((v, actionId, event) -> {
            if (reconciling || !isSubmit(actionId, event)) return false;
            final String text = Utf8.wellFormed(v.getText());
            surface.send(r -> r.text(NveAbi.EVENT_SUBMIT, id, now(), text));
            return true;
        });
        edit.setOnFocusChangeListener((v, focused) -> {
            if (reconciling) return;  // detaching / re-adding views moves focus without the user
            surface.send(r -> r.empty(focused ? NveAbi.EVENT_FOCUS : NveAbi.EVENT_BLUR, id, now()));
        });
        return edit;
    }

    /**
     * An IME action ({@code event == null}) or the ACTION_DOWN half of a hardware Enter
     * ({@code IME_NULL} arrives for both down and up): exactly one submit per press.
     */
    static boolean isSubmit(int actionId, KeyEvent event) {
        if (event == null) return actionId != EditorInfo.IME_ACTION_NONE && actionId != EditorInfo.IME_NULL;
        return event.getAction() == KeyEvent.ACTION_DOWN
                && (event.getKeyCode() == KeyEvent.KEYCODE_ENTER || event.getKeyCode() == KeyEvent.KEYCODE_NUMPAD_ENTER);
    }

    /**
     * Scroll container for `nve.scroll.axes`: vertical ({@link ScrollView}), horizontal
     * ({@link HorizontalScrollView}), both (a horizontal scroller inside a vertical one), or none
     * (a clipping frame the runtime may still scroll programmatically).
     */
    private Node scroll(int id, int axes, FrameHost surface) {
        final FrameLayout content = new FrameLayout(context);
        content.setClipChildren(false);
        final ViewGroup.LayoutParams wrap = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        final ViewGroup outer;
        if (axes == NveTables.Values.ScrollAxes.HORIZONTAL) {
            outer = new HorizontalScrollView(context);
            outer.addView(content, wrap);
            listen(outer, id, content, surface);
        } else if (axes == NveTables.Values.ScrollAxes.BOTH) {
            outer = new ScrollView(context);
            final HorizontalScrollView inner = new HorizontalScrollView(context);
            inner.addView(content, wrap);
            outer.addView(inner, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
            listen(outer, id, content, surface);
            listen(inner, id, content, surface);
        } else if (axes == NveTables.Values.ScrollAxes.VERTICAL) {
            outer = new ScrollView(context);
            outer.addView(content, wrap);
            listen(outer, id, content, surface);
        } else {
            outer = new FrameLayout(context);
            outer.setClipChildren(true);
            outer.addView(content, wrap);
        }
        return new Node(id, Kind.SCROLL, outer, content, axes);
    }

    private void listen(View scroller, int id, FrameLayout content, FrameHost surface) {
        scroller.setOnScrollChangeListener((v, x, y, oldX, oldY) -> {
            if (reconciling || (x == oldX && y == oldY)) return;  // content resizes clamp scroll too
            surface.scrollBy(id, (x - oldX) / density, (y - oldY) / density);
            final float cw = content.getWidth() / density;
            final float ch = content.getHeight() / density;
            final float sx = x / density;
            final float sy = y / density;
            surface.send(r -> r.scroll(NveAbi.EVENT_SCROLL, id, now(), sx, sy, cw, ch));
        });
    }

    private void bindPress(Node n, boolean pressable, FrameHost surface) {
        if (n.kind == Kind.INPUT || n.kind == Kind.SCROLL) return;
        if (!pressable) {
            n.view.setOnClickListener(null);
            n.view.setClickable(false);
            return;
        }
        n.view.setOnClickListener(v -> surface.send(r -> r.point(NveAbi.EVENT_PRESS, n.id, now(), n.w * 0.5f, n.h * 0.5f)));
    }

    /** Re-reads the image when its resource id changed or {@code force} (dirty bytes, new view). */
    private void bindImage(ImageView view, Node n, RuntimeSceneReader s, int i, FrameHost surface, boolean force) {
        final int row = s.rowOf(tables.image, i);
        view.setScaleType(NveViewStyle.scaleType(tables.image.u32(row, NveTables.Image.FIT)));
        final String alt = s.text(tables.image, row, NveTables.Image.ALT);
        if (view.getContentDescription() == null && !alt.isEmpty()) view.setContentDescription(alt);
        final int res = tables.image.u32(row, NveTables.Image.SRC);
        if (res == n.resourceId && !force && !s.isResourceDirty(res)) return;
        bindResource(n, res);
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

    private void bindCanvas(NveCanvasView view, Node n, RuntimeSceneReader s, int i, FrameHost surface, boolean force) {
        final int res = tables.canvas.u32(s.rowOf(tables.canvas, i), NveModuleTables.Canvas.DRAW_LIST);
        if (res == n.resourceId && !force && !s.isResourceDirty(res)) return;
        bindResource(n, res);
        final ByteBuffer bytes = res == 0 ? null : surface.resource(res);
        if (bytes == null) {
            view.clearDrawList();
        } else {
            view.setDrawList(bytes);  // copied: the frame's buffer dies at release
        }
    }

    private Bitmap decode(ImageSource src) {
        try {
            final byte[] inline = src.bytes();
            if (inline != null) return BitmapFactory.decodeByteArray(inline, 0, inline.length);
            final String url = src.url;
            if (url.startsWith("asset://")) {
                try (InputStream in = context.getAssets().open(url.substring("asset://".length()))) {
                    return BitmapFactory.decodeStream(in);
                }
            }
            if (url.startsWith("file://")) return BitmapFactory.decodeFile(url.substring("file://".length()));
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

    /**
     * Absolute rects → position relative to the parent's content (scroll content is unscrolled);
     * a scroll parent's content box follows its children.
     */
    private void place(RuntimeSceneReader s, int i) {
        if (i == SceneSemantics.ROOT_INDEX || !s.isLive(i)) return;
        final Node n = nodes.get(s.nodeId(i));
        if (n == null) return;
        final int parentIndex = parentIndex(s, i);
        final boolean nested = parentIndex > 0 && parentIndex != SceneSemantics.ROOT_INDEX;
        final float x = SceneSemantics.finite(s.x(i) - (nested ? s.x(parentIndex) : 0f), 0f);
        final float y = SceneSemantics.finite(s.y(i) - (nested ? s.y(parentIndex) : 0f), 0f);
        final float w = Math.max(0f, SceneSemantics.finite(s.width(i), 0f));
        final float h = Math.max(0f, SceneSemantics.finite(s.height(i), 0f));
        if (n.kind == Kind.SCROLL) sizeScrollContent(s, i, n, w, h);
        if (nested) {
            final Node parent = nodes.get(s.nodeId(parentIndex));
            if (parent != null && parent.kind == Kind.SCROLL) {
                sizeScrollContent(s, parentIndex, parent, Math.max(0f, s.width(parentIndex)), Math.max(0f, s.height(parentIndex)));
            }
        }
        if (x == n.x && y == n.y && w == n.w && h == n.h) return;
        n.x = x;
        n.y = y;
        n.w = w;
        n.h = h;
        final ViewGroup.LayoutParams current = n.view.getLayoutParams();
        final FrameLayout.LayoutParams lp = current instanceof FrameLayout.LayoutParams
                ? (FrameLayout.LayoutParams) current : new FrameLayout.LayoutParams(0, 0);
        lp.width = px(w);
        lp.height = px(h);
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
        if (lp == null || (lp.width == px(right) && lp.height == px(bottom))) return;
        lp.width = px(right);
        lp.height = px(bottom);
        n.content.setLayoutParams(lp);
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
