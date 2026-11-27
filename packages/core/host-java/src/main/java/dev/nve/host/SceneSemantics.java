package dev.nve.host;

import dev.nve.host.generated.NveModuleTables;
import dev.nve.host.generated.NveTables;

/**
 * How a native host realizes one runtime node from its published tables, shared by every Java /
 * Kotlin renderer (Android Views, Compose): the node kind from table presence, pressability from
 * {@code nve.interaction}, and the inherited {@code nve.typography} text style.
 */
public final class SceneSemantics {
    private SceneSemantics() {}

    /** Node index of {@code NodeId::ROOT}. */
    public static final int ROOT_INDEX = 1;

    /** How a node is realized, from the tables it carries (first match wins). */
    public enum Kind { CONTAINER, TEXT, INPUT, IMAGE, CANVAS, SCROLL }

    /** The frame's standard tables ({@code null} when not published), re-bound per frame. */
    public static final class Tables {
        public TableReader visual;
        public TableReader typography;
        public TableReader text;
        public TableReader input;
        public TableReader image;
        public TableReader canvas;
        public TableReader scroll;
        public TableReader interaction;
        public TableReader accessibility;
        public TableReader debugId;

        public Tables bind(RuntimeSceneReader s) {
            visual = s.findTable(NveTables.Visual.KEY);
            typography = s.findTable(NveTables.Typography.KEY);
            text = s.findTable(NveTables.Text.KEY);
            input = s.findTable(NveTables.TextInput.KEY);
            image = s.findTable(NveTables.Image.KEY);
            canvas = s.findTable(NveModuleTables.Canvas.KEY);
            scroll = s.findTable(NveTables.Scroll.KEY);
            interaction = s.findTable(NveTables.Interaction.KEY);
            accessibility = s.findTable(NveTables.Accessibility.KEY);
            debugId = s.findTable(NveTables.DebugId.KEY);
            return this;
        }
    }

    public static Kind kindOf(RuntimeSceneReader s, Tables t, int index) {
        if (s.rowOf(t.input, index) >= 0) return Kind.INPUT;
        if (s.rowOf(t.image, index) >= 0) return Kind.IMAGE;
        if (s.rowOf(t.canvas, index) >= 0) return Kind.CANVAS;
        if (s.rowOf(t.scroll, index) >= 0) return Kind.SCROLL;
        if (s.rowOf(t.text, index) >= 0 && s.childCount(index) == 0) return Kind.TEXT;
        return Kind.CONTAINER;
    }

    /** {@code nve.interaction.disabled}. */
    public static boolean disabled(RuntimeSceneReader s, Tables t, int index) {
        final int r = s.rowOf(t.interaction, index);
        return r >= 0 && t.interaction.bool(r, NveTables.Interaction.DISABLED);
    }

    /** Subscribed to presses and not disabled. */
    public static boolean pressable(RuntimeSceneReader s, Tables t, int index) {
        final int r = s.rowOf(t.interaction, index);
        return r >= 0 && !t.interaction.bool(r, NveTables.Interaction.DISABLED)
                && (t.interaction.u32(r, NveTables.Interaction.EVENT_MASK) & NveTables.Values.EventMask.PRESS) != 0;
    }

    /** {@code v} when finite, else {@code fallback} (hostile / corrupt rows never reach the UI as NaN). */
    public static float finite(float v, float fallback) {
        return Float.isFinite(v) ? v : fallback;
    }

    private static final int[] TYPO_DEFAULTS = NveTables.Typography.defaults();
    public static final int DEFAULT_COLOR = NveTables.Typography.COLOR.extract(TYPO_DEFAULTS[NveTables.Typography.COLOR.word]);
    public static final float DEFAULT_FONT_SIZE =
            Float.intBitsToFloat(NveTables.Typography.FONT_SIZE.extract(TYPO_DEFAULTS[NveTables.Typography.FONT_SIZE.word]));
    public static final int DEFAULT_FONT_WEIGHT =
            NveTables.Typography.FONT_WEIGHT.extract(TYPO_DEFAULTS[NveTables.Typography.FONT_WEIGHT.word]);
    private static final int STYLE_FIELDS = NveTables.Values.TypographySet.COLOR | NveTables.Values.TypographySet.FONT_SIZE
            | NveTables.Values.TypographySet.FONT_WEIGHT | NveTables.Values.TypographySet.FONT_STYLE | NveTables.Values.TypographySet.TEXT_ALIGN;

    /** Inherited text style: explicit fields (by {@code SET_*} bit) win, ancestors fill the rest. */
    public static final class TextStyle {
        public int color;
        public float fontSize;
        public int fontWeight;
        public boolean italic;
        public int align;

        /**
         * Resolves node {@code index}'s style by walking at most {@code nodeCapacity} ancestors (a
         * corrupt parent cycle cannot hang); returns {@code this}.
         */
        public TextStyle resolve(RuntimeSceneReader s, Tables t, int index) {
            color = DEFAULT_COLOR;
            fontSize = DEFAULT_FONT_SIZE;
            fontWeight = DEFAULT_FONT_WEIGHT;
            italic = false;
            align = NveTables.Values.TextAlign.START;
            final TableReader ty = t.typography;
            int pending = STYLE_FIELDS;
            for (int i = index, steps = 0; ty != null && i > 0 && pending != 0 && steps < s.nodeCapacity(); steps++) {
                final int r = s.rowOf(ty, i);
                if (r >= 0) {
                    final int set = ty.u32(r, NveTables.Typography.SET) & pending;
                    if ((set & NveTables.Values.TypographySet.COLOR) != 0) color = ty.color(r, NveTables.Typography.COLOR);
                    if ((set & NveTables.Values.TypographySet.FONT_SIZE) != 0) {
                        final float size = ty.f32(r, NveTables.Typography.FONT_SIZE);
                        fontSize = size > 0f && Float.isFinite(size) ? size : DEFAULT_FONT_SIZE;
                    }
                    if ((set & NveTables.Values.TypographySet.FONT_WEIGHT) != 0) fontWeight = ty.u32(r, NveTables.Typography.FONT_WEIGHT);
                    if ((set & NveTables.Values.TypographySet.FONT_STYLE) != 0) {
                        italic = ty.u32(r, NveTables.Typography.FONT_STYLE) == NveTables.Values.FontStyle.ITALIC;
                    }
                    if ((set & NveTables.Values.TypographySet.TEXT_ALIGN) != 0) align = ty.u32(r, NveTables.Typography.TEXT_ALIGN);
                    pending &= ~set;
                }
                final int parent = s.parent(i);
                i = parent == 0 ? 0 : s.resolve(parent);
            }
            return this;
        }
    }
}
