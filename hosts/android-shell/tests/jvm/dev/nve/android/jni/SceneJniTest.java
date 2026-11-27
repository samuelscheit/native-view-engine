package dev.nve.android.jni;

import dev.nve.host.FieldSpec;
import dev.nve.host.ImageSource;
import dev.nve.host.Key64;
import dev.nve.host.NativeRuntime;
import dev.nve.host.Rgba;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.RuntimeSurface;
import dev.nve.host.SceneSemantics;
import dev.nve.host.TableReader;
import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveModuleTables;
import dev.nve.host.generated.NveRuntimeAbi;
import dev.nve.host.generated.NveTables;
import java.nio.ByteBuffer;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * Real `libnve_runtime` through `libnve_android_shell` on a desktop JVM: packed scene frames over
 * the standard tables, resources, deltas, viewport, host text measurer, hit testing, scrolling,
 * and app hot reload.
 *
 * <p>Args: fixture dir, hot-v1 app, hot-v2 app, bad-interface app.
 */
public final class SceneJniTest {
    private SceneJniTest() {}

    static final Key64 DRAW_LIST_KIND = Key64.of("nve.res.draw_list");

    public static void main(String[] args) throws Exception {
        System.loadLibrary("nve_android_shell");
        final Expect c = new Expect();
        final Fixtures fx = new Fixtures(Path.of(args[0]));
        frames(c, fx);
        layoutAndMeasure(c, fx);
        hotReload(c, Path.of(args[1]), Path.of(args[2]), Path.of(args[3]));
        surface(c, fx);
        c.finish("android_scene_jni_test");
    }

    static TableReader table(RuntimeSceneReader s, Key64 key) {
        final TableReader t = s.findTable(key);
        if (t == null) throw new IllegalStateException("table " + key + " not published");
        return t;
    }

    static int row(RuntimeSceneReader s, Key64 key, int nodeId) {
        return s.rowOf(table(s, key), s.resolve(nodeId));
    }

    static String str(RuntimeSceneReader s, Key64 key, int nodeId, FieldSpec f) {
        final int r = row(s, key, nodeId);
        return r < 0 ? null : s.text(table(s, key), r, f);
    }

    static void frames(Expect c, Fixtures fx) throws Exception {
        final long h = NativeRuntime.create(1, 360f, 800f, 1f);
        c.that(h != 0L, "create");
        final Fixtures.Scene scene = new Fixtures.Scene(h);
        c.eq(NativeRuntime.submitFrame(h, fx.frame("showcase_1.nve2"), fx.frame("showcase_1.nve2").remaining()), 0, "submit keyframe");
        RuntimeSceneReader s = scene.acquire();
        c.that(s.frameId() > 0, "frame id");
        for (String n : new String[] {"column", "title", "input", "image", "scroll", "row0", "row1", "row2", "canvas", "button"}) {
            c.that(s.resolve(fx.id(n)) > 0, n + " is live");
        }
        final int title = fx.id("title");
        c.eq(str(s, NveTables.Text.KEY, title, NveTables.Text.TEXT), "Hello NVE", "title text");
        final TableReader typo = table(s, NveTables.Typography.KEY);
        final int tr = s.rowOf(typo, s.resolve(title));
        c.that(typo.f32(tr, NveTables.Typography.FONT_SIZE) == 24f, "font size");
        c.eq(typo.u32(tr, NveTables.Typography.FONT_WEIGHT), 700, "font weight");
        c.eq(Rgba.r(typo.color(tr, NveTables.Typography.COLOR)), 248, "text color");
        final TableReader visual = table(s, NveTables.Visual.KEY);
        final int col = s.resolve(fx.id("column"));
        c.that(!visual.isSparse(), "visual is dense");
        c.eq(Rgba.b(visual.color(s.rowOf(visual, col), NveTables.Visual.BACKGROUND)), 42, "column background");
        c.that(visual.f32(s.rowOf(visual, col), NveTables.Visual.BORDER_RADIUS) == 12f, "border radius");
        c.that(Math.abs(visual.f32(s.rowOf(visual, col), NveTables.Visual.OPACITY) - 0.9f) < 1e-6, "opacity");
        final int input = fx.id("input");
        c.eq(str(s, NveTables.TextInput.KEY, input, NveTables.TextInput.VALUE), "draft", "input value");
        c.eq(str(s, NveTables.TextInput.KEY, input, NveTables.TextInput.PLACEHOLDER), "Type here", "placeholder");
        c.that(table(s, NveTables.TextInput.KEY).bool(row(s, NveTables.TextInput.KEY, input), NveTables.TextInput.SECURE), "secure");
        c.eq(str(s, NveTables.Accessibility.KEY, input, NveTables.Accessibility.LABEL), "Password", "a11y label");
        final int button = fx.id("button");
        c.eq(table(s, NveTables.Accessibility.KEY).u32(row(s, NveTables.Accessibility.KEY, button), NveTables.Accessibility.ROLE),
                NveTables.Accessibility.ROLE_BUTTON, "button role");
        c.eq(str(s, NveTables.DebugId.KEY, button, NveTables.DebugId.TEST_ID), "submit-btn", "test id");
        semantics(c, fx, s);
        c.that(s.primaryTable(s.resolve(fx.id("scroll"))) != null, "scroll container realizes a table");
        c.eq(s.childCount(s.resolve(fx.id("scroll"))), 3, "scroll children");
        c.eq(s.childId(s.resolve(fx.id("scroll")), 2), fx.id("row2"), "third row");
        final int bi = s.resolve(button);
        c.that(s.width(bi) == 120f && s.height(bi) == 44f, "button rect");

        // Resources: image URL and canvas draw list, zero-copy by id.
        final TableReader image = table(s, NveTables.Image.KEY);
        final int ir = s.rowOf(image, s.resolve(fx.id("image")));
        c.eq(image.u32(ir, NveTables.Image.SRC), fx.id("logo_res"), "image src id");
        c.eq(image.u32(ir, NveTables.Image.FIT), NveTables.Image.FIT_CONTAIN, "image fit");
        final long[] kf = new long[1];
        final ByteBuffer logo = NativeRuntime.resource(h, fx.id("logo_res"), kf);
        c.that(logo != null && kf[0] == ImageSource.KIND.value(), "image resource kind");
        c.eq(s.sceneEpoch() != 0 || s.frameId() > 0, true, "frame header");
        final List<Integer> createdRes = new ArrayList<>();
        for (int i = 0; i < s.dirtyResourceCount(); i++) createdRes.add(s.dirtyResource(i));
        c.that(createdRes.contains(fx.id("logo_res")) && createdRes.contains(fx.id("canvas_res")), "keyframe resources are dirty: " + createdRes);
        c.eq(logo == null ? null : ImageSource.decode(logo).url, "asset://logo.png", "image url");
        final ByteBuffer draw = NativeRuntime.resource(h, fx.id("canvas_res"), kf);
        c.that(draw != null && draw.capacity() > 0 && kf[0] == DRAW_LIST_KIND.value(), "draw list resource (module kind mirrored raw)");
        c.eq(table(s, NveModuleTables.Canvas.KEY).u32(row(s, NveModuleTables.Canvas.KEY, fx.id("canvas")), NveModuleTables.Canvas.DRAW_LIST),
                fx.id("canvas_res"), "canvas draw list id");
        c.that(NativeRuntime.resource(h, 0, kf) == null && NativeRuntime.resource(h, 9999, kf) == null, "missing resources");
        final ByteBuffer again = NativeRuntime.resource(h, fx.id("logo_res"), null);
        c.that(again != null && again.capacity() == logo.capacity(), "null kind array is allowed");
        scene.release();
        c.that(NativeRuntime.resource(h, fx.id("logo_res"), kf) == null, "no resources without a scene");

        // Delta: text edit, image swap, row removal.
        final ByteBuffer f2 = fx.frame("showcase_2.nve2");
        c.eq(NativeRuntime.submitFrame(h, f2, f2.remaining()), 0, "submit delta");
        s = scene.acquire();
        c.eq(str(s, NveTables.Text.KEY, title, NveTables.Text.TEXT), "Hello again", "edited text");
        final List<Integer> removed = new ArrayList<>();
        for (int i = 0; i < s.removedCount(); i++) removed.add(s.removedId(i));
        c.that(removed.contains(fx.id("row2")), "removed row reported");
        c.eq(s.resolve(fx.id("row2")), -1, "removed row is gone");
        c.eq(s.childCount(s.resolve(fx.id("scroll"))), 2, "scroll children after removal");
        boolean titleDirty = false;
        for (int i = 0; i < s.dirtyCount(); i++) titleDirty |= s.dirtyIndex(i) == s.resolve(title);
        c.that(titleDirty, "edited node is dirty");
        final List<Integer> deltaRes = new ArrayList<>();
        for (int i = 0; i < s.dirtyResourceCount(); i++) deltaRes.add(s.dirtyResource(i));
        c.that(deltaRes.contains(fx.id("hero_res")) && !deltaRes.contains(fx.id("canvas_res")),
                "only (re)defined resources are dirty: " + deltaRes);
        final ByteBuffer hero = NativeRuntime.resource(h, fx.id("hero_res"), kf);
        final ImageSource heroSrc = hero == null ? null : ImageSource.decode(hero);
        c.that(heroSrc != null && heroSrc.url == null && heroSrc.encoded.remaining() == 4 && heroSrc.encoded.get(1) == 'P', "inline image bytes");
        if (fx.id("hero_res") != fx.id("logo_res")) {
            c.that(NativeRuntime.resource(h, fx.id("logo_res"), kf) == null, "released image resource is gone");
        }
        scene.release();

        // Bad input never crashes: heap buffer, short length, corrupt bytes.
        c.eq(NativeRuntime.submitFrame(h, ByteBuffer.allocate(8), 8), NveRuntimeAbi.STATUS_INVALID_ARG, "heap buffer rejected");
        c.eq(NativeRuntime.submitFrame(h, f2, f2.capacity() + 1), NveRuntimeAbi.STATUS_INVALID_ARG, "length past capacity");
        final ByteBuffer junk = ByteBuffer.allocateDirect(32);
        c.that(NativeRuntime.submitFrame(h, junk, 32) != 0, "corrupt frame rejected");
        c.that(!NativeRuntime.lastError(h).isEmpty(), "corrupt frame explains itself");
        c.eq(NativeRuntime.acquireScene(h, new long[3], new ByteBuffer[3]), NveRuntimeAbi.STATUS_INVALID_ARG, "short packed arrays");
        NativeRuntime.destroy(h);
    }

    /** {@link SceneSemantics} over real tables: kinds, pressability, and inherited typography. */
    static void semantics(Expect c, Fixtures fx, RuntimeSceneReader s) {
        final SceneSemantics.Tables t = new SceneSemantics.Tables().bind(s);
        c.eq(SceneSemantics.kindOf(s, t, s.resolve(fx.id("title"))), SceneSemantics.Kind.TEXT, "title kind");
        c.eq(SceneSemantics.kindOf(s, t, s.resolve(fx.id("input"))), SceneSemantics.Kind.INPUT, "input kind");
        c.eq(SceneSemantics.kindOf(s, t, s.resolve(fx.id("image"))), SceneSemantics.Kind.IMAGE, "image kind");
        c.eq(SceneSemantics.kindOf(s, t, s.resolve(fx.id("canvas"))), SceneSemantics.Kind.CANVAS, "canvas kind");
        c.eq(SceneSemantics.kindOf(s, t, s.resolve(fx.id("scroll"))), SceneSemantics.Kind.SCROLL, "scroll kind");
        c.eq(SceneSemantics.kindOf(s, t, s.resolve(fx.id("column"))), SceneSemantics.Kind.CONTAINER, "column kind");
        c.that(SceneSemantics.pressable(s, t, s.resolve(fx.id("button"))), "button is pressable");
        c.that(!SceneSemantics.pressable(s, t, s.resolve(fx.id("title"))), "title is not pressable");
        final SceneSemantics.TextStyle st = new SceneSemantics.TextStyle();
        st.resolve(s, t, s.resolve(fx.id("row0")));
        c.that(Rgba.r(st.color) == 1 && Rgba.g(st.color) == 2 && Rgba.b(st.color) == 3, "row inherits the column color");
        c.that(st.italic && st.align == NveTables.Typography.TEXT_ALIGN_CENTER, "row inherits italic + center through the scroll");
        c.that(st.fontSize == SceneSemantics.DEFAULT_FONT_SIZE && st.fontWeight == SceneSemantics.DEFAULT_FONT_WEIGHT, "unset fields keep defaults");
        st.resolve(s, t, s.resolve(fx.id("title")));
        c.that(Rgba.r(st.color) == 248 && st.fontSize == 24f && st.fontWeight == 700, "explicit fields win");
        c.that(st.italic && st.align == NveTables.Typography.TEXT_ALIGN_CENTER, "title inherits what it does not set");
        st.resolve(s, t, 0);
        c.that(st.fontSize == SceneSemantics.DEFAULT_FONT_SIZE, "null index resolves to defaults");
    }

    static void layoutAndMeasure(Expect c, Fixtures fx) throws Exception {
        final long h = NativeRuntime.create(0, Float.NaN, -1f, 0f);
        c.that(h != 0L, "create with sanitized config");
        final Fixtures.Scene scene = new Fixtures.Scene(h);
        final ByteBuffer f1 = fx.frame("showcase_1.nve2");
        c.eq(NativeRuntime.submitFrame(h, f1, f1.remaining()), 0, "submit");
        final int title = fx.id("title");
        RuntimeSceneReader s = scene.acquire();
        final float heuristicW = s.width(s.resolve(title));
        scene.release();

        final List<String> measured = new ArrayList<>();
        final float[] lastStyle = new float[3];
        c.eq(NativeRuntime.setTextMeasurer(h, (text, fontSize, fontWeight, maxWidth, out) -> {
            measured.add(text);
            lastStyle[0] = fontSize;
            lastStyle[1] = fontWeight;
            lastStyle[2] = maxWidth;
            if (text.startsWith("Row")) throw new IllegalStateException("measurer failure must not crash");
            if (text.isEmpty()) return;  // writes nothing: 0 x 0
            out[0] = text.equals("Hello NVE") ? 10f * text.length() : Float.NaN;
            out[1] = 20f;
        }), 0, "install measurer");
        c.eq(NativeRuntime.tick(h, 0f, null), 0, "relayout with the measurer");
        s = scene.acquire();
        final int ti = s.resolve(title);
        c.that(s.width(ti) == 90f && s.height(ti) == 20f, "host measurer sizes the title: " + s.width(ti) + "x" + s.height(ti));
        c.that(measured.contains("Hello NVE") && measured.stream().anyMatch(t -> t.startsWith("Row")), "measurer sees every text leaf");
        c.that(!Float.isNaN(lastStyle[2]) && lastStyle[2] >= 0f, "max width contract");
        final int row0 = s.resolve(fx.id("row0"));
        c.that(s.height(row0) == 50f, "rows whose measurer throws keep their own fixed layout");
        scene.release();
        c.eq(NativeRuntime.setTextMeasurer(h, null), 0, "remove measurer");
        c.eq(NativeRuntime.tick(h, 0f, null), 0, "relayout without the measurer");
        s = scene.acquire();
        c.that(s.width(s.resolve(title)) == heuristicW, "heuristic restored");
        scene.release();

        c.eq(NativeRuntime.setViewport(h, 200f, 500f, 2f), 0, "viewport");
        final int[] out = new int[1];
        c.eq(NativeRuntime.tick(h, 0.016f, out), 0, "tick");
        c.eq(NativeRuntime.tick(h, Float.NaN, out), 0, "NaN dt is sanitized");
        s = scene.acquire();
        c.that(s.width(1) == 200f && s.height(1) == 500f, "root follows viewport: " + s.width(1) + "x" + s.height(1));
        final int bi = s.resolve(fx.id("button"));
        final float bx = s.x(bi) + s.width(bi) / 2f;
        final float by = s.y(bi) + s.height(bi) / 2f;
        final int sc = s.resolve(fx.id("scroll"));
        scene.release();
        c.eq(NativeRuntime.hitTest(h, bx, by, out), 0, "hit test status");
        c.eq(out[0], fx.id("button"), "hit test finds the pressable button");
        out[0] = -1;
        c.eq(NativeRuntime.hitTest(h, -500f, -500f, out), 0, "a miss is not an error");
        c.eq(out[0], 0, "hit test outside");
        c.eq(NativeRuntime.hitTest(h, Float.NaN, 1f, out), 0, "NaN hit test is a miss");
        c.eq(out[0], 0, "NaN hit test finds nothing");
        final float[] xy = new float[2];
        c.eq(NativeRuntime.scrollBy(h, fx.id("scroll"), 0f, 30f, xy), 0, "scroll");
        c.that(xy[1] == 30f, "scroll offset " + xy[1]);
        c.eq(NativeRuntime.scrollBy(h, fx.id("scroll"), 0f, 1000f, xy), 0, "overscroll");
        c.that(xy[1] == 70f, "scroll clamps to content (3 x 50 - 80): " + xy[1]);
        c.that(NativeRuntime.scrollBy(h, fx.id("title"), 0f, 10f, null) == NveRuntimeAbi.STATUS_NOT_FOUND, "non-scroll node");
        c.eq(NativeRuntime.scrollBy(h, fx.id("scroll"), Float.NaN, 0f, xy), NveRuntimeAbi.STATUS_INVALID_ARG, "NaN scroll rejected");
        c.that(xy[1] == 70f, "rejected scroll keeps the offset");
        c.that(sc > 0, "scroll resolved");
        c.eq(NativeRuntime.setWakeListener(h, () -> { }), 0, "wake listener");
        c.eq(NativeRuntime.setWakeListener(h, null), 0, "clear wake listener");
        NativeRuntime.destroy(h);
        NativeRuntime.destroy(0L);
        c.eq(NativeRuntime.tick(0L, 0f, null), NveRuntimeAbi.STATUS_INVALID_ARG, "null handle");
        c.eq(NativeRuntime.lastError(0L), "", "null handle has no error");
    }

    static String onlyText(Fixtures.Scene scene) {
        final RuntimeSceneReader s = scene.acquire();
        final TableReader text = s.findTable(NveTables.Text.KEY);
        final String out = text == null || text.rowCount() == 0 ? null : s.text(text, 0, NveTables.Text.TEXT);
        scene.release();
        return out;
    }

    static void hotReload(Expect c, Path v1, Path v2, Path bad) {
        final long h = NativeRuntime.create(3, 400f, 400f, 1f);
        final Fixtures.Scene scene = new Fixtures.Scene(h);
        c.eq(NativeRuntime.attachApp(h, v1.toString()), 0, "attach v1");
        c.eq(NativeRuntime.tick(h, 0f, null), 0, "tick v1");
        c.eq(onlyText(scene), "generation one", "v1 renders");
        c.eq(NativeRuntime.attachApp(h, v2.toString()), 0, "hot reload v2");
        c.eq(NativeRuntime.tick(h, 0f, null), 0, "tick v2");
        c.eq(onlyText(scene), "generation two", "v2 replaces v1");
        c.that(NativeRuntime.attachApp(h, bad.toString()) != 0, "null interface rejected");
        c.that(NativeRuntime.attachApp(h, "/nonexistent/libn\u00f6pe.so") == NveRuntimeAbi.STATUS_NOT_FOUND
                && NativeRuntime.lastError(h).contains("dlopen(/nonexistent/libn\u00f6pe.so)"), "missing library, non-ASCII error survives JNI");
        c.eq(NativeRuntime.tick(h, 0f, null), 0, "tick after failed reloads");
        c.eq(onlyText(scene), "generation two", "failed reloads keep the running app");
        c.that(NativeRuntime.pairConnect(h, "ws://127.0.0.1:9/x", "t") == NveRuntimeAbi.STATUS_INVALID_ARG, "no pairing while an app runs");
        c.eq(NativeRuntime.detachApp(h), 0, "detach");
        c.eq(NativeRuntime.tick(h, 0f, null), 0, "tick detached");
        final RuntimeSceneReader s = scene.acquire();
        c.that(s.findTable(NveTables.Text.KEY) == null || s.findTable(NveTables.Text.KEY).rowCount() == 0, "detached scene is empty");
        scene.release();
        NativeRuntime.destroy(h);
    }

    /**
     * Calls made from inside {@code onFrame} (listeners firing while a host mutates its UI) never
     * touch the acquired scene: sends / scrolls are deferred past {@code releaseScene}, structural
     * calls throw, {@code close} is deferred, and the reader is detached afterwards.
     */
    static void reentrancy(Expect c, Fixtures fx, RuntimeSurface surface) throws Exception {
        final RuntimeSceneReader[] seen = new RuntimeSceneReader[1];
        final int[] statuses = new int[3];
        final boolean[] threw = new boolean[1];
        final int[] renders = new int[1];
        surface.setFrameConsumer((s, sf) -> {
            seen[0] = s;
            statuses[0] = sf.send(r -> r.empty(NveAbi.EVENT_FOCUS, fx.id("input"), 2L));
            statuses[1] = sf.scrollBy(fx.id("scroll"), 0f, 10f);
            statuses[2] = sf.hitTest(1f, 1f);
            try {
                sf.setViewport(10f, 10f, 1f);
            } catch (IllegalStateException e) {
                threw[0] = true;
            }
            if (renders[0]++ == 0) c.that(!sf.render(), "render inside onFrame only schedules");
        });
        c.eq(surface.setViewport(320f, 600f, 1f), 0, "viewport for re-entrancy");
        c.that(surface.render(), "frame delivered");
        c.that(statuses[0] == 0 && statuses[1] == 0 && statuses[2] == 0, "in-frame send / scroll deferred, hit test inert");
        c.that(threw[0], "structural call inside onFrame throws");
        c.eq(seen[0].nodeCapacity(), 0, "reader detached after the frame");
        c.throwsAny(() -> seen[0].x(1), "detached reader refuses reads");
        final RuntimeSurface closing = new RuntimeSurface(10, 100f, 100f, 1f, Runnable::run);
        final ByteBuffer f1 = fx.frame("showcase_1.nve2");
        c.eq(closing.submitFrame(f1, f1.remaining()), 0, "submit before in-frame close");
        closing.setFrameConsumer((s, sf) -> sf.close());
        c.that(closing.render(), "frame with in-frame close");
        c.eq(closing.handle(), 0L, "close deferred to after the frame");
        c.that(!closing.render(), "closed surface renders nothing");
    }

    /** `RuntimeSurface`: frames reach the consumer once per change, resources only inside it. */
    static void surface(Expect c, Fixtures fx) throws Exception {
        final List<String> titles = new ArrayList<>();
        final long[] logoLen = {-1L};
        try (RuntimeSurface surface = new RuntimeSurface(9, 360f, 800f, 1f, Runnable::run)) {
            surface.setFrameConsumer((s, sf) -> {
                titles.add(str(s, NveTables.Text.KEY, fx.id("title"), NveTables.Text.TEXT));
                final ByteBuffer logo = sf.resource(fx.id("logo_res"));
                logoLen[0] = logo == null || !sf.resourceKind().equals(ImageSource.KIND) ? -1L : logo.capacity();
            });
            final ByteBuffer f1 = fx.frame("showcase_1.nve2");
            c.eq(surface.submitFrame(f1, f1.remaining()), 0, "surface submit");
            c.that(surface.render(), "first frame delivered");
            c.that(!surface.render(), "unchanged scene is not redelivered");
            c.eq(titles, List.of("Hello NVE"), "consumer saw the title once");
            c.that(logoLen[0] > 0, "resources readable inside the consumer");
            c.that(surface.resource(fx.id("logo_res")) == null, "resources unreadable outside the consumer");
            c.eq(surface.setViewport(300f, 600f, 1f), 0, "surface viewport");
            c.that(surface.render(), "viewport change redelivers");
            c.eq(surface.send(r -> r.point(NveAbi.EVENT_PRESS, fx.id("button"), 1L, 1f, 1f)), 0, "send");
            c.that(surface.hitTest(-1f, -1f) == 0, "surface hit test");
            reentrancy(c, fx, surface);
        }
    }
}
