package dev.nve.android;

import android.view.View;
import android.view.ViewGroup;

import dev.nve.host.SceneSemantics.Kind;

/** One live node's realized View and what {@link NveSceneRenderer} last applied to it. */
final class NveViewNode {
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
    /** A scroll offset waits for the view's next layout (before it, a scroll view clamps to 0). */
    boolean scrollPending;
    final NveViewStyle.ViewState style = new NveViewStyle.ViewState();
    /** Resource id an image / canvas shows ({@code 0}: none). */
    int resourceId;
    /** Renderer pass that last read this node's resource bytes / sized its scroll content. */
    int boundPass;
    int sizedPass;
    /** Runtime value of an input as of the last applied frame. */
    String value = "";
    float x;
    float y;
    float w = -1f;
    float h = -1f;

    NveViewNode(int id, Kind kind, View view, ViewGroup content, int axes) {
        this.id = id;
        this.kind = kind;
        this.view = view;
        this.content = content;
        this.axes = axes;
    }
}
