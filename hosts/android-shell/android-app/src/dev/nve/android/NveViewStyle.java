package dev.nve.android;

import android.graphics.drawable.GradientDrawable;
import android.os.Build;
import android.text.InputType;
import android.text.method.PasswordTransformationMethod;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.accessibility.AccessibilityNodeInfo;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.RadioButton;
import android.widget.Switch;
import android.widget.TextView;

import dev.nve.android.generated.NveTables;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.TableReader;

/** Applies the published standard tables of one node to its Android {@link View}. */
final class NveViewStyle {
    private NveViewStyle() {}

    /** Per-frame table lookups (by key, resolved once per frame). */
    static final class Tables {
        TableReader visual;
        TableReader typography;
        TableReader text;
        TableReader input;
        TableReader image;
        TableReader canvas;
        TableReader scroll;
        TableReader interaction;
        TableReader accessibility;
        TableReader debugId;

        void bind(RuntimeSceneReader s) {
            visual = s.findTable(NveTables.Visual.KEY);
            typography = s.findTable(NveTables.Typography.KEY);
            text = s.findTable(NveTables.Text.KEY);
            input = s.findTable(NveTables.TextInput.KEY);
            image = s.findTable(NveTables.Image.KEY);
            canvas = s.findTable(NveTables.Canvas.KEY);
            scroll = s.findTable(NveTables.Scroll.KEY);
            interaction = s.findTable(NveTables.Interaction.KEY);
            accessibility = s.findTable(NveTables.Accessibility.KEY);
            debugId = s.findTable(NveTables.DebugId.KEY);
        }
    }

    /** Inherited text style of a node (explicit `Typography` fields win, ancestors fill the rest). */
    static final class TextStyle {
        int color;
        float fontSize;
        int fontWeight;
        boolean italic;
        int align;
    }

    static int argb(int rgba) {
        return ((rgba >>> 24) << 24) | ((rgba & 0xFF) << 16) | (((rgba >>> 8) & 0xFF) << 8) | ((rgba >>> 16) & 0xFF);
    }

    /** Background, border, radius, opacity, z order, and visibility from `nve.visual`. */
    static void applyBox(View view, Tables t, RuntimeSceneReader s, int index, float density) {
        final int row = s.rowOf(t.visual, index);
        if (row < 0) {
            view.setBackground(null);
            view.setAlpha(1f);
            view.setTranslationZ(0f);
            view.setVisibility(View.VISIBLE);
            return;
        }
        final TableReader v = t.visual;
        final int bg = v.color(row, NveTables.Visual.BACKGROUND);
        final int border = v.color(row, NveTables.Visual.BORDER_COLOR);
        final float borderWidth = Math.max(0f, v.f32(row, NveTables.Visual.BORDER_WIDTH));
        final float radius = Math.max(0f, v.f32(row, NveTables.Visual.BORDER_RADIUS));
        if ((bg >>> 24) != 0 || ((border >>> 24) != 0 && borderWidth > 0f) || radius > 0f) {
            final GradientDrawable d = new GradientDrawable();
            d.setColor(argb(bg));
            d.setCornerRadius(radius * density);
            if ((border >>> 24) != 0 && borderWidth > 0f) d.setStroke(Math.max(1, Math.round(borderWidth * density)), argb(border));
            view.setBackground(d);
            view.setClipToOutline(radius > 0f);
        } else {
            view.setBackground(null);
            view.setClipToOutline(false);
        }
        view.setAlpha(Math.max(0f, Math.min(1f, v.f32(row, NveTables.Visual.OPACITY))));
        view.setTranslationZ(v.i32(row, NveTables.Visual.Z_INDEX) * density);
        view.setVisibility(v.u32(row, NveTables.Visual.VISIBILITY) == NveTables.Visual.VISIBILITY_HIDDEN ? View.INVISIBLE : View.VISIBLE);
    }

    /** Resolves `nve.typography` inheritance by walking ancestors (O(depth)). */
    static TextStyle textStyle(Tables t, RuntimeSceneReader s, int index, TextStyle out) {
        out.color = NveTables.Typography.DEFAULT_COLOR;
        out.fontSize = Float.intBitsToFloat(NveTables.Typography.DEFAULT_FONT_SIZE);
        out.fontWeight = NveTables.Typography.DEFAULT_FONT_WEIGHT;
        out.italic = false;
        out.align = NveTables.Typography.TEXT_ALIGN_START;
        // `Typography::SET_*` bits: color, font size, font weight, font style, text align.
        int pending = (1 << 0) | (1 << 1) | (1 << 6) | (1 << 7) | (1 << 11);
        for (int i = index; i > 0 && pending != 0 && t.typography != null; ) {
            final int row = s.rowOf(t.typography, i);
            if (row >= 0) {
                final TableReader ty = t.typography;
                final int set = ty.u32(row, NveTables.Typography.SET) & pending;
                if ((set & 1) != 0) out.color = ty.color(row, NveTables.Typography.COLOR);
                if ((set & 2) != 0) out.fontSize = ty.f32(row, NveTables.Typography.FONT_SIZE);
                if ((set & (1 << 6)) != 0) out.fontWeight = ty.u32(row, NveTables.Typography.FONT_WEIGHT);
                if ((set & (1 << 7)) != 0) out.italic = ty.u32(row, NveTables.Typography.FONT_STYLE) == NveTables.Typography.FONT_STYLE_ITALIC;
                if ((set & (1 << 11)) != 0) out.align = ty.u32(row, NveTables.Typography.TEXT_ALIGN);
                pending &= ~set;
            }
            final int parent = s.parent(i);
            i = parent == 0 ? 0 : s.resolve(parent);
        }
        return out;
    }

    static void applyText(TextView tv, TextStyle st) {
        tv.setTextColor(argb(st.color));
        tv.setTextSize(TypedValue.COMPLEX_UNIT_DIP, Float.isFinite(st.fontSize) && st.fontSize > 0f ? st.fontSize : 16f);
        tv.setTypeface(NveTextMeasurer.typeface(st.fontWeight, st.italic));
        final int h;
        switch (st.align) {
            case NveTables.Typography.TEXT_ALIGN_CENTER: h = Gravity.CENTER_HORIZONTAL; break;
            case NveTables.Typography.TEXT_ALIGN_RIGHT:
            case NveTables.Typography.TEXT_ALIGN_END: h = Gravity.END; break;
            default: h = Gravity.START; break;
        }
        tv.setGravity(Gravity.TOP | h);
    }

    /** Value, placeholder, secure / multiline input types, and read-only from `nve.text_input`. */
    static void applyInput(EditText edit, Tables t, RuntimeSceneReader s, int index) {
        final int row = s.rowOf(t.input, index);
        if (row < 0) return;
        final TableReader in = t.input;
        final String placeholder = s.text(in, row, NveTables.TextInput.PLACEHOLDER);
        if (!placeholder.contentEquals(edit.getHint() == null ? "" : edit.getHint())) edit.setHint(placeholder.isEmpty() ? null : placeholder);
        final boolean secure = in.bool(row, NveTables.TextInput.SECURE);
        final boolean multiline = in.bool(row, NveTables.TextInput.MULTILINE);
        final int kind = in.u32(row, NveTables.TextInput.INPUT_KIND);
        int type = InputType.TYPE_CLASS_TEXT;
        if (secure || kind == NveTables.TextInput.INPUT_KIND_PASSWORD) type |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
        else if (kind == NveTables.TextInput.INPUT_KIND_EMAIL) type |= InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS;
        else if (kind == NveTables.TextInput.INPUT_KIND_URL) type |= InputType.TYPE_TEXT_VARIATION_URI;
        else if (kind == NveTables.TextInput.INPUT_KIND_NUMBER) type = InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_FLAG_DECIMAL;
        else if (kind == NveTables.TextInput.INPUT_KIND_TEL) type = InputType.TYPE_CLASS_PHONE;
        if (multiline) type |= InputType.TYPE_TEXT_FLAG_MULTI_LINE;
        if (edit.getInputType() != type) {
            edit.setInputType(type);
            edit.setSingleLine(!multiline);
            edit.setTransformationMethod(secure ? PasswordTransformationMethod.getInstance() : null);
        }
        edit.setEnabled(!in.bool(row, NveTables.TextInput.READ_ONLY));
    }

    static ImageView.ScaleType scaleType(int fit) {
        switch (fit) {
            case NveTables.Image.FIT_CONTAIN: return ImageView.ScaleType.FIT_CENTER;
            case NveTables.Image.FIT_FILL: return ImageView.ScaleType.FIT_XY;
            case NveTables.Image.FIT_NONE: return ImageView.ScaleType.CENTER;
            case NveTables.Image.FIT_SCALE_DOWN: return ImageView.ScaleType.CENTER_INSIDE;
            default: return ImageView.ScaleType.CENTER_CROP;
        }
    }

    /** Label, hint, role, heading, hidden, checked / selected, disabled, and test id for TalkBack. */
    static void applyAccessibility(View view, Tables t, RuntimeSceneReader s, int index, boolean interactive) {
        final int row = s.rowOf(t.accessibility, index);
        final int idRow = s.rowOf(t.debugId, index);
        view.setTag(idRow < 0 ? null : s.text(t.debugId, idRow, NveTables.DebugId.TEST_ID));
        final int irow = s.rowOf(t.interaction, index);
        final boolean disabled = irow >= 0 && t.interaction.bool(irow, NveTables.Interaction.DISABLED);
        if (!(view instanceof EditText)) view.setEnabled(!disabled);
        if (row < 0) {
            view.setContentDescription(null);
            view.setAccessibilityDelegate(null);
            view.setImportantForAccessibility(interactive ? View.IMPORTANT_FOR_ACCESSIBILITY_YES : View.IMPORTANT_FOR_ACCESSIBILITY_AUTO);
            return;
        }
        final TableReader a = t.accessibility;
        final String label = s.text(a, row, NveTables.Accessibility.LABEL);
        final String hint = s.text(a, row, NveTables.Accessibility.HINT);
        final String valueText = s.text(a, row, NveTables.Accessibility.VALUE_TEXT);
        final int role = a.u32(row, NveTables.Accessibility.ROLE);
        final int checked = a.u32(row, NveTables.Accessibility.CHECKED);
        final boolean selected = a.u32(row, NveTables.Accessibility.SELECTED) == NveTables.Accessibility.SELECTED_TRUE;
        view.setContentDescription(label.isEmpty() ? null : label);
        view.setImportantForAccessibility(a.bool(row, NveTables.Accessibility.HIDDEN)
                ? View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS : View.IMPORTANT_FOR_ACCESSIBILITY_YES);
        view.setSelected(selected);
        view.setAccessibilityDelegate(new View.AccessibilityDelegate() {
            @Override
            public void onInitializeAccessibilityNodeInfo(View host, AccessibilityNodeInfo info) {
                super.onInitializeAccessibilityNodeInfo(host, info);
                final String cls = className(role);
                if (cls != null) info.setClassName(cls);
                info.setEnabled(!disabled);
                if (checked != NveTables.Accessibility.CHECKED_UNSET) {
                    info.setCheckable(true);
                    info.setChecked(checked == NveTables.Accessibility.CHECKED_TRUE);
                }
                if (Build.VERSION.SDK_INT >= 28 && role == NveTables.Accessibility.ROLE_HEADING) info.setHeading(true);
                if (Build.VERSION.SDK_INT >= 26 && !hint.isEmpty()) info.setHintText(hint);
                if (Build.VERSION.SDK_INT >= 30 && !valueText.isEmpty()) info.setStateDescription(valueText);
            }
        });
    }

    private static String className(int role) {
        switch (role) {
            case NveTables.Accessibility.ROLE_BUTTON:
            case NveTables.Accessibility.ROLE_LINK: return Button.class.getName();
            case NveTables.Accessibility.ROLE_CHECKBOX: return CheckBox.class.getName();
            case NveTables.Accessibility.ROLE_SWITCH: return Switch.class.getName();
            case NveTables.Accessibility.ROLE_RADIO: return RadioButton.class.getName();
            case NveTables.Accessibility.ROLE_IMAGE: return ImageView.class.getName();
            default: return null;
        }
    }
}
