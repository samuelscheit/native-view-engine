package dev.nve.android;

import android.content.Context;
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
import dev.nve.host.SceneReader;
import dev.nve.host.RuntimeSurface;
import dev.nve.host.SceneSemantics;
import dev.nve.host.SceneSemantics.Kind;
import dev.nve.host.Utf8;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveTables;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

/**
 * Reconciles each runtime frame ({@link SceneReader}) into native Android Views: one View
 * per live node, kinds realized by {@link SceneSemantics}, styles from `nve.visual` /
 * `nve.typography` / `nve.accessibility`, parent-relative positions, and user input sent back as
 * `EventRing` events.
 *
 * <p>O(changed): a frame that directly follows the last applied one ({@link
 * SceneReader#FRAME_NEXT}) touches only its removed ids, content-dirty nodes (the runtime
 * includes text inheriting a changed typography), parents whose children changed or whose child
 * view was re-created, moved nodes (geometry only), and the views bound to its dirty resource ids
 * (indexed by resource id). A redelivered frame is a no-op. A new scene epoch drops every view
 * first (ids and resources of another epoch are unrelated), and any other gap reconciles all live
 * nodes. Per-frame work is deduplicated with O(1) {@link #pass} stamps: a parent's children are
 * relinked, a scroll parent's content is sized, and a resource's bytes are decoded at most once
 * per frame. Listener callbacks that fire while a frame is applied (focus, scroll, text changes
 * caused by the renderer itself) are not user input and never reach the app.
 */
final class NveSceneRenderer implements RuntimeSurface.FrameConsumer {
    private final Context context;
    private final FrameLayout root;
    private final float density;
    private final NveViewStyle.ViewState rootStyle = new NveViewStyle.ViewState();
    private final Map<Integer, NveViewNode> nodes = new HashMap<>();
    private final Map<View, NveViewNode> byView = new IdentityHashMap<>();
    /** Image / canvas views by the resource id they show: dirty resources rebind in O(users). */
    private final NveResourceViews resources;
    private final SceneSemantics.Tables tables = new SceneSemantics.Tables();
    private final SceneSemantics.TextStyle textStyle = new SceneSemantics.TextStyle();
    private final List<View> desired = new ArrayList<>();
    /** Scroll nodes whose applied runtime offset is non-zero (reset when they leave `scroll_offsets`). */
    private final List<NveViewNode> scrolled = new ArrayList<>();
    /** Node indexes whose child list must be reconciled this frame ({@code relinkPass}: by index). */
    private int[] relink = new int[16];
    private int relinkCount;
    private int[] relinkPass = new int[16];
    /** Scroll nodes whose content box must be re-sized this frame (once each, after placing). */
    private final List<NveViewNode> sizeScrolls = new ArrayList<>();
    /** Counts applied frames: per-frame "done already" stamps compare against it. */
    private int pass;
    /** True while a frame is applied: listener callbacks are renderer-caused, not user input. */
    private boolean reconciling;
    private long lastFrameId;
    private int lastEpoch;

    NveSceneRenderer(Context context, FrameLayout root, float density) {
        this.context = context;
        this.root = root;
        this.density = density;
        this.resources = new NveResourceViews(context);
    }

    @Override
    public void onFrame(SceneReader s, RuntimeSurface surface) {
        apply(s, surface);
    }

    /** Applies frame {@code s}; {@code surface} serves its resources and receives the input. */
    void apply(SceneReader s, FrameHost surface) {
        final int relation = s.relationTo(lastFrameId, lastEpoch);
        if (relation == SceneReader.FRAME_SAME) return;
        if (++pass == 0) {  // wrapped: no stale stamp may equal the new pass
            pass = 1;
            Arrays.fill(relinkPass, 0);
            for (NveViewNode n : nodes.values()) n.boundPass = n.sizedPass = 0;
        }
        reconciling = true;
        try {
            tables.bind(s);
            if (relation == SceneReader.FRAME_NEXT) {
                applyDelta(s, surface);
            } else {
                if (s.sceneEpoch() != lastEpoch) clear();
                applyAll(s, surface);
            }
            applyScrollOffsets(s);
            lastFrameId = s.frameId();
            lastEpoch = s.sceneEpoch();
        } finally {
            reconciling = false;
        }
    }

    /** Every live node: removes views of dead ids, upserts, links, and places all. */
    private void applyAll(SceneReader s, FrameHost surface) {
        final List<Integer> dead = new ArrayList<>();
        for (NveViewNode n : nodes.values()) {
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
        sizeScrollContents(s);
    }

    /** Only what changed since the previous frame. */
    private void applyDelta(SceneReader s, FrameHost surface) {
        for (int k = 0; k < s.removedCount(); k++) drop(s.removedId(k));
        relinkCount = 0;
        for (int d = 0; d < s.dirtyCount(); d++) {
            final int i = s.dirtyIndex(d);
            if (upsert(s, surface, i, false)) relink(s, parentIndex(s, i));  // a re-created view moves into its parent
            relink(s, i);
        }
        for (int k = 0; k < relinkCount; k++) reconcileChildren(s, relink[k]);
        for (int d = 0; d < s.dirtyCount(); d++) place(s, s.dirtyIndex(d));
        for (int m = 0; m < s.movedCount(); m++) place(s, s.movedIndex(m));
        sizeScrollContents(s);
        for (int r = 0; r < s.dirtyResourceCount(); r++) resources.rebind(s, tables, surface, s.dirtyResource(r), pass);
    }

    /**
     * Runtime / app scroll positions (`scroll_offsets`, O(scrolled containers)): scroll-to and
     * restore reach the views; a user scroll already synced through `scrollBy` is a no-op here.
     */
    private void applyScrollOffsets(SceneReader s) {
        for (int k = scrolled.size() - 1; k >= 0; k--) {
            final NveViewNode n = scrolled.get(k);
            if (s.scrollIndexOf(n.id) < 0) scrollTo(n, 0f, 0f);
        }
        scrolled.clear();
        for (int k = 0; k < s.scrollCount(); k++) {
            final NveViewNode n = nodes.get(s.scrollNodeId(k));
            if (n == null || n.kind != Kind.SCROLL) continue;
            scrollTo(n, s.scrollX(n.id), s.scrollY(n.id));
            if (n.scrollX != 0f || n.scrollY != 0f) scrolled.add(n);
        }
    }

    /**
     * Shows runtime offset {@code (x, y)}. A view not laid out yet (new, or its content box just
     * re-sized) would clamp it against stale bounds, so it is applied after the next layout
     * instead; either way the scroll it causes is not user input.
     */
    private void scrollTo(NveViewNode n, float x, float y) {
        n.scrollX = x;
        n.scrollY = y;
        final View outer = n.view;
        if (outer.isLaidOut() && !outer.isLayoutRequested()) {
            showScroll(n);
            return;
        }
        if (n.scrollPending) return;  // the pending layout applies the latest offset
        n.scrollPending = true;
        outer.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            @Override
            public void onLayoutChange(View v, int l, int t, int r, int b, int ol, int ot, int or, int ob) {
                v.removeOnLayoutChangeListener(this);
                n.scrollPending = false;
                if (nodes.get(n.id) != n) return;  // dropped meanwhile
                final boolean wasReconciling = reconciling;
                reconciling = true;
                try {
                    showScroll(n);
                } finally {
                    reconciling = wasReconciling;
                }
            }
        });
    }

    /**
     * Every axis setting scrolls programmatically: `NONE` is a clipping frame ({@code overflow:
     * hidden}), which a plain {@link View#scrollTo} shifts like a scroll-to on the web.
     */
    private void showScroll(NveViewNode n) {
        final View outer = n.view;
        if (n.axes == NveTables.Values.ScrollAxes.BOTH) {
            outer.scrollTo(0, px(n.scrollY));
            ((ViewGroup) outer).getChildAt(0).scrollTo(px(n.scrollX), 0);
        } else {
            outer.scrollTo(px(n.scrollX), px(n.scrollY));
        }
    }

    /** Queues node {@code index}'s child list for this frame (once; leaves have none to link). */
    private void relink(SceneReader s, int index) {
        if (index <= 0) return;
        if (index != SceneSemantics.ROOT_INDEX) {
            final NveViewNode n = nodes.get(s.nodeId(index));
            if (n == null || n.content == null) return;
        }
        if (index >= relinkPass.length) relinkPass = Arrays.copyOf(relinkPass, Math.max(index + 1, relinkPass.length * 2));
        if (relinkPass[index] == pass) return;
        relinkPass[index] = pass;
        if (relinkCount == relink.length) relink = Arrays.copyOf(relink, relinkCount * 2);
        relink[relinkCount++] = index;
    }

    /** Drops every view and binding (a new scene epoch, or the activity going away). */
    void clear() {
        for (NveViewNode n : nodes.values()) detach(n.view);
        nodes.clear();
        byView.clear();
        resources.clear();
        scrolled.clear();
        lastFrameId = 0L;
    }

    /** Raw {@code NodeId} realized by {@code view} or its nearest realized ancestor ({@code 0} = none). */
    int nodeIdOf(View view) {
        for (View v = view; v != null; ) {
            final NveViewNode n = byView.get(v);
            if (n != null) return n.id;
            final ViewParent p = v.getParent();
            v = p instanceof View ? (View) p : null;
        }
        return 0;
    }

    /** The view realizing {@code nodeId}, or {@code null} (tests, accessibility tooling). */
    View viewOf(int nodeId) {
        final NveViewNode n = nodes.get(nodeId);
        return n == null ? null : n.view;
    }

    private void drop(int nodeId) {
        final NveViewNode n = nodes.remove(nodeId);
        if (n == null) return;
        byView.remove(n.view);
        scrolled.remove(n);
        resources.unbind(n);
        detach(n.view);
    }

    private static int parentIndex(SceneReader s, int i) {
        final int parent = s.parent(i);
        return parent == 0 ? 0 : s.resolve(parent);
    }

    /** Creates or restyles node {@code i}; {@code true} when its view was (re-)created. */
    private boolean upsert(SceneReader s, FrameHost surface, int i, boolean full) {
        if (!s.isLive(i)) return false;
        if (i == SceneSemantics.ROOT_INDEX) {
            NveViewStyle.applyBox(root, rootStyle, tables, s, i, density);
            return false;
        }
        final int id = s.nodeId(i);
        final Kind kind = SceneSemantics.kindOf(s, tables, i);
        final int axes = kind == Kind.SCROLL ? tables.scroll.u32(s.rowOf(tables.scroll, i), NveTables.Scroll.AXES) : 0;
        NveViewNode n = nodes.get(id);
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
                n.value = value;
                showValue(edit, value);
                break;
            }
            case IMAGE:
                resources.bindImage(n, s, tables, i, surface, full || created, pass);
                break;
            case CANVAS:
                resources.bindCanvas(n, s, tables, i, surface, full || created, pass);
                break;
            default:
                break;
        }
        return created;
    }

    /** Shows {@code value} unless already shown, keeping the caret / selection (clamped). */
    private static void showValue(EditText edit, String value) {
        if (value.contentEquals(edit.getText())) return;
        final int start = Math.min(Math.max(edit.getSelectionStart(), 0), value.length());
        final int end = Math.min(Math.max(edit.getSelectionEnd(), 0), value.length());
        edit.setText(value);
        edit.setSelection(start, end);
    }

    private NveViewNode create(Kind kind, int axes, int id, FrameHost surface) {
        switch (kind) {
            case TEXT: {
                final TextView tv = new TextView(context);
                tv.setIncludeFontPadding(false);
                tv.setPadding(0, 0, 0, 0);
                return new NveViewNode(id, kind, tv, null, 0);
            }
            case INPUT:
                return new NveViewNode(id, kind, input(id, surface), null, 0);
            case IMAGE:
                return new NveViewNode(id, kind, new ImageView(context), null, 0);
            case CANVAS: {
                final NveCanvasView canvas = new NveCanvasView(context);
                canvas.setDensity(density);
                return new NveViewNode(id, kind, canvas, null, 0);
            }
            case SCROLL:
                return scroll(id, axes, surface);
            default: {
                final FrameLayout frame = new FrameLayout(context);
                frame.setClipChildren(false);
                return new NveViewNode(id, kind, frame, frame, 0);
            }
        }
    }

    private EditText input(int id, FrameHost surface) {
        final EditText edit = new EditText(context);
        edit.setIncludeFontPadding(false);
        edit.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
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
                if (surface.dispatchesSynchronously()) showVerdict(id);
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
     * A local app has answered a text change by the time {@code send} returns: an accepted edit
     * already arrived as a frame, so an input still differing from its runtime value was
     * rejected (a controlled field) and shows the app's value again. A paired server answers
     * later, so its fields keep the user's typing.
     */
    private void showVerdict(int id) {
        final NveViewNode n = nodes.get(id);
        if (n == null || n.kind != Kind.INPUT) return;
        reconciling = true;
        try {
            showValue((EditText) n.view, n.value);
        } finally {
            reconciling = false;
        }
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
     * (a clipping frame the runtime may still scroll programmatically, {@link #showScroll}).
     */
    private NveViewNode scroll(int id, int axes, FrameHost surface) {
        final FrameLayout content = new FrameLayout(context);
        content.setClipChildren(false);
        final ViewGroup.LayoutParams wrap = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        final ViewGroup outer;
        if (axes == NveTables.Values.ScrollAxes.HORIZONTAL) {
            outer = new HorizontalScrollView(context);
            outer.addView(content, wrap);
            listen(outer, id, content, outer, outer, surface);
        } else if (axes == NveTables.Values.ScrollAxes.BOTH) {
            outer = new ScrollView(context);
            final HorizontalScrollView inner = new HorizontalScrollView(context);
            inner.addView(content, wrap);
            outer.addView(inner, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
            listen(outer, id, content, inner, outer, surface);
            listen(inner, id, content, inner, outer, surface);
        } else if (axes == NveTables.Values.ScrollAxes.VERTICAL) {
            outer = new ScrollView(context);
            outer.addView(content, wrap);
            listen(outer, id, content, outer, outer, surface);
        } else {
            outer = new FrameLayout(context);
            outer.setClipChildren(true);
            outer.addView(content, wrap);
        }
        return new NveViewNode(id, Kind.SCROLL, outer, content, axes);
    }

    /**
     * Syncs {@code scroller}'s native scrolls: its delta to the runtime, and the container's full
     * position (x from {@code across}, y from {@code down}: both axes of a two-scroller BOTH
     * container in every event) to the app.
     */
    private void listen(View scroller, int id, FrameLayout content, View across, View down, FrameHost surface) {
        scroller.setOnScrollChangeListener((v, x, y, oldX, oldY) -> {
            if (reconciling || (x == oldX && y == oldY)) return;  // content resizes clamp scroll too
            surface.scrollBy(id, (x - oldX) / density, (y - oldY) / density);
            final float cw = content.getWidth() / density;
            final float ch = content.getHeight() / density;
            final float sx = across.getScrollX() / density;
            final float sy = down.getScrollY() / density;
            surface.send(r -> r.scroll(NveAbi.EVENT_SCROLL, id, now(), sx, sy, cw, ch));
        });
    }

    private void bindPress(NveViewNode n, boolean pressable, FrameHost surface) {
        if (n.kind == Kind.INPUT || n.kind == Kind.SCROLL) return;
        if (!pressable) {
            n.view.setOnClickListener(null);
            n.view.setClickable(false);
            return;
        }
        n.view.setOnClickListener(v -> surface.send(r -> r.point(NveAbi.EVENT_PRESS, n.id, now(), n.w * 0.5f, n.h * 0.5f)));
    }

    private void reconcileChildren(SceneReader s, int i) {
        final ViewGroup host = i == SceneSemantics.ROOT_INDEX ? root : contentOf(s.nodeId(i));
        if (host == null) return;
        desired.clear();
        for (int k = 0; k < s.childCount(i); k++) {
            final NveViewNode child = nodes.get(s.childId(i, k));
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
        final NveViewNode n = nodes.get(nodeId);
        return n == null ? null : n.content;
    }

    /**
     * Absolute rects → position relative to the parent's content (scroll content is unscrolled);
     * a scroll node and a scroll parent get their content box re-sized ({@link #sizeScrollContents}).
     */
    private void place(SceneReader s, int i) {
        if (i == SceneSemantics.ROOT_INDEX || !s.isLive(i)) return;
        final NveViewNode n = nodes.get(s.nodeId(i));
        if (n == null) return;
        final int parentIndex = parentIndex(s, i);
        final boolean nested = parentIndex > 0 && parentIndex != SceneSemantics.ROOT_INDEX;
        final float x = SceneSemantics.finite(s.x(i) - (nested ? s.x(parentIndex) : 0f), 0f);
        final float y = SceneSemantics.finite(s.y(i) - (nested ? s.y(parentIndex) : 0f), 0f);
        final float w = Math.max(0f, SceneSemantics.finite(s.width(i), 0f));
        final float h = Math.max(0f, SceneSemantics.finite(s.height(i), 0f));
        if (n.kind == Kind.SCROLL) sizeScrollOnce(n);
        if (nested) {
            final NveViewNode parent = nodes.get(s.nodeId(parentIndex));
            if (parent != null && parent.kind == Kind.SCROLL) sizeScrollOnce(parent);
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

    /** Queues scroll node {@code n}'s content box for {@link #sizeScrollContents} (once per frame). */
    private void sizeScrollOnce(NveViewNode n) {
        if (n.sizedPass == pass) return;
        n.sizedPass = pass;
        sizeScrolls.add(n);
    }

    /** Sizes each queued scroll content box once: O(children) per container, not per moved child. */
    private void sizeScrollContents(SceneReader s) {
        for (int k = 0; k < sizeScrolls.size(); k++) {
            final NveViewNode n = sizeScrolls.get(k);
            final int i = s.resolve(n.id);
            if (i > 0) sizeScrollContent(s, i, n);
        }
        sizeScrolls.clear();
    }

    /** A scroll content box spans the container and every child (relative to the container). */
    private void sizeScrollContent(SceneReader s, int i, NveViewNode n) {
        float right = Math.max(0f, SceneSemantics.finite(s.width(i), 0f));
        float bottom = Math.max(0f, SceneSemantics.finite(s.height(i), 0f));
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
