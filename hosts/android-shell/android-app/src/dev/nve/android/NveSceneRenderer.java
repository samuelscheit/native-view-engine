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

import dev.nve.android.generated.NveCoreAbi;
import dev.nve.android.generated.NveTables;
import dev.nve.host.ImageSource;
import dev.nve.host.NativeRuntime;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.RuntimeSurface;

import java.io.InputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;

/**
 * Reconciles each runtime frame ({@link RuntimeSceneReader}) into native Android Views: one View
 * per live node, kinds realized from table presence (`nve.text_input`, `nve.image`, `nve.canvas`,
 * `nve.scroll`, `nve.text` leaves, containers), styles from `nve.visual` / `nve.typography` /
 * `nve.accessibility`, absolute rects converted to parent-relative positions, and input sent back
 * as `EventRing` events.
 */
final class NveSceneRenderer implements RuntimeSurface.FrameConsumer {
    static final int ROOT_INDEX = 1;
    /** `EventMask::PRESS`. */
    static final int PRESS_MASK = 1 << (NveCoreAbi.EVENT_PRESS - 1);

    private enum Kind { CONTAINER, TEXT, INPUT, IMAGE, CANVAS, SCROLL }

    private static final class Node {
        final Kind kind;
        final View view;
        /** Children host (the scroll content for scroll containers, else the view itself). */
        final ViewGroup content;
        int resourceId;
        float x;
        float y;
        float w = -1f;
        float h = -1f;

        Node(Kind kind, View view, ViewGroup content) {
            this.kind = kind;
            this.view = view;
            this.content = content;
        }
    }

    private final Context context;
    private final FrameLayout root;
    private final float density;
    private final Map<Integer, Node> nodes = new HashMap<>();
    private final NveViewStyle.Tables tables = new NveViewStyle.Tables();
    private final NveViewStyle.TextStyle textStyle = new NveViewStyle.TextStyle();
    private final List<View> desired = new ArrayList<>();
    private boolean suppressTextEvents;
    private long lastFrameId = -1L;

    NveSceneRenderer(Context context, FrameLayout root, float density) {
        this.context = context;
        this.root = root;
        this.density = density;
    }

    @Override
    public void onFrame(RuntimeSceneReader s, RuntimeSurface surface) {
        tables.bind(s);
        final boolean full = lastFrameId < 0 || s.frameId() < lastFrameId;
        lastFrameId = s.frameId();
        for (int i = 0; i < s.removedCount(); i++) drop(s.removedId(i));
        if (full) {
            for (Iterator<Map.Entry<Integer, Node>> it = nodes.entrySet().iterator(); it.hasNext(); ) {
                final Map.Entry<Integer, Node> e = it.next();
                if (s.resolve(e.getKey()) < 0) {
                    detach(e.getValue().view);
                    it.remove();
                }
            }
            for (int i = 1; i < s.nodeCapacity(); i++) if (s.isLive(i)) upsert(s, surface, i);
        } else {
            for (int d = 0; d < s.dirtyCount(); d++) upsert(s, surface, s.dirtyIndex(d));
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
        lastFrameId = -1L;
    }

    private void drop(int nodeId) {
        final Node n = nodes.remove(nodeId);
        if (n != null) detach(n.view);
    }

    private Kind kindOf(RuntimeSceneReader s, int i) {
        if (s.rowOf(tables.input, i) >= 0) return Kind.INPUT;
        if (s.rowOf(tables.image, i) >= 0) return Kind.IMAGE;
        if (s.rowOf(tables.canvas, i) >= 0) return Kind.CANVAS;
        if (s.rowOf(tables.scroll, i) >= 0) return Kind.SCROLL;
        if (s.rowOf(tables.text, i) >= 0 && s.childCount(i) == 0) return Kind.TEXT;
        return Kind.CONTAINER;
    }

    private void upsert(RuntimeSceneReader s, RuntimeSurface surface, int i) {
        if (!s.isLive(i)) return;
        final int id = s.nodeId(i);
        if (i == ROOT_INDEX) {
            NveViewStyle.applyBox(root, tables, s, i, density);
            return;
        }
        final Kind kind = kindOf(s, i);
        Node n = nodes.get(id);
        if (n == null || n.kind != kind) {
            if (n != null) detach(n.view);
            n = create(kind, id, s, i, surface);
            nodes.put(id, n);
        }
        NveViewStyle.applyBox(n.view, tables, s, i, density);
        final int irow = s.rowOf(tables.interaction, i);
        final boolean pressable = irow >= 0 && (tables.interaction.u32(irow, NveTables.Interaction.EVENT_MASK) & PRESS_MASK) != 0
                && !tables.interaction.bool(irow, NveTables.Interaction.DISABLED);
        NveViewStyle.applyAccessibility(n.view, tables, s, i, pressable);
        bindPress(n, id, pressable, surface);
        switch (kind) {
            case TEXT: {
                final TextView tv = (TextView) n.view;
                final String text = s.text(tables.text, s.rowOf(tables.text, i), NveTables.Text.TEXT);
                if (!text.contentEquals(tv.getText())) tv.setText(text);
                NveViewStyle.applyText(tv, NveViewStyle.textStyle(tables, s, i, textStyle));
                break;
            }
            case INPUT: {
                final EditText edit = (EditText) n.view;
                NveViewStyle.applyInput(edit, tables, s, i);
                NveViewStyle.applyText(edit, NveViewStyle.textStyle(tables, s, i, textStyle));
                final String value = s.text(tables.input, s.rowOf(tables.input, i), NveTables.TextInput.VALUE);
                if (!value.contentEquals(edit.getText())) {
                    suppressTextEvents = true;
                    edit.setText(value);
                    edit.setSelection(value.length());
                    suppressTextEvents = false;
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

    private Node create(Kind kind, int id, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        switch (kind) {
            case TEXT: {
                final TextView tv = new TextView(context);
                tv.setIncludeFontPadding(false);
                tv.setPadding(0, 0, 0, 0);
                return new Node(kind, tv, null);
            }
            case INPUT:
                return new Node(kind, input(id, surface), null);
            case IMAGE:
                return new Node(kind, new ImageView(context), null);
            case CANVAS: {
                final NveCanvasView canvas = new NveCanvasView(context);
                canvas.setDensity(density);
                return new Node(kind, canvas, null);
            }
            case SCROLL:
                return scroll(id, s, i, surface);
            default: {
                final FrameLayout frame = new FrameLayout(context);
                frame.setClipChildren(false);
                return new Node(kind, frame, frame);
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
                if (suppressTextEvents) return;
                surface.events().text(NveCoreAbi.EVENT_TEXT_CHANGE, id, now(), e.toString());
                surface.dispatch();
            }
        });
        edit.setOnEditorActionListener((v, actionId, event) -> {
            surface.events().text(NveCoreAbi.EVENT_SUBMIT, id, now(), v.getText().toString());
            surface.dispatch();
            return true;
        });
        edit.setOnFocusChangeListener((v, focused) -> {
            surface.events().empty(focused ? NveCoreAbi.EVENT_FOCUS : NveCoreAbi.EVENT_BLUR, id, now());
            surface.dispatch();
        });
        return edit;
    }

    private Node scroll(int id, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        final int axes = tables.scroll.u32(s.rowOf(tables.scroll, i), NveTables.Scroll.AXES);
        final FrameLayout content = new FrameLayout(context);
        content.setClipChildren(false);
        final ViewGroup scroller = axes == NveTables.Scroll.AXES_HORIZONTAL ? new HorizontalScrollView(context) : new ScrollView(context);
        scroller.addView(content, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        final float[] xy = new float[2];
        scroller.setOnScrollChangeListener((v, x, y, oldX, oldY) -> {
            if (x == oldX && y == oldY) return;
            surface.scrollBy(id, (x - oldX) / density, (y - oldY) / density, xy);
            surface.events().scroll(NveCoreAbi.EVENT_SCROLL, id, now(), x / density, y / density,
                    content.getWidth() / density, content.getHeight() / density);
            surface.dispatch();
        });
        return new Node(Kind.SCROLL, scroller, content);
    }

    private void bindPress(Node n, int id, boolean pressable, RuntimeSurface surface) {
        if (n.kind == Kind.INPUT || n.kind == Kind.SCROLL) return;
        if (!pressable) {
            n.view.setOnClickListener(null);
            n.view.setClickable(false);
            return;
        }
        n.view.setOnClickListener(v -> {
            surface.events().point(NveCoreAbi.EVENT_PRESS, id, now(), n.w * 0.5f, n.h * 0.5f);
            surface.dispatch();
        });
    }

    private void bindImage(ImageView view, Node n, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        final int row = s.rowOf(tables.image, i);
        view.setScaleType(NveViewStyle.scaleType(tables.image.u32(row, NveTables.Image.FIT)));
        final String alt = s.text(tables.image, row, NveTables.Image.ALT);
        if (view.getContentDescription() == null && !alt.isEmpty()) view.setContentDescription(alt);
        final int res = tables.image.u32(row, NveTables.Image.SRC);
        if (res == n.resourceId) return;
        n.resourceId = res;
        final ByteBuffer bytes = res == 0 ? null : surface.resource(res);
        view.setImageBitmap(bytes != null && surface.resourceKind().equals(ImageSource.KIND) ? decode(ImageSource.decode(bytes)) : null);
    }

    private void bindCanvas(NveCanvasView view, Node n, RuntimeSceneReader s, int i, RuntimeSurface surface) {
        final int res = tables.canvas.u32(s.rowOf(tables.canvas, i), NveTables.Canvas.DRAW_LIST);
        final ByteBuffer bytes = res == 0 ? null : surface.resource(res);
        final boolean ephemeral = bytes != null && (surface.resourceFlags() & NativeRuntime.RESOURCE_FLAG_EPHEMERAL) != 0;
        if (res == n.resourceId && !ephemeral) return;
        n.resourceId = res;
        if (bytes == null || bytes.capacity() == 0) {
            view.clearDrawList();
        } else {
            view.setDrawListCopy(bytes, 0, bytes.capacity());  // the frame's buffer dies at release
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
        final ViewGroup host = i == ROOT_INDEX ? root : contentOf(s.nodeId(i));
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
        if (i == ROOT_INDEX) return;
        final Node n = nodes.get(s.nodeId(i));
        if (n == null) return;
        final int parentIndex = s.parent(i) == 0 ? 0 : s.resolve(s.parent(i));
        final float px = parentIndex > 0 && parentIndex != ROOT_INDEX ? s.x(parentIndex) : 0f;
        final float py = parentIndex > 0 && parentIndex != ROOT_INDEX ? s.y(parentIndex) : 0f;
        final float x = s.x(i) - px;
        final float y = s.y(i) - py;
        final float w = Math.max(0f, s.width(i));
        final float h = Math.max(0f, s.height(i));
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
        if (n.kind == Kind.SCROLL) sizeScrollContent(s, i, n);
    }

    private void sizeScrollContent(RuntimeSceneReader s, int i, Node n) {
        float right = s.width(i);
        float bottom = s.height(i);
        for (int k = 0; k < s.childCount(i); k++) {
            final int c = s.resolve(s.childId(i, k));
            if (c < 0) continue;
            right = Math.max(right, s.x(c) - s.x(i) + s.width(c));
            bottom = Math.max(bottom, s.y(c) - s.y(i) + s.height(c));
        }
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
