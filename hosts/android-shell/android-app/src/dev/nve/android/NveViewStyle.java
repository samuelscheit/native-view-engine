package dev.nve.android;

import android.graphics.drawable.GradientDrawable;
import android.os.Build;
import android.text.InputType;
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

import dev.nve.host.Rgba;
import dev.nve.host.RuntimeSceneReader;
import dev.nve.host.SceneSemantics;
import dev.nve.host.SceneSemantics.Tables;
import dev.nve.host.SceneSemantics.TextStyle;
import dev.nve.host.TableReader;
import dev.nve.host.generated.NveTables;

/**
 * Applies the published standard tables of one node to its Android {@link View}. Per-view
 * styling objects (background drawable, accessibility delegate) live in {@link ViewState} and are
 * reused across frames, so restyling a node allocates nothing in steady state.
 */
final class NveViewStyle {
    private NveViewStyle() {}

    /** Reusable per-view styling state. */
    static final class ViewState {
        GradientDrawable background;
        final A11y a11y = new A11y();
        /** Input type and read-only flag last applied to an {@link EditText}. */
        int inputType = -1;
        boolean readOnly;
    }

    /** Background, border, radius, opacity, z order, and visibility from `nve.visual`. */
    static void applyBox(View view, ViewState st, Tables t, RuntimeSceneReader s, int index, float density) {
        final int row = s.rowOf(t.visual, index);
        if (row < 0) {
            view.setBackground(null);
            view.setClipToOutline(false);
            view.setAlpha(1f);
            view.setTranslationZ(0f);
            view.setVisibility(View.VISIBLE);
            return;
        }
        final TableReader v = t.visual;
        final int bg = v.color(row, NveTables.Visual.BACKGROUND);
        final int border = v.color(row, NveTables.Visual.BORDER_COLOR);
        final float borderWidth = Math.max(0f, SceneSemantics.finite(v.f32(row, NveTables.Visual.BORDER_WIDTH), 0f));
        final float radius = Math.max(0f, SceneSemantics.finite(v.f32(row, NveTables.Visual.BORDER_RADIUS), 0f));
        final boolean stroke = Rgba.a(border) != 0 && borderWidth > 0f;
        if (Rgba.a(bg) != 0 || stroke || radius > 0f) {
            if (st.background == null) st.background = new GradientDrawable();
            final GradientDrawable d = st.background;
            d.setColor(Rgba.toArgb(bg));
            d.setCornerRadius(radius * density);
            d.setStroke(stroke ? Math.max(1, Math.round(borderWidth * density)) : 0, Rgba.toArgb(border));
            if (view.getBackground() != d) view.setBackground(d);
            view.setClipToOutline(radius > 0f);
        } else {
            view.setBackground(null);
            view.setClipToOutline(false);
        }
        view.setAlpha(Math.max(0f, Math.min(1f, SceneSemantics.finite(v.f32(row, NveTables.Visual.OPACITY), 1f))));
        view.setTranslationZ(v.i32(row, NveTables.Visual.Z_INDEX) * density);
        // INVISIBLE: keeps its space, takes no touches, and is skipped by accessibility.
        view.setVisibility(v.u32(row, NveTables.Visual.VISIBILITY) == NveTables.Values.Visibility.HIDDEN ? View.INVISIBLE : View.VISIBLE);
    }

    static void applyText(TextView tv, TextStyle st) {
        tv.setTextColor(Rgba.toArgb(st.color));
        tv.setTextSize(TypedValue.COMPLEX_UNIT_DIP, st.fontSize);  // sanitized by SceneSemantics
        tv.setTypeface(NveTextMeasurer.typeface(st.fontWeight, st.italic));
        final int h;
        switch (st.align) {
            case NveTables.Values.TextAlign.CENTER: h = Gravity.CENTER_HORIZONTAL; break;
            case NveTables.Values.TextAlign.RIGHT:
            case NveTables.Values.TextAlign.END: h = Gravity.END; break;
            default: h = Gravity.START; break;
        }
        tv.setGravity(Gravity.TOP | h);
    }

    /** Android input type of a `nve.text_input` row: kind, secure (password), multi-line. */
    static int inputType(boolean secure, boolean multiline, int kind) {
        int type;
        if (kind == NveTables.Values.InputKind.NUMBER) {
            type = InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_FLAG_DECIMAL;
        } else if (kind == NveTables.Values.InputKind.TEL) {
            type = InputType.TYPE_CLASS_PHONE;
        } else {
            type = InputType.TYPE_CLASS_TEXT;
            if (secure || kind == NveTables.Values.InputKind.PASSWORD) {
                type |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
            } else if (kind == NveTables.Values.InputKind.EMAIL) {
                type |= InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS;
            } else if (kind == NveTables.Values.InputKind.URL) {
                type |= InputType.TYPE_TEXT_VARIATION_URI;
            }
            if (multiline) type |= InputType.TYPE_TEXT_FLAG_MULTI_LINE;
        }
        if (secure && type == InputType.TYPE_CLASS_NUMBER) type |= InputType.TYPE_NUMBER_VARIATION_PASSWORD;
        return type;
    }

    /**
     * Placeholder, input type (secure / password, multi-line), read-only, and disabled
     * (`nve.interaction.disabled`) from `nve.text_input`. {@code setInputType} alone sets the
     * matching transformation (password dots) and single-line mode, so nothing re-applies them out
     * of order; read-only drops the key listener (re-created by {@code setInputType}).
     */
    static void applyInput(EditText edit, ViewState st, Tables t, RuntimeSceneReader s, int index) {
        final int row = s.rowOf(t.input, index);
        if (row < 0) return;
        final TableReader in = t.input;
        final String placeholder = s.text(in, row, NveTables.TextInput.PLACEHOLDER);
        final CharSequence hint = edit.getHint();
        if (!placeholder.contentEquals(hint == null ? "" : hint)) edit.setHint(placeholder.isEmpty() ? null : placeholder);
        final int type = inputType(in.bool(row, NveTables.TextInput.SECURE), in.bool(row, NveTables.TextInput.MULTILINE),
                in.u32(row, NveTables.TextInput.INPUT_KIND));
        final boolean readOnly = in.bool(row, NveTables.TextInput.READ_ONLY);
        if (st.inputType != type || st.readOnly != readOnly) {
            edit.setInputType(type);
            if (readOnly) edit.setKeyListener(null);
            st.inputType = type;
            st.readOnly = readOnly;
        }
        edit.setEnabled(!SceneSemantics.disabled(s, t, index));
    }

    static ImageView.ScaleType scaleType(int fit) {
        switch (fit) {
            case NveTables.Values.ImageFit.CONTAIN: return ImageView.ScaleType.FIT_CENTER;
            case NveTables.Values.ImageFit.FILL: return ImageView.ScaleType.FIT_XY;
            case NveTables.Values.ImageFit.NONE: return ImageView.ScaleType.CENTER;
            case NveTables.Values.ImageFit.SCALE_DOWN: return ImageView.ScaleType.CENTER_INSIDE;
            default: return ImageView.ScaleType.CENTER_CROP;
        }
    }

    /** TalkBack node info of one view, refreshed in place each frame. */
    static final class A11y extends View.AccessibilityDelegate {
        int role;
        int checked;
        boolean disabled;
        String hint = "";
        String valueText = "";

        @Override
        public void onInitializeAccessibilityNodeInfo(View host, AccessibilityNodeInfo info) {
            super.onInitializeAccessibilityNodeInfo(host, info);
            final String cls = className(role);
            if (cls != null) info.setClassName(cls);
            info.setEnabled(!disabled);
            if (checked != NveTables.Values.AriaTriState.UNSET) {
                info.setCheckable(true);
                setChecked(info, checked);
            }
            if (Build.VERSION.SDK_INT >= 28 && role == NveTables.Values.AriaRole.HEADING) info.setHeading(true);
            if (Build.VERSION.SDK_INT >= 26 && !hint.isEmpty()) info.setHintText(hint);
            if (Build.VERSION.SDK_INT >= 30 && !valueText.isEmpty()) info.setStateDescription(valueText);
        }
    }

    /** Tri-state `aria-checked` (API 36: `mixed` as a partial check; earlier: checked or not). */
    @SuppressWarnings("deprecation")
    private static void setChecked(AccessibilityNodeInfo info, int checked) {
        if (Build.VERSION.SDK_INT >= 36) {
            info.setChecked(checked == NveTables.Values.AriaTriState.TRUE ? AccessibilityNodeInfo.CHECKED_STATE_TRUE
                    : checked == NveTables.Values.AriaTriState.MIXED ? AccessibilityNodeInfo.CHECKED_STATE_PARTIAL
                    : AccessibilityNodeInfo.CHECKED_STATE_FALSE);
        } else {
            info.setChecked(checked == NveTables.Values.AriaTriState.TRUE);
        }
    }

    /** Label, hint, role, heading, hidden, checked / selected, disabled, and test id for TalkBack. */
    static void applyAccessibility(View view, ViewState st, Tables t, RuntimeSceneReader s, int index, boolean interactive) {
        final int row = s.rowOf(t.accessibility, index);
        final int idRow = s.rowOf(t.debugId, index);
        view.setTag(idRow < 0 ? null : s.text(t.debugId, idRow, NveTables.DebugId.TEST_ID));
        final boolean disabled = SceneSemantics.disabled(s, t, index);
        if (!(view instanceof EditText)) view.setEnabled(!disabled);  // inputs: `applyInput`
        if (row < 0) {
            view.setContentDescription(null);
            view.setAccessibilityDelegate(null);
            view.setImportantForAccessibility(interactive ? View.IMPORTANT_FOR_ACCESSIBILITY_YES : View.IMPORTANT_FOR_ACCESSIBILITY_AUTO);
            return;
        }
        final TableReader a = t.accessibility;
        final String label = s.text(a, row, NveTables.Accessibility.LABEL);
        final A11y d = st.a11y;
        d.hint = s.text(a, row, NveTables.Accessibility.HINT);
        d.valueText = s.text(a, row, NveTables.Accessibility.VALUE_TEXT);
        d.role = a.u32(row, NveTables.Accessibility.ROLE);
        d.checked = a.u32(row, NveTables.Accessibility.CHECKED);
        d.disabled = disabled;
        view.setContentDescription(label.isEmpty() ? null : label);
        view.setImportantForAccessibility(a.bool(row, NveTables.Accessibility.HIDDEN)
                ? View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS : View.IMPORTANT_FOR_ACCESSIBILITY_YES);
        view.setSelected(a.u32(row, NveTables.Accessibility.SELECTED) == NveTables.Values.AriaTriState.TRUE);
        view.setAccessibilityDelegate(d);
    }

    private static String className(int role) {
        switch (role) {
            case NveTables.Values.AriaRole.BUTTON:
            case NveTables.Values.AriaRole.LINK: return Button.class.getName();
            case NveTables.Values.AriaRole.CHECKBOX: return CheckBox.class.getName();
            case NveTables.Values.AriaRole.SWITCH: return Switch.class.getName();
            case NveTables.Values.AriaRole.RADIO: return RadioButton.class.getName();
            case NveTables.Values.AriaRole.IMAGE: return ImageView.class.getName();
            default: return null;
        }
    }
}
