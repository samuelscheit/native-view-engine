package dev.nve.android.jni;

import dev.nve.android.generated.NveTables;
import dev.nve.host.FieldSpec;
import dev.nve.host.ImageSource;
import dev.nve.host.Key64;
import dev.nve.host.NativeRuntime;
import dev.nve.host.Rgba;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.RuntimeSurface;
import dev.nve.host.TableReader;
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
        final long[] kf = new long[2];
        final ByteBuffer logo = NativeRuntime.resource(h, fx.id("logo_res"), kf);
        c.that(logo != null && kf[0] == ImageSource.KIND.value() && kf[1] == 0L, "image resource kind");
        c.eq(logo == null ? null : ImageSource.decode(logo).url, "asset://logo.png", "image url");
        final ByteBuffer draw = NativeRuntime.resource(h, fx.id("canvas_res"), kf);
        c.that(draw != null && draw.capacity() > 0 && kf[0] == DRAW_LIST_KIND.value(), "draw list resource (module kind mirrored raw)");
        c.eq(table(s, NveTables.Canvas.KEY).u32(row(s, NveTables.Canvas.KEY, fx.id("canvas")), NveTables.Canvas.DRAW_LIST),
                fx.id("canvas_res"), "canvas draw list id");
        c.that(NativeRuntime.resource(h, 0, kf) == null && NativeRuntime.resource(h, 9999, kf) == null, "missing resources");
        final ByteBuffer cached = NativeRuntime.resource(h, fx.id("logo_res"), null);
        c.that(cached != null && cached.capacity() == logo.capacity(), "resource buffers are cached per frame");
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
        final ByteBuffer hero = NativeRuntime.resource(h, fx.id("hero_res"), kf);
        final ImageSource heroSrc = hero == null ? null : ImageSource.decode(hero);
        c.that(heroSrc != null && heroSrc.url == null && heroSrc.encoded.remaining() == 4 && heroSrc.encoded.get(1) == 'P', "inline image bytes");
        if (fx.id("hero_res") != fx.id("logo_res")) {
            c.that(NativeRuntime.resource(h, fx.id("logo_res"), kf) == null, "released image resource is gone");
        }
        scene.release();

        // Bad input never crashes: heap buffer, short length, corrupt bytes.
        c.eq(NativeRuntime.submitFrame(h, ByteBuffer.allocate(8), 8), NativeRuntime.STATUS_INVALID_ARG, "heap buffer rejected");
        c.eq(NativeRuntime.submitFrame(h, f2, f2.capacity() + 1), NativeRuntime.STATUS_INVALID_ARG, "length past capacity");
        final ByteBuffer junk = ByteBuffer.allocateDirect(32);
        c.that(NativeRuntime.submitFrame(h, junk, 32) != 0, "corrupt frame rejected");
        c.that(!NativeRuntime.lastError(h).isEmpty(), "corrupt frame explains itself");
        c.eq(NativeRuntime.acquireScene(h, new long[3], new ByteBuffer[3]), NativeRuntime.STATUS_INVALID_ARG, "short packed arrays");
        NativeRuntime.destroy(h);
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
            if (text.startsWith("Row")) return false;  // falls back to the heuristic
            out[0] = 10f * text.length();
            out[1] = 20f;
            return true;
        }), 0, "install measurer");
        s = scene.acquire();
        final int ti = s.resolve(title);
        c.that(s.width(ti) == 90f && s.height(ti) == 20f, "host measurer sizes the title: " + s.width(ti) + "x" + s.height(ti));
        c.that(measured.contains("Hello NVE") && measured.stream().anyMatch(t -> t.startsWith("Row")), "measurer sees every text leaf");
        c.that(!Float.isNaN(lastStyle[2]) && lastStyle[2] >= 0f, "max width contract");
        final int row0 = s.resolve(fx.id("row0"));
        c.that(s.height(row0) == 50f, "declined rows keep their own layout");
        scene.release();
        c.eq(NativeRuntime.setTextMeasurer(h, null), 0, "remove measurer");
        s = scene.acquire();
        c.that(s.width(s.resolve(title)) == heuristicW, "heuristic restored");
        scene.release();

        c.eq(NativeRuntime.setViewport(h, 200f, 500f, 2f), 0, "viewport");
        c.that(NativeRuntime.tick(h, 0.016f) >= 0, "tick");
        s = scene.acquire();
        c.that(s.width(1) == 200f && s.height(1) == 500f, "root follows viewport: " + s.width(1) + "x" + s.height(1));
        final int bi = s.resolve(fx.id("button"));
        final float bx = s.x(bi) + s.width(bi) / 2f;
        final float by = s.y(bi) + s.height(bi) / 2f;
        final int sc = s.resolve(fx.id("scroll"));
        scene.release();
        c.eq(NativeRuntime.hitTest(h, bx, by), fx.id("button"), "hit test finds the pressable button");
        c.eq(NativeRuntime.hitTest(h, -500f, -500f), 0, "hit test outside");
        final float[] xy = new float[2];
        c.eq(NativeRuntime.scrollBy(h, fx.id("scroll"), 0f, 30f, xy), 0, "scroll");
        c.that(xy[1] == 30f, "scroll offset " + xy[1]);
        c.eq(NativeRuntime.scrollBy(h, fx.id("scroll"), 0f, 1000f, xy), 0, "overscroll");
        c.that(xy[1] == 70f, "scroll clamps to content (3 x 50 - 80): " + xy[1]);
        c.that(NativeRuntime.scrollBy(h, fx.id("title"), 0f, 10f, null) == NativeRuntime.STATUS_NOT_FOUND, "non-scroll node");
        c.that(sc > 0, "scroll resolved");
        c.eq(NativeRuntime.setWakeListener(h, () -> { }), 0, "wake listener");
        c.eq(NativeRuntime.setWakeListener(h, null), 0, "clear wake listener");
        NativeRuntime.destroy(h);
        NativeRuntime.destroy(0L);
        c.eq(NativeRuntime.tick(0L, 0f), -NativeRuntime.STATUS_INVALID_ARG, "null handle");
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
        c.that(NativeRuntime.tick(h, 0f) >= 0, "tick v1");
        c.eq(onlyText(scene), "generation one", "v1 renders");
        c.eq(NativeRuntime.attachApp(h, v2.toString()), 0, "hot reload v2");
        c.that(NativeRuntime.tick(h, 0f) >= 0, "tick v2");
        c.eq(onlyText(scene), "generation two", "v2 replaces v1");
        c.that(NativeRuntime.attachApp(h, bad.toString()) != 0, "null interface rejected");
        c.that(NativeRuntime.attachApp(h, "/nonexistent/libnope.so") == NativeRuntime.STATUS_NOT_FOUND
                && NativeRuntime.lastError(h).contains("dlopen"), "missing library");
        c.that(NativeRuntime.tick(h, 0f) >= 0, "tick after failed reloads");
        c.eq(onlyText(scene), "generation two", "failed reloads keep the running app");
        c.that(NativeRuntime.pairConnect(h, "ws://127.0.0.1:9/x", "t") == NativeRuntime.STATUS_INVALID_ARG, "no pairing while an app runs");
        c.eq(NativeRuntime.detachApp(h), 0, "detach");
        c.that(NativeRuntime.tick(h, 0f) >= 0, "tick detached");
        final RuntimeSceneReader s = scene.acquire();
        c.that(s.findTable(NveTables.Text.KEY) == null || s.findTable(NveTables.Text.KEY).rowCount() == 0, "detached scene is empty");
        scene.release();
        NativeRuntime.destroy(h);
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
            surface.events().point(dev.nve.android.generated.NveCoreAbi.EVENT_PRESS, fx.id("button"), 1L, 1f, 1f);
            c.that(surface.dispatch() >= 0 && surface.events().count() == 0, "dispatch drains the ring");
            c.that(surface.hitTest(-1f, -1f) == 0, "surface hit test");
        }
    }
}
