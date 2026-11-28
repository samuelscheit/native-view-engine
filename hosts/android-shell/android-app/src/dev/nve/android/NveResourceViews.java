package dev.nve.android;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.widget.ImageView;

import dev.nve.host.FrameHost;
import dev.nve.host.ImageSource;
import dev.nve.host.SceneReader;
import dev.nve.host.SceneSemantics;
import dev.nve.host.SceneSemantics.Kind;
import dev.nve.host.generated.NveModuleTables;
import dev.nve.host.generated.NveTables;

import java.io.InputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Image / canvas views and the resource ids they show: binds resource bytes into the views and,
 * indexed by resource id, rebinds a dirty id's users in O(users). Every bind stamps the
 * renderer's pass, so bytes are read and decoded at most once per node per frame.
 */
final class NveResourceViews {
    private final Context context;
    private final Map<Integer, List<NveViewNode>> byResource = new HashMap<>();

    NveResourceViews(Context context) {
        this.context = context;
    }

    void clear() {
        byResource.clear();
    }

    /** Forgets {@code n} (its view is dropped). */
    void unbind(NveViewNode n) {
        index(n, 0);
    }

    /** Re-reads the image when its resource id changed or {@code force} (dirty bytes, new view). */
    void bindImage(NveViewNode n, SceneReader s, SceneSemantics.Tables t, int i, FrameHost surface, boolean force, int pass) {
        final ImageView view = (ImageView) n.view;
        final int row = s.rowOf(t.image, i);
        view.setScaleType(NveViewStyle.scaleType(t.image.u32(row, NveTables.Image.FIT)));
        final String alt = s.text(t.image, row, NveTables.Image.ALT);
        if (view.getContentDescription() == null && !alt.isEmpty()) view.setContentDescription(alt);
        final int res = t.image.u32(row, NveTables.Image.SRC);
        if (res == n.resourceId && !force && !s.isResourceDirty(res)) return;
        index(n, res);
        n.boundPass = pass;
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

    void bindCanvas(NveViewNode n, SceneReader s, SceneSemantics.Tables t, int i, FrameHost surface, boolean force, int pass) {
        final NveCanvasView view = (NveCanvasView) n.view;
        final int res = t.canvas.u32(s.rowOf(t.canvas, i), NveModuleTables.Canvas.DRAW_LIST);
        if (res == n.resourceId && !force && !s.isResourceDirty(res)) return;
        index(n, res);
        n.boundPass = pass;
        final ByteBuffer bytes = res == 0 ? null : surface.resource(res);
        if (bytes == null) {
            view.clearDrawList();
        } else {
            view.setDrawList(bytes);  // copied: the frame's buffer dies at release
        }
    }

    /**
     * Re-reads the bytes of every image / canvas showing resource {@code id}, except those this
     * pass already bound (a dirty node showing a dirty resource is decoded once, not twice).
     * Clean users keep their resource id, so {@code users} is not modified while iterated.
     */
    void rebind(SceneReader s, SceneSemantics.Tables t, FrameHost surface, int id, int pass) {
        final List<NveViewNode> users = byResource.get(id);
        if (users == null) return;
        for (int k = 0; k < users.size(); k++) {
            final NveViewNode n = users.get(k);
            if (n.boundPass == pass) continue;
            final int i = s.resolve(n.id);
            if (i <= 0) continue;
            if (n.kind == Kind.IMAGE) bindImage(n, s, t, i, surface, true, pass);
            if (n.kind == Kind.CANVAS) bindCanvas(n, s, t, i, surface, true, pass);
        }
    }

    private void index(NveViewNode n, int id) {
        if (n.resourceId == id) return;
        if (n.resourceId != 0) {
            final List<NveViewNode> users = byResource.get(n.resourceId);
            if (users != null && users.remove(n) && users.isEmpty()) byResource.remove(n.resourceId);
        }
        n.resourceId = id;
        if (id != 0) byResource.computeIfAbsent(id, k -> new ArrayList<>(1)).add(n);
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
}
