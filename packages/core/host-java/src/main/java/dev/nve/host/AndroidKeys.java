package dev.nve.host;

/**
 * Android {@code KeyEvent} key codes (stable platform constants) as W3C UI Events
 * {@code KeyboardEvent.key} / {@code .code} strings, shared by every Android host (Views,
 * Compose) so apps see the same keys as on the web.
 */
public final class AndroidKeys {
    private AndroidKeys() {}

    // android.view.KeyEvent.KEYCODE_* values.
    static final int BACK = 4;
    static final int DIGIT_0 = 7;
    static final int DIGIT_9 = 16;
    static final int DPAD_UP = 19;
    static final int DPAD_DOWN = 20;
    static final int DPAD_LEFT = 21;
    static final int DPAD_RIGHT = 22;
    static final int LETTER_A = 29;
    static final int LETTER_Z = 54;
    static final int COMMA = 55;
    static final int PERIOD = 56;
    static final int ALT_LEFT = 57;
    static final int ALT_RIGHT = 58;
    static final int SHIFT_LEFT = 59;
    static final int SHIFT_RIGHT = 60;
    static final int TAB = 61;
    static final int SPACE = 62;
    static final int ENTER = 66;
    static final int DEL = 67;
    static final int MINUS = 69;
    static final int EQUALS = 70;
    static final int SLASH = 76;
    static final int PAGE_UP = 92;
    static final int PAGE_DOWN = 93;
    static final int ESCAPE = 111;
    static final int FORWARD_DEL = 112;
    static final int CTRL_LEFT = 113;
    static final int CTRL_RIGHT = 114;
    static final int CAPS_LOCK = 115;
    static final int META_LEFT = 117;
    static final int META_RIGHT = 118;
    static final int MOVE_HOME = 122;
    static final int MOVE_END = 123;
    static final int INSERT = 124;
    static final int F1 = 131;
    static final int F12 = 142;
    static final int NUMPAD_ENTER = 160;

    /**
     * W3C {@code key}: the produced character for printable keys ({@code unicodeChar} from
     * {@code KeyEvent.getUnicodeChar()}, already shifted), else the named key, else
     * {@code "Unidentified"}. Surrogate code points (never characters on their own) and the
     * {@code COMBINING_ACCENT} flag bit are not printable characters.
     */
    public static String key(int keyCode, int unicodeChar) {
        if (unicodeChar > 0x1F && unicodeChar != 0x7F && Character.isValidCodePoint(unicodeChar)
                && (unicodeChar < Character.MIN_SURROGATE || unicodeChar > Character.MAX_SURROGATE)) {
            return new String(Character.toChars(unicodeChar));
        }
        final String named = named(keyCode);
        return named != null ? named : "Unidentified";
    }

    private static String named(int keyCode) {
        switch (keyCode) {
            case ENTER:
            case NUMPAD_ENTER: return "Enter";
            case DEL: return "Backspace";
            case FORWARD_DEL: return "Delete";
            case TAB: return "Tab";
            case ESCAPE: return "Escape";
            case SPACE: return " ";
            case DPAD_UP: return "ArrowUp";
            case DPAD_DOWN: return "ArrowDown";
            case DPAD_LEFT: return "ArrowLeft";
            case DPAD_RIGHT: return "ArrowRight";
            case MOVE_HOME: return "Home";
            case MOVE_END: return "End";
            case PAGE_UP: return "PageUp";
            case PAGE_DOWN: return "PageDown";
            case INSERT: return "Insert";
            case SHIFT_LEFT:
            case SHIFT_RIGHT: return "Shift";
            case CTRL_LEFT:
            case CTRL_RIGHT: return "Control";
            case ALT_LEFT:
            case ALT_RIGHT: return "Alt";
            case META_LEFT:
            case META_RIGHT: return "Meta";
            case CAPS_LOCK: return "CapsLock";
            case BACK: return "GoBack";
            default:
                return keyCode >= F1 && keyCode <= F12 ? "F" + (keyCode - F1 + 1) : null;
        }
    }

    /** W3C {@code code} (physical key), {@code ""} when the key has no standard code. */
    public static String code(int keyCode) {
        if (keyCode >= LETTER_A && keyCode <= LETTER_Z) return "Key" + (char) ('A' + keyCode - LETTER_A);
        if (keyCode >= DIGIT_0 && keyCode <= DIGIT_9) return "Digit" + (keyCode - DIGIT_0);
        if (keyCode >= F1 && keyCode <= F12) return "F" + (keyCode - F1 + 1);
        switch (keyCode) {
            case ENTER: return "Enter";
            case NUMPAD_ENTER: return "NumpadEnter";
            case DEL: return "Backspace";
            case FORWARD_DEL: return "Delete";
            case TAB: return "Tab";
            case ESCAPE: return "Escape";
            case SPACE: return "Space";
            case DPAD_UP: return "ArrowUp";
            case DPAD_DOWN: return "ArrowDown";
            case DPAD_LEFT: return "ArrowLeft";
            case DPAD_RIGHT: return "ArrowRight";
            case MOVE_HOME: return "Home";
            case MOVE_END: return "End";
            case PAGE_UP: return "PageUp";
            case PAGE_DOWN: return "PageDown";
            case INSERT: return "Insert";
            case SHIFT_LEFT: return "ShiftLeft";
            case SHIFT_RIGHT: return "ShiftRight";
            case CTRL_LEFT: return "ControlLeft";
            case CTRL_RIGHT: return "ControlRight";
            case ALT_LEFT: return "AltLeft";
            case ALT_RIGHT: return "AltRight";
            case META_LEFT: return "MetaLeft";
            case META_RIGHT: return "MetaRight";
            case CAPS_LOCK: return "CapsLock";
            case COMMA: return "Comma";
            case PERIOD: return "Period";
            case MINUS: return "Minus";
            case EQUALS: return "Equal";
            case SLASH: return "Slash";
            default: return "";
        }
    }
}
